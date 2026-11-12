//! Group changes presage doesn't offer, made through libsignal-service's
//! public push API: currently leaving a group.
//!
//! A group change is a protobuf `GroupChange.Actions` PATCHed to the storage
//! service, authenticated with a zero-knowledge group credential for the day.
//! The server signs the change; we then send it to the remaining members in
//! a group update message so their clients apply it too.

use presage::libsignal_service::configuration::{
    Endpoint, ServiceConfiguration, ServiceCredentials, SignalServers,
};
use presage::libsignal_service::prelude::ProtobufMessage;
use presage::libsignal_service::proto::{self, group_change};
use presage::libsignal_service::protocol::{Aci, Pni, ServiceId};
use presage::libsignal_service::push_service::{HttpAuth, HttpAuthOverride, PushService};
use presage::libsignal_service::utils::TryIntoE164 as _;
use presage::libsignal_service::zkgroup::{
    self, ServerPublicParams, Timestamp,
    auth::{AuthCredentialWithPni, AuthCredentialWithPniResponse},
    groups::{GroupMasterKey, GroupSecretParams},
};
use presage::manager::RegistrationData;
use serde::Deserialize;
use signal_core::GroupKey;
use uuid::Uuid;

const SECONDS_PER_DAY: u64 = 86_400;

/// The account credentials needed for group requests.
pub struct Account {
    servers: SignalServers,
    aci: Uuid,
    pni: Uuid,
    credentials: ServiceCredentials,
}

impl Account {
    pub fn from_registration(data: &RegistrationData) -> Result<Self, String> {
        // presage keeps the account password crate-private; it is part of the
        // serialized registration data it persists, so read it from there.
        let password = serde_json::to_value(data)
            .ok()
            .and_then(|v| v.get("password")?.as_str().map(str::to_owned))
            .ok_or("account password unavailable")?;
        let phonenumber = data
            .phone_number
            .clone()
            .try_into_e164()
            .map_err(|e| format!("invalid phone number: {e:?}"))?;
        Ok(Self {
            servers: data.signal_servers,
            aci: data.service_ids.aci,
            pni: data.service_ids.pni,
            credentials: ServiceCredentials {
                aci: Some(data.service_ids.aci),
                pni: Some(data.service_ids.pni),
                phonenumber,
                password: Some(password),
                device_id: data.device_id.and_then(|d| d.try_into().ok()),
            },
        })
    }
}

impl Account {
    /// An authenticated push service for this account.
    pub fn push_service(&self) -> PushService {
        PushService::new(
            self.servers,
            Some(self.credentials.clone()),
            crate::USER_AGENT,
        )
    }
}

/// A signed group change, ready to be sent to members.
pub struct SignedChange {
    pub revision: u32,
    pub bytes: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credentials {
    credentials: Vec<TemporalCredential>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemporalCredential {
    #[serde(with = "presage::libsignal_service::utils::serde_base64")]
    credential: Vec<u8>,
    redemption_time: u64,
}

/// Removes this account from the group on the server.
pub async fn leave(account: &Account, key: GroupKey) -> Result<SignedChange, String> {
    let service = PushService::new(
        account.servers,
        Some(account.credentials.clone()),
        crate::USER_AGENT,
    );
    let config: ServiceConfiguration = account.servers.into();
    let server_params = config.zkgroup_server_public_params;
    let secret_params = GroupSecretParams::derive_from_master_key(GroupMasterKey::new(key.0));

    let credential = todays_credential(&service).await?;
    let auth = || group_auth(account, &server_params, &secret_params, &credential);

    // the change must name the next revision (Signal's current clients use
    // the v2 endpoints, which wrap the group and the change in a response)
    let group: proto::GroupResponse = protobuf(
        service
            .request(
                reqwest::Method::GET,
                Endpoint::storage("/v2/groups/"),
                HttpAuthOverride::Identified(auth()?),
            )
            .map_err(|e| e.to_string())?
            .send()
            .await,
    )
    .await?;
    let revision = group
        .group
        .ok_or("the server didn't return the group")?
        .version;

    let actions = leave_actions(&secret_params, account.aci, revision + 1);
    let response: proto::GroupChangeResponse = protobuf(
        service
            .request(
                reqwest::Method::PATCH,
                Endpoint::storage("/v2/groups/"),
                HttpAuthOverride::Identified(auth()?),
            )
            .map_err(|e| e.to_string())?
            .header("Content-Type", "application/x-protobuf")
            .body(actions.encode_to_vec())
            .send()
            .await,
    )
    .await?;
    let change = response
        .group_change
        .ok_or("the server didn't return the group change")?;
    Ok(SignedChange {
        revision: actions.version,
        bytes: change.encode_to_vec(),
    })
}

/// The `Actions` removing `me` from the group at `revision`.
pub fn leave_actions(
    secret_params: &GroupSecretParams,
    me: Uuid,
    revision: u32,
) -> group_change::Actions {
    let me = zkgroup::serialize(&secret_params.encrypt_service_id(ServiceId::Aci(Aci::from(me))));
    group_change::Actions {
        group_id: secret_params.get_group_identifier().to_vec(),
        version: revision,
        delete_members: vec![group_change::actions::DeleteMemberAction {
            deleted_user_id: me,
        }],
        ..Default::default()
    }
}

async fn todays_credential(
    service: &PushService,
) -> Result<(u64, AuthCredentialWithPniResponse), String> {
    let today = today_seconds();
    let path = format!(
        "/v1/certificate/auth/group?redemptionStartSeconds={today}&redemptionEndSeconds={}&pniAsServiceId=true",
        today + 7 * SECONDS_PER_DAY
    );
    let response = service
        .request(
            reqwest::Method::GET,
            Endpoint::service(path),
            HttpAuthOverride::NoOverride,
        )
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("group credentials request failed ({status})"));
    }
    let parsed: Credentials = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let todays = parsed
        .credentials
        .into_iter()
        .find(|c| c.redemption_time == today)
        .ok_or("no group credential for today")?;
    let response =
        AuthCredentialWithPniResponse::new(&todays.credential).map_err(|e| e.to_string())?;
    Ok((today, response))
}

/// Basic-auth for a group request: the group's public params and a fresh
/// presentation of today's credential.
fn group_auth(
    account: &Account,
    server_params: &ServerPublicParams,
    secret_params: &GroupSecretParams,
    (today, response): &(u64, AuthCredentialWithPniResponse),
) -> Result<HttpAuth, String> {
    let credential: AuthCredentialWithPni = response
        .clone()
        .receive(
            server_params,
            Aci::from(account.aci),
            Pni::from(account.pni),
            Timestamp::from_epoch_seconds(*today),
        )
        .map_err(|e| format!("invalid group credential: {e}"))?;
    let randomness: [u8; 32] = rand::random();
    let presentation = credential.present(server_params, secret_params, randomness);
    Ok(HttpAuth {
        username: hex::encode(zkgroup::serialize(&secret_params.get_public_params())),
        password: hex::encode(zkgroup::serialize(&presentation)),
    })
}

fn today_seconds() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now - now % SECONDS_PER_DAY
}

async fn protobuf<T: ProtobufMessage + Default>(
    response: Result<reqwest::Response, reqwest::Error>,
) -> Result<T, String> {
    let response = response.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    match status {
        200..=299 => T::decode(body.as_ref()).map_err(|e| e.to_string()),
        403 => Err("you aren't allowed to make this change (are you still a member?)".into()),
        409 => Err("the group changed meanwhile; try again".into()),
        s => Err(format!("group request failed (HTTP {s})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presage::libsignal_service::groups_v2::GroupOperations;

    #[test]
    fn leave_actions_remove_only_me() {
        let me = Uuid::from_u128(0xAAAA);
        let params = GroupSecretParams::derive_from_master_key(GroupMasterKey::new([9; 32]));
        let actions = leave_actions(&params, me, 6);
        assert_eq!(actions.version, 6);
        assert_eq!(actions.group_id, params.get_group_identifier().to_vec());
        assert_eq!(actions.delete_members.len(), 1);
        assert!(actions.add_members.is_empty() && actions.modify_title.is_none());

        // the encrypted id decrypts back to us with the group's keys; wrap the
        // actions as a (server-signed) change and decode it like a client would
        let change = proto::GroupChange {
            actions: group_change::Actions {
                source_user_id: actions.delete_members[0].deleted_user_id.clone(),
                ..actions
            }
            .encode_to_vec(),
            ..Default::default()
        };
        let decoded = GroupOperations::new(params)
            .decrypt_group_change(change)
            .unwrap();
        let editor: Uuid = decoded.editor.into();
        assert_eq!(editor, me);
        assert_eq!(decoded.version, 6);
    }

    #[test]
    fn credentials_day_boundary() {
        assert_eq!(today_seconds() % SECONDS_PER_DAY, 0);
    }
}

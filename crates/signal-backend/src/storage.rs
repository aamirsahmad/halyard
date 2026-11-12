//! Reads chat state the phone keeps in Signal's storage service: whether a
//! chat was accepted ("whitelisted") and whether it is blocked. This is what
//! decides message requests in Signal's own clients.

use presage::libsignal_service::StorageService;
use presage::libsignal_service::master_key::{MasterKey, StorageServiceKey};
use presage::libsignal_service::proto::{manifest_record, storage_record};
use presage::libsignal_service::push_service::PushService;
use signal_core::{ConversationId, GroupKey, RemoteChatState};

const READ_CHUNK: usize = 100;

/// Accepted/blocked state of every contact and group listed in storage.
pub async fn chat_states(
    service: PushService,
    master_key: &MasterKey,
) -> Result<Vec<RemoteChatState>, String> {
    let storage = StorageService::new(service, StorageServiceKey::from_master_key(master_key))
        .await
        .map_err(|e| e.to_string())?;
    let manifest = storage.manifest().await.map_err(|e| e.to_string())?;
    let keys: Vec<Vec<u8>> = manifest
        .identifiers
        .iter()
        .filter(|id| {
            matches!(
                id.r#type(),
                manifest_record::identifier::Type::Contact
                    | manifest_record::identifier::Type::Groupv2
            )
        })
        .map(|id| id.raw.clone())
        .collect();
    let mut states = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(READ_CHUNK) {
        let records = storage
            .read_items(chunk.to_vec(), Some(&manifest.record_ikm))
            .await
            .map_err(|e| e.to_string())?;
        states.extend(records.iter().filter_map(|r| state_of(r.record.as_ref()?)));
    }
    Ok(states)
}

/// The chat state a storage record describes, if it is a contact or group.
pub fn state_of(record: &storage_record::Record) -> Option<RemoteChatState> {
    match record {
        storage_record::Record::Contact(c) => {
            let uuid = crate::interpret::parse_aci(
                Some(c.aci.as_str()).filter(|a| !a.is_empty()),
                Some(c.aci_binary.as_slice()).filter(|b| !b.is_empty()),
            )?;
            Some(RemoteChatState {
                conversation: ConversationId::Direct(uuid),
                accepted: c.whitelisted,
                blocked: c.blocked,
            })
        }
        storage_record::Record::GroupV2(g) => {
            let key: [u8; 32] = g.master_key.as_slice().try_into().ok()?;
            Some(RemoteChatState {
                conversation: ConversationId::Group(GroupKey(key)),
                accepted: g.whitelisted,
                blocked: g.blocked,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use presage::libsignal_service::proto::{ContactRecord, GroupV2Record};
    use uuid::Uuid;

    #[test]
    fn contact_records_by_binary_or_string_aci() {
        let u = Uuid::from_u128(0xA11CE);
        let by_binary = storage_record::Record::Contact(ContactRecord {
            aci_binary: u.as_bytes().to_vec(),
            whitelisted: true,
            ..Default::default()
        });
        assert_eq!(
            state_of(&by_binary),
            Some(RemoteChatState {
                conversation: ConversationId::Direct(u),
                accepted: true,
                blocked: false,
            })
        );
        let by_string = storage_record::Record::Contact(ContactRecord {
            aci: u.to_string(),
            blocked: true,
            ..Default::default()
        });
        let st = state_of(&by_string).unwrap();
        assert!(st.blocked && !st.accepted);
        // a contact without an ACI (phone number only) is skipped
        let none = storage_record::Record::Contact(ContactRecord {
            e164: "+15550100".into(),
            ..Default::default()
        });
        assert_eq!(state_of(&none), None);
    }

    #[test]
    fn group_records() {
        let g = storage_record::Record::GroupV2(GroupV2Record {
            master_key: vec![5; 32],
            whitelisted: true,
            ..Default::default()
        });
        assert_eq!(
            state_of(&g).unwrap().conversation,
            ConversationId::Group(GroupKey([5; 32]))
        );
        let bad = storage_record::Record::GroupV2(GroupV2Record {
            master_key: vec![1, 2],
            ..Default::default()
        });
        assert_eq!(state_of(&bad), None);
    }
}

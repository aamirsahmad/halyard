//! Stable identifiers for conversations.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A 32 byte Signal group (v2) master key. It doubles as the group identifier
/// on the client side, exactly like Signal Desktop does with `groupId`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupKey(pub [u8; 32]);

impl fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // never log a full master key: it grants access to the group
        write!(f, "GroupKey({}…)", &hex::encode(self.0)[..8])
    }
}

/// Identifies a conversation (thread): either a 1:1 chat with an account
/// (identified by its ACI) or a v2 group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConversationId {
    Direct(Uuid),
    Group(GroupKey),
}

impl ConversationId {
    pub fn is_group(&self) -> bool {
        matches!(self, ConversationId::Group(_))
    }

    pub fn direct_uuid(&self) -> Option<Uuid> {
        match self {
            ConversationId::Direct(u) => Some(*u),
            ConversationId::Group(_) => None,
        }
    }

    pub fn group_key(&self) -> Option<&GroupKey> {
        match self {
            ConversationId::Group(k) => Some(k),
            ConversationId::Direct(_) => None,
        }
    }

    /// Bytes used to derive a stable avatar colour.
    pub fn color_seed(&self) -> Vec<u8> {
        match self {
            ConversationId::Direct(u) => u.as_bytes().to_vec(),
            ConversationId::Group(k) => k.0.to_vec(),
        }
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConversationId::Direct(u) => write!(f, "direct:{u}"),
            ConversationId::Group(k) => write!(f, "group:{}", hex::encode(k.0)),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseIdError {
    #[error("unknown conversation id prefix")]
    UnknownPrefix,
    #[error("invalid uuid")]
    InvalidUuid,
    #[error("invalid group key")]
    InvalidGroupKey,
}

impl FromStr for ConversationId {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix("direct:") {
            let uuid = Uuid::parse_str(rest).map_err(|_| ParseIdError::InvalidUuid)?;
            Ok(ConversationId::Direct(uuid))
        } else if let Some(rest) = s.strip_prefix("group:") {
            let bytes = hex::decode(rest).map_err(|_| ParseIdError::InvalidGroupKey)?;
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| ParseIdError::InvalidGroupKey)?;
            Ok(ConversationId::Group(GroupKey(key)))
        } else {
            Err(ParseIdError::UnknownPrefix)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_roundtrip() {
        let id = ConversationId::Direct(Uuid::new_v4());
        let s = id.to_string();
        assert!(s.starts_with("direct:"));
        assert_eq!(s.parse::<ConversationId>().unwrap(), id);
    }

    #[test]
    fn group_roundtrip() {
        let id = ConversationId::Group(GroupKey([7u8; 32]));
        let s = id.to_string();
        assert_eq!(s.len(), "group:".len() + 64);
        assert_eq!(s.parse::<ConversationId>().unwrap(), id);
    }

    #[test]
    fn parse_errors() {
        assert_eq!(
            "foo:bar".parse::<ConversationId>(),
            Err(ParseIdError::UnknownPrefix)
        );
        assert_eq!(
            "direct:nope".parse::<ConversationId>(),
            Err(ParseIdError::InvalidUuid)
        );
        assert_eq!(
            "group:abcd".parse::<ConversationId>(),
            Err(ParseIdError::InvalidGroupKey)
        );
        assert_eq!(
            "group:zz".parse::<ConversationId>(),
            Err(ParseIdError::InvalidGroupKey)
        );
    }

    #[test]
    fn debug_does_not_leak_group_key() {
        let k = GroupKey([0xab; 32]);
        let dbg = format!("{k:?}");
        assert!(dbg.len() < 24, "{dbg}");
    }
}

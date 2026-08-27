use crate::config::Platform;
use serde::{Deserialize, Serialize};

/// First version of InkRiver's device synchronization event protocol.
pub const SYNC_PROTOCOL_VERSION: i64 = 1;

/// Persistent hybrid logical time used to order concurrent events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridLogicalClock {
    pub physical_milliseconds: i64,
    pub logical_counter: i64,
}

/// Stable identity and current journal allocation state of this installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncIdentity {
    pub device_id: String,
    pub next_sequence: i64,
    pub clock: HybridLogicalClock,
    pub is_enabled: bool,
}

/// Stable reference to one event in an immutable per-device journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEventId {
    pub device_id: String,
    pub sequence: i64,
}

/// Minimal article identity and display metadata carried by state events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncArticleRef {
    pub subscription_id: String,
    pub entry_key: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub author: Option<String>,
    pub published_at: Option<String>,
}

/// Version-one business events exchanged by InkRiver installations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncEventPayload {
    SubscriptionCreated {
        subscription_id: String,
        normalized_url: String,
        platform_hint: Platform,
        is_active: bool,
        parent_tombstone: Option<SyncEventId>,
    },
    SubscriptionActiveSet {
        subscription_id: String,
        is_active: bool,
    },
    SubscriptionPlatformSet {
        subscription_id: String,
        platform_hint: Platform,
    },
    SubscriptionDeleted {
        subscription_id: String,
    },
    ArticleReadSet {
        article: SyncArticleRef,
        is_read: bool,
    },
    ArticleFavoriteSet {
        article: SyncArticleRef,
        is_favorite: bool,
    },
    ArticleArchived {
        article: SyncArticleRef,
    },
}

impl SyncEventPayload {
    /// Returns the stable discriminator duplicated in the SQLite journal index.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::SubscriptionCreated { .. } => "subscription_created",
            Self::SubscriptionActiveSet { .. } => "subscription_active_set",
            Self::SubscriptionPlatformSet { .. } => "subscription_platform_set",
            Self::SubscriptionDeleted { .. } => "subscription_deleted",
            Self::ArticleReadSet { .. } => "article_read_set",
            Self::ArticleFavoriteSet { .. } => "article_favorite_set",
            Self::ArticleArchived { .. } => "article_archived",
        }
    }
}

/// Immutable event envelope persisted in the local synchronization journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncEvent {
    pub device_id: String,
    pub sequence: i64,
    pub clock: HybridLogicalClock,
    pub protocol_version: i64,
    pub kind: String,
    pub payload: SyncEventPayload,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_payload_round_trips_with_stable_kind() {
        let payload = SyncEventPayload::SubscriptionCreated {
            subscription_id: "feed-id".to_string(),
            normalized_url: "https://example.com/feed".to_string(),
            platform_hint: Platform::Other,
            is_active: true,
            parent_tombstone: Some(SyncEventId {
                device_id: "device-id".to_string(),
                sequence: 4,
            }),
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains(r#""kind":"subscription_created""#));
        assert_eq!(
            serde_json::from_str::<SyncEventPayload>(&json).unwrap(),
            payload
        );
        assert_eq!(payload.kind(), "subscription_created");
    }
}

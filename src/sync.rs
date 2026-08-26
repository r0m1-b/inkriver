use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

/// Immutable event envelope persisted in the local synchronization journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncEvent {
    pub device_id: String,
    pub sequence: i64,
    pub clock: HybridLogicalClock,
    pub protocol_version: i64,
    pub kind: String,
    pub payload: Value,
}

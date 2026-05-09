use std::fmt;

use serde::{Deserialize, Serialize};

pub const META_WATERMARK_CHECKPOINT: &str = "watermark:checkpoint";
pub const META_DATASET_NAME: &str = "dataset:name";
pub const META_DATASET_NETWORK: &str = "dataset:network";
pub const META_DATASET_RANGE_START: &str = "dataset:range_start";
pub const META_DATASET_RANGE_END: &str = "dataset:range_end";
pub const META_BACKEND_NAME: &str = "backend:name";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ColumnFamily {
    Meta,
    Checkpoint,
    TxByDigest,
    ObjectVersion,
    ObjectLastSeen,
    EventByType,
    OwnerTouchedObjects,
}

impl ColumnFamily {
    pub const ALL: [Self; 7] = [
        Self::Meta,
        Self::Checkpoint,
        Self::TxByDigest,
        Self::ObjectVersion,
        Self::ObjectLastSeen,
        Self::EventByType,
        Self::OwnerTouchedObjects,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Meta => "cf_meta",
            Self::Checkpoint => "cf_checkpoint",
            Self::TxByDigest => "cf_tx_by_digest",
            Self::ObjectVersion => "cf_object_version",
            Self::ObjectLastSeen => "cf_object_last_seen",
            Self::EventByType => "cf_event_by_type",
            Self::OwnerTouchedObjects => "cf_owner_touched_objects",
        }
    }
}

impl fmt::Display for ColumnFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

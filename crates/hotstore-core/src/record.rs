use anyhow::Context;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub network: String,
    pub sequence_number: u64,
    pub timestamp_ms: u64,
    pub tx_count: u32,
    pub event_count: u32,
    pub object_change_count: u32,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxRecord {
    pub digest: Vec<u8>,
    pub checkpoint: u64,
    pub tx_index: u32,
    pub sender: Option<Vec<u8>>,
    pub status: String,
    pub gas_used: Option<u64>,
    pub event_count: u32,
    pub changed_object_count: u32,
    pub raw_effects_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub event_type: String,
    pub checkpoint: u64,
    pub tx_digest: Vec<u8>,
    pub sender: Option<Vec<u8>>,
    pub package_id: Option<String>,
    pub module: Option<String>,
    pub event_name: Option<String>,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRecord {
    pub object_id: Vec<u8>,
    pub version: u64,
    pub checkpoint: u64,
    pub owner: Option<Vec<u8>>,
    pub type_tag: Option<String>,
    pub raw_object_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerTouchedObjectRecord {
    pub owner: Vec<u8>,
    pub object_id: Vec<u8>,
    pub version: u64,
    pub checkpoint: u64,
    pub type_tag: Option<String>,
}

pub fn serialize_record<T>(value: &T) -> anyhow::Result<Vec<u8>>
where
    T: Serialize,
{
    bincode::serialize(value).context("failed to serialize hotstore record")
}

pub fn deserialize_record<T>(bytes: &[u8]) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    bincode::deserialize(bytes).context("failed to deserialize hotstore record")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trip_uses_stable_binary_encoding() {
        let record = ObjectRecord {
            object_id: vec![0xAA; 32],
            version: 7,
            checkpoint: 42,
            owner: Some(vec![0xBB; 32]),
            type_tag: Some("0x2::coin::Coin<SUI>".to_owned()),
            raw_object_bytes: Some(vec![1, 2, 3, 4]),
        };

        let bytes = serialize_record(&record).expect("object record serializes");
        let decoded: ObjectRecord = deserialize_record(&bytes).expect("object record deserializes");

        assert_eq!(decoded, record);
    }
}

use sha2::{Digest, Sha256};

pub const HASH16_LEN: usize = 16;

pub fn encode_u64_be(x: u64) -> [u8; 8] {
    x.to_be_bytes()
}

pub fn encode_u32_be(x: u32) -> [u8; 4] {
    x.to_be_bytes()
}

pub fn key_checkpoint(seq: u64) -> Vec<u8> {
    encode_u64_be(seq).to_vec()
}

pub fn key_tx_by_digest(digest: &[u8]) -> Vec<u8> {
    digest.to_vec()
}

pub fn key_object_version(object_id: &[u8], version: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(object_id.len() + 8);
    key.extend_from_slice(object_id);
    key.extend_from_slice(&encode_u64_be(version));
    key
}

pub fn key_object_last_seen(object_id: &[u8]) -> Vec<u8> {
    object_id.to_vec()
}

pub fn event_type_hash(event_type: &str) -> [u8; HASH16_LEN] {
    hash16(event_type.as_bytes())
}

pub fn type_tag_hash(type_tag: Option<&str>) -> [u8; HASH16_LEN] {
    match type_tag {
        Some(type_tag) => hash16(type_tag.as_bytes()),
        None => [0; HASH16_LEN],
    }
}

pub fn key_event_by_type(
    event_type: &str,
    checkpoint: u64,
    tx_index: u32,
    event_index: u32,
) -> Vec<u8> {
    let mut key = Vec::with_capacity(HASH16_LEN + 8 + 4 + 4);
    key.extend_from_slice(&event_type_hash(event_type));
    key.extend_from_slice(&encode_u64_be(checkpoint));
    key.extend_from_slice(&encode_u32_be(tx_index));
    key.extend_from_slice(&encode_u32_be(event_index));
    key
}

pub fn key_owner_touched_object(
    owner: &[u8],
    type_tag: Option<&str>,
    object_id: &[u8],
    version: u64,
) -> Vec<u8> {
    let mut key = Vec::with_capacity(owner.len() + HASH16_LEN + object_id.len() + 8);
    key.extend_from_slice(owner);
    key.extend_from_slice(&type_tag_hash(type_tag));
    key.extend_from_slice(object_id);
    key.extend_from_slice(&encode_u64_be(version));
    key
}

pub fn key_owner_touched_objects_prefix(owner: &[u8], type_tag: Option<&str>) -> Vec<u8> {
    let mut key = Vec::with_capacity(owner.len() + type_tag.map(|_| HASH16_LEN).unwrap_or(0));
    key.extend_from_slice(owner);
    if let Some(type_tag) = type_tag {
        key.extend_from_slice(&type_tag_hash(Some(type_tag)));
    }
    key
}

fn hash16(input: &[u8]) -> [u8; HASH16_LEN] {
    let digest = Sha256::digest(input);
    let mut out = [0; HASH16_LEN];
    out.copy_from_slice(&digest[..HASH16_LEN]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_big_endian_encoding_matches_expected_bytes() {
        assert_eq!(
            encode_u64_be(0x0102_0304_0506_0708),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(encode_u32_be(0x0102_0304), [1, 2, 3, 4]);
    }

    #[test]
    fn checkpoint_keys_sort_in_sequence_order() {
        let mut keys = [key_checkpoint(10), key_checkpoint(2), key_checkpoint(9)];
        keys.sort();

        assert_eq!(
            keys,
            [key_checkpoint(2), key_checkpoint(9), key_checkpoint(10)]
        );
    }

    #[test]
    fn object_version_keys_sort_versions_lexicographically() {
        let object_id = [0xAB; 32];
        let mut keys = [
            key_object_version(&object_id, 15),
            key_object_version(&object_id, 1),
            key_object_version(&object_id, 9),
        ];
        keys.sort();

        assert_eq!(
            keys,
            [
                key_object_version(&object_id, 1),
                key_object_version(&object_id, 9),
                key_object_version(&object_id, 15),
            ]
        );
    }

    #[test]
    fn event_type_hash_is_deterministic() {
        assert_eq!(
            event_type_hash("deepbook::order::Fill"),
            [
                0x92, 0xF2, 0xDE, 0x6B, 0x28, 0x85, 0x45, 0x80, 0x98, 0xF1, 0xFA, 0xB9, 0xCF, 0xD6,
                0x19, 0x55,
            ]
        );
    }

    #[test]
    fn event_keys_sort_by_checkpoint_then_tx_then_event_index() {
        let mut keys = [
            key_event_by_type("deepbook::order::Fill", 100, 7, 1),
            key_event_by_type("deepbook::order::Fill", 100, 2, 3),
            key_event_by_type("deepbook::order::Fill", 99, 9, 9),
        ];
        keys.sort();

        assert_eq!(
            keys,
            [
                key_event_by_type("deepbook::order::Fill", 99, 9, 9),
                key_event_by_type("deepbook::order::Fill", 100, 2, 3),
                key_event_by_type("deepbook::order::Fill", 100, 7, 1),
            ]
        );
    }

    #[test]
    fn owner_touched_object_keys_are_deterministic() {
        let owner = [0x11; 32];
        let object_id = [0x22; 32];

        let key = key_owner_touched_object(&owner, Some("0x2::coin::Coin<SUI>"), &object_id, 5);

        assert_eq!(key.len(), 32 + HASH16_LEN + 32 + 8);
        assert_eq!(&key[..32], &owner);
        assert_eq!(
            &key[32..48],
            &[
                0x3A, 0x5D, 0xC3, 0x0F, 0x3F, 0xB6, 0x33, 0xA5, 0xBB, 0x20, 0x2C, 0xDC, 0x78, 0x3E,
                0xA3, 0x4C,
            ]
        );
        assert_eq!(&key[48..80], &object_id);
        assert_eq!(&key[80..], &encode_u64_be(5));
    }

    #[test]
    fn owner_prefix_without_type_tag_stays_on_owner_boundary() {
        let owner = [0x33; 32];

        assert_eq!(
            key_owner_touched_objects_prefix(&owner, None),
            owner.to_vec()
        );
        assert_eq!(
            key_owner_touched_objects_prefix(&owner, Some("wallet::position::Position")).len(),
            32 + HASH16_LEN
        );
    }
}

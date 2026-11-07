//! Builders for the two ops Charter sends: create (flags 3) and extend (design 2.1, 5).

use alloy_primitives::{Address, B128, B256, U256, keccak256};
use arkiv_bindings::{Attribute, Ident32, Operation};

pub use arkiv_bindings::AttributeValue;

use crate::{ARKIV_ADDRESS, StoreError};

/// `readonly | permissionlessExtension`, set on every Charter entity.
pub const ENTITY_FLAGS: u8 = 3;

/// 128 random bits for a create's salt.
pub fn random_salt() -> u128 {
    u128::from_be_bytes(B128::random().0)
}

/// A create with relative lifetime `min_lifetime` blocks; attributes are `(name, value,
/// indexed)` and are sorted for the wire.
pub fn create_op(
    attrs: &[(&str, AttributeValue, bool)],
    min_lifetime: u64,
    salt: u128,
) -> Result<Operation, StoreError> {
    let wire = attrs
        .iter()
        .map(|(name, value, indexed)| {
            let id =
                Ident32::encode(name).map_err(|e| StoreError::InvalidOp(format!("{name}: {e}")))?;
            Attribute::from_value(id, value, *indexed)
                .map_err(|e| StoreError::InvalidOp(format!("{name}: {e}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Operation::create(salt, 0, min_lifetime, ENTITY_FLAGS, wire))
}

/// Extend to `min_lifetime` blocks from the landing block.
pub fn extend_op(key: B256, min_lifetime: u64) -> Operation {
    Operation::extend_expiry(key, 0, min_lifetime)
}

/// `keccak256(chain_id || ARKIV_ADDRESS || owner || nonce || salt)`, as the node mints keys.
pub fn derive_key(chain_id: u64, owner: Address, nonce: u64, salt: u128) -> B256 {
    let mut buf = Vec::with_capacity(32 + 20 + 20 + 8 + 16);
    buf.extend_from_slice(&U256::from(chain_id).to_be_bytes::<32>());
    buf.extend_from_slice(ARKIV_ADDRESS.as_slice());
    buf.extend_from_slice(owner.as_slice());
    buf.extend_from_slice(&nonce.to_be_bytes());
    buf.extend_from_slice(&salt.to_be_bytes());
    keccak256(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkiv_bindings::Create;

    #[test]
    fn create_sorts_attributes_and_sets_flags() {
        let op = create_op(
            &[
                ("ag.v", AttributeValue::U64(1), false),
                ("ag.kind", AttributeValue::Str("chunk".into()), true),
            ],
            100,
            random_salt(),
        )
        .expect("valid");
        let c: Create = op.payload_of().expect("decodes");
        assert_eq!((c.creationFlags, c.minLifetime, c.expiresAt), (3, 100, 0));
        assert_eq!(
            c.attributes[0].name,
            Ident32::encode("ag.kind").expect("ok").into_word()
        );
        assert!(c.attributes[0].is_indexed() && !c.attributes[1].is_indexed());
    }

    #[test]
    fn create_rejects_bad_names_and_indexed_bytes() {
        let bad_name = create_op(&[("$key", AttributeValue::U64(1), true)], 1, 0);
        assert_eq!(bad_name.map_err(|e| e.code()), Err("E_INTERNAL"));
        let indexed_bytes = create_op(&[("ag.body", AttributeValue::Bytes(vec![1]), true)], 1, 0);
        assert!(indexed_bytes.is_err());
        assert_ne!(random_salt(), random_salt());
    }
}

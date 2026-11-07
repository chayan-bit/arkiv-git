//! `cob`: one signed op of a collaborative object (docs/p3/cob-ops.md).

use super::{Body, cbor_codec, require};
use crate::attrs::Attribute;
use crate::types::{Hash32, Nid, Rid};
use crate::{FormatError, Kind, ZERO, field};

/// Longest `type_name`, e.g. `xyz.arkiv.patch`.
pub const MAX_TYPE_NAME: usize = 64;
/// Longest `action`.
pub const MAX_ACTION: usize = 32;
/// Largest payload (the 64 KiB entity cap usually binds first).
pub const MAX_PAYLOAD: usize = 65_536;
/// The action of the op that creates an object.
pub const CREATE: &str = "create";

/// One op. Field keys follow the P1 reservation (design 2.2): rid, type, object, parents.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct CobOpBody {
    #[cbor(n(1), with = "minicbor::bytes")]
    pub rid: Rid,
    /// `[a-z0-9._-]`, 1..64 B.
    #[n(2)]
    pub type_name: String,
    /// Object id (the create op's id); zero iff `action = "create"`.
    #[cbor(n(3), with = "minicbor::bytes")]
    pub object: Hash32,
    /// Ids of ops of the same object this op saw; may be empty.
    #[cbor(n(4), with = "crate::cbor::hashes")]
    pub parents: Vec<Hash32>,
    /// `[a-z0-9._-]`, 1..32 B.
    #[n(5)]
    pub action: String,
    /// One canonical CBOR value, no floats or tags, at most 64 KiB.
    #[cbor(n(6), with = "minicbor::bytes")]
    pub payload: Vec<u8>,
    /// Author's clock, display only: ordering comes from Arkiv.
    #[n(7)]
    pub ts_ms: u64,
}

impl CobOpBody {
    pub fn is_create(&self) -> bool {
        self.action == CREATE
    }

    /// The object this op belongs to: `h` itself for a create op.
    pub fn object_id(&self, h: &Hash32) -> Hash32 {
        if self.is_create() { *h } else { self.object }
    }
}

fn name_ok(s: &str, max: usize) -> bool {
    (1..=max).contains(&s.len())
        && s.bytes()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

impl Body for CobOpBody {
    const KIND: Kind = Kind::Cob;
    cbor_codec!();

    fn validate(&self) -> Result<(), FormatError> {
        require(
            name_ok(&self.type_name, MAX_TYPE_NAME),
            field("cob", "type_name", "1..64 B of [a-z0-9._-]"),
        )?;
        require(
            name_ok(&self.action, MAX_ACTION),
            field("cob", "action", "1..32 B of [a-z0-9._-]"),
        )?;
        require(
            self.is_create() == (self.object == ZERO),
            field("cob", "object", "zero iff action is create"),
        )?;
        require(
            !self.is_create() || self.parents.is_empty(),
            field("cob", "parents", "a create op has no parents"),
        )?;
        require(
            self.payload.len() <= MAX_PAYLOAD,
            field("cob", "payload", "over 64 KiB"),
        )?;
        require(
            crate::cbor::is_canonical_value(&self.payload),
            field(
                "cob",
                "payload",
                "not one canonical CBOR value without floats",
            ),
        )
    }

    fn extra_attributes(&self, signer: &Nid, h: &Hash32) -> Vec<Attribute> {
        vec![
            Attribute::b32("ag.rid", self.rid),
            Attribute::b32("ag.nid", *signer),
            Attribute::b32("ag.h", *h),
            Attribute::b32("ag.cob", self.object_id(h)),
            Attribute::str("ag.type", &self.type_name),
            Attribute::str("ag.action", &self.action),
        ]
    }
}

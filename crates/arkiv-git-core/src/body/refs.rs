use super::{Body, cbor_codec, require};
use crate::attrs::Attribute;
use crate::types::{Hash32, Nid, Rid};
use crate::{FormatError, Kind, field};

const ALLOWED_PREFIXES: [&str; 3] = ["refs/heads/", "refs/tags/", "refs/notes/"];

/// One `(ref name, oid)` pair.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(array)]
pub struct RefEntry {
    #[n(0)]
    pub name: String,
    #[cbor(n(1), with = "minicbor::bytes")]
    pub oid: Vec<u8>,
}

/// `refs`: one peer's signed refs.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct RefsBody {
    #[cbor(n(1), with = "minicbor::bytes")]
    pub rid: Rid,
    /// 1 = SHA-1, 2 = SHA-256.
    #[n(2)]
    pub object_hash: u8,
    /// Strictly ascending by name bytes.
    #[n(3)]
    pub refs: Vec<RefEntry>,
}

impl RefsBody {
    pub fn oid_len(&self) -> Option<usize> {
        match self.object_hash {
            1 => Some(20),
            2 => Some(32),
            _ => None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.refs
            .binary_search_by(|e| e.name.as_str().cmp(name))
            .ok()
            .map(|i| self.refs[i].oid.as_slice())
    }
}

/// gix-validate reference-name rules plus the prefixes design section 2.2 allows.
pub fn ref_name_ok(name: &str) -> bool {
    ALLOWED_PREFIXES.iter().any(|p| name.starts_with(p))
        && gix_validate::reference::name(name.as_bytes().into()).is_ok()
}

impl Body for RefsBody {
    const KIND: Kind = Kind::Refs;
    cbor_codec!();

    fn validate(&self) -> Result<(), FormatError> {
        let oid_len = self
            .oid_len()
            .ok_or(field("refs", "object_hash", "must be 1 or 2"))?;
        require(
            self.refs
                .windows(2)
                .all(|w| w[0].name.as_bytes() < w[1].name.as_bytes()),
            field("refs", "refs", "names must be strictly ascending"),
        )?;
        require(
            self.refs.iter().all(|e| ref_name_ok(&e.name)),
            field("refs", "refs", "invalid or disallowed ref name"),
        )?;
        require(
            self.refs.iter().all(|e| e.oid.len() == oid_len),
            field("refs", "refs", "oid length does not match object hash"),
        )
    }

    fn extra_attributes(&self, signer: &Nid, h: &Hash32) -> Vec<Attribute> {
        vec![
            Attribute::b32("ag.rid", self.rid),
            Attribute::b32("ag.nid", *signer),
            Attribute::b32("ag.h", *h),
        ]
    }
}

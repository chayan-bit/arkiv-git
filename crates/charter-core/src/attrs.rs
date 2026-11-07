//! Arkiv attributes and the Arkiv-side rules they must obey (`docs/p0/arkiv-limits.md`).

use crate::FormatError;
use crate::types::Hash32;

pub const MAX_ATTRIBUTES: usize = 32;
pub const MAX_NAME_LEN: usize = 32;
pub const MAX_STR_LEN: usize = 128;
pub const MAX_BYTES_LEN: usize = 131_072;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValue {
    Str(String),
    U64(u64),
    /// Never indexed by Arkiv.
    Bytes(Vec<u8>),
    Bytes32(Hash32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: AttrValue,
}

impl Attribute {
    pub fn new(name: &str, value: AttrValue) -> Self {
        Self {
            name: name.to_owned(),
            value,
        }
    }

    pub(crate) fn b32(name: &str, v: Hash32) -> Self {
        Self::new(name, AttrValue::Bytes32(v))
    }

    pub(crate) fn u64(name: &str, v: u64) -> Self {
        Self::new(name, AttrValue::U64(v))
    }

    pub(crate) fn str(name: &str, v: &str) -> Self {
        Self::new(name, AttrValue::Str(v.to_owned()))
    }

    /// Design section 2.1 table: everything is indexed except `ag.v` and `bytes` values.
    pub fn indexed(&self) -> bool {
        !matches!(self.value, AttrValue::Bytes(_)) && self.name != "ag.v"
    }
}

fn name_ok(n: &str) -> bool {
    let mut chars = n.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    first_ok
        && n.len() <= MAX_NAME_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Count, name grammar and length, value sizes, and strictly ascending name order.
pub fn check_arkiv_rules(attrs: &[Attribute]) -> Result<(), FormatError> {
    if attrs.len() > MAX_ATTRIBUTES {
        return Err(FormatError::AttributeRule("more than 32 attributes"));
    }
    if attrs.iter().any(|a| !name_ok(&a.name)) {
        return Err(FormatError::AttributeRule(
            "attribute name grammar or length",
        ));
    }
    if !attrs.windows(2).all(|w| w[0].name < w[1].name) {
        return Err(FormatError::AttributeRule(
            "attributes not strictly sorted by name",
        ));
    }
    attrs.iter().try_for_each(|a| match &a.value {
        AttrValue::Str(s) if s.len() > MAX_STR_LEN => {
            Err(FormatError::AttributeRule("str value over 128 B"))
        }
        AttrValue::Bytes(b) if b.len() > MAX_BYTES_LEN => {
            Err(FormatError::AttributeRule("bytes value over 128 KiB"))
        }
        _ => Ok(()),
    })
}

/// Sorts by name, as Arkiv requires.
pub(crate) fn sorted(mut v: Vec<Attribute>) -> Vec<Attribute> {
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Attributes of a `chunk` entity (no envelope; design section 2.2).
pub fn chunk_attributes(
    rid: &Hash32,
    pack_id: &Hash32,
    index: u64,
    bytes: &[u8],
) -> Vec<Attribute> {
    sorted(vec![
        Attribute::str("ag.kind", crate::Kind::Chunk.name()),
        Attribute::u64("ag.v", crate::ENTITY_LAYOUT_VERSION),
        Attribute::new("ag.body", AttrValue::Bytes(bytes.to_vec())),
        Attribute::b32("ag.rid", *rid),
        Attribute::b32("ag.pack", *pack_id),
        Attribute::u64("ag.i", index),
    ])
}

//! CBOR body types for every signed kind (design section 2.2).

mod anchor;
mod cob;
mod id;
mod peer;
mod refs;

pub use anchor::{AnchorBody, ManifestBody, Require};
pub use cob::{CREATE, CobOpBody, MAX_ACTION, MAX_PAYLOAD, MAX_TYPE_NAME};
pub use id::IdBody;
pub use peer::{BindBody, IdAccBody, PeerBody};
pub use refs::{RefEntry, RefsBody};

use crate::attrs::Attribute;
use crate::types::{Hash32, Nid};
use crate::{FormatError, Kind};

/// A signed entity body: canonical CBOR, field rules, and the kind-specific attributes.
pub trait Body: Sized + Clone {
    const KIND: Kind;

    /// Canonical CBOR bytes.
    fn encode(&self) -> Vec<u8>;

    /// Decodes canonical CBOR (re-encode check) and enforces the field rules.
    fn decode(bytes: &[u8]) -> Result<Self, FormatError>;

    /// Field rules from the CDDL and its notes.
    fn validate(&self) -> Result<(), FormatError>;

    /// Attributes beyond `ag.kind`, `ag.v`, `ag.body`, derived from body, signer and `h`.
    fn extra_attributes(&self, signer: &Nid, h: &Hash32) -> Vec<Attribute>;
}

macro_rules! cbor_codec {
    () => {
        fn encode(&self) -> Vec<u8> {
            crate::cbor::to_vec(self)
        }

        fn decode(bytes: &[u8]) -> Result<Self, crate::FormatError> {
            let v: Self = crate::cbor::decode_canonical(bytes)?;
            crate::Body::validate(&v)?;
            Ok(v)
        }
    };
}
pub(crate) use cbor_codec;

pub(crate) fn require(ok: bool, err: FormatError) -> Result<(), FormatError> {
    if ok { Ok(()) } else { Err(err) }
}

/// `seq = 0` iff `prev` is all zero (bind and anchor chains).
pub(crate) fn seq_prev_ok(seq: u64, prev: &Hash32) -> bool {
    (seq == 0) == (*prev == crate::ZERO)
}

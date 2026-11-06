//! arkiv-git-core: entity formats and the verification rules of arkiv-git P1.
//!
//! Pure: no I/O, no async. The sync layer feeds it [`Observed`] entities (attributes plus
//! creator, creation position and expiry, as Arkiv queries return them) and completeness
//! observations, and gets back verified chains, heads, canonical refs and stable errors.
//! See `docs/p1/design.md` sections 2, 3 and 7.

mod attrs;
mod body;
mod canonical;
mod cbor;
mod chain;
mod chunk;
mod entity;
mod envelope;
mod error;
mod kind;
mod marks;
mod types;

pub use attrs::{AttrValue, Attribute, check_arkiv_rules, chunk_attributes};
pub use body::{
    AnchorBody, BindBody, Body, IdAccBody, IdBody, ManifestBody, PeerBody, ProgramPin, RefEntry,
    RefsBody, Require,
};
pub use canonical::{CanonicalRefs, canonical_refs};
pub use chain::{
    AcceptedHead, AnchorChain, AnchorLink, BindChain, BindLink, CompletenessObs, Equivocation,
    HeadInputs, IdentityChain, PeerInfo, Revision, Start, accept_head, anchor_chain, bind_chain,
    check_completeness, check_control_set, check_requires, identity_chain, maj, peer_key,
};
pub use chunk::{CHUNK_SIZE, MAX_PACK_SIZE, chunk_count, chunk_hashes, split, verify_chunk};
pub use entity::{ENTITY_LAYOUT_VERSION, Entity, entity_attributes, open, verify_entity};
pub use envelope::{AG_BODY_MAX, ENVELOPE_OVERHEAD, Envelope, content_hash, max_body_len, sig_msg};
pub use error::{ErrorCode, FormatError, Missing, VerifyError};
pub use kind::{Kind, Plane};
pub use marks::{IdMark, PeerMark};
pub use types::{Address, Hash32, Nid, Observed, Position, Rid, ZERO, in_creation_order};

pub(crate) use error::field;

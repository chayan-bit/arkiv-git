//! Signature algorithm registry.

/// Composite public key length: ML-DSA-65 (1,952 B) || Ed25519 (32 B).
pub const PK_LEN: usize = 1984;
/// Composite signature length: ML-DSA-65 (3,309 B) || Ed25519 (64 B).
pub const SIG_LEN: usize = 3373;
/// Composite private key length: ML-DSA-65 seed (32 B) || Ed25519 seed (32 B).
pub const SECRET_LEN: usize = 64;

/// Signature algorithms arkiv-git knows. The `u16` id is what `nid` hashes
/// (`H("nid", u16be(alg) || pk)`) and what `PeerBody` field 1 carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Alg {
    /// `id-MLDSA65-Ed25519-SHA512` (OID 1.3.6.1.5.5.7.6.48), RFC 10073.
    MlDsa65Ed25519,
}

impl Alg {
    /// Registry id. Project-local (no multicodec code exists for composite keys).
    pub const fn id(self) -> u16 {
        match self {
            Alg::MlDsa65Ed25519 => 1,
        }
    }

    pub const fn from_id(id: u16) -> Option<Alg> {
        match id {
            1 => Some(Alg::MlDsa65Ed25519),
            _ => None,
        }
    }

    pub const fn pk_len(self) -> usize {
        match self {
            Alg::MlDsa65Ed25519 => PK_LEN,
        }
    }

    pub const fn sig_len(self) -> usize {
        match self {
            Alg::MlDsa65Ed25519 => SIG_LEN,
        }
    }
}

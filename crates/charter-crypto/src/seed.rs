//! The 32-byte master seed and derivation of the composite signing key from it.

use crate::{CryptoError, SECRET_LEN, SecretKey};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const SIG_SALT: &[u8] = b"arkiv-git/v1/sig"; // protocol constant: unchanged by the Charter rename

/// The peer's 32-byte master seed (design section 3.9 `keys/seed`). Zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MasterSeed([u8; 32]);

impl MasterSeed {
    /// A fresh seed from the OS RNG.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut s = [0u8; 32];
        getrandom::fill(&mut s).map_err(|_| CryptoError::SignFailed)?;
        Ok(Self(s))
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw seed, for persisting to `keys/seed` (mode 0600).
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }

    /// HKDF-SHA256 expand of the seed: `salt`, `info`, 32 bytes out.
    pub(crate) fn expand(&self, salt: &[u8], info: &[u8]) -> Zeroizing<[u8; 32]> {
        let mut okm = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(Some(salt), &self.0)
            .expand(info, okm.as_mut())
            .unwrap_or_else(|_| unreachable!("32 B is a valid HKDF-SHA256 output length"));
        okm
    }

    /// The composite identity / device signing key. Each component gets its own
    /// HKDF output (salt `arkiv-git/v1/sig`, info `ML-DSA-65` / `Ed25519`), which is the
    /// "seed cryptographically expanded to produce seeds for both components" that
    /// RFC 10073 section 4.1 ("Allowed Modifications to the Key Generation Process") permits.
    pub fn signing_key(&self) -> SecretKey {
        let ml = self.expand(SIG_SALT, b"ML-DSA-65");
        let ed = self.expand(SIG_SALT, b"Ed25519");
        let mut sk = Zeroizing::new([0u8; SECRET_LEN]);
        sk[..32].copy_from_slice(&*ml);
        sk[32..].copy_from_slice(&*ed);
        SecretKey::from_bytes(&*sk)
            .unwrap_or_else(|_| unreachable!("SECRET_LEN bytes always decode"))
    }
}

impl core::fmt::Debug for MasterSeed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MasterSeed(..)")
    }
}

/// Errors of the crypto layer, each mapped to a stable code (design section 7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("signature does not verify")]
    BadSignature,
    #[error("key or signature has the wrong length for its algorithm")]
    BadLength,
    #[error("public key is not a valid key for its algorithm")]
    BadKey,
    #[error("context string longer than 255 bytes")]
    ContextTooLong,
    #[error("signing failed (randomness or rejection sampling)")]
    SignFailed,
    #[error("unknown signature algorithm id")]
    UnknownAlg,
}

impl CryptoError {
    pub const fn code(self) -> &'static str {
        match self {
            CryptoError::BadSignature => "E_BAD_SIG",
            CryptoError::BadLength | CryptoError::BadKey | CryptoError::UnknownAlg => {
                "E_BAD_FORMAT"
            }
            CryptoError::ContextTooLong | CryptoError::SignFailed => "E_INTERNAL",
        }
    }
}

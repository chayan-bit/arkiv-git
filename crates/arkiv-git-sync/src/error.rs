//! `SyncError`: wraps core and store errors with context; every variant has a stable code
//! from design 7.3 (shared `ErrorCode` from `arkiv-git-core`).

use arkiv_git_core::{ErrorCode, FormatError, VerifyError};
use arkiv_git_store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Verify(#[from] VerifyError),
    #[error(transparent)]
    Format(#[from] FormatError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("git {cmd}: {msg}")]
    Git { cmd: String, msg: String },
    #[error("{path}: {msg}")]
    Io { path: String, msg: String },
    /// A failure the sync layer itself detects (chunks, packs, connectivity, push races).
    #[error("{msg}")]
    Coded { code: ErrorCode, msg: String },
}

pub type Result<T, E = SyncError> = std::result::Result<T, E>;

impl SyncError {
    pub fn coded(code: ErrorCode, msg: impl Into<String>) -> Self {
        Self::Coded {
            code,
            msg: msg.into(),
        }
    }

    pub fn io(path: impl std::fmt::Display, e: impl std::fmt::Display) -> Self {
        Self::Io {
            path: path.to_string(),
            msg: e.to_string(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Verify(e) => e.code(),
            Self::Format(e) => e.code(),
            Self::Store(e) => store_code(e),
            Self::Git { .. } | Self::Io { .. } => ErrorCode::Internal,
            Self::Coded { code, .. } => *code,
        }
    }
}

/// Store codes are strings; `E_TX_TOO_LARGE` and `E_GAS_CAP` are our bugs, so `E_INTERNAL`.
pub fn store_code(e: &StoreError) -> ErrorCode {
    match e.code() {
        "E_RPC" => ErrorCode::Rpc,
        "E_RATE_LIMITED" => ErrorCode::RateLimited,
        "E_WRONG_CHAIN" => ErrorCode::WrongChain,
        "E_INCONSISTENT_VIEW" => ErrorCode::InconsistentView,
        "E_PAGING" => ErrorCode::Paging,
        "E_INSUFFICIENT_FUNDS" => ErrorCode::InsufficientFunds,
        _ => ErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkiv_git_store::U256;

    #[test]
    fn codes_follow_the_design_table() {
        let cases: Vec<(SyncError, ErrorCode)> = vec![
            (VerifyError::IdNotLive.into(), ErrorCode::IdNotLive),
            (FormatError::NonCanonical.into(), ErrorCode::BadFormat),
            (StoreError::Rpc("x".into()).into(), ErrorCode::Rpc),
            (
                StoreError::RateLimited { retry_after: None }.into(),
                ErrorCode::RateLimited,
            ),
            (
                StoreError::WrongChain {
                    expected: 1,
                    got: 2,
                }
                .into(),
                ErrorCode::WrongChain,
            ),
            (
                StoreError::InconsistentView("c".into()).into(),
                ErrorCode::InconsistentView,
            ),
            (StoreError::Paging("p".into()).into(), ErrorCode::Paging),
            (
                StoreError::InsufficientFunds {
                    address: Default::default(),
                    need: U256::from(2),
                    have: U256::ZERO,
                }
                .into(),
                ErrorCode::InsufficientFunds,
            ),
            (
                StoreError::GasCap { gas: 2, cap: 1 }.into(),
                ErrorCode::Internal,
            ),
            (
                SyncError::Git {
                    cmd: "x".into(),
                    msg: "y".into(),
                },
                ErrorCode::Internal,
            ),
            (SyncError::io("p", "e"), ErrorCode::Internal),
            (
                SyncError::coded(ErrorCode::BadPack, "bad"),
                ErrorCode::BadPack,
            ),
        ];
        for (e, code) in cases {
            assert_eq!(e.code(), code, "{e}");
        }
    }
}

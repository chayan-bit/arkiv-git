//! `StoreError`: every failure the store reports, each with a stable code (design 7.3).

use std::time::Duration;

use alloy_primitives::{Address, TxHash, U256};

/// A store failure. [`StoreError::code`] is the stable string users and tests match on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// Transport or node failure with no more specific meaning.
    #[error("rpc: {0}")]
    Rpc(String),
    /// A non-success HTTP status (413 on an oversized request, 5xx).
    #[error("http status {status}: {body}")]
    Http { status: u16, body: String },
    /// HTTP 429, e.g. `ANON_COST_LIMITED`; retry no earlier than `retry_after`.
    #[error("rate limited by the RPC (retry after {retry_after:?})")]
    RateLimited { retry_after: Option<Duration> },
    /// The RPC serves a different chain than the one configured.
    #[error("wrong chain: expected {expected}, RPC serves {got}")]
    WrongChain { expected: u64, got: u64 },
    /// Two reads that must agree did not (count changed while paging, nonce moved).
    #[error("inconsistent view: {0}")]
    InconsistentView(String),
    /// Keyset paging cannot make progress (more than a page shares one value).
    #[error("paging: {0}")]
    Paging(String),
    /// The sender's nonce is below the node's tracked nonce.
    #[error("nonce too low: {0}")]
    NonceTooLow(String),
    /// A tx was never included after every resend.
    #[error("tx {0} dropped after resends")]
    Dropped(TxHash),
    /// Signed tx above the public RPC's raw size cap (a bug in batching).
    #[error("tx of {size} B exceeds the {cap} B raw tx cap")]
    TxTooLarge { size: usize, cap: usize },
    /// Tx gas above the per-tx cap (a bug in batching).
    #[error("tx gas {gas} exceeds the {cap} cap")]
    GasCap { gas: u64, cap: u64 },
    /// The sender cannot pay for the tx.
    #[error("insufficient funds at {address}: need {need}, have {have}")]
    InsufficientFunds {
        address: Address,
        need: U256,
        have: U256,
    },
    /// Execution reverted; `reason` is the decoded `IEntityRegistry` error when known.
    #[error("reverted{}: {reason}", tx.map(|h| format!(" in {h}")).unwrap_or_default())]
    Reverted { tx: Option<TxHash>, reason: String },
    /// A query or its options are invalid, locally or as judged by the node.
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    /// An operation could not be built (bad attribute name, unindexable type, size).
    #[error("invalid operation: {0}")]
    InvalidOp(String),
}

impl StoreError {
    /// The stable error code from design 7.3.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Rpc(_) | Self::Http { .. } | Self::NonceTooLow(_) | Self::Dropped(_) => "E_RPC",
            Self::RateLimited { .. } => "E_RATE_LIMITED",
            Self::WrongChain { .. } => "E_WRONG_CHAIN",
            Self::InconsistentView(_) => "E_INCONSISTENT_VIEW",
            Self::Paging(_) => "E_PAGING",
            Self::TxTooLarge { .. } => "E_TX_TOO_LARGE",
            Self::GasCap { .. } => "E_GAS_CAP",
            Self::InsufficientFunds { .. } => "E_INSUFFICIENT_FUNDS",
            Self::Reverted { .. } | Self::InvalidQuery(_) | Self::InvalidOp(_) => "E_INTERNAL",
        }
    }

    /// Whether retrying later (with backoff) can succeed.
    pub fn is_retriable(&self) -> bool {
        matches!(
            self.code(),
            "E_RPC" | "E_RATE_LIMITED" | "E_INCONSISTENT_VIEW" | "E_PAGING"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_retriability_follows_the_table() {
        let cases = [
            (StoreError::Rpc("x".into()), "E_RPC", true),
            (
                StoreError::RateLimited { retry_after: None },
                "E_RATE_LIMITED",
                true,
            ),
            (
                StoreError::WrongChain {
                    expected: 1,
                    got: 2,
                },
                "E_WRONG_CHAIN",
                false,
            ),
            (
                StoreError::InconsistentView("c".into()),
                "E_INCONSISTENT_VIEW",
                true,
            ),
            (StoreError::Paging("p".into()), "E_PAGING", true),
            (
                StoreError::TxTooLarge { size: 2, cap: 1 },
                "E_TX_TOO_LARGE",
                false,
            ),
            (StoreError::GasCap { gas: 2, cap: 1 }, "E_GAS_CAP", false),
            (
                StoreError::Reverted {
                    tx: None,
                    reason: "NotOwner".into(),
                },
                "E_INTERNAL",
                false,
            ),
            (StoreError::InvalidQuery("q".into()), "E_INTERNAL", false),
            (StoreError::InvalidOp("o".into()), "E_INTERNAL", false),
            (StoreError::Dropped(TxHash::ZERO), "E_RPC", true),
            (
                StoreError::Http {
                    status: 413,
                    body: String::new(),
                },
                "E_RPC",
                true,
            ),
        ];
        for (err, code, retriable) in cases {
            assert_eq!(err.code(), code, "{err}");
            assert_eq!(err.is_retriable(), retriable, "{err}");
        }
        let funds = StoreError::InsufficientFunds {
            address: Address::ZERO,
            need: U256::from(2),
            have: U256::from(1),
        };
        assert_eq!(funds.code(), "E_INSUFFICIENT_FUNDS");
        assert!(funds.to_string().contains("need 2, have 1"));
    }
}

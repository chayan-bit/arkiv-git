//! charter-escrow: an accountable escrow agent for repo programs (docs/p3/escrow.md).
//!
//! One agent key holds deposited tGLM and pays the obligations a repo program emits. The
//! agent is trusted to act, not to be honest: every deposit, payout and refund is a signed
//! `xyz.arkiv.escrow` op tied to a chain tx, and [`audit::audit`] recomputes everything
//! from the ops, the program's obligations and chain receipts, with no keys.

pub mod agent;
pub mod audit;
pub mod keys;
pub mod ledger;
pub mod obligation;
pub mod ops;

pub use agent::{Agent, Intent, Report, Transfer};
pub use audit::{AuditReport, Flag, audit};
pub use keys::EscrowKeys;
pub use ledger::{Appointment, Deposit, Ledger, Settlement};
pub use obligation::{FixedObligations, Obligation, ObligationSource};
pub use ops::TYPE;

use charter_core::ErrorCode;
use charter_sync::SyncError;

pub use charter_sync::Result;

pub(crate) fn bad(msg: impl Into<String>) -> SyncError {
    SyncError::coded(ErrorCode::BadFormat, msg)
}

pub(crate) fn internal(msg: impl Into<String>) -> SyncError {
    SyncError::coded(ErrorCode::Internal, msg)
}

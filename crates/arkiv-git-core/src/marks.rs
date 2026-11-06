//! High-water marks (design 3.9). Marks only increase.

use crate::VerifyError;
use crate::types::Hash32;

/// Per-peer mark: the newest anchor this client accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMark {
    pub seq: u64,
    pub hash: Hash32,
    pub block: u64,
}

impl PeerMark {
    /// The mark to store after accepting `new`; never lowers or forks the mark.
    pub fn advance(&self, new: &PeerMark) -> Result<PeerMark, VerifyError> {
        if new.seq < self.seq {
            return Err(VerifyError::StaleHead {
                seq: Some(new.seq),
                mark: self.seq,
            });
        }
        if new.seq == self.seq && new.hash != self.hash {
            return Err(VerifyError::HistoryRewritten { seq: self.seq });
        }
        Ok(new.clone())
    }
}

/// Identity mark: the newest identity revision this client accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdMark {
    pub rev: u64,
    pub hash: Hash32,
}

impl IdMark {
    pub fn advance(&self, new: &IdMark) -> Result<IdMark, VerifyError> {
        if new.rev < self.rev || (new.rev == self.rev && new.hash != self.hash) {
            return Err(VerifyError::StaleIdentity { rev: new.rev });
        }
        Ok(new.clone())
    }
}

//! arkiv-git-cli: the `git-remote-arkiv` helper and the `arkiv-git` tool (design 4.1, 7.3).

pub mod backend;
pub mod cmd;
pub mod helper;
pub mod units;
pub mod zk;

use arkiv_git_core::ErrorCode;
use arkiv_git_store::StoreError;
use arkiv_git_sync::SyncError;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Usage(String),
    #[error(transparent)]
    Sync(#[from] SyncError),
}

impl From<StoreError> for CliError {
    fn from(e: StoreError) -> Self {
        Self::Sync(e.into())
    }
}

impl CliError {
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Usage(_) => None,
            Self::Sync(e) => Some(e.code()),
        }
    }

    /// `arkiv-git` exit status: 1 verification, 2 usage, 3 transport, 4 funds.
    pub fn exit_code(&self) -> i32 {
        use ErrorCode::*;
        match self.code() {
            None => 2,
            Some(InsufficientFunds) => 4,
            Some(Rpc | RateLimited | InconsistentView | Paging | WrongChain) => 3,
            Some(_) => 1,
        }
    }

    /// One line: `<CODE>: <message>` (or `usage: <message>`).
    pub fn line(&self) -> String {
        let msg = self.to_string().replace('\n', " ");
        match self.code() {
            None => format!("usage: {msg}"),
            Some(c) => format!("{}: {msg}", c.as_str()),
        }
    }
}

pub type Result<T, E = CliError> = std::result::Result<T, E>;

/// Runs `f` on a fresh multi-threaded runtime.
pub fn block_on<F: std::future::Future>(f: F) -> std::result::Result<F::Output, CliError> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Sync(SyncError::io("tokio runtime", e)))?;
    Ok(rt.block_on(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_design_table() {
        let c = |code| CliError::Sync(SyncError::coded(code, "x"));
        assert_eq!(CliError::Usage("u".into()).exit_code(), 2);
        assert_eq!(c(ErrorCode::BadSig).exit_code(), 1);
        assert_eq!(c(ErrorCode::Rpc).exit_code(), 3);
        assert_eq!(c(ErrorCode::InsufficientFunds).exit_code(), 4);
        assert_eq!(c(ErrorCode::NotLive).line(), "E_NOT_LIVE: x");
        assert_eq!(CliError::Usage("a\nb".into()).line(), "usage: a b");
        assert_eq!(
            CliError::from(StoreError::Rpc("down".into())).exit_code(),
            3
        );
    }
}

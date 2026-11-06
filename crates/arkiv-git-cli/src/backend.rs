//! Backend selection: `ARKIV_GIT_BACKEND=mem:<path>` is a file-persisted `MemArkiv` (tests,
//! demos), anything else is Rabdi over JSON-RPC with the optional `ARKIV_ACCESS_KEY`.

use std::path::PathBuf;

use arkiv_git_store::fake::FileMemArkiv;
use arkiv_git_store::{
    Address, Arkiv, Operation, Query, QueryOpts, QueryPage, Receipt, RpcArkiv, RpcConfig, SignedTx,
    StoreError, TxHash, U256,
};
use arkiv_git_sync::Config;

use crate::CliError;

/// Chain id of the file-backed fake.
pub const MEM_CHAIN_ID: u64 = 7;

pub enum Backend {
    Mem(Box<FileMemArkiv>),
    Rpc(RpcArkiv),
}

/// Where `ARKIV_GIT_BACKEND` points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Mem(PathBuf),
    Rabdi,
}

pub fn target(var: Option<&str>) -> Result<Target, CliError> {
    match var.filter(|v| !v.is_empty()) {
        None | Some("rabdi") => Ok(Target::Rabdi),
        Some(v) => match v.strip_prefix("mem:") {
            Some(p) if !p.is_empty() => Ok(Target::Mem(PathBuf::from(p))),
            _ => Err(CliError::Usage(format!(
                "ARKIV_GIT_BACKEND must be `mem:<path>` or `rabdi`, got {v:?}"
            ))),
        },
    }
}

/// Lifetime override in blocks from `var` (test runs keep data short-lived).
fn lifetime(var: &str, default: u64) -> Result<u64, CliError> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => {
            crate::units::blocks(&v).map_err(|e| CliError::Usage(format!("{var}: {e}")))
        }
        _ => Ok(default),
    }
}

/// Connects to the configured backend and builds the matching sync config.
pub async fn connect() -> Result<(Backend, Config), CliError> {
    let home = Config::default_home();
    let var = std::env::var("ARKIV_GIT_BACKEND").ok();
    let (backend, mut cfg) = match target(var.as_deref())? {
        Target::Mem(path) => {
            let mem = FileMemArkiv::open(&path, MEM_CHAIN_ID)?;
            (Backend::Mem(Box::new(mem)), Config::for_fake(home))
        }
        Target::Rabdi => {
            let key = std::env::var("ARKIV_ACCESS_KEY").ok();
            let rpc = RpcArkiv::connect(RpcConfig::rabdi(key)).await?;
            (Backend::Rpc(rpc), Config::new(home))
        }
    };
    cfg.data_lifetime = lifetime("ARKIV_GIT_DATA_LIFETIME", cfg.data_lifetime)?;
    cfg.control_lifetime = lifetime("ARKIV_GIT_CONTROL_LIFETIME", cfg.control_lifetime)?;
    Ok((backend, cfg))
}

fn dev_only() -> CliError {
    CliError::Usage(
        "fund --dev only works with ARKIV_GIT_BACKEND=mem:<path>; on Rabdi get tGLM from \
         hub.arkiv.network/faucet"
            .into(),
    )
}

/// Refuses before connecting unless `ARKIV_GIT_BACKEND` is `mem:<path>`.
pub fn require_mem() -> Result<(), CliError> {
    match target(std::env::var("ARKIV_GIT_BACKEND").ok().as_deref())? {
        Target::Mem(_) => Ok(()),
        Target::Rabdi => Err(dev_only()),
    }
}

/// Dev faucet: credits `wei` to `wallet` on the `mem:` fake chain and returns the new
/// balance. Any other backend refuses, since real chains are funded from a faucet.
pub async fn dev_fund(b: &Backend, wallet: Address, wei: U256) -> Result<U256, CliError> {
    let Backend::Mem(mem) = b else {
        return Err(dev_only());
    };
    mem.fund(wallet, wei)?;
    Ok(mem.balance(wallet).await?)
}

macro_rules! both {
    ($self:ident, $b:ident => $e:expr) => {
        match $self {
            Backend::Mem($b) => $e,
            Backend::Rpc($b) => $e,
        }
    };
}

impl Arkiv for Backend {
    async fn chain_id(&self) -> Result<u64, StoreError> {
        both!(self, b => b.chain_id().await)
    }
    async fn head(&self) -> Result<u64, StoreError> {
        both!(self, b => b.head().await)
    }
    async fn base_fee(&self) -> Result<u128, StoreError> {
        both!(self, b => b.base_fee().await)
    }
    async fn query(&self, q: &Query, o: &QueryOpts) -> Result<QueryPage, StoreError> {
        both!(self, b => b.query(q, o).await)
    }
    async fn entity_nonce(&self, c: Address) -> Result<u64, StoreError> {
        both!(self, b => b.entity_nonce(c).await)
    }
    async fn balance(&self, a: Address) -> Result<U256, StoreError> {
        both!(self, b => b.balance(a).await)
    }
    async fn tx_count(&self, a: Address) -> Result<u64, StoreError> {
        both!(self, b => b.tx_count(a).await)
    }
    async fn estimate(&self, from: Address, ops: &[Operation]) -> Result<u64, StoreError> {
        both!(self, b => b.estimate(from, ops).await)
    }
    async fn send(&self, tx: SignedTx) -> Result<TxHash, StoreError> {
        both!(self, b => b.send(tx).await)
    }
    async fn receipt(&self, h: TxHash) -> Result<Option<Receipt>, StoreError> {
        both!(self, b => b.receipt(h).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_variable_selects_mem_or_rabdi() {
        assert_eq!(target(None).expect("default"), Target::Rabdi);
        assert_eq!(target(Some("")).expect("empty"), Target::Rabdi);
        assert_eq!(target(Some("rabdi")).expect("rabdi"), Target::Rabdi);
        assert_eq!(
            target(Some("mem:/x/log")).expect("mem"),
            Target::Mem(PathBuf::from("/x/log"))
        );
        assert!(target(Some("mem:")).is_err());
        assert!(target(Some("http://x")).is_err());
    }
}

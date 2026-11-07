//! `chart escrow`: the accountable escrow agent (docs/p3/escrow.md).

use std::path::{Path, PathBuf};
use std::time::Duration;

use charter_core::Rid;
use charter_escrow::ops::{self, DEPOSIT, DepositPayload};
use charter_escrow::{
    Agent, EscrowKeys, FixedObligations, Obligation, ObligationSource, audit, ledger,
};
use charter_store::{Address, Arkiv, TxHash, TxPipeline};
use charter_sync::ids::{parse_id, to_text};
use charter_sync::{Config, Keys, Repo};
use clap::{Args, Subcommand};
use tokio::runtime::Handle;

use crate::cmd::rid_of;
use crate::{CliError, Result};

#[derive(Debug, Subcommand)]
pub enum EscrowCmd {
    /// Create the escrow identity and payout key under $CHARTER_HOME/escrow.
    Init,
    /// Print the escrow identity id and address.
    Show,
    /// As a delegate, appoint an escrow for a repo.
    Appoint {
        #[arg(long)]
        repo: String,
        /// The escrow's identity id.
        #[arg(long)]
        escrow: String,
        #[arg(long)]
        address: Address,
    },
    /// Send tGLM to the repo's escrow from the wallet (or name a sent `--tx`) and claim it.
    Deposit(DepositArgs),
    /// Pay due obligations and refund expired deposits, every `--interval` seconds.
    Run(RunArgs),
    /// Recompute the ledger with no keys; exits 1 if anything is flagged.
    Audit(AuditArgs),
}

#[derive(Debug, Args)]
pub struct DepositArgs {
    #[arg(long)]
    pub repo: String,
    /// Wei, decimal.
    #[arg(long)]
    pub amount: String,
    /// Free-form key, e.g. `bounty:<issue id>`.
    #[arg(long)]
    pub purpose: String,
    /// Block after which an unconsumed deposit is refunded.
    #[arg(long)]
    pub expiry: Option<u64>,
    /// An already sent transfer to claim instead of sending one.
    #[arg(long)]
    pub tx: Option<TxHash>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[arg(long)]
    pub repo: String,
    /// JSON array of obligations instead of the repo's pinned program.
    #[arg(long)]
    pub obligations: Option<PathBuf>,
    #[arg(long)]
    pub once: bool,
    #[arg(long, default_value_t = 30)]
    pub interval: u64,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[arg(long)]
    pub repo: String,
    /// JSON array of obligations instead of the repo's pinned program.
    #[arg(long)]
    pub obligations: Option<PathBuf>,
    /// Blocks a funded obligation may wait before it is a missing payout.
    #[arg(long, default_value_t = 300)]
    pub grace: u64,
}

/// Obligations of the repo's pinned program: each call fetches the repo into the local
/// cache (fast-forwards need the objects) and folds it.
pub struct ProgramObligations<'a, A> {
    pub a: &'a A,
    pub home: PathBuf,
}

/// One program obligation as the escrow reads it; JSON integer amounts become strings.
fn to_obligation(mut v: serde_json::Value) -> charter_sync::Result<Obligation> {
    if let Some(n) = v
        .get("amount")
        .filter(|a| a.is_number())
        .map(ToString::to_string)
    {
        v["amount"] = n.into();
    }
    serde_json::from_value(v.clone()).map_err(|e| {
        charter_sync::SyncError::coded(
            charter_core::ErrorCode::BadFormat,
            format!("program obligation {v}: {e}"),
        )
    })
}

impl<A: Arkiv> ObligationSource for ProgramObligations<'_, A> {
    fn obligations(&self, rid: &Rid) -> charter_sync::Result<Vec<Obligation>> {
        let repo = Repo::open(&self.home, rid)?;
        // The trait is sync but runs inside the CLI's multi-threaded runtime.
        let fold = charter_sync::program::obligations_in(self.a, &repo);
        let values = tokio::task::block_in_place(|| Handle::current().block_on(fold))?;
        values
            .into_iter()
            .filter(|v| v["kind"] == "pay")
            .map(to_obligation)
            .collect()
    }
}

/// `--obligations <file>` if given, else the pinned program.
pub enum Source<'a, A> {
    File(FixedObligations),
    Program(ProgramObligations<'a, A>),
}

impl<A: Arkiv> ObligationSource for Source<'_, A> {
    fn obligations(&self, rid: &Rid) -> charter_sync::Result<Vec<Obligation>> {
        match self {
            Self::File(f) => f.obligations(rid),
            Self::Program(p) => p.obligations(rid),
        }
    }
}

fn source<'a, A: Arkiv>(a: &'a A, cfg: &Config, file: &Option<PathBuf>) -> Result<Source<'a, A>> {
    Ok(match file {
        Some(p) => Source::File(obligations(p)?),
        None => Source::Program(ProgramObligations {
            a,
            home: cfg.home.clone(),
        }),
    })
}

fn obligations(p: &Path) -> Result<FixedObligations> {
    let bytes = std::fs::read(p).map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))?;
    let v: Vec<Obligation> = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))?;
    Ok(FixedObligations(v))
}

impl EscrowCmd {
    /// The repo a chain command acts on (`init` and `show` have none).
    pub fn repo(&self) -> Option<&str> {
        match self {
            Self::Init | Self::Show => None,
            Self::Appoint { repo, .. } => Some(repo),
            Self::Deposit(d) => Some(&d.repo),
            Self::Run(r) => Some(&r.repo),
            Self::Audit(x) => Some(&x.repo),
        }
    }
}

/// Commands that need no chain.
pub fn local(cmd: &EscrowCmd, home: &Path) -> Option<Result<()>> {
    let keys = match cmd {
        EscrowCmd::Init => EscrowKeys::init(home),
        EscrowCmd::Show => EscrowKeys::require(home),
        _ => return None,
    };
    Some(keys.map_err(CliError::from).map(|k| {
        println!("escrow:  {}", to_text(&k.id.nid()));
        println!("address: {}", k.address());
        println!(
            "wallet:  {} (funds the escrow's ops; keep it apart from deposits)",
            k.id.wallet().address()
        );
    }))
}

pub async fn run<A: Arkiv>(a: &A, cfg: &Config, cmd: &EscrowCmd) -> Result<()> {
    match cmd {
        EscrowCmd::Init | EscrowCmd::Show => Ok(()),
        EscrowCmd::Appoint {
            repo,
            escrow,
            address,
        } => {
            let (rid, nid) = (rid_of(repo)?, parse_id(escrow)?);
            let keys = Keys::require(&cfg.home)?;
            let obj = ops::appoint(a, cfg, &keys, &rid, &nid, *address).await?;
            println!("{}", to_text(&obj));
            Ok(())
        }
        EscrowCmd::Deposit(d) => deposit(a, cfg, d).await,
        EscrowCmd::Run(r) => {
            let src = source(a, cfg, &r.obligations)?;
            let keys = EscrowKeys::require(&cfg.home)?;
            let agent = Agent {
                a,
                cfg,
                keys: &keys,
                rid: rid_of(&r.repo)?,
                source: &src,
            };
            if !r.once {
                agent.run(Duration::from_secs(r.interval)).await?;
            }
            let rep = agent.run_once().await?;
            rep.paid.iter().for_each(|k| println!("paid {k}"));
            rep.refunded.iter().for_each(|k| println!("refunded {k}"));
            rep.skipped.iter().for_each(|s| println!("skipped {s}"));
            Ok(())
        }
        EscrowCmd::Audit(x) => audit_cmd(a, cfg, x).await,
    }
}

async fn deposit<A: Arkiv>(a: &A, cfg: &Config, d: &DepositArgs) -> Result<()> {
    let rid = rid_of(&d.repo)?;
    let keys = Keys::require(&cfg.home)?;
    let appt = ledger::load(a, &rid).await?.appointment;
    let tx = match d.tx {
        Some(tx) => tx,
        None => {
            let value = charter_escrow::obligation::parse_wei(&d.amount)?;
            let chain_id = a.chain_id().await?;
            let mut p = TxPipeline::new(a, keys.wallet(), chain_id, cfg.pipeline.clone());
            p.transfer(appt.address, value).await?.tx_hash
        }
    };
    let p = DepositPayload {
        tx,
        amount: d.amount.clone(),
        purpose: d.purpose.clone(),
        expiry: d.expiry,
    };
    ops::post(a, cfg, &keys, &rid, appt.object, DEPOSIT, &p).await?;
    println!("{tx}");
    Ok(())
}

async fn audit_cmd<A: Arkiv>(a: &A, cfg: &Config, x: &AuditArgs) -> Result<()> {
    let src = source(a, cfg, &x.obligations)?;
    let r = audit(a, &rid_of(&x.repo)?, &src, x.grace).await?;
    let l = &r.ledger;
    println!(
        "escrow:   {} at {}",
        to_text(&l.appointment.nid),
        l.appointment.address
    );
    println!(
        "credited: {} wei in {} deposit(s)",
        l.credited(),
        l.deposits.len()
    );
    for (op, why) in &l.rejected {
        println!("rejected: deposit op {}: {why}", to_text(op));
    }
    println!("settled:  {} op(s)", l.settlements.len());
    println!("balance:  {} wei, owes {}", r.balance, r.owed);
    r.flags.iter().for_each(|f| println!("FLAG {f}"));
    match r.flags.len() {
        0 => Ok(()),
        n => Err(CliError::Flagged(format!("audit found {n} flag(s)"))),
    }
}

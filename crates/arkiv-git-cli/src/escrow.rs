//! `arkiv-git escrow`: the accountable escrow agent (docs/p3/escrow.md).

use std::path::{Path, PathBuf};
use std::time::Duration;

use arkiv_git_escrow::ops::{self, DEPOSIT, DepositPayload};
use arkiv_git_escrow::{Agent, EscrowKeys, FixedObligations, Obligation, audit, ledger};
use arkiv_git_store::{Address, Arkiv, TxHash, TxPipeline};
use arkiv_git_sync::ids::{parse_id, to_text};
use arkiv_git_sync::{Config, Keys};
use clap::{Args, Subcommand};

use crate::cmd::rid_of;
use crate::{CliError, Result};

#[derive(Debug, Subcommand)]
pub enum EscrowCmd {
    /// Create the escrow identity and payout key under $ARKIV_GIT_HOME/escrow.
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
    /// JSON array of the program's obligations.
    #[arg(long)]
    pub obligations: PathBuf,
    #[arg(long)]
    pub once: bool,
    #[arg(long, default_value_t = 30)]
    pub interval: u64,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    #[arg(long)]
    pub repo: String,
    #[arg(long)]
    pub obligations: PathBuf,
    /// Blocks a funded obligation may wait before it is a missing payout.
    #[arg(long, default_value_t = 300)]
    pub grace: u64,
}

fn obligations(p: &Path) -> Result<FixedObligations> {
    let bytes = std::fs::read(p).map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))?;
    let v: Vec<Obligation> = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Usage(format!("{}: {e}", p.display())))?;
    Ok(FixedObligations(v))
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
            let src = obligations(&r.obligations)?;
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
        EscrowCmd::Audit(x) => audit_cmd(a, x).await,
    }
}

async fn deposit<A: Arkiv>(a: &A, cfg: &Config, d: &DepositArgs) -> Result<()> {
    let rid = rid_of(&d.repo)?;
    let keys = Keys::require(&cfg.home)?;
    let appt = ledger::load(a, &rid).await?.appointment;
    let tx = match d.tx {
        Some(tx) => tx,
        None => {
            let value = arkiv_git_escrow::obligation::parse_wei(&d.amount)?;
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

async fn audit_cmd<A: Arkiv>(a: &A, x: &AuditArgs) -> Result<()> {
    let src = obligations(&x.obligations)?;
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

//! The per-repo escrow ledger, recomputed from verified ops and chain txs. The agent and
//! the audit run the same fold, so the agent cannot keep a private view of the books.

use std::collections::{BTreeMap, BTreeSet};

use arkiv_git_core::{Hash32, Nid, Rid, VerifiedOp};
use arkiv_git_store::{Address, Arkiv, Receipt, TxHash, TxInfo, U256};
use arkiv_git_sync::ids::parse_id;
use arkiv_git_sync::state::State;
use arkiv_git_sync::view;

use crate::obligation::{Obligation, parse_wei};
use crate::ops::{self, APPOINT, AppointPayload, DEPOSIT, DepositPayload, PAID, REFUNDED};
use crate::ops::{SettlePayload, payload};
use crate::{Result, internal};

/// The latest `appoint` op posted by a current delegate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appointment {
    pub op: Hash32,
    pub object: Hash32,
    pub nid: Nid,
    pub address: Address,
}

/// A credited deposit: its tx went to the escrow address with at least `amount`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deposit {
    pub op: Hash32,
    pub tx: TxHash,
    /// The tx sender; refunds go here.
    pub from: Address,
    pub amount: U256,
    pub purpose: String,
    pub expiry: Option<u64>,
    pub block: u64,
}

/// A `paid` or `refunded` op by the appointed escrow, with what the chain says of its tx.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    pub op: Hash32,
    pub action: String,
    pub key: String,
    pub tx: TxHash,
    pub to: Address,
    pub amount: U256,
    pub deposit: Option<TxHash>,
    /// The mined block if the tx is a successful transfer from the escrow address to `to`
    /// of exactly `amount`, else why not.
    pub chain: std::result::Result<u64, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledger {
    pub appointment: Appointment,
    pub deposits: Vec<Deposit>,
    /// Deposit ops not credited, with the reason.
    pub rejected: Vec<(Hash32, String)>,
    pub settlements: Vec<Settlement>,
}

/// One chain tx and its receipt, if both exist and it is mined.
async fn chain_tx<A: Arkiv>(a: &A, h: TxHash) -> Result<Option<(TxInfo, Receipt)>> {
    let Some(t) = a.transaction(h).await? else {
        return Ok(None);
    };
    Ok(a.receipt(h).await?.map(|r| (t, r)))
}

/// Verified escrow ops of `rid` folded against the chain.
pub async fn load<A: Arkiv>(a: &A, rid: &Rid) -> Result<Ledger> {
    let ops = ops::list(a, rid).await?;
    let (chain, _) = Box::pin(view::identity(a, rid, &State::default())).await?;
    fold(a, &ops, &chain.head().body.delegates).await
}

fn appointment(ops: &[VerifiedOp], delegates: &[Nid]) -> Result<Appointment> {
    let op = ops
        .iter()
        .rfind(|o| o.body.action == APPOINT && delegates.contains(&o.author))
        .ok_or_else(|| internal("no escrow appointed by a delegate of this repo"))?;
    let p: AppointPayload = payload(op)?;
    Ok(Appointment {
        op: op.id,
        object: op.object(),
        nid: p.nid()?,
        address: p.address,
    })
}

pub async fn fold<A: Arkiv>(a: &A, ops: &[VerifiedOp], delegates: &[Nid]) -> Result<Ledger> {
    let appt = appointment(ops, delegates)?;
    let (mut deposits, mut rejected, mut settlements) = (vec![], vec![], vec![]);
    let mut claimed = BTreeSet::new();
    for op in ops {
        match op.body.action.as_str() {
            DEPOSIT => match credit(a, &appt, op, &mut claimed).await? {
                Ok(d) => deposits.push(d),
                Err(why) => rejected.push((op.id, why)),
            },
            PAID | REFUNDED if op.author == appt.nid => {
                let Ok(p) = payload::<SettlePayload>(op) else {
                    continue;
                };
                let Ok(amount) = parse_wei(&p.amount) else {
                    continue;
                };
                let chain = match chain_tx(a, p.tx).await? {
                    None => Err("tx not mined".to_string()),
                    Some((t, r)) => transfer_ok(&t, &r, appt.address, p.to, amount),
                };
                settlements.push(Settlement {
                    op: op.id,
                    action: op.body.action.clone(),
                    key: p.key,
                    tx: p.tx,
                    to: p.to,
                    amount,
                    deposit: p.deposit,
                    chain,
                });
            }
            _ => {}
        }
    }
    Ok(Ledger {
        appointment: appt,
        deposits,
        rejected,
        settlements,
    })
}

fn transfer_ok(
    t: &TxInfo,
    r: &Receipt,
    from: Address,
    to: Address,
    amount: U256,
) -> std::result::Result<u64, String> {
    match () {
        _ if !r.success => Err("tx reverted".into()),
        _ if t.from != from => Err(format!("tx sent by {}, not the escrow", t.from)),
        _ if t.to != Some(to) => Err(format!("tx went to {:?}, not {to}", t.to)),
        _ if t.value != amount => Err(format!("tx moved {} wei, not {amount}", t.value)),
        _ => Ok(r.block_number),
    }
}

/// Credits a deposit op, or says why not. The first claim of a tx wins.
async fn credit<A: Arkiv>(
    a: &A,
    appt: &Appointment,
    op: &VerifiedOp,
    claimed: &mut BTreeSet<TxHash>,
) -> Result<std::result::Result<Deposit, String>> {
    let Ok(p) = payload::<DepositPayload>(op) else {
        return Ok(Err("malformed deposit payload".into()));
    };
    let Ok(amount) = parse_wei(&p.amount) else {
        return Ok(Err(format!("bad amount {:?}", p.amount)));
    };
    if claimed.contains(&p.tx) {
        return Ok(Err(format!("tx {} already claimed", p.tx)));
    }
    let Some((t, r)) = chain_tx(a, p.tx).await? else {
        return Ok(Err(format!("tx {} not mined", p.tx)));
    };
    let why = match () {
        _ if !r.success => Some("tx reverted".to_string()),
        _ if t.to != Some(appt.address) => Some(format!("tx went to {:?}, not the escrow", t.to)),
        _ if t.from == appt.address => Some("tx sent by the escrow itself".into()),
        _ if t.value < amount => Some(format!("tx moved {} wei < {amount}", t.value)),
        _ if amount.is_zero() => Some("zero amount".into()),
        _ => None,
    };
    if let Some(w) = why {
        return Ok(Err(w));
    }
    claimed.insert(p.tx);
    Ok(Ok(Deposit {
        op: op.id,
        tx: p.tx,
        from: t.from,
        amount,
        purpose: p.purpose,
        expiry: p.expiry,
        block: r.block_number,
    }))
}

/// The key a refund of deposit `tx` settles.
pub fn refund_key(tx: &TxHash) -> String {
    format!("refund:{tx}")
}

/// A funded, unsettled pay obligation and the block from which it counts as due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Due {
    pub obligation: Obligation,
    pub amount: U256,
    pub due_since: u64,
}

impl Ledger {
    pub fn settled(&self, key: &str) -> bool {
        self.settlements.iter().any(|s| s.key == key)
    }

    pub fn credited(&self) -> U256 {
        self.deposits.iter().map(|d| d.amount).sum()
    }

    /// Credited minus every settlement, per purpose. A payout counts against its
    /// obligation's purpose, a refund against its deposit's purpose.
    pub fn available(&self, obligations: &[Obligation]) -> BTreeMap<String, U256> {
        let mut avail: BTreeMap<String, U256> = BTreeMap::new();
        for d in &self.deposits {
            *avail.entry(d.purpose.clone()).or_default() += d.amount;
        }
        for s in &self.settlements {
            let purpose = match s.deposit {
                Some(tx) => self
                    .deposits
                    .iter()
                    .find(|d| d.tx == tx)
                    .map(|d| &d.purpose),
                None => obligations
                    .iter()
                    .find(|o| o.key == s.key)
                    .map(|o| &o.purpose),
            };
            if let Some(v) = purpose.and_then(|p| avail.get_mut(p)) {
                *v = v.saturating_sub(s.amount);
            }
        }
        avail
    }

    /// Unsettled pay obligations the ledger can fund, allocated in program order.
    pub fn due(&self, obligations: &[Obligation]) -> Result<Vec<Due>> {
        let mut avail = self.available(obligations);
        let mut out = vec![];
        for o in obligations
            .iter()
            .filter(|o| o.is_pay() && !self.settled(&o.key))
        {
            let amount = o.wei()?;
            let Some(v) = avail.get_mut(&o.purpose).filter(|v| **v >= amount) else {
                continue;
            };
            *v -= amount;
            let funded = self
                .deposits
                .iter()
                .filter(|d| d.purpose == o.purpose)
                .map(|d| d.block)
                .max()
                .unwrap_or(0);
            out.push(Due {
                obligation: o.clone(),
                amount,
                due_since: funded.max(o.since_block),
            });
        }
        Ok(out)
    }

    /// Credited deposits past expiry whose purpose no pay obligation uses, not yet refunded.
    pub fn refundable(&self, obligations: &[Obligation], head: u64) -> Vec<&Deposit> {
        self.deposits
            .iter()
            .filter(|d| d.expiry.is_some_and(|e| head > e))
            .filter(|d| {
                !obligations
                    .iter()
                    .any(|o| o.is_pay() && o.purpose == d.purpose)
            })
            .filter(|d| !self.settled(&refund_key(&d.tx)))
            .collect()
    }
}

/// The address an obligation's `to` names: a `0x` address, or an identity id's latest
/// control address for the repo.
pub async fn resolve_to<A: Arkiv>(a: &A, rid: &Rid, to: &str) -> Result<Option<Address>> {
    if let Ok(addr) = to.parse::<Address>() {
        return Ok(Some(addr));
    }
    let Ok(nid) = parse_id(to) else {
        return Ok(None);
    };
    let peer = Box::pin(view::resolve_peer(a, rid, &nid)).await?;
    Ok(peer.and_then(|p| p.binds.links.last().map(|l| Address::from(l.body.control))))
}

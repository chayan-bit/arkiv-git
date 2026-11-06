//! `arkiv-git escrow audit`: anyone, with no keys, recomputes the ledger from the ops, the
//! program's obligations and chain receipts, and flags what an honest agent never does.

use std::collections::BTreeMap;

use arkiv_git_core::{Hash32, Rid};
use arkiv_git_store::{Arkiv, TxHash, U256};
use arkiv_git_sync::ids::hex32;

use crate::Result;
use crate::ledger::{self, Ledger, Settlement, refund_key};
use crate::obligation::{Obligation, ObligationSource};
use crate::ops::{PAID, REFUNDED};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flag {
    /// A funded pay obligation still unsettled `grace` blocks after it became due.
    MissingPayout { key: String, due_since: u64 },
    /// Money left the escrow with no obligation or refund right behind it.
    UnauthorizedPayout { detail: String },
    /// One key, or one tx, settled more than once.
    DoublePay { key: String, ops: Vec<Hash32> },
    /// A settlement op whose tx does not show the transfer it claims.
    BadReceipt { op: Hash32, reason: String },
    /// The escrow holds less than credited minus settled.
    Shortfall { balance: U256, owed: U256 },
}

impl std::fmt::Display for Flag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPayout { key, due_since } => {
                write!(
                    f,
                    "missing payout: {key} funded and due since block {due_since}"
                )
            }
            Self::UnauthorizedPayout { detail } => write!(f, "unauthorized payout: {detail}"),
            Self::DoublePay { key, ops } => {
                let ids: Vec<String> = ops.iter().map(hex32).collect();
                write!(f, "double pay: {key} settled by {}", ids.join(", "))
            }
            Self::BadReceipt { op, reason } => write!(f, "bad receipt: op {}: {reason}", hex32(op)),
            Self::Shortfall { balance, owed } => {
                write!(
                    f,
                    "balance shortfall: escrow holds {balance} wei, owes {owed}"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditReport {
    pub ledger: Ledger,
    pub balance: U256,
    pub owed: U256,
    pub flags: Vec<Flag>,
}

/// Audits `rid`; `grace` is how many blocks a funded obligation may wait for its payout.
pub async fn audit<A: Arkiv, O: ObligationSource>(
    a: &A,
    rid: &Rid,
    source: &O,
    grace: u64,
) -> Result<AuditReport> {
    // Boxed: the fold's future is large enough to overflow a 2 MiB test thread stack.
    let l = Box::pin(ledger::load(a, rid)).await?;
    let obligations = source.obligations(rid)?;
    let head = a.head().await?;
    let mut flags = vec![];
    for d in l.due(&obligations)? {
        if head >= d.due_since.saturating_add(grace) {
            let (key, due_since) = (d.obligation.key, d.due_since);
            flags.push(Flag::MissingPayout { key, due_since });
        }
    }
    flags.extend(doubles(&l.settlements));
    let mut verified = 0u64;
    for s in &l.settlements {
        match &s.chain {
            Err(reason) => flags.push(Flag::BadReceipt {
                op: s.op,
                reason: reason.clone(),
            }),
            Ok(block) => {
                verified += 1;
                if let Some(detail) =
                    Box::pin(authorized(a, rid, &l, &obligations, s, *block)).await?
                {
                    flags.push(Flag::UnauthorizedPayout { detail });
                }
            }
        }
    }
    let address = l.appointment.address;
    let mined = a.tx_count_mined(address).await?;
    if mined > verified {
        let detail = format!(
            "{} tx(s) from {address} have no settlement op",
            mined - verified
        );
        flags.push(Flag::UnauthorizedPayout { detail });
    }
    let settled: U256 = l
        .settlements
        .iter()
        .filter(|s| s.chain.is_ok())
        .map(|s| s.amount)
        .sum();
    let owed = l.credited().saturating_sub(settled);
    let balance = a.balance(address).await?;
    if balance < owed {
        flags.push(Flag::Shortfall { balance, owed });
    }
    Ok(AuditReport {
        ledger: l,
        balance,
        owed,
        flags,
    })
}

/// Settlements sharing a key or a tx (one transfer claimed twice).
fn doubles(settlements: &[Settlement]) -> Vec<Flag> {
    let mut by_key: BTreeMap<&str, Vec<Hash32>> = BTreeMap::new();
    let mut by_tx: BTreeMap<TxHash, Vec<Hash32>> = BTreeMap::new();
    for s in settlements {
        by_key.entry(&s.key).or_default().push(s.op);
        by_tx.entry(s.tx).or_default().push(s.op);
    }
    let keys = by_key.into_iter().map(|(k, ops)| (k.to_string(), ops));
    let txs = by_tx.into_iter().map(|(t, ops)| (format!("tx {t}"), ops));
    keys.chain(txs)
        .filter(|(_, ops)| ops.len() > 1)
        .map(|(key, ops)| Flag::DoublePay { key, ops })
        .collect()
}

/// Why a verified transfer was not authorized, or `None` if it was.
async fn authorized<A: Arkiv>(
    a: &A,
    rid: &Rid,
    l: &Ledger,
    obligations: &[Obligation],
    s: &Settlement,
    block: u64,
) -> Result<Option<String>> {
    let key = &s.key;
    match s.action.as_str() {
        PAID => {
            let Some(o) = obligations.iter().find(|o| o.is_pay() && o.key == *key) else {
                return Ok(Some(format!("{key}: no pay obligation with this key")));
            };
            let to = ledger::resolve_to(a, rid, &o.to).await?;
            Ok(match () {
                _ if o.wei().ok() != Some(s.amount) => {
                    Some(format!("{key}: paid {} wei, owed {}", s.amount, o.amount))
                }
                _ if to != Some(s.to) => Some(format!("{key}: paid {}, owed to {}", s.to, o.to)),
                _ => None,
            })
        }
        REFUNDED => {
            let d = s
                .deposit
                .and_then(|tx| l.deposits.iter().find(|d| d.tx == tx));
            let Some(d) = d else {
                return Ok(Some(format!("{key}: refund of no credited deposit")));
            };
            let used = obligations
                .iter()
                .any(|o| o.is_pay() && o.purpose == d.purpose);
            Ok(match () {
                _ if *key != refund_key(&d.tx) => Some(format!("{key}: wrong refund key")),
                _ if !d.expiry.is_some_and(|e| block > e) => {
                    Some(format!("{key}: refunded before expiry"))
                }
                _ if used => Some(format!("{key}: purpose {:?} has obligations", d.purpose)),
                _ if s.to != d.from || s.amount != d.amount => Some(format!(
                    "{key}: refund must return {} wei to {}",
                    d.amount, d.from
                )),
                _ => None,
            })
        }
        other => Ok(Some(format!("{key}: unknown action {other}"))),
    }
}

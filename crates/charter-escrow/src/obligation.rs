//! What a repo program owes: the escrow's only source of authority to pay.
//!
//! The Rhai program engine (another branch) returns `obligations` as maps; this crate
//! codes against [`ObligationSource`] so wiring the engine in is one adapter.

use charter_core::Rid;
use charter_store::U256;
use serde::{Deserialize, Serialize};

use crate::{Result, bad};

/// One obligation, e.g. `#{ kind: "pay", to, amount, reason, key }` from a program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obligation {
    /// Only `pay` moves money; other kinds are ignored here.
    pub kind: String,
    /// `0x` EVM address, or an identity id (paid to its latest control address for the repo).
    pub to: String,
    /// Wei, decimal string.
    pub amount: String,
    #[serde(default)]
    pub reason: String,
    /// Unique per program; the escrow pays each key at most once.
    pub key: String,
    /// The deposit purpose that funds it, e.g. `bounty:<issue id>` (extension field).
    #[serde(default)]
    pub purpose: String,
    /// Block at which it became due, for the audit's grace period (extension field, 0 = always).
    #[serde(default)]
    pub since_block: u64,
}

impl Obligation {
    pub fn is_pay(&self) -> bool {
        self.kind == "pay"
    }

    pub fn wei(&self) -> Result<U256> {
        parse_wei(&self.amount)
    }
}

/// Wei from a decimal string.
pub fn parse_wei(s: &str) -> Result<U256> {
    U256::from_str_radix(s, 10).map_err(|e| bad(format!("amount {s:?}: {e}")))
}

/// Where obligations come from: the repo program, or a fixed list in tests and the CLI.
pub trait ObligationSource {
    fn obligations(&self, rid: &Rid) -> Result<Vec<Obligation>>;
}

/// A fixed list, the same for every repo.
#[derive(Debug, Clone, Default)]
pub struct FixedObligations(pub Vec<Obligation>);

impl ObligationSource for FixedObligations {
    fn obligations(&self, _rid: &Rid) -> Result<Vec<Obligation>> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_maps_parse_and_amounts_are_decimal_wei() {
        let v = serde_json::json!([{
            "kind": "pay", "to": "0x0000000000000000000000000000000000000001",
            "amount": "1000000000000000000000", "reason": "bounty", "key": "b1"
        }]);
        let obs: Vec<Obligation> = serde_json::from_value(v).expect("parse");
        assert!(obs[0].is_pay());
        assert_eq!(obs[0].wei().expect("wei"), U256::from(10u128.pow(21)));
        assert_eq!((obs[0].purpose.as_str(), obs[0].since_block), ("", 0));
        assert!(parse_wei("0x10").is_err() && parse_wei("-1").is_err());
        let src = FixedObligations(obs.clone());
        assert_eq!(src.obligations(&[0; 32]).expect("list"), obs);
    }
}

//! Repo programs (docs/p3/programs.md): a Rhai script pinned by hash in the identity doc,
//! folded deterministically over Arkiv-ordered ops and ref updates. Pure, no I/O.

mod engine;
mod fold;
mod value;

use serde::{Deserialize, Serialize};

pub use value::cbor_from_json;

/// The only engine tag this build runs.
pub const ENGINE_RHAI_V1: &str = "rhai/1";
/// Tag of `H(tag, source)` for program pins.
pub const HASH_TAG: &str = "program";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProgramError {
    #[error("unknown program engine {0:?} (only {ENGINE_RHAI_V1})")]
    UnknownEngine(String),
    #[error("program does not compile: {0}")]
    Compile(String),
    #[error("program has no `fn init(repo)`")]
    NoInit,
    #[error("program init failed: {0}")]
    Init(String),
    #[error("event at Arkiv order {0} arrives after a later one")]
    OutOfOrder(u64),
    #[error("program state is not serializable: {0}")]
    State(String),
}

/// Identity facts handed to `init`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    #[serde(with = "hex::serde")]
    pub rid: [u8; 32],
    #[serde(with = "value::hexes")]
    pub delegates: Vec<[u8; 32]>,
    pub quorum: u64,
    pub name: String,
}

/// A verified COB op, flattened (mirrors `VerifiedOp` + `CobOpBody` from the COB branch).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProgramOp {
    #[serde(with = "hex::serde")]
    pub id: [u8; 32],
    #[serde(with = "hex::serde")]
    pub author: [u8; 32],
    #[serde(with = "hex::serde", default)]
    pub rid: [u8; 32],
    #[serde(with = "hex::serde")]
    pub object: [u8; 32],
    #[serde(rename = "type")]
    pub type_name: String,
    pub action: String,
    #[serde(with = "value::hexes", default)]
    pub parents: Vec<[u8; 32]>,
    /// Canonical CBOR; in JSON input it is written as a plain JSON value.
    #[serde(deserialize_with = "value::json_payload")]
    pub payload: Vec<u8>,
    #[serde(default)]
    pub ts_ms: u64,
    pub arkiv_order: u64,
}

/// One ref move by one author; `old` is empty when the ref is new, `new` empty on delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdate {
    #[serde(with = "hex::serde")]
    pub author: [u8; 32],
    #[serde(rename = "ref")]
    pub name: String,
    #[serde(with = "hex::serde", default)]
    pub old: Vec<u8>,
    #[serde(with = "hex::serde")]
    pub new: Vec<u8>,
    pub arkiv_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum Event {
    Op(ProgramOp),
    Push(RefUpdate),
}

impl Event {
    pub fn order(&self) -> u64 {
        match self {
            Self::Op(o) => o.arkiv_order,
            Self::Push(p) => p.arkiv_order,
        }
    }
}

/// Result of a fold. `state_json` has sorted map keys, so it is byte-identical across clients.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub state_json: String,
    pub accepted: Vec<RefUpdate>,
    pub rejected: Vec<(RefUpdate, String)>,
    pub obligations: Vec<serde_json::Value>,
    pub logs: Vec<String>,
}

/// Which optional ABI functions the script defines.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Abi {
    on_op: bool,
    check_push: bool,
    on_push: bool,
    obligations: bool,
}

/// A compiled repo program.
#[derive(Debug, Clone)]
pub struct Program {
    ast: rhai::AST,
    abi: Abi,
}

impl Program {
    /// Compiles `source` for `engine_tag`; it must define `fn init(repo)`.
    pub fn load(engine_tag: &str, source: &str) -> Result<Self, ProgramError> {
        if engine_tag != ENGINE_RHAI_V1 {
            return Err(ProgramError::UnknownEngine(engine_tag.into()));
        }
        let ast = engine::compile(source)?;
        let has = |name: &str, arity: usize| {
            ast.iter_functions()
                .any(|f| f.name == name && f.params.len() == arity)
        };
        if !has("init", 1) {
            return Err(ProgramError::NoInit);
        }
        let abi = Abi {
            on_op: has("on_op", 2),
            check_push: has("check_push", 2),
            on_push: has("on_push", 2),
            obligations: has("obligations", 1),
        };
        Ok(Self { ast, abi })
    }

    /// `H("program", source)`, the hash an identity doc pins.
    pub fn hash(source: &str) -> [u8; 32] {
        arkiv_git_crypto::h(HASH_TAG, source.as_bytes())
    }

    /// Runs `init`, then every event in Arkiv order, then `obligations`.
    pub fn fold(
        &self,
        repo: RepoInfo,
        events: impl IntoIterator<Item = Event>,
    ) -> Result<Outcome, ProgramError> {
        fold::run(self, &repo, events)
    }
}

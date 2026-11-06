//! The fold: `init`, then each event in Arkiv order, then `obligations`.
//!
//! Failure policy: an error in `init` fails the fold; in `on_op` or `on_push` the event is
//! logged and the state is unchanged; in `check_push` the push is rejected (fail closed).

use std::collections::BTreeMap;

use rhai::{Array, Dynamic};

use crate::engine::Runtime;
use crate::value::{op_map, push_map, repo_map};
use crate::{Event, Outcome, Program, ProgramError, ProgramOp, RefUpdate, RepoInfo};

pub(crate) const NOT_FF: &str = "not a fast-forward of the accepted head";

struct Folder<'a> {
    prog: &'a Program,
    rt: Runtime,
    state: Dynamic,
    heads: BTreeMap<String, Vec<u8>>,
    accepted: Vec<RefUpdate>,
    rejected: Vec<(RefUpdate, String)>,
}

pub(crate) fn run(
    prog: &Program,
    repo: &RepoInfo,
    seed: BTreeMap<String, Vec<u8>>,
    events: impl IntoIterator<Item = Event>,
) -> Result<Outcome, ProgramError> {
    let rt = Runtime::new();
    let state = repo_map(repo)
        .and_then(|r| rt.call(&prog.ast, "init", (r,)))
        .map_err(ProgramError::Init)?;
    let mut f = Folder {
        prog,
        rt,
        state,
        heads: seed,
        accepted: vec![],
        rejected: vec![],
    };
    let mut last = 0;
    for ev in events {
        if ev.order() < last {
            return Err(ProgramError::OutOfOrder(ev.order()));
        }
        last = ev.order();
        match ev {
            Event::Op(op) => f.op(&op),
            Event::Push(p) => f.push(p),
        }
    }
    f.finish()
}

impl Folder<'_> {
    fn call(&self, name: &str, arg: Result<Dynamic, String>) -> Result<Dynamic, String> {
        self.rt
            .call(&self.prog.ast, name, (self.state.clone(), arg?))
    }

    fn op(&mut self, op: &ProgramOp) {
        if !self.prog.abi.on_op {
            return;
        }
        match self.call("on_op", op_map(op)) {
            Ok(s) => self.state = s,
            Err(e) => self
                .rt
                .log(format!("op {} skipped: {e}", hex::encode(op.id))),
        }
    }

    fn push(&mut self, p: RefUpdate) {
        let ff = match self.heads.get(&p.name) {
            Some(head) => *head == p.old,
            None => p.old.is_empty(),
        };
        let verdict = match self.prog.abi.check_push {
            true => self
                .call("check_push", push_map(&p, ff))
                .map_err(|e| format!("program error: {e}"))
                .and_then(|a| decide(a, ff)),
            false => decide(Dynamic::UNIT, ff),
        };
        if let Err(reason) = verdict {
            self.rejected.push((p, reason));
            return;
        }
        match p.new.is_empty() {
            true => self.heads.remove(&p.name),
            false => self.heads.insert(p.name.clone(), p.new.clone()),
        };
        if self.prog.abi.on_push {
            match self.call("on_push", push_map(&p, ff)) {
                Ok(s) => self.state = s,
                Err(e) => self.rt.log(format!("on_push for {} failed: {e}", p.name)),
            }
        }
        self.accepted.push(p);
    }

    fn obligations(&self) -> Vec<serde_json::Value> {
        if !self.prog.abi.obligations {
            return vec![];
        }
        let r = self
            .rt
            .call(&self.prog.ast, "obligations", (self.state.clone(),))
            .and_then(|v| v.try_cast::<Array>().ok_or("not an array".into()))
            .and_then(|a| {
                a.iter()
                    .map(|o| serde_json::to_value(o).map_err(|e| e.to_string()))
                    .collect()
            });
        r.unwrap_or_else(|e| {
            self.rt.log(format!("obligations failed: {e}"));
            vec![]
        })
    }

    fn finish(self) -> Result<Outcome, ProgramError> {
        let obligations = self.obligations();
        let state_json =
            serde_json::to_string(&self.state).map_err(|e| ProgramError::State(e.to_string()))?;
        Ok(Outcome {
            state_json,
            accepted: self.accepted,
            rejected: self.rejected,
            obligations,
            logs: self.rt.logs.take(),
            heads: self.heads,
        })
    }
}

/// `()` allows a fast-forward; `true` allows anything; a string is the reject reason.
fn decide(answer: Dynamic, ff: bool) -> Result<(), String> {
    if answer.is_unit() {
        return if ff { Ok(()) } else { Err(NOT_FF.into()) };
    }
    if let Ok(b) = answer.as_bool() {
        return if b {
            Ok(())
        } else {
            Err("rejected by program".into())
        };
    }
    match answer.into_immutable_string() {
        Ok(s) => Err(s.to_string()),
        Err(t) => Err(format!("check_push returned unsupported {t}")),
    }
}

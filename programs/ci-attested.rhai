// CI attestation.
// refs/heads/main moves only to a commit that at least need() of the
// configured attesters reported as passing (k-of-n).

// The attester ids (hex nids) and k. Edit these, then re-pin the program.
fn attesters() {
    [
        "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1",
        "a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2",
        "a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3",
    ]
}

fn need() {
    2
}

fn init(repo) {
    #{ passes: #{} }
}

fn on_op(state, op) {
    if op.type != "xyz.arkiv.attest" || op.action != "ci" {
        return state;
    }
    if !(op.author in attesters()) || op.payload.result != "pass" {
        return state;
    }
    let commit = op.payload.commit;
    let who = state.passes[commit] ?? #{};
    who[op.author] = true;
    state.passes[commit] = who;
    state
}

fn check_push(state, push) {
    if push.ref != "refs/heads/main" {
        return;
    }
    let n = (state.passes[push.head] ?? #{}).len();
    if n >= need() {
        return;
    }
    `main needs ${need()} CI passes for ${push.head}, has ${n}`
}

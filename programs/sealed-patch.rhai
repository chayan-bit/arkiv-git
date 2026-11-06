// Sealed patches.
// A commit op pins sha256(key) for a sealed object; a later reveal op
// publishes the key. The reveal counts only if the hash matches.
// Until then the committer owes a release_key obligation.

fn init(repo) {
    #{ sealed: #{}, revealed: #{} }
}

fn on_commit(state, op) {
    if !(op.object in state.sealed) {
        let p = op.payload;
        state.sealed[op.object] = #{ by: op.author, target: p.object, commitment: p.commitment };
    }
    state
}

fn on_reveal(state, op) {
    let s = state.sealed[op.object];
    if type_of(s) != "map" || op.object in state.revealed {
        return state;
    }
    if sha256_hex(op.payload.key) != s.commitment {
        print(`reveal ${op.id} rejected: key does not match the commitment`);
        return state;
    }
    state.revealed[op.object] = op.payload.key;
    state
}

fn on_op(state, op) {
    if op.type != "xyz.arkiv.sealed" {
        return state;
    }
    switch op.action {
        "commit" => on_commit(state, op),
        "reveal" => on_reveal(state, op),
        _ => state,
    }
}

fn obligations(state) {
    let out = [];
    for id in state.sealed.keys() {
        if !(id in state.revealed) {
            let s = state.sealed[id];
            out.push(#{ kind: "release_key", object: id, target: s.target, by: s.by });
        }
    }
    out
}

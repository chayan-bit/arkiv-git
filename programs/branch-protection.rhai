// Branch protection.
// refs/heads/main moves only to the head of a patch that at least
// repo.quorum delegates approved at exactly that head.
// Other refs follow the default rule (fast-forward of the accepted head).

fn init(repo) {
    #{ quorum: repo.quorum, delegates: repo.delegates, patches: #{} }
}

fn on_patch(state, op) {
    let p = state.patches[op.object];
    if type_of(p) != "map" {
        p = #{ author: op.author };
    } else if p.author != op.author {
        return state; // only the patch author may revise
    }
    p.head = op.payload.head;
    p.approvals = #{}; // a new head needs fresh approvals
    state.patches[op.object] = p;
    state
}

fn on_review(state, op) {
    let id = op.payload.patch;
    let p = state.patches[id];
    if type_of(p) == "map" && op.author in state.delegates && op.payload.head == p.head {
        p.approvals[op.author] = true;
        state.patches[id] = p;
    }
    state
}

fn on_op(state, op) {
    if op.type == "xyz.arkiv.patch" && (op.action == "create" || op.action == "revise") {
        return on_patch(state, op);
    }
    if op.type == "xyz.arkiv.review" && op.action == "approve" {
        return on_review(state, op);
    }
    state
}

fn check_push(state, push) {
    if push.ref != "refs/heads/main" {
        return;
    }
    for p in state.patches.values() {
        if p.head == push.head && p.approvals.len() >= state.quorum {
            return;
        }
    }
    `main needs a patch at ${push.head} with ${state.quorum} delegate approvals`
}

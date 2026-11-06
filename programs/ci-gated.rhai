// CI-gated branch protection (docs/p3/arkiv-ci.md).
// refs/heads/main moves only to the head of a patch that has, at exactly that head,
// (1) at least repo.quorum delegate approvals and
// (2) at least need() passing `xyz.arkiv.attest` `ci` attestations from attesters().
// Attestation payload: { head, status: "pass" | "fail", suite, log_sha256, ... }.
// An attester's latest attestation for a head wins, so a later "fail" overrides its "pass".
// Other refs follow the default rule (fast-forward of the accepted head).

// The attester ids (hex nids) and k. Edit these, then re-pin the program.
// Default: the arkiv-git CI runner z76sCb7au55NQrrbUC8YdoGoLSnwqw5vBvFzumNtjCBoi.
fn attesters() {
    ["5aa6c964c24dda968807ce8334b6183bfe7e074848ede2f26f71c024f0c2b075"]
}

fn need() {
    1
}

fn init(repo) {
    #{ quorum: repo.quorum, delegates: repo.delegates, attesters: attesters(), need: need(),
        patches: #{}, ci: #{} }
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

fn on_attest(state, op) {
    let p = op.payload;
    if !(op.author in state.attesters) || type_of(p.head) != "string"
        || !(p.status == "pass" || p.status == "fail") {
        return state;
    }
    let who = state.ci[p.head] ?? #{};
    who[op.author] = p.status;
    state.ci[p.head] = who;
    state
}

fn on_op(state, op) {
    if op.type == "xyz.arkiv.patch" && (op.action == "create" || op.action == "revise") {
        return on_patch(state, op);
    }
    if op.type == "xyz.arkiv.review" && op.action == "approve" {
        return on_review(state, op);
    }
    if op.type == "xyz.arkiv.attest" && op.action == "ci" {
        return on_attest(state, op);
    }
    state
}

fn passes(state, head) {
    let n = 0;
    for s in (state.ci[head] ?? #{}).values() {
        if s == "pass" {
            n += 1;
        }
    }
    n
}

fn check_push(state, push) {
    if push.ref != "refs/heads/main" {
        return;
    }
    let approved = false;
    for p in state.patches.values() {
        if p.head == push.head && p.approvals.len() >= state.quorum {
            approved = true;
        }
    }
    let n = passes(state, push.head);
    if approved && n >= state.need {
        return;
    }
    let missing = "";
    if !approved {
        missing = `${state.quorum} delegate approvals of a patch at this head`;
    }
    if n < state.need {
        if missing != "" {
            missing += " and ";
        }
        missing += `${state.need} passing CI attestations (has ${n})`;
    }
    `main at ${push.head} needs ${missing}`
}

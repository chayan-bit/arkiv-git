// Bounties.
// An issue create op carries a bounty; pledge ops add to it.
// When a patch merged op closes the issue and an accepted push moves main
// to that patch head, the program owes the patch author the bounty.
// A payment receipt op clears the debt. Whether the receipt tx really
// happened on chain is checked outside the program.

fn init(repo) {
    #{ bounty: #{}, patches: #{}, merged: #{}, owed: #{}, paid: #{} }
}

fn on_op(state, op) {
    let p = op.payload;
    switch op.type {
        "xyz.arkiv.issue" if op.action == "create" => {
            state.bounty[op.object] = p.bounty ?? 0;
        }
        "xyz.arkiv.bounty" if op.action == "pledge" && p.issue in state.bounty => {
            state.bounty[p.issue] += p.amount;
        }
        "xyz.arkiv.patch" if op.action == "create" => {
            state.patches[op.object] = #{ author: op.author, head: p.head };
        }
        "xyz.arkiv.patch" if op.action == "merged" && p.patch in state.patches => {
            let patch = state.patches[p.patch];
            state.merged[patch.head] = #{ to: patch.author, issue: p.closes };
        }
        "xyz.arkiv.payment" if !(p.tx in state.paid) => {
            for k in state.owed.keys() {
                let o = state.owed[k];
                if o.to == p.to && o.amount == p.amount {
                    state.owed.remove(k);
                    state.paid[p.tx] = k;
                    break;
                }
            }
        }
    }
    state
}

// Runs after an accepted push.
fn on_push(state, push) {
    let m = state.merged[push.head];
    if push.ref != "refs/heads/main" || type_of(m) != "map" {
        return state;
    }
    let amount = state.bounty[m.issue] ?? 0;
    if amount > 0 {
        let reason = `bounty for issue ${m.issue}`;
        state.owed[m.issue] = #{ kind: "pay", to: m.to, amount: amount, reason: reason };
        state.bounty[m.issue] = 0;
    }
    state.merged.remove(push.head);
    state
}

fn obligations(state) {
    state.owed.values()
}

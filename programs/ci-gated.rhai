// CI-gated branch protection (docs/p3/arkiv-ci.md).
// refs/heads/main moves only to the head of a patch that has, at exactly that head,
// (1) at least repo.quorum delegate approvals and
// (2) at least need() passing `xyz.arkiv.attest` `ci` attestations from attesters().
// Attestation payload: { head, status: "pass" | "fail", suite, log_sha256, ... }.
// An attester's latest attestation for a head wins, so a later "fail" overrides its "pass".
// Other refs follow the default rule (fast-forward of the accepted head).
// Releases (docs/p3/releases.md) are state, never a gate: state.releases[tag] is valid only
// if a delegate created it, its commit has need() CI passes and is an accepted main head
// (or an approved patch head, for pushes before the pin), and every artifact has need()
// `xyz.arkiv.attest` `artifact` attestations {tag, commit, name, sha256} with its sha256.

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
        patches: #{}, ci: #{}, main: #{}, releases: #{}, artifacts: #{} }
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

fn apply(state, op) {
    if op.type == "xyz.arkiv.patch" && (op.action == "create" || op.action == "revise") {
        return on_patch(state, op);
    }
    if op.type == "xyz.arkiv.review" && op.action == "approve" {
        return on_review(state, op);
    }
    if op.type == "xyz.arkiv.attest" && op.action == "ci" {
        return on_attest(state, op);
    }
    if op.type == "xyz.arkiv.attest" && op.action == "artifact" {
        return on_artifact(state, op);
    }
    if op.type == "xyz.arkiv.release" {
        return on_release(state, op);
    }
    state
}

fn on_op(state, op) {
    refresh(apply(state, op))
}

fn on_push(state, push) {
    if push.ref == "refs/heads/main" && push.head != "" {
        state.main[push.head] = true;
    }
    refresh(state)
}

fn is_hex(s, n) {
    if type_of(s) != "string" || s.len() != n {
        return false;
    }
    for c in s.chars() {
        if !"0123456789abcdef".contains(c) {
            return false;
        }
    }
    true
}

fn is_oid(s) {
    is_hex(s, 40) || is_hex(s, 64)
}

fn on_artifact(state, op) {
    let p = op.payload;
    if !(op.author in state.attesters) || type_of(p.tag) != "string"
        || type_of(p.name) != "string" || !is_oid(p.commit) || !is_hex(p.sha256, 64) {
        return state;
    }
    let t = state.artifacts[p.tag] ?? #{};
    let n = t[p.name] ?? #{};
    n[op.author] = #{ commit: p.commit, sha256: p.sha256 }; // latest per attester wins
    t[p.name] = n;
    state.artifacts[p.tag] = t;
    state
}

// A well-formed artifact entry, or () to ignore it.
fn clean_artifact(a) {
    if type_of(a) != "map" || type_of(a.name) != "string" || a.name == ""
        || !is_hex(a.sha256, 64) || type_of(a.size) != "i64" || a.size < 0 {
        return ();
    }
    let locs = [];
    if type_of(a.locations) == "array" {
        for l in a.locations {
            if type_of(l) == "string" && !(l in locs) {
                locs.push(l);
            }
        }
    }
    let target = if type_of(a.target) == "string" { a.target } else { "" };
    #{ name: a.name, target: target, sha256: a.sha256, size: a.size, locations: locs }
}

// New names are added; a known name only gains locations, and only if sha256 and size match.
fn merge_artifacts(r, list) {
    if type_of(list) != "array" {
        return r;
    }
    for x in list {
        let a = clean_artifact(x);
        if type_of(a) != "map" {
            continue;
        }
        let found = false;
        for i in 0..r.artifacts.len() {
            let cur = r.artifacts[i];
            if cur.name == a.name {
                found = true;
                if cur.sha256 == a.sha256 && cur.size == a.size {
                    for l in a.locations {
                        if !(l in cur.locations) {
                            cur.locations.push(l);
                        }
                    }
                    r.artifacts[i] = cur;
                }
            }
        }
        if !found {
            r.artifacts.push(a);
        }
    }
    r
}

fn on_release(state, op) {
    let p = op.payload;
    if !(op.author in state.delegates) {
        return state;
    }
    if op.action == "create" {
        if type_of(p.tag) != "string" || p.tag == "" || !is_oid(p.commit)
            || type_of(state.releases[p.tag]) == "map" {
            return state; // a tag is created once
        }
        let notes = if type_of(p.notes) == "string" { p.notes } else { "" };
        let r = #{ object: op.object, author: op.author, commit: p.commit, notes: notes,
            artifacts: [], valid: false, missing: [] };
        state.releases[p.tag] = merge_artifacts(r, p.artifacts);
    } else if op.action == "revise" {
        for tag in state.releases.keys() {
            if state.releases[tag].object == op.object {
                state.releases[tag] = merge_artifacts(state.releases[tag], p.artifacts);
            }
        }
    }
    state
}

fn approved(state, head) {
    for p in state.patches.values() {
        if p.head == head && p.approvals.len() >= state.quorum {
            return true;
        }
    }
    false
}

fn release_missing(state, tag, r) {
    let missing = [];
    let n = passes(state, r.commit);
    if n < state.need {
        missing.push(`${state.need} passing CI attestations at ${r.commit} (has ${n})`);
    }
    if !(state.main[r.commit] ?? false) && !approved(state, r.commit) {
        missing.push(`${r.commit} is neither an accepted main head nor an approved patch head`);
    }
    if r.artifacts.is_empty() {
        missing.push("at least one artifact");
    }
    let atts = state.artifacts[tag] ?? #{};
    for a in r.artifacts {
        let k = 0;
        for v in (atts[a.name] ?? #{}).values() {
            if v.sha256 == a.sha256 && v.commit == r.commit {
                k += 1;
            }
        }
        if k < state.need {
            missing.push(`artifact ${a.name} needs ${state.need} matching attestations (has ${k})`);
        }
    }
    missing
}

fn refresh(state) {
    for tag in state.releases.keys() {
        let r = state.releases[tag];
        r.missing = release_missing(state, tag, r);
        r.valid = r.missing.is_empty();
        state.releases[tag] = r;
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
    let ok = approved(state, push.head);
    let n = passes(state, push.head);
    if ok && n >= state.need {
        return;
    }
    let missing = "";
    if !ok {
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

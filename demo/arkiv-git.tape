# arkiv-git demo for ETHGlobal Mumbai 2026.
# Build first: `mac-heavy cargo build --release -p arkiv-git-cli`.
# Render from the repo root: `nix-shell -p vhs --run 'vhs demo/arkiv-git.tape'`.
# Long flows run on the in-memory Arkiv (mem: backend, dev faucet); the last scene clones the live Rabdi repo.

Output "/tmp/arkiv-git-demo.mp4"
Output "/tmp/arkiv-git-demo.gif"

Require git

Set Shell "bash"
Set FontSize 18
Set Width 1600
Set Height 900
Set Padding 24
Set TypingSpeed 18ms
Set Framerate 15
Set PlaybackSpeed 1.0
Set WaitTimeout 60s
Set WaitPattern /^\$ ?$/

Hide
Type "export PS1='$ ' R=$PWD; export PATH=$R/target/release:$PATH; unset ARKIV_ACCESS_KEY PROMPT_COMMAND" Enter
Type "export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_AUTHOR_NAME=alice GIT_AUTHOR_EMAIL=alice@example.com GIT_COMMITTER_NAME=alice GIT_COMMITTER_EMAIL=alice@example.com" Enter
Type "rm -rf /tmp/ag && mkdir -p /tmp/ag/zk && cp $R/crates/arkiv-git-zk/tests/fixtures/demo.receipt $R/crates/arkiv-git-zk/tests/fixtures/demo.public.json /tmp/ag/zk/ && cd /tmp/ag" Enter
Type "export ARKIV_GIT_BACKEND=mem:/tmp/ag/chain.log" Enter
Type `as() { local who=$1; shift; ARKIV_GIT_HOME=/tmp/ag/$who "$@"; }` Enter
Type "clear" Enter
Wait
Show

Type "# arkiv-git: git hosting on Arkiv. `as <user>` runs a command with that user's own keys." Enter
Type "# Scenes 1-5 use an in-memory Arkiv with a dev faucet (fast); scene 6 is the live Rabdi testnet." Enter
Sleep 2.5s

Type "# 1. Push and clone over arkiv::" Enter
Type "as alice arkiv-git keys init && as alice arkiv-git fund --dev 1000" Enter
Wait
Sleep 1.5s
Type "as alice arkiv-git init --name demo | tee init.txt; URL=$(awk '/url:/{print $2}' init.txt)" Enter
Wait
Sleep 1.5s
Type "git init -q -b main work && echo v0 > work/README && git -C work add -A && git -C work commit -qm 'feat: v0'" Enter
Wait
Type "as alice git -C work push $URL main" Enter
Wait
Sleep 1.5s
Type "as bob git clone -q $URL bob-clone && git -C bob-clone log --oneline" Enter
Wait
Sleep 2.5s
Type "clear" Enter
Wait

Type "# 2. A pinned Rhai program protects main: it moves only to a patch head a delegate approved" Enter
Type "as alice arkiv-git program pin --repo $URL --file $R/programs/branch-protection.rhai | grep -E 'published|program:'" Enter
Wait
Sleep 2s
Type "echo v1 > work/README && git -C work commit -qam 'feat: v1' && H=$(git -C work rev-parse HEAD)" Enter
Wait
Type "as alice git -C work push $URL main   # direct push: refused locally, no gas spent" Enter
Wait
Sleep 3s
Type "as bob arkiv-git keys init >/dev/null && as bob arkiv-git fund --dev 1000 >/dev/null" Enter
Wait
Type `P=$(as bob arkiv-git op post --repo $URL --type xyz.arkiv.patch --action create --payload-json "{\"head\":\"$H\"}")` Enter
Wait
Type `RV=$(as alice arkiv-git op post --repo $URL --type xyz.arkiv.review --action create --payload-json "{\"patch\":\"$P\"}")` Enter
Wait
Type `as alice arkiv-git op post --repo $URL --type xyz.arkiv.review --action approve --object $RV --parent $RV --payload-json "{\"patch\":\"$P\",\"head\":\"$H\"}"` Enter
Wait
Sleep 1.5s
Type "as alice git -C work push $URL main   # same push, now approved" Enter
Wait
Sleep 2s
Type "as carol git clone -q $URL carol-clone && git -C carol-clone log --oneline -1   # carol has no keys" Enter
Wait
Sleep 2s
Type "as carol arkiv-git program status --repo $URL" Enter
Wait
Sleep 4s
Type "clear" Enter
Wait

Type "# 3. Zero-knowledge receipt for a sealed patch (RISC Zero), checked offline" Enter
Type "arkiv-git zk verify --receipt zk/demo.receipt --public zk/demo.public.json" Enter
Wait
Sleep 2s
Type `sed 's/"spec_hash": "22/"spec_hash": "33/' zk/demo.public.json > zk/tampered.json   # change one public input` Enter
Wait
Type "arkiv-git zk verify --receipt zk/demo.receipt --public zk/tampered.json" Enter
Wait
Sleep 3s

Type "# 4. Timelock (drand quicknet): sealed now, nobody can open it before the round" Enter
Type "echo 'embargoed security fix' > fix.txt && arkiv-git timelock seal --until +1h --in fix.txt --out fix.tlock" Enter
Wait
Sleep 1.5s
Type "arkiv-git timelock open --in fix.tlock --out opened.txt" Enter
Wait
Sleep 3s
Type "clear" Enter
Wait

Type "# 5. Private repo: everything is sealed on Arkiv; an outsider cannot clone it" Enter
Type "PRIV=$(as alice arkiv-git init --private --name secret | awk '/url:/{print $2}') && as alice git -C work push -q $PRIV main" Enter
Wait
Sleep 1s
Type "as eve git clone $PRIV eve-clone" Enter
Wait
Sleep 3.5s
Type "clear" Enter
Wait

Type "# 6. Live on the Arkiv Rabdi testnet: the branch-protected repo from the README, no keys needed" Enter
Type "unset ARKIV_GIT_BACKEND" Enter
Type "as judge git clone -q arkiv::z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH live && git -C live log --oneline" Enter
Wait
Sleep 2s
Type "as judge arkiv-git program status --repo arkiv::z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH" Enter
Wait
Sleep 5s

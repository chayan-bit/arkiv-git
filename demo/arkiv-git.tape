# arkiv-git demo for ETHGlobal Mumbai 2026: every scene runs live on the Arkiv Rabdi testnet.
# Setup off camera first (creates the peer homes under /tmp/agl, funds alice and bob, writes the access key file):
#   ARKIV_GIT_LIVE_VIDEO=<dir> ARKIV_GIT_FUND_LOCK=<lock> demo/live-setup.sh fund
# Render from the repo root with the same <dir> in the environment:
#   ARKIV_GIT_LIVE_VIDEO=<dir> nix-shell -p vhs --run 'vhs demo/arkiv-git.tape'
# Then return the leftovers: ARKIV_GIT_LIVE_VIDEO=<dir> demo/live-setup.sh sweep

Output "/tmp/arkiv-git-demo-live.mp4"
Output "/tmp/arkiv-git-demo-live.gif"

Require git
Require curl
Require jq

Set Shell "bash"
Set FontSize 18
Set Width 1600
Set Height 900
Set Padding 24
Set TypingSpeed 12ms
Set Framerate 12
Set PlaybackSpeed 3.0
Set WaitTimeout 600s
Set WaitPattern /^\$ ?$/

Hide
Type "export PS1='$ ' PATH=/tmp/agl/.bin:$PATH RPC=https://rpc.rabdi.db-chain.testnet.arkiv.network; unset PROMPT_COMMAND ARKIV_GIT_BACKEND HISTFILE" Enter
Type ". $ARKIV_GIT_LIVE_VIDEO/access.sh && export ARKIV_GIT_DATA_LIFETIME=129600 ARKIV_GIT_CONTROL_LIFETIME=129600" Enter
Type "export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1" Enter
Type `as() { local u=$1; shift; ARKIV_GIT_HOME=/tmp/agl/.h/$u/.arkiv-git GIT_AUTHOR_NAME=$u GIT_AUTHOR_EMAIL=$u@example.com GIT_COMMITTER_NAME=$u GIT_COMMITTER_EMAIL=$u@example.com "$@"; }` Enter
Type "cd /tmp/agl && rm -rf work bob-clone carol-clone bob-vault dave-vault live init.txt && clear" Enter
Wait
Show

Type "# arkiv-git: git hosting on Arkiv. Every command below runs LIVE on the Rabdi testnet (chain 7794095)." Enter
Type "# `as <user>` runs a command with that user's own keys; alice and bob were funded off camera." Enter
Type "# Playback is sped up 3x so the real chain waits fit; nothing is pre-recorded or mocked." Enter
Sleep 9s

Type "# 1. alice creates a repo and pushes over arkiv::, bob clones it" Enter
Type "as alice arkiv-git init --name arkiv-git-live-video > init.txt && head -2 init.txt && URL=$(awk '/^url:/{print $2}' init.txt)" Enter
Wait
Sleep 4s
Type "git init -q -b main work && echo v0 > work/README && git -C work add -A && as alice git -C work commit -qm 'feat: v0'" Enter
Wait
Type "as alice git -C work push $URL main" Enter
Wait
Sleep 4s
Type "as bob git clone -q $URL bob-clone && git -C bob-clone log --oneline" Enter
Wait
Sleep 4s
Type "# the repo's identity entity, read straight from the Rabdi JSON-RPC (no client, no keys)" Enter
Type `E=$(curl -s $RPC -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"arkiv_query\",\"params\":[\"ag.name = str('arkiv-git-live-video')\",{\"limit\":1,\"select\":{\"key\":true,\"creator\":true}}]}" | jq -c '.result.data[0]') && echo "$E"` Enter
Wait
Sleep 3s
Type `echo "explorer: https://rabdi.explorer.arkiv.network/address/$(echo "$E" | jq -r .creator)"` Enter
Wait
Sleep 8s
Type "clear" Enter
Wait

Type "# 2. alice pins a Rhai program: main moves only to a patch head that a delegate approved" Enter
Type "as alice arkiv-git program pin --repo $URL --file branch-protection.rhai | grep -E 'program:'" Enter
Wait
Sleep 4s
Type "echo v1 > work/README && as alice git -C work commit -qam 'feat: v1 by alice'" Enter
Wait
Type "as alice git -C work push $URL main   # direct push by the delegate: refused before any tx" Enter
Wait
Sleep 9s
Type "git -C work reset -q --hard HEAD~1 && clear" Enter
Wait

Type "# 3. bob (not a delegate) pushes v1 to his own namespace and opens a patch" Enter
Type "echo v1 > bob-clone/README && as bob git -C bob-clone commit -qam 'feat: v1 by bob' && H=$(git -C bob-clone rev-parse HEAD)" Enter
Wait
Type "as bob git -C bob-clone push $URL main" Enter
Wait
Sleep 3s
Type `P=$(as bob arkiv-git op post --repo $URL --type xyz.arkiv.patch --action create --payload-json "{\"title\":\"v1\",\"head\":\"$H\"}") && echo "patch $P"` Enter
Wait
Sleep 3s
Type "# alice reviews and approves the patch at exactly that head" Enter
Type `RV=$(as alice arkiv-git op post --repo $URL --type xyz.arkiv.review --action create --payload-json "{\"patch\":\"$P\"}") && echo "review $RV"` Enter
Wait
Type `as alice arkiv-git op post --repo $URL --type xyz.arkiv.review --action approve --object $RV --parent $RV --payload-json "{\"patch\":\"$P\",\"head\":\"$H\"}"` Enter
Wait
Sleep 3s
Type "BOB=$(as bob arkiv-git keys show | awk '/^nid:/{print $2}') && as alice git -C work fetch -q $URL/$BOB main && git -C work merge -q --ff-only FETCH_HEAD" Enter
Wait
Type "as alice git -C work push $URL main   # same v1, now approved: accepted" Enter
Wait
Sleep 4s
Type "as carol git clone -q $URL carol-clone && git -C carol-clone log --oneline -1   # carol has no keys" Enter
Wait
Sleep 3s
Type "as carol arkiv-git program status --repo $URL | grep -E '^(program|head):'" Enter
Wait
Sleep 9s
Type "clear" Enter
Wait

Type "# 4. private repo: everything on Arkiv is sealed to an MLS group; bob is added, dave is not" Enter
Type "PRIV=$(as alice arkiv-git init --private --name vault | awk '/^url:/{print $2}') && echo $PRIV && as alice git -C work push -q $PRIV main" Enter
Wait
Sleep 3s
Type "KP=$(as bob arkiv-git member keypackage | awk '/^key package:/{print $3}') && as alice arkiv-git member add --repo $PRIV --key-package $KP --role read" Enter
Wait
Sleep 3s
Type "as bob git clone -q $PRIV bob-vault && git -C bob-vault log --oneline -1   # member: decrypts and clones" Enter
Wait
Sleep 3s
Type "as dave git clone $PRIV dave-vault   # dave has keys but is not a member" Enter
Wait
Sleep 9s
Type "clear" Enter
Wait

Type "# 5. the long-lived repo from the README: anyone clones it from Rabdi with no keys" Enter
Type "as judge git clone -q arkiv::z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH live && git -C live log --oneline" Enter
Wait
Sleep 4s
Type "as judge arkiv-git status arkiv::z5b15ZhFLNackCqiyAL5y4dMj4MFGfVrtgLmZ9PPDU1AH" Enter
Wait
Sleep 12s

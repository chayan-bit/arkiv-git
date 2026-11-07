# Live AI-agent bounty on Rabdi with Codex as the agent (docs/live/agent-bounty.md).
# Run after `setup`, funding and `alice` (its ops are read back live by `recap`), with /tmp/charter the repo checkout, /tmp/agent-demo the
# demo ROOT and ARKIV_ACCESS_KEY already exported (never typed here):
#   cd /tmp/charter && nix-shell -p vhs --run 'vhs demo/agent-bounty.tape'
# Output goes to the demo root; the rendered files belong on the demo-assets branch.
Output "/tmp/agent-demo/agent-bounty.gif"
Output "/tmp/agent-demo/agent-bounty.mp4"
Set Shell bash
Set FontSize 13
Set Width 1280
Set Height 720
Set Framerate 12
Set TypingSpeed 15ms
Set PlaybackSpeed 10

Hide
Type "export PS1='> ' MODE=live ROOT=/tmp/agent-demo && cd /tmp/charter && clear"
Enter
Show

Type "# chart on the live Rabdi testnet: a Codex agent claims a bounty through chart-mcp"
Enter
Type "demo/agent-bounty.sh recap"
Enter
Wait@5m
Type "demo/agent-bounty.sh agent"
Enter
Wait@25m
Type "demo/agent-bounty.sh merge"
Enter
Wait@15m
Type "demo/agent-bounty.sh verify"
Enter
Wait@5m
Sleep 30s

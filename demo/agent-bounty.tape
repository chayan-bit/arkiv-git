# Live AI-agent bounty on Rabdi (docs/p3/mcp.md, docs/live/agent-bounty.md).
# Run after `setup` and funding, from a shell where /tmp/arkiv-git is the repo checkout,
# /tmp/agent-demo the demo ROOT, and ARKIV_ACCESS_KEY is already exported (never typed here):
#   nix-shell -p vhs --run 'vhs demo/agent-bounty.tape'
Output demo/agent-bounty.gif
Set Shell bash
Set FontSize 14
Set Width 1300
Set Height 760
Set TypingSpeed 20ms
Set PlaybackSpeed 6

Hide
Type "export PS1='> ' MODE=live ROOT=/tmp/agent-demo && cd /tmp/arkiv-git && clear"
Enter
Show

Type "demo/agent-bounty.sh alice"
Enter
Wait@10m
Type "demo/agent-bounty.sh agent"
Enter
Wait@20m
Type "demo/agent-bounty.sh merge"
Enter
Wait@10m
Type "demo/agent-bounty.sh verify"
Enter
Wait@5m
Sleep 4s

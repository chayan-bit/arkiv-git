"""Prints a headless `codex exec --json` session as short lines: the agent's messages, its
MCP tool calls with their results (truncated), file edits and shell commands.
Reads JSONL on stdin, writes stdout."""

import json
import sys

BOLD, YELLOW, GREEN, RED, OFF = "\033[1m", "\033[33m", "\033[32m", "\033[31m", "\033[0m"


def short(v, n=110):
    s = v if isinstance(v, str) else json.dumps(v, separators=(",", ":"))
    s = " ".join(s.split())
    return s if len(s) <= n else s[: n - 3] + "..."


def result_text(item):
    if item.get("error"):
        return f"{RED}<- error{OFF} {short(item['error'].get('message', item['error']))}"
    content = (item.get("result") or {}).get("content") or []
    text = " ".join(c.get("text", "") for c in content if isinstance(c, dict))
    return f"{GREEN}<-{OFF} {short(text, 100)}"


def show(item):
    kind = item.get("type")
    if kind == "agent_message" and item.get("text", "").strip():
        print(f"{BOLD}agent:{OFF} {short(item['text'], 300)}")
    elif kind == "mcp_tool_call":
        print(f"{YELLOW}-> mcp:{item['tool']}{OFF} {short(item.get('arguments'))}")
        print(f"   {result_text(item)}")
    elif kind == "file_change":
        for c in item.get("changes") or []:
            print(f"{YELLOW}-> edit{OFF} {c.get('kind', '')} {c.get('path', '').split('/session/')[-1]}")
    elif kind == "command_execution":
        print(f"{YELLOW}-> shell{OFF} {short(item.get('command', ''))} (exit {item.get('exit_code')})")


for line in sys.stdin:
    try:
        ev = json.loads(line)
    except ValueError:
        continue
    if ev.get("type") == "item.completed":
        show(ev.get("item") or {})
    elif ev.get("type") == "turn.completed":
        u = ev.get("usage") or {}
        print(f"agent done: {u.get('input_tokens')} input, {u.get('output_tokens')} output tokens")
    elif ev.get("type") in ("turn.failed", "error"):
        print(f"{RED}agent failed:{OFF} {short(ev, 300)}")
    sys.stdout.flush()

"""Prints a headless Claude Code stream-json session as short lines: tool calls, their
results (truncated) and the agent's text. Reads stdin, writes stdout."""

import json
import sys


def short(v, n=110):
    s = v if isinstance(v, str) else json.dumps(v, separators=(",", ":"))
    s = " ".join(s.split())
    return s if len(s) <= n else s[: n - 3] + "..."


for line in sys.stdin:
    try:
        ev = json.loads(line)
    except ValueError:
        continue
    kind = ev.get("type")
    for block in (ev.get("message") or {}).get("content") or []:
        if not isinstance(block, dict):
            continue
        if kind == "assistant" and block.get("type") == "text" and block["text"].strip():
            print(f"\033[1magent:\033[0m {short(block['text'], 300)}", flush=True)
        elif kind == "assistant" and block.get("type") == "tool_use":
            name = block["name"].replace("mcp__arkiv-git__", "mcp:")
            print(f"\033[33m-> {name}\033[0m {short(block.get('input'))}", flush=True)
        elif kind == "user" and block.get("type") == "tool_result":
            body = block.get("content")
            if isinstance(body, list):
                body = " ".join(b.get("text", "") for b in body if isinstance(b, dict))
            tag = "\033[31m<- error\033[0m" if block.get("is_error") else "\033[32m<-\033[0m"
            print(f"   {tag} {short(body or '', 100)}", flush=True)
    if kind == "result":
        cost = ev.get("total_cost_usd")
        print(f"agent done: {ev.get('subtype')}, {ev.get('num_turns')} turns"
              + (f", ${cost:.2f}" if cost else ""), flush=True)

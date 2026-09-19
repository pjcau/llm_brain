#!/usr/bin/env python3
"""Replace the ```mermaid blocks marked with {/* diagram: NAME */} in the docs
with the content of diagrams/NAME.mmd. Single source of truth: diagrams/."""
import re, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
PAT = re.compile(r"(\{/\* diagram: ([\w-]+) \*/\}\n)```mermaid\n.*?```", re.S)
changed = 0
for md in ROOT.glob("docs/**/*.md"):
    s = md.read_text()
    def rep(m):
        src = ROOT / "diagrams" / f"{m.group(2)}.mmd"
        if not src.exists():
            sys.exit(f"missing {src}")
        return f"{m.group(1)}```mermaid\n{src.read_text().rstrip()}\n```"
    n = PAT.sub(rep, s)
    if n != s:
        md.write_text(n); changed += 1
print(f"updated {changed} file(s)")

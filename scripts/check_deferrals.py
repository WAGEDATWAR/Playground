#!/usr/bin/env python3
"""The deferral register check (docs/DEFERRALS.md).

Fails when:
  * an ID appears twice in the register,
  * a `// DEFERRED: DF-nnn` marker in the code names an ID that is not in the register,
  * a register entry of kind Stub has no marker in the code.
Closed entries (the "Closed" table) are allowed to have no marker.
"""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
doc = (root / "docs" / "DEFERRALS.md").read_text(encoding="utf-8")
open_part, _, closed_part = doc.partition("\n## Closed")

ids = re.findall(r"^\| (DF-\d{3}) \|", open_part, re.M)
closed = set(re.findall(r"^\| (DF-\d{3}) \|", closed_part, re.M))
stubs = set(re.findall(r"^\| (DF-\d{3}) \| Stub \|", open_part, re.M))

errors = []
for i in sorted(set(ids)):
    if ids.count(i) > 1:
        errors.append(f"{i} is listed twice in docs/DEFERRALS.md")
for i in sorted(closed & set(ids)):
    errors.append(f"{i} is both open and closed")

markers = {}
for base in ("crates", "tools"):
    for path in (root / base).rglob("*.rs"):
        if "target" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r"DEFERRED: (DF-\d{3})", text):
            markers.setdefault(m.group(1), []).append(str(path.relative_to(root)))

known = set(ids) | closed
for i, files in sorted(markers.items()):
    if i not in known:
        errors.append(f"marker {i} in {files[0]} is not in docs/DEFERRALS.md")
for i in sorted(stubs):
    if i not in markers:
        errors.append(f"{i} is a Stub in docs/DEFERRALS.md but no `DEFERRED: {i}` marker is in the code")

if errors:
    print("Deferral register problems:")
    for e in errors:
        print("  " + e)
    sys.exit(1)
print(f"deferral register ok: {len(set(ids))} open entries, {len(stubs)} stubs marked in code, {len(closed)} closed")

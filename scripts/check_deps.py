#!/usr/bin/env python3
"""Enforce the workspace dependency rule (Architecture Blueprint v2.0 §1, §2, §23.15).

Fails if a workspace crate depends on a workspace crate outside its allowed set, if anything
depends on pg-app, or if a forbidden external crate appears in a crate that must not use it.
Run from the repository root:  python scripts/check_deps.py
"""
import json
import subprocess
import sys

ALLOWED = {
    "pg-host": set(),
    "pg-host-os": {"pg-host"},
    "pg-canon": set(),
    "pg-content": {"pg-canon"},
    "pg-core": {"pg-canon", "pg-content", "pg-host"},
    "pg-api": set(),
    "pg-script": {"pg-core", "pg-api"},
    "pg-persist": {"pg-core", "pg-content", "pg-host"},
    "pg-ai": {"pg-core", "pg-content", "pg-host"},
    "pg-worldgen": {"pg-core", "pg-content"},
    "pg-ui-model": {"pg-core"},
    "pg-runtime": {"pg-core", "pg-content", "pg-script", "pg-persist", "pg-ai",
                   "pg-worldgen", "pg-ui-model", "pg-host"},
    "pg-render": {"pg-ui-model"},
    "pg-app": {"pg-runtime", "pg-render", "pg-host-os", "pg-ui-model", "pg-host", "pg-core"},
    "pg-cli": {"pg-runtime", "pg-host-os", "pg-core", "pg-content", "pg-persist", "pg-host", "pg-ai"},
}

# External crates that only specific workspace crates may use.
RESTRICTED = {
    "mlua": {"pg-script"},          # all Luau access goes through ScriptVm in pg-script
    "wgpu": {"pg-render", "pg-app"},
    "winit": {"pg-render", "pg-app"},
    "egui": {"pg-app"},
    "rand": {"pg-cli"},             # never in the deterministic core or runtime logic
}


def main() -> int:
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"], text=True))
    names = {p["name"] for p in meta["packages"]}
    errors = []

    for pkg in meta["packages"]:
        name = pkg["name"]
        if name not in ALLOWED:
            errors.append(f"{name}: not listed in scripts/check_deps.py ALLOWED (add it deliberately)")
            continue
        for dep in pkg["dependencies"]:
            dname = dep["name"]
            if dname in names:
                if dname == "pg-app":
                    errors.append(f"{name}: nothing may depend on pg-app")
                elif dname not in ALLOWED[name]:
                    errors.append(f"{name}: forbidden workspace dependency on {dname}")
            elif dname in RESTRICTED and name not in RESTRICTED[dname]:
                errors.append(f"{name}: external crate '{dname}' is restricted to {sorted(RESTRICTED[dname])}")

    for name in ALLOWED:
        if name not in names:
            errors.append(f"{name}: listed in ALLOWED but missing from the workspace")

    if errors:
        print("Dependency rule violations:")
        for e in errors:
            print("  - " + e)
        return 1
    print(f"dependency rule OK ({len(names)} crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

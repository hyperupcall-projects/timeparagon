#!/usr/bin/env python3
"""Apply Clippy's machine-applicable suggestions only to hk's selected files.

Unlike cargo clippy --fix, this never writes to other files in the workspace.
Suggestions containing multiple edits are accepted or rejected as a group.
Offsets in Rust diagnostics are UTF-8 byte offsets, not character offsets.

Usage: clippy-fix.py [--all-targets] FILE...
Without --all-targets only library and binary targets are compiled, which keeps
the pre-commit hook from building every test, example and bench.
"""

import json
from pathlib import Path
import subprocess
import sys


CLIPPY = ["cargo", "clippy", "--workspace", "--locked"]


def suggestions(message):
    spans = [s for s in message.get("spans", [])
             if s.get("suggested_replacement") is not None]
    if spans and all(s.get("suggestion_applicability") == "MachineApplicable"
                     for s in spans):
        yield spans
    for child in message.get("children", []):
        yield from suggestions(child)


def apply_fixes(messages, selected):
    originals = {p: p.read_bytes() for p in selected}
    edits = {p: set() for p in selected}
    for message in messages:
        for spans in suggestions(message):
            group = []
            for span in spans:
                path = Path(span["file_name"]).resolve()
                start, end = span["byte_start"], span["byte_end"]
                if path not in selected or not 0 <= start <= end <= len(originals[path]):
                    break
                group.append((path, start, end, span["suggested_replacement"].encode()))
            else:
                # Deduplicate diagnostics emitted for multiple Cargo targets.
                # Defer conflicting suggestions to the next Clippy pass.
                pending = {p: set(values) for p, values in edits.items()}
                for path, start, end, replacement in group:
                    edit = (start, end, replacement)
                    if edit in pending[path]:
                        continue
                    if any((start < b and a < end) or start == a
                           for a, b, _ in pending[path]):
                        break
                    pending[path].add(edit)
                else:
                    edits = pending
    changed = False
    for path, replacements in edits.items():
        if not replacements:
            continue
        contents = originals[path]
        for start, end, replacement in sorted(replacements, reverse=True):
            contents = contents[:start] + replacement + contents[end:]
        if contents != originals[path]:
            if path.read_bytes() != originals[path]:
                raise RuntimeError(f"{path} changed while Clippy was running")
            path.write_bytes(contents)
            print(f"Clippy fixed {path}", file=sys.stderr)
            changed = True
    return changed


def main():
    args = sys.argv[1:]
    clippy = CLIPPY + ["--all-targets"] if "--all-targets" in args else CLIPPY
    selected = {Path(f).resolve() for f in args if f != "--all-targets"
                and f.endswith(".rs") and Path(f).is_file() and not Path(f).is_symlink()}
    # Re-run after fixes so offsets stay valid and newly exposed suggestions can
    # be applied. A final strict check reports all remaining warnings/errors.
    for _ in range(4):
        result = subprocess.run(clippy + ["--message-format=json"],
                                stdout=subprocess.PIPE, check=False)
        messages = []
        for line in result.stdout.splitlines():
            event = json.loads(line)
            if event.get("reason") == "compiler-message":
                messages.append(event["message"])
        if result.returncode:
            for message in messages:
                print(message.get("rendered") or message["message"], file=sys.stderr)
            return result.returncode
        if not apply_fixes(messages, selected):
            break
    return subprocess.run(clippy + ["--", "-D", "warnings"], check=False).returncode


if __name__ == "__main__":
    sys.exit(main())

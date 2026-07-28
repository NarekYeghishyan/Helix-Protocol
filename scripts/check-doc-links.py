#!/usr/bin/env python3
"""Verify every intra-repository markdown link resolves — file *and* anchor.

Broken links are the quiet way documentation stops being trustworthy: nothing
fails, the reader just lands somewhere unhelpful and stops believing the rest.
This existed because sixteen anchors were silently broken by a heading edit —
finding statuses were written into the headings (`### F-8 ... *(Medium, fixed)*`),
so every link to a finding broke the moment its status changed. Statuses now live
on their own line and the headings are stable.

Anchors follow GitHub's slug rules: strip inline formatting, lowercase, drop
anything that is not a letter, digit, space, hyphen or underscore, then replace
each space with a hyphen — runs of spaces are *not* collapsed, which is why
`F-2 — Reward liability` slugs to `f-2--reward-liability`.

    python3 scripts/check-doc-links.py        # from the repository root
"""

import os
import re
import sys

SKIP_DIRS = {".git", "target", "node_modules", ".vscode"}


def slugify(heading: str) -> str:
    text = heading.strip()
    text = re.sub(r"`([^`]*)`", r"\1", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = text.replace("**", "").replace("*", "").replace("_", " ")
    text = text.lower()
    text = re.sub(r"[^\w\s-]", "", text, flags=re.UNICODE)
    return text.replace(" ", "-")


def headings(path: str) -> list[str]:
    found, fenced = [], False
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            if line.lstrip().startswith("```"):
                fenced = not fenced
                continue
            if fenced:
                continue
            match = re.match(r"^(#{1,6})\s+(.*)", line)
            if match:
                found.append(slugify(match.group(2)))
    return found


def main() -> int:
    root = os.getcwd()
    docs = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        docs += [os.path.join(dirpath, f) for f in filenames if f.endswith(".md")]

    slugs = {path: headings(path) for path in docs}
    problems = []

    for path in docs:
        rel = os.path.relpath(path, root)
        with open(path, encoding="utf-8") as handle:
            text = handle.read()

        for match in re.finditer(r"\[[^\]]*\]\(([^)]+)\)", text):
            href = match.group(1).strip()
            if not href or href.startswith(("http://", "https://", "mailto:", "#!")):
                continue

            file_part, _, fragment = href.partition("#")
            target = (
                os.path.normpath(os.path.join(os.path.dirname(path), file_part))
                if file_part
                else path
            )

            if not os.path.exists(target):
                problems.append(f"{rel}: missing file -> {href}")
                continue

            # Only markdown targets have anchors worth checking; a fragment on a
            # source file is a line reference, which is not ours to validate.
            if fragment and target.endswith(".md"):
                if fragment not in slugs.get(target, headings(target)):
                    problems.append(f"{rel}: missing anchor -> {href}")

    for problem in problems:
        print(f"  {problem}")
    print(f"{len(problems)} broken link(s) across {len(docs)} markdown files")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

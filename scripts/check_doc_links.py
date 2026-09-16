#!/usr/bin/env python3
"""Doc link gate: every relative markdown link or backtick-quoted repo path in
projects *.md must resolve.

Checks inline links ``[text](path)`` and backtick-quoted paths like
``engineering/DECISIONS.md`` anywhere in *.md files (fenced code blocks are
skipped). Resolution order: relative to the referencing file, then relative
to the repo root, then a plain basename search (for odd renames like
`LICENSE-CC0-CCC`). Site-relative paths (`/docs/x`, http(s), mailto, `#`)
and parameterized paths (`$VAR`, `{...}`) are skipped.

Lines that quote *source* filenames as provenance are exempt when the same
line carries a disposition word (``digested``, ``deleted``, ``subsumed``,
``moved``) or the ``(was `old-name`)*`` rewrite marker; journal
``<!-- src: ... -->`` tags and per-file deletion tables are also exempt —
they record the deleted inventory by construction.
"""
import os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIR_SKIP = {".git", "target", "node_modules", ".venv", "__pycache__"}
ROOT_SEGMENTS = {d for d in os.listdir(ROOT) if os.path.isdir(os.path.join(ROOT, d)) and d not in DIR_SKIP}

FENCE = re.compile(r"^\s*```")
LINK = re.compile(r"\[[^\]]{0,200}\]\(([^)\s]{1,200})\)")
TICK = re.compile(r"`([A-Za-z0-9_./{}+-]{1,180}\.(?:md|sh|py|c|h|rs|json|txt|csv|ebnf|yml|yaml|toml|s|S|jsonl|lock))`")

SITE_OR_PARAM = re.compile(r"^(https?://|mailto:|#|/|ftp://)")
PARAM_CHARS = re.compile(r"[$*{}%~]|\\\s|\(")
DISPOSITION = re.compile(
    r"digested|wiped|deleted|subsumed|was `[^`]+`|folded from|folded into|"
    r"pinned like its source|was\s+deleted")
SRC_TAG = re.compile(r"^<!--\s*src:\s")
TABLE_ROW = re.compile(r"\|\s*(?:`[^\n`]+`\s*)*(?:digested|wiped|deleted|subsumed|→)", re.I)


def resolvable(rel, base):
    if SITE_OR_PARAM.match(rel) or PARAM_CHARS.search(rel):
        return True
    if os.path.isabs(rel):
        return os.path.isfile(rel) or os.path.isdir(rel)
    cands = [os.path.normpath(os.path.join(base, rel)),
             os.path.normpath(os.path.join(ROOT, rel))]
    if any(os.path.isfile(c) or os.path.isdir(c) for c in cands):
        return True
    base_s = rel.split("/", 1)[0]
    if base_s.startswith(("http", "www", "ftp")):
        return True
    name = os.path.basename(rel)
    if "/" not in rel:  # bare filename: last-resort basename hunt
        for bp, dirs, files in os.walk(ROOT):
            dirs[:] = [d for d in dirs if d not in DIR_SKIP]
            if name in files:
                return True
    return False


def check_file(path):
    bad, fenced = [], False
    base = os.path.dirname(path)
    journal_mode = f"{os.sep}journal{os.sep}" in path
    prev = ""
    with open(path, encoding="utf-8") as fh:
        for i, line in enumerate(fh, 1):
            is_table = line.lstrip().startswith("|")
            if FENCE.match(line):
                fenced = not fenced
                continue
            if fenced or SRC_TAG.match(line) or "dl-skip" in line:
                prev = line
                continue
            exempt = bool(DISPOSITION.search(line)) or (prev.strip().endswith(")+") and DISPOSITION.search(prev)) or bool(DISPOSITION.search(prev) and line.strip() and not line.lstrip().startswith("#"))
            for m in LINK.finditer(line):
                target = m.group(1).split("#", 1)[0].split("?", 1)[0]
                if not exempt and target and not resolvable(target, base):
                    bad.append((i, m.group(1)))
            if is_table or journal_mode:  # table cells are path shorthands; journals cite deleted sources
                prev = line
                continue
            for m in TICK.finditer(line):
                s = m.group(1).split("#", 1)[0].split("?", 1)[0]
                if not s or exempt:
                    continue
                first = s.split("/", 1)[0]
                if s.startswith(("./", "../")) or first in ROOT_SEGMENTS:
                    if not resolvable(s, base):
                        bad.append((i, s))
            prev = line
    return bad


def main():
    fails = 0
    for bp, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in DIR_SKIP]
        for fn in files:
            if not fn.endswith(".md"):
                continue
            p = os.path.join(bp, fn)
            for ln, ref in check_file(p):
                fails += 1
                print(f"{p.replace(ROOT + '/', '')}:{ln}: broken reference `{ref}`",
                      file=sys.stderr)
    if fails:
        print(f"\ndoc-link gate FAILED: {fails} broken reference(s)", file=sys.stderr)
        sys.exit(1)
    print("doc-link gate OK: all markdown references resolve")


if __name__ == "__main__":
    main()

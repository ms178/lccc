#!/usr/bin/env python3
"""edg_changes_mine.py — mine EDG's src/Changes into C-knowledge artifacts.

EDG's `src/Changes` (202 940 lines, 13 583 dated entries, 1992–2026) is the
densest known record of "what actually bites when implementing C/C++":
every entry is a resolved bug with IDs, standard references (WG14/SG15
N-papers), and before/after semantics.  LCCC needs the C slice of that
knowledge — transplant item E7 in docs/EDG_TRANSPLANT_ANALYSIS.md.

Outputs (written under docs/ by default):

  edg_changes_c_index.md      every entry: date, bug IDs, title, C-score
  edg_changes_c_extract.md    full bodies of the C-relevant entries
  edg_changes_stats.json      aggregate counts (decades, keywords, years)

Classification is keyword-scored (see C_KEYWORDS / CPP_ONLY_KEYWORDS): an
entry is C-relevant if it hits C keywords, or hits no C++-only keyword at
all and mentions generic-implementation topics (integer constants, bit
fields, initializers, …).  `--min-score` tunes recall/precision.

Usage:
  scripts/edg_changes_mine.py --edg-root /home/user/edg-compiler
  scripts/edg_changes_mine.py --edg-root ... --min-score 2 --out-dir docs
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HEADER_RE = re.compile(
    r"^(\d{1,2}/\d{1,2}/\d{2,4})\s+\[([^\]]*)\]?\s*$")
HEADER_START_OPEN_RE = re.compile(
    r"^(\d{1,2}/\d{1,2}/\d{2,4})\s+\[([^\]]*)$")

# Topics that make an entry relevant to a C compiler implementation.
C_KEYWORDS = {
    "c23": 3, "c17": 3, "c11": 3, "c99": 3, "c90": 2, "c18": 2, "c2x": 3,
    "c mode": 2, "in c": 1, "gnu c": 2, "iso c": 2, "the c standard": 2,
    "wg14": 3, "c compiler": 2, "c language": 2, "c source": 1,
    "bit field": 2, "bit-field": 2, "flexible array": 3,
    "compound literal": 3, "designated initializer": 3,
    "variable-length array": 3, "vla": 3, "statement expression": 3,
    "__builtin": 2, "_static_assert": 3, "static assert": 2,
    "typeof": 2, "__auto_type": 3, "restrict": 2, "tentative": 2,
    "string literal": 1, "character constant": 1, "integer constant": 1,
    "floating-point constant": 1, "hexadecimal floating": 2,
    "preprocessor": 2, "macro expansion": 2, "macro": 1,
    "conditional compilation": 2, "include": 1,
    "anonymous union": 2, "anonymous struct": 2,
    "initialization": 1, "initializer": 2, "aggregate": 1,
    "enum": 1, "enumerator": 1, "plain char": 2, "signed char": 1,
    "complex": 1, "imaginary": 1, "_generic": 3, "generic selection": 3,
    "atomic": 1, "alignment": 1, "offsetof": 2, "va_arg": 2,
    "variadic": 1, "old-style": 2, "k&r": 3, "implicit int": 2,
    "integer promotion": 2, "usual arithmetic conversions": 3,
    "sequence point": 3, "unsequenced": 3, "undefined behavior": 1,
    "implementation-defined": 2, "locale": 1, "trigraph": 2,
    "digraph": 2, "utf-8": 1, "unicode": 1, "identifier": 1,
    "linkage": 1, "toplevel": 1, "inline": 1, "typeof": 2,
    "sizeof": 1, "alignof": 2, "_alignas": 2, "alignas": 2,
}

# Topics that mark an entry as C++-only (down-weight unless C topics hit).
CPP_ONLY_KEYWORDS = {
    "template": 2, "constexpr": 2, "consteval": 3, "concept": 3,
    "requires": 2, "namespace": 2, "lambda": 2, "coroutine": 3,
    "module": 2, "import": 1, "co_await": 3, "co_yield": 3,
    "operator<=>": 3, "spaceship": 3, "overload resolution": 2,
    "class template": 3, "specialization": 2, "instantiation": 2,
    "virtual function": 2, "vtable": 2, "destructor": 1, "constructor": 1,
    "exception": 1, "try block": 2, "catch": 1, "throw": 1,
    "std::": 2, "decltype": 2, "typename": 2, "nullptr": 1,
    "friend": 2, "private": 1, "protected": 1, "public": 1,
    "c++": 1, "cxx": 1, "cli": 2, "c++/cx": 3,
}


def parse_entries(text: str) -> list[dict]:
    entries: list[dict] = []
    cur: dict | None = None
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        m = HEADER_RE.match(line)
        if m and (m.group(2) is not None and
                  (line.rstrip().endswith("]") or "[" not in line)):
            if cur is not None:
                entries.append(cur)
            cur = {"date": m.group(1), "ids": m.group(2),
                   "title": "", "body": []}
            i += 1
            continue
        m2 = HEADER_START_OPEN_RE.match(line)
        if m2 and not line.rstrip().endswith("]"):
            # Multi-line bug-ID list: "[id,id,\n  id,id]" (red-team fix).
            ids = [m2.group(2).rstrip(",")]
            i += 1
            while i < len(lines):
                piece = lines[i].strip()
                i += 1
                if piece.endswith("]"):
                    ids.append(piece[:-1].rstrip(","))
                    break
                ids.append(piece.rstrip(","))
            if cur is not None:
                entries.append(cur)
            cur = {"date": m2.group(1),
                   "ids": ",".join(x.strip() for x in ids if x.strip()),
                   "title": "", "body": []}
            continue
        if cur is None:
            i += 1
            continue
        if not cur["title"]:
            if line.strip():
                cur["title"] = line.strip()
            i += 1
            continue
        cur["body"].append(line)
        i += 1
    if cur is not None:
        entries.append(cur)
    for e in entries:
        # Trim trailing blank lines from bodies.
        body = e["body"]
        while body and not body[-1].strip():
            body.pop()
        e["body"] = "\n".join(body)
    return entries


def score_entry(e: dict) -> tuple[int, list[str]]:
    hay = (e["title"] + "\n" + e["body"]).lower()
    hits, score = [], 0
    for kw, w in C_KEYWORDS.items():
        if kw in hay:
            hits.append(kw)
            score += w
    for kw, w in CPP_ONLY_KEYWORDS.items():
        if kw in hay:
            score -= w
    return score, hits


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--edg-root", type=Path,
                    default=Path("/home/user/edg-compiler"))
    ap.add_argument("--out-dir", type=Path, default=None,
                    help="default: <repo>/docs")
    ap.add_argument("--min-score", type=int, default=2,
                    help="C-relevance threshold (default 2)")
    args = ap.parse_args(argv)

    changes = args.edg_root / "src" / "Changes"
    if not changes.is_file():
        print(f"error: {changes} not found", file=sys.stderr)
        return 2
    repo = Path(__file__).resolve().parent.parent
    out_dir = (args.out_dir or repo / "docs").resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    entries = parse_entries(changes.read_text(encoding="utf-8",
                                              errors="replace"))
    kept: list[dict] = []
    index_lines = [
        "# EDG `src/Changes` — complete entry index (mined %d entries)" % len(entries),
        "",
        "Generated by `scripts/edg_changes_mine.py` from edgcpp/compiler "
        "`src/Changes`. Columns: date · score · bug IDs · title.  The "
        "C-relevant subset is expanded in `edg_changes_c_extract.md`.",
        "",
    ]
    score_hist: dict[int, int] = {}
    years: dict[str, int] = {}
    for e in entries:
        score, hits = score_entry(e)
        score_hist[score] = score_hist.get(score, 0) + 1
        y = e["date"].split("/")[-1]
        years[y] = years.get(y, 0) + 1
        e["score"], e["hits"] = score, hits
        title = e["title"].replace("|", "\\|")
        index_lines.append(f"| {e['date']} | {score:+d} | {e['ids']} | {title} |")
        if score >= args.min_score:
            kept.append(e)

    (out_dir / "edg_changes_c_index.md").write_text(
        "\n".join(index_lines) + "\n", encoding="utf-8")

    extract = [
        "# EDG `src/Changes` — C-relevant entries (full text)",
        "",
        f"Generated by `scripts/edg_changes_mine.py --min-score "
        f"{args.min_score}`; {len(kept)} of {len(entries)} entries qualify. "
        "Each entry is a resolved EDG bug with IDs and before/after "
        "semantics — read this as a checklist of C implementation edge "
        "cases before touching LCCC's frontend, const-eval or layout code. "
        "Apache-2.0 WITH LLVM-exception (edgcpp/compiler); quotes kept "
        "verbatim with attribution.",
        "",
        "---",
        "",
    ]
    for e in kept:
        extract.append(f"## {e['date']} · `{e['ids']}`")
        extract.append("")
        extract.append(f"**{e['title']}**  ")
        extract.append(f"*C-score {e['score']:+d}; hits: "
                       f"{', '.join(e['hits'][:12])}*")
        extract.append("")
        extract.append("```text")
        extract.append(e["body"])
        extract.append("```")
        extract.append("")
    (out_dir / "edg_changes_c_extract.md").write_text(
        "\n".join(extract) + "\n", encoding="utf-8")

    (out_dir / "edg_changes_stats.json").write_text(json.dumps(
        {"entries_total": len(entries), "entries_kept": len(kept),
         "min_score": args.min_score,
         "score_histogram": {str(k): v for k, v in sorted(score_hist.items())},
         "entries_per_year": dict(sorted(years.items()))},
        indent=2, sort_keys=True) + "\n", encoding="utf-8")

    b_extract = (out_dir / "edg_changes_c_extract.md").stat().st_size
    b_index = (out_dir / "edg_changes_c_index.md").stat().st_size
    print(f"entries: {len(entries)} total, {len(kept)} C-relevant "
          f"(min-score {args.min_score})")
    print(f"extract: {b_extract} bytes  index: {b_index} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())

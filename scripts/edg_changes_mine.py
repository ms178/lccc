#!/usr/bin/env python3
"""edg_changes_mine.py — mine EDG's src/Changes into C-knowledge artifacts.

EDG's `src/Changes` (202 940 lines, 1992–2026) is the densest known record of
"what actually bites when implementing C/C++": every entry is a resolved bug
with IDs, standard references (WG14/SG15 N-papers), and before/after
semantics.  LCCC needs the C slice of that knowledge — transplant item E7 in
docs/EDG_TRANSPLANT_ANALYSIS.md.

Entry grammar (two eras — PR #719 review finding F1; the pre-2008 era was
silently swallowed by the first revision of this tool):

  bracketed (8/7/08 onward):
      M/D/YY  [EDGcpfe/NNNNN, ...]        (bug-ID lists may wrap lines)
      Title line (may wrap: consecutive non-blank lines up to first blank)
      blank line
      body...

  unbracketed (4/30/92 .. 8/7/08):
      M/D/YY   Title text                (date + 2+ spaces + title)
      (title may wrap the same way)
      blank line
      body...

Scoring uses ANCHORED regexes with word boundaries and merged overlap groups
(review finding F2: raw substring matching scored "GNU C++" as C-relevant and
double-counted initializer/initialization).

Outputs (docs/ by default):
  edg_changes_c_index.md      all entries: ISO date, score, bug IDs, title
  edg_changes_c_extract.md    full bodies of C-relevant entries
  edg_changes_stats.json      aggregates (years, decades, scores, sanity)

Usage:
  scripts/edg_changes_mine.py --edg-root /home/user/edg-compiler
  scripts/edg_changes_mine.py selftest
  scripts/edg_changes_mine.py --edg-root ... --min-score 2 --out-dir docs
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

# Bracketed header: "8/11/08  [EDGcpfe/9141]" — bug-ID list may wrap.
BRACKET_OPEN_RE = re.compile(
    r"^(\d{1,2}/\d{1,2}/\d{2,4})\s+\[([^\]]*)$")
BRACKET_CLOSED_RE = re.compile(
    r"^(\d{1,2}/\d{1,2}/\d{2,4})\s+\[([^\]]*)\]\s*$")
# Unbracketed header: "8/7/08   Constant expressions ..." — date padded to
# a ~9-column field then the title.  One-space cases exist ("12/22/07
# Anachronisms ..."); body-wrap references like "4/14/17 for EDGcpfe/..." or
# "10/5/01 describing ..." are rejected by the connector-word rule plus the
# blank-line-before rule (review finding F1 follow-through).
PLAIN_HEADER_RE = re.compile(r"^(\d{1,2}/\d{1,2}/\d{2,4})\s{1,}(\S.*)$")
# Body-wrap reference starters (mid-sentence continuations after a date).
PLAIN_WRAP_RE = re.compile(
    r"^(?:for|and|or|entry|see|per|from|with|on|the|describing|but|which|"
    r"however|when|where|this|that|it|was|is|are|has|have|had|to|of|in\s+the)\b")


def _plausible_date(date: str, allow_typos: bool = False) -> bool:
    try:
        m, d, y = date.split("/")
        m, d = int(m), int(d)
        if y.isdigit() and len(y) == 3 and not allow_typos:
            return False  # upstream typo years (e.g. 10/25/127) — see below
        return 1 <= m <= 12 and 1 <= d <= 31
    except ValueError:
        return False


def _is_plain_header(line: str, prev_blank: bool) -> re.Match | None:
    """A plain (unbracketed) header: blank line before, plausible date, and
    not a mid-sentence body-wrap reference ("... 4/14/17 for EDGcpfe/...").
    No column-padding rule: early-1990s entries pad less than later ones
    ("6/7/93 Variable size array representation" is a real header)."""
    if not prev_blank:
        return None
    m = PLAIN_HEADER_RE.match(line)
    if not m or not _plausible_date(m.group(1)):
        return None
    if PLAIN_WRAP_RE.match(m.group(2)):
        return None
    return m


# Upstream date typos adjudicated by reverse-chronological neighbors
# (documented in docs/edg_changes_stats.json as odd_years_flagged when
# encountered off-table).  Full-date match -> corrected 4-digit year.
DATE_TYPOS = {
    "10/25/127": "2017",  # EDGcpfe/18864; sits among 2017 entries
    "10/1/37": "1997",    # between 10/2/97 and later 1997 entries
    "5/9/69": "1996",     # directly after 6/1/96
    "8/3/82": "1992",     # directly after 8/4/92
}


def normalize_year(date: str) -> str:
    """Return the 4-digit year, repairing known upstream typos.

    2-digit years follow the corpus reality: 92-96 -> 19xx (EDG starts 1992),
    otherwise 20xx.
    """
    if date in DATE_TYPOS:
        return DATE_TYPOS[date]
    y = date.split("/")[-1]
    if len(y) == 2:
        n = int(y)
        return f"19{n:02d}" if 92 <= n <= 99 else f"20{n:02d}"
    return y if len(y) == 4 else f"20{y[-2:]:02d}"


def parse_entries(text: str) -> list[dict]:
    """Parse both header eras; join wrapped titles (review findings F1, F7b)."""
    entries: list[dict] = []
    lines = text.splitlines()
    n = len(lines)
    i = 0
    prev_blank = True

    def finish(e: dict) -> None:
        body = e["body"]
        while body and not body[-1].strip():
            body.pop()
        while body and not body[0].strip():
            body.pop(0)
        e["body"] = "\n".join(body)
        e["year"] = normalize_year(e["date"])
        entries.append(e)

    while i < n:
        line = lines[i]
        cur: dict | None = None
        m_closed = BRACKET_CLOSED_RE.match(line)
        m_open = BRACKET_OPEN_RE.match(line)
        m_plain = PLAIN_HEADER_RE.match(line)
        if m_closed and _plausible_date(m_closed.group(1), allow_typos=True):
            cur = {"date": m_closed.group(1), "ids": m_closed.group(2),
                   "title": "", "body": []}
            i += 1
        elif (m_open and not line.rstrip().endswith("]")
              and _plausible_date(m_open.group(1), allow_typos=True)):
            ids = [m_open.group(2).rstrip(",")]
            i += 1
            while i < n:
                piece = lines[i].strip()
                i += 1
                if piece.endswith("]"):
                    ids.append(piece[:-1].rstrip(","))
                    break
                ids.append(piece.rstrip(","))
            cur = {"date": m_open.group(1),
                   "ids": ",".join(x.strip() for x in ids if x.strip()),
                   "title": "", "body": []}
        elif _is_plain_header(line, prev_blank):
            m_plain = PLAIN_HEADER_RE.match(line)
            cur = {"date": m_plain.group(1), "ids": "",
                   "title": m_plain.group(2).strip(), "body": []}
            i += 1
        else:
            prev_blank = not line.strip()
            i += 1
            continue

        # Title block: consecutive non-blank lines up to the first blank line
        # (titles wrap flush-left; the structural blank line delimits the body).
        while i < n and lines[i].strip():
            cur["title"] = (cur["title"] + " " + lines[i].strip()).strip()
            i += 1
        # Body runs until the next header of either era.
        while i < n:
            nxt = lines[i]
            if ((BRACKET_CLOSED_RE.match(nxt) or BRACKET_OPEN_RE.match(nxt))
                    and _plausible_date(nxt.split("[")[0].strip(),
                                        allow_typos=True)):
                break
            if _is_plain_header(nxt, not lines[i - 1].strip()):
                break
            cur["body"].append(nxt)
            i += 1
        finish(cur)
        prev_blank = True
    return entries


# ---------------------------------------------------------------------------
# Anchored scoring (review finding F2).  Each rule: (compiled regex, weight).
# Overlap groups are merged into single patterns so one token cannot
# double-score (initializer/initialization, enum/enumerator, alignas,
# static-assert).
# ---------------------------------------------------------------------------
C_RULES: list[tuple[re.Pattern, int]] = [
    (re.compile(r"\bc23\b|\bc2x\b|\bc18\b"), 3),
    (re.compile(r"\bc17\b|\bc11\b|\bc99\b|\bc90\b"), 3),
    (re.compile(r"\bwg14\b"), 3),
    (re.compile(r"\bin c\b(?!\+\+)"), 1),
    (re.compile(r"\bgnu c\b(?!\+\+)"), 2),
    (re.compile(r"\biso c\b(?!\+\+)"), 2),
    (re.compile(r"\bc mode\b"), 2),
    (re.compile(r"\bc language\b|\bc compiler\b|\bc source\b"), 2),
    (re.compile(r"\bbit[- ]fields?\b"), 2),
    (re.compile(r"\bflexible array\b"), 3),
    (re.compile(r"\bcompound literal\b"), 3),
    (re.compile(r"\bdesignated initializer\b"), 3),
    (re.compile(r"\bvariable[- ]length arrays?\b|\bvla\b"), 3),
    (re.compile(r"\bstatement expression\b"), 3),
    (re.compile(r"\b__builtin\w*"), 2),
    # NOTE: the haystack is lowercased (`hay = ...lower()`); every pattern
    # literal here must therefore be lowercase or it can never match (review
    # P2: `_Static_assert`/`_Complex`/`_Generic` were silent dead rules).
    (re.compile(r"\b_static_assert\b|\bstatic_assert\b|\bstatic assert\b"), 3),
    (re.compile(r"\btypeof\b|\b__auto_type\b"), 2),
    (re.compile(r"\brestrict\b|\b__restrict__?\b"), 2),
    (re.compile(r"\btentative\b"), 2),
    (re.compile(r"\bstring literal\b"), 1),
    (re.compile(r"\bcharacter constant\b"), 1),
    (re.compile(r"\binteger constant\b"), 1),
    (re.compile(r"\bfloating[- ]point constant\b|\bhexadecimal floating\b"), 1),
    (re.compile(r"\bpreprocessor\b"), 2),
    (re.compile(r"\bmacro expansion\b|\bmacro\b"), 1),
    (re.compile(r"\bconditional compilation\b"), 2),
    (re.compile(r"#\s*include\b|\binclude file\b"), 1),
    (re.compile(r"\banonymous (?:union|struct)\b"), 2),
    (re.compile(r"\binitializer\b|\binitialization\b"), 2),
    (re.compile(r"\benumerator\b|\benumeration\b|\benum\b"), 1),
    (re.compile(r"\bplain char\b"), 2),
    (re.compile(r"\b(?:_complex|__complex__)\b|\bcomplex (?:type|number|"
                r"float|double|long|int|arithmetic)\b"), 1),
    (re.compile(r"\b_generic\b|\bgeneric selection\b"), 3),
    (re.compile(r"\batomic\b"), 1),
    (re.compile(r"\boffsetof\b"), 2),
    (re.compile(r"\bva_arg\b"), 2),
    (re.compile(r"\bvariadic\b"), 1),
    (re.compile(r"\bold-style\b|\bk&r\b"), 2),
    (re.compile(r"\bimplicit int\b"), 2),
    (re.compile(r"\binteger promotion\b"), 2),
    (re.compile(r"\busual arithmetic conversions\b"), 3),
    (re.compile(r"\bsequence point\b|\bunsequenced\b"), 3),
    (re.compile(r"\bundefined behavior\b"), 1),
    (re.compile(r"\bimplementation-defined\b"), 2),
    (re.compile(r"\btrigraph|\bdigraph\b"), 2),
    (re.compile(r"\butf-8\b|\bunicode\b"), 1),
    (re.compile(r"\bidentifier\b"), 1),
    (re.compile(r"\blinkage\b"), 1),
    (re.compile(r"\binline\b"), 1),
    (re.compile(r"\bsizeof\b|\balignof\b"), 1),
    (re.compile(r"\b_?alignas\b"), 2),
]

CPP_RULES: list[tuple[re.Pattern, int]] = [
    (re.compile(r"\bc\+\+"), 3),
    (re.compile(r"\btempl(?:ate|ates)\b"), 2),
    (re.compile(r"\bconstexpr\b|\bconsteval\b|\bconstinit\b"), 2),
    (re.compile(r"\bconcept(?:s)?\b|\brequires\b"), 3),
    (re.compile(r"\bnamespace\b"), 2),
    (re.compile(r"\blambda\b"), 2),
    (re.compile(r"\bcoroutine|\bco_await\b|\bco_yield\b|\bco_return\b"), 3),
    (re.compile(r"\bmodules?\b|\bimport\b"), 2),
    (re.compile(r"\boperator<=>\b|\bspaceship\b"), 3),
    (re.compile(r"\bspecialization\b|\binstantiation\b"), 2),
    (re.compile(r"\bvirtual function\b|\bvtable\b|\bvftable\b"), 2),
    (re.compile(r"\bconstructor\b|\bdestructor\b"), 1),
    (re.compile(r"\bexception\b|\bcatch\b|\bthrow\b|\btry block\b"), 1),
    (re.compile(r"\bstd::"), 2),
    (re.compile(r"\bdecltype\b|\btypename\b|\bnullptr\b"), 2),
    (re.compile(r"\bfriend\b"), 2),
    (re.compile(r"\bprivate\b|\bprotected\b"), 1),
    (re.compile(r"\bc\+\+/cli\b|\bcli\b|\bc\+\+/cx\b"), 3),
]


def score_entry(e: dict) -> tuple[int, list[str]]:
    hay = (e["title"] + "\n" + e["body"]).lower()
    hits: list[str] = []
    score = 0
    for pat, w in C_RULES:
        if pat.search(hay):
            hits.append(pat.pattern[:28])
            score += w
    for pat, w in CPP_RULES:
        if pat.search(hay):
            score -= w
    return score, hits


def parse_entries_selftest() -> int:
    """Grammar + scoring unit tests (no EDG tree needed)."""
    sample = (
        "Version 7.0, September 28, 2026\n"
        "\n"
        "9/15/26  [EDGcpfe/25951,EDGcpfe/29043]\n"
        "C23: New tag compatibility\n"
        "rules\n"
        "\n"
        "In C23 mode structs match.\n"
        "\n"
        "8/19/26  [EDGcpfe/25497,EDGcpfe/26173,\n"
        "EDGcpfe/26947]\n"
        "Multi id header\n"
        "\n"
        "Body here.\n"
        "\n"
        "8/7/08   Constant expressions unconditionally\n"
        "recorded in IL\n"
        "\n"
        "Old-style body text.\n"
        "\n"
        "4/14/17 for EDGcpfe/17414 and EDGcpfe/17706).  However, the\n"
        "front end continued past the wrap.\n"
        "\n"
        "5/4/92   Support for access declarations of overloaded functions\n"
        "\n"
        "Ancient body.\n"
        "\n"
        "12/22/07 Anachronisms mode and reference to function\n"
        "\n"
        "Single-space padded header body.\n"
    )
    es = parse_entries(sample)
    failed = 0

    def check(name: str, cond: bool) -> None:
        nonlocal failed
        print(f"{'PASS' if cond else 'FAIL'} {name}")
        failed += 0 if cond else 1

    check("entry count (5, wrap ref NOT an entry)", len(es) == 5)
    check("bracketed single", es[0]["ids"] == "EDGcpfe/25951,EDGcpfe/29043")
    check("wrapped title joined",
          es[0]["title"] == "C23: New tag compatibility rules")
    check("body correct", es[0]["body"] == "In C23 mode structs match.")
    check("multi-line ids", es[1]["ids"] == "EDGcpfe/25497,EDGcpfe/26173,EDGcpfe/26947")
    check("plain header title", es[2]["title"].startswith("Constant expressions"))
    check("plain wrapped title",
          es[2]["title"] == "Constant expressions unconditionally recorded in IL")
    check("plain body", es[2]["body"].startswith("Old-style body text."))
    check("ancient entry", es[3]["title"].startswith("Support for access"))
    check("wrap-ref text stayed in body (not an entry)",
          "for EDGcpfe/17414" in es[2]["body"])
    check("entry count with 1-space header (5)", len(es) == 5)
    check("1-space padded header parsed",
          es[4]["title"].startswith("Anachronisms mode"))
    check("1-space header body",
          es[4]["body"] == "Single-space padded header body.")

    # scoring
    e_gpp = {"title": "GNU C++ compatibility: mem-initializer in class",
             "body": "In GCC mode the initialization was wrong."}
    s, _ = score_entry(e_gpp)
    check(f"GNU C++/class/initializer entry scores < 0 (got {s})", s < 0)
    e_c = {"title": "C23: empty initializers in C mode",
           "body": "An empty initializer {} is now valid in ISO C."}
    s2, _ = score_entry(e_c)
    check(f"C23 entry scores >= 6 (got {s2})", s2 >= 6)
    e_dup = {"title": "mem-initializer initialization",
             "body": "The initialization and initializer list."}
    s3, _ = score_entry(e_dup)
    check(f"initializer/initialization no double-count (got {s3})", s3 == 2)
    # The haystack is lowercased: keyword rules with uppercase literals were
    # dead (review P2).  _Generic(3) + _Complex(1) + _Static_assert(3) = 7.
    e_kw = {"title": "_Generic, _Complex, _Static_assert keywords",
            "body": "_Generic _Complex _Static_assert"}
    s4, _ = score_entry(e_kw)
    check(f"lowercased-hay keyword rules are live (got {s4})", s4 == 7)

    # date normalization
    check("year 127 typo -> 2017", normalize_year("10/25/127") == "2017")
    check("92 -> 1992", normalize_year("5/4/92") == "1992")
    check("08 -> 2008", normalize_year("8/7/08") == "2008")
    check("37 typo -> 1997", normalize_year("10/1/37") == "1997")
    check("69 typo -> 1996", normalize_year("5/9/69") == "1996")
    check("82 typo -> 1992", normalize_year("8/3/82") == "1992")
    print(f"selftest: {'ALL PASS' if not failed else f'{failed} FAILED'}")
    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("mode", nargs="?", choices=["mine", "selftest"],
                    default="mine")
    ap.add_argument("--edg-root", type=Path,
                    default=Path("/home/user/edg-compiler"))
    ap.add_argument("--out-dir", type=Path, default=None,
                    help="default: <repo>/docs")
    ap.add_argument("--min-score", type=int, default=2,
                    help="C-relevance threshold (default 2)")
    args = ap.parse_args(argv)

    if args.mode == "selftest":
        return parse_entries_selftest()

    changes = args.edg_root / "src" / "Changes"
    if not changes.is_file():
        print(f"error: {changes} not found", file=sys.stderr)
        return 2
    repo = Path(__file__).resolve().parent.parent
    out_dir = (args.out_dir or repo / "docs").resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    entries = parse_entries(changes.read_text(encoding="utf-8",
                                              errors="replace"))
    # Sanity gate: both eras must parse.  ~13.5k entries total is the
    # ground truth (7 416 bracketed + ~4 979 padded + ~1 169 one-space
    # headers); failing loudly beats shipping a mega-blob again.
    if len(entries) < 13000:
        print(f"error: only {len(entries)} entries parsed — header-grammar "
              "regression? refusing to emit artifacts", file=sys.stderr)
        return 3

    kept: list[dict] = []
    index_lines = [
        "# EDG `src/Changes` — complete entry index",
        "",
        f"Generated by `scripts/edg_changes_mine.py` from edgcpp/compiler "
        f"`src/Changes`. Parsed **{len(entries)}** entries (both header "
        "eras, 1992–2026).  The C-relevant subset is expanded in "
        "`edg_changes_c_extract.md`.",
        "",
        "| Date | Score | Bug IDs | Title |",
        "| --- | ---: | --- | --- |",
    ]
    score_hist: dict[int, int] = {}
    years: dict[str, int] = {}
    decades: dict[str, int] = {}
    for e in entries:
        score, hits = score_entry(e)
        score_hist[score] = score_hist.get(score, 0) + 1
        years[e["year"]] = years.get(e["year"], 0) + 1
        decade = e["year"][:3] + "0s"
        decades[decade] = decades.get(decade, 0) + 1
        e["score"], e["hits"] = score, hits
        title = e["title"].replace("|", "\\|")
        index_lines.append(
            f"| {e['year']}-{e['date'].split('/')[0].zfill(2)}-"
            f"{e['date'].split('/')[1].zfill(2)} | {score:+d} "
            f"| {e['ids']} | {title} |")
        if score >= args.min_score:
            kept.append(e)

    (out_dir / "edg_changes_c_index.md").write_text(
        "\n".join(l.rstrip() for l in index_lines) + "\n", encoding="utf-8")

    extract = [
        "# EDG `src/Changes` — C-relevant entries (full text)",
        "",
        f"Generated by `scripts/edg_changes_mine.py --min-score "
        f"{args.min_score}`; {len(kept)} of {len(entries)} entries qualify "
        "(both header eras parsed; anchored scoring).  Each entry is a "
        "resolved EDG bug with IDs and before/after semantics — read this "
        "as a checklist of C implementation edge cases before touching "
        "LCCC's frontend, const-eval or layout code.  Apache-2.0 WITH "
        "LLVM-exception (edgcpp/compiler); quotes kept verbatim with "
        "attribution (third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt).",
        "",
    ]
    for e in kept:
        extract.append("---")
        extract.append("")
        extract.append(f"## {e['year']}-{e['date'].split('/')[0].zfill(2)}-"
                       f"{e['date'].split('/')[1].zfill(2)}"
                       + (f" · `{e['ids']}`" if e["ids"] else ""))
        extract.append("")
        extract.append(f"**{e['title']}**")
        extract.append("")
        extract.append(f"*C-score {e['score']:+d}*")
        extract.append("")
        extract.append("```text")
        extract.append(e["body"])
        extract.append("```")
        extract.append("")
    # `extract` holds multi-line entry blocks: normalize per LINE, not per
    # block, or interior quoted lines keep their trailing whitespace (which
    # git apply flags on every added line).
    (out_dir / "edg_changes_c_extract.md").write_text(
        "\n".join(l.rstrip()
                 for block in extract for l in block.split("\n")) + "\n",
        encoding="utf-8")

    # Year sanity: any year outside 1992..2026 is an upstream date typo.
    odd_years = {y: c for y, c in years.items()
                 if not (y.isdigit() and 1992 <= int(y) <= 2026)}
    (out_dir / "edg_changes_stats.json").write_text(json.dumps(
        {"entries_total": len(entries), "entries_kept": len(kept),
         "min_score": args.min_score,
         "score_histogram": {str(k): v for k, v in sorted(score_hist.items())},
         "entries_per_year": dict(sorted(years.items())),
         "entries_per_decade": dict(sorted(decades.items())),
         "odd_years_flagged": odd_years},
        indent=2, sort_keys=True) + "\n", encoding="utf-8")

    b_extract = (out_dir / "edg_changes_c_extract.md").stat().st_size
    b_index = (out_dir / "edg_changes_c_index.md").stat().st_size
    print(f"entries: {len(entries)} total, {len(kept)} C-relevant "
          f"(min-score {args.min_score})")
    print(f"extract: {b_extract} bytes  index: {b_index} bytes")
    if odd_years:
        print(f"warning: odd years upstream: {odd_years}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

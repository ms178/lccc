#!/usr/bin/env python3
"""edg_corpus_mine.py — license-aware EDG test-corpus miner for LCCC.

EDG (https://github.com/edgcpp/compiler) open-sourced its front end together
with a large single-file test corpus.  Two subtrees matter to LCCC:

  tests/tests/imported/clang/c/**   1 529 C tests, Apache-2.0 WITH LLVM
      exception  ->  LEGALLY SAFE to adapt into the LCCC tree with
      attribution (SPDX headers are preserved on every copied file).
      Coverage: C/, Parser/, Preprocessor/, Sema/ — WG14 N-papers,
      unsequenced-modification semantics, preprocessor corner cases.

  tests/tests/imported/gnu/c/**     7 165 GCC execution tests, GPL-3.0
      ->  NEVER copied into the LCCC tree.  We only emit a *manifest of
      testcase names* (facts, not copyrightable expression) so a locally
      checked-out GCC testsuite can supply the originals at run time.

Modes
-----
  clang-c    mine the Apache-licensed Clang C corpus into
             tests/corpus/clang-c/<category>/ with .meta.json sidecars
  gnu-manifest  emit tests/corpus/gnu-torture-manifest.jsonl (names +
             directives only; zero source text) plus a small summary JSON
  summary    print corpus statistics (no files written)

Typical use
-----------
  git clone --filter=blob:none --depth 1 https://github.com/edgcpp/compiler.git \\
      ../edg-compiler
  scripts/edg_corpus_mine.py summary  --edg-root ../edg-compiler
  scripts/edg_corpus_mine.py clang-c  --edg-root ../edg-compiler
  scripts/edg_corpus_mine.py gnu-manifest --edg-root ../edg-compiler

Design notes
------------
* The `.sft.c` directive header (//type:, //options:, //cases:, //match_regex:)
  is parsed into the sidecar so a future driver can (a) expect accept/reject,
  (b) run with the recorded flags, (c) verify recorded diagnostics
  (`expected-warning`/`expected-error` annotations are extracted too).
* Tests whose `//type:` says `fn`/`cn` (negative) or `fc`/`cc` (catastrophic)
  are marked xfail-by-design for a "must compile" harness; the sidecar keeps
  the expectation so the driver can invert its pass condition.
* Nothing here executes the tests; this is the extraction step of transplant
  item E1 (docs/EDG_TRANSPLANT_ANALYSIS.md §4.2).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import sys
from collections import Counter
from pathlib import Path

SPDX = (
    "// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception\n"
    "// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH\n"
    "// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.\n"
)

CLANG_C_REL = Path("tests/tests/imported/clang/c")
GNU_C_REL = Path("tests/tests/imported/gnu/c")

# .sft directives we understand (testing.rst in the EDG docs).
DIRECTIVE_RE = re.compile(r"^//(type|options|options_all|options_sep|cases|"
                          r"match_regex|remark|require|filter|name):\s*(.*)$")
# LLVM verify-style diagnostic expectations in the body.
EXPECT_RE = re.compile(r"expected-(warning|error|note|no-diagnostics)\b"
                       r"(?:\s*\{\{(?P<msg>[^}]*)\}\})?")

VALID_TYPES = {"fn", "fp", "fc", "cn", "cp", "cc", "ln", "lp", "rn", "rp",
               "ra", "cppbe", "s"}


def parse_sft(path: Path) -> dict:
    """Parse the directive header and diagnostic expectations of one .sft file.

    Returns a dict with: type, options (list), cases (int|None), match_regex,
    remark, raw directives, expected diagnostics list, TEST_NUMBER variants.

    Directive-scan discipline (red-team fix): EDG directives live only in the
    leading comment block ("directives at the start of the test file" —
    doc/source/testing.rst).  A mid-file comment like ``//type: int`` must NOT
    reclassify the test, so scanning stops at the first code line.  The
    ``//options_sep:`` directive is honoured when splitting ``//options:``,
    even when it appears after them in the header.
    """
    info: dict = {
        "type": "cp",           # EDG default for compile positive
        "options": [],
        "options_all": "",
        "options_sep": ":",
        "cases": None,
        "match_regex": None,
        "remark": None,
        "directives": {},
        "expected": [],
        "has_main": False,
        "header_lines": 0,
    }
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:  # pragma: no cover - defensive
        info["error"] = str(exc)
        return info

    raw_options = None
    raw_options_sep = None
    in_header = True
    for lineno, line in enumerate(text.splitlines()):
        stripped = line.strip()
        if in_header:
            # Leading block: blank lines, // comments, /* */ banners, # lines.
            if (stripped == "" or stripped.startswith(("//", "/*", "*", "#"))):
                m = DIRECTIVE_RE.match(stripped)
                if m:
                    key, val = m.group(1), m.group(2).strip()
                    info["directives"][key] = val
                    if key == "type" and val in VALID_TYPES:
                        info["type"] = val
                    elif key == "options":
                        raw_options = val
                    elif key == "options_sep":
                        raw_options_sep = val or ":"
                    elif key == "options_all":
                        info["options_all"] = val
                    elif key == "cases":
                        try:
                            info["cases"] = int(val)
                        except ValueError:
                            pass
                    elif key == "match_regex":
                        info["match_regex"] = val
                    elif key == "remark":
                        info["remark"] = val
                info["header_lines"] = lineno + 1
                continue
            in_header = False
        em = EXPECT_RE.search(line)
        if em:
            info["expected"].append({
                "kind": em.group(1),
                "msg": (em.group("msg") or "").strip(),
            })

    # Honour //options_sep (may follow //options in the header).
    sep = raw_options_sep or ":"
    info["options_sep"] = sep
    if raw_options is not None:
        info["options"] = [o for o in raw_options.split(sep) if o]

    info["has_main"] = re.search(r"\bmain\s*\(", text) is not None
    info["test_number_uses"] = text.count("TEST_NUMBER")
    return info


SIZE_CAP_BYTES = 256 * 1024
# Generated Arm SME/SVE feature-matrix tests: multi-MB combinatorial giants
# with zero relevance to LCCC's x86-64 C mission; skipped by name pattern.
GIANT_RE = re.compile(r"^(arm_sme|arm_sve|arm_cde|arm_mve|arm_neon)_")


def mine_clang_c(edg_root: Path, out_root: Path, force: bool,
                 index_only: bool, include_giants: bool) -> int:
    src_root = edg_root / CLANG_C_REL
    if not src_root.is_dir():
        print(f"error: {src_root} not found (is --edg-root an EDG clone?)",
              file=sys.stderr)
        return 2
    dest_root = out_root / "clang-c"
    if dest_root.exists():
        if not force:
            print(f"error: {dest_root} exists; use --force to re-mine",
                  file=sys.stderr)
            return 2
        shutil.rmtree(dest_root)
    dest_root.mkdir(parents=True, exist_ok=True)

    copied = 0
    skipped = 0
    types: Counter[str] = Counter()
    categories: Counter[str] = Counter()
    index: list[dict] = []
    for sft in sorted(src_root.rglob("*.sft.c")):
        rel = sft.relative_to(src_root)
        category = rel.parts[0] if len(rel.parts) > 1 else "top"
        stem = sft.name[: -len(".sft.c")]
        size = sft.stat().st_size
        giant = bool(GIANT_RE.match(stem))
        skip_reason = None
        if giant and not include_giants:
            skip_reason = "giant-generated"
        elif size > SIZE_CAP_BYTES and not include_giants:
            skip_reason = "size-cap"
        info = parse_sft(sft)
        types[info["type"]] += 1
        categories[category] += 1
        record = {
            "origin": str(Path("tests/tests/imported/clang/c") / rel),
            "category": category,
            "type": info["type"],
            "lccc_expect": ("skip" if info["type"] == "s"
                            else "accept" if info["type"] in ("fp", "cp", "lp",
                                                             "rp", "ra")
                            else "reject"),
            "bytes": size,
            **{k: v for k, v in info.items()
               if k not in ("directives", "type")},
        }
        if skip_reason or index_only:
            record["skip_reason"] = skip_reason or "index-only"
            skipped += 1
            index.append(record)
            continue
        dest = dest_root / category / f"{stem}.c"
        dest.parent.mkdir(parents=True, exist_ok=True)
        body = sft.read_text(encoding="utf-8", errors="replace")
        # SPDX attribution first; keep the original directives for provenance.
        dest.write_text(SPDX + body, encoding="utf-8")
        record["license"] = "Apache-2.0 WITH LLVM-exception"
        record["sha256"] = hashlib.sha256(
            dest.read_bytes()).hexdigest()
        dest.with_suffix(".c.meta.json").write_text(
            json.dumps(record, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        record["file"] = str(dest.relative_to(out_root))
        index.append(record)
        copied += 1

    (dest_root / "corpus-index.json").write_text(
        json.dumps({"count": len(index), "copied": copied, "skipped": skipped,
                    "by_type": dict(types), "by_category": dict(categories),
                    "files": index},
                   indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (dest_root / "LICENSE-NOTICE.txt").write_text(
        "Files in this directory originate from the EDG C/C++ front end test\n"
        "suite (https://github.com/edgcpp/compiler), subtrees\n"
        "tests/tests/imported/clang/**, licensed Apache-2.0 WITH LLVM\n"
        "exception (see the LICENSE.txt in that subtree).  Each adapted file\n"
        "carries an SPDX attribution header.  No GPL-licensed GNU testsuite\n"
        "material is present here; those are referenced by name only in\n"
        "../gnu-torture-manifest.jsonl.\n", encoding="utf-8")
    print(f"clang-c: {len(index)} tests indexed, {copied} copied, "
          f"{skipped} skipped -> {dest_root}")
    print(f"  by type: {dict(types)}")
    print(f"  by category: {dict(categories)}")
    return 0


def mine_gnu_manifest(edg_root: Path, out_root: Path) -> int:
    src_root = edg_root / GNU_C_REL
    if not src_root.is_dir():
        print(f"error: {src_root} not found", file=sys.stderr)
        return 2
    entries = []
    types: Counter[str] = Counter()
    categories: Counter[str] = Counter()
    for sft in sorted(src_root.rglob("*.sft.c")):
        info = parse_sft(sft)
        rel = sft.relative_to(src_root)
        category = str(rel.parent) if len(rel.parts) > 1 else "torture-top"
        # Names + directive *values* are uncopyrightable facts; NO body text.
        entries.append({
            "name": sft.name[: -len(".sft.c")],
            "category": category,
            "type": info["type"],
            "options": info["options"],
            "cases": info["cases"],
            "has_main": info["has_main"],
            "test_number_uses": info["test_number_uses"],
        })
        types[info["type"]] += 1
        categories[category] += 1
    out = out_root / "gnu-torture-manifest.jsonl"
    agg = out_root / "gnu-torture-manifest-summary.json"
    out_root.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as fh:
        for e in entries:
            fh.write(json.dumps(e, sort_keys=True, separators=(",", ":")))
            fh.write("\n")
    agg.write_text(json.dumps(
        {"license": "GPL-3.0 (names/directives only; source NOT redistributed)",
         "source_subtree": "tests/tests/imported/gnu/c",
         "how_to_obtain": ("clone https://github.com/edgcpp/compiler locally "
                           "(or check out gcc.git for the original "
                           "gcc.c-torture/execute tests) and point the "
                           "runner at the matching files; these are GCC "
                           "execution tests like 20000108-1"),
         "count": len(entries), "by_type": dict(types),
         "by_category": {k: v for k, v in sorted(categories.items())}},
        indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"gnu-manifest: {len(entries)} names -> {out}")
    print(f"  summary    -> {agg}")
    print(f"  by type: {dict(types)}")
    print(f"  top categories: {categories.most_common(10)}")
    return 0


def selftest() -> int:
    """Parse-level unit tests over synthetic .sft content (no EDG needed)."""
    import tempfile
    cases = [
        # (name, content, assertion callable)
        ("type-default",
         "int main(void){return 0;}\n",
         lambda i: i["type"] == "cp"),
        ("type-header",
         "//type: fn\nint f(void){return 0;}\n",
         lambda i: i["type"] == "fn"),
        ("type-midfile-must-not-leak",
         "//type: fp\nint f(void){return 0;}\n//type: fn\n",
         lambda i: i["type"] == "fp"),
        ("options-sep-after-options",
         "//options: -a!-b=xx\n//options_sep: !\n",
         lambda i: i["options"] == ["-a", "-b=xx"]),
        ("options-default-sep",
         "//options: -a:-b\n",
         lambda i: i["options"] == ["-a", "-b"]),
        ("expected-diag",
         "// RUN: x -verify %s\nf(); // expected-warning {{boom}}\n",
         lambda i: i["expected"] == [{"kind": "warning", "msg": "boom"}]),
        ("cases-and-main",
         "//cases: 3\nint main(void){return 0;}\n",
         lambda i: i["cases"] == 3 and i["has_main"]),
        ("skip-type",
         "//type: s\n",
         lambda i: i["type"] == "s"),
    ]
    failed = 0
    with tempfile.TemporaryDirectory(prefix="sft-selftest-") as td:
        for name, content, check in cases:
            p = Path(td) / f"{name}.sft.c"
            p.write_text(content, encoding="utf-8")
            info = parse_sft(p)
            try:
                ok = bool(check(info))
            except Exception as exc:  # pragma: no cover - defensive
                print(f"FAIL {name}: exception {exc}")
                failed += 1
                continue
            print(f"{'PASS' if ok else 'FAIL'} {name}")
            failed += 0 if ok else 1
    print(f"selftest: {len(cases) - failed}/{len(cases)} passed")
    return 1 if failed else 0


def summary(edg_root: Path) -> int:
    ok = 0
    for label, rel, pat in (
        ("clang C corpus", CLANG_C_REL, "*.sft.c"),
        ("clang C++ corpus", Path("tests/tests/imported/clang/cpp"), "*.sft.cpp"),
        ("gnu C corpus", GNU_C_REL, "*.sft.c"),
        ("gnu C++ corpus", Path("tests/tests/imported/gnu/cpp"), "*.sft.cpp"),
        ("EDG-authored tests", Path("tests/tests/edg"), "*.sft.*"),
    ):
        d = edg_root / rel
        if not d.is_dir():
            print(f"  {label:22s}: MISSING")
            continue
        # files only — rglob("*") would also count directories (red-team fix)
        n = sum(1 for p in d.rglob(pat) if p.is_file())
        lic = "GPL-3.0" if "gnu" in str(rel) else "Apache-2.0+LLVM-exc"
        print(f"  {label:22s}: {n:6d} tests  [{lic}]")
        ok += 1
    return 0 if ok else 2


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("mode", choices=["clang-c", "gnu-manifest", "summary",
                                    "selftest"])
    ap.add_argument("--edg-root", type=Path,
                    default=Path(__import__("os").environ.get(
                        "EDG_ROOT", "/home/user/edg-compiler")),
                    help="path to an edgcpp/compiler clone")
    ap.add_argument("--out-root", type=Path, default=None,
                    help="output root (default: <repo>/tests/corpus)")
    ap.add_argument("--force", action="store_true",
                    help="overwrite an existing clang-c mine")
    ap.add_argument("--index-only", action="store_true",
                    help="clang-c: write corpus-index.json only (no test bodies)")
    ap.add_argument("--include-giants", action="store_true",
                    help="clang-c: also copy the multi-MB generated "
                         "Arm SME/SVE feature-matrix tests")
    args = ap.parse_args(argv)

    if args.mode == "selftest":
        return selftest()

    edg_root = args.edg_root.resolve()
    if not edg_root.is_dir():
        print(f"error: EDG root {edg_root} does not exist.\n"
              "  git clone --filter=blob:none --depth 1 "
              "https://github.com/edgcpp/compiler.git /home/user/edg-compiler",
              file=sys.stderr)
        return 2
    repo = Path(__file__).resolve().parent.parent
    out_root = (args.out_root or repo / "tests" / "corpus").resolve()

    if args.mode == "summary":
        return summary(edg_root)
    if args.mode == "clang-c":
        return mine_clang_c(edg_root, out_root, args.force,
                            args.index_only, args.include_giants)
    return mine_gnu_manifest(edg_root, out_root)


if __name__ == "__main__":
    sys.exit(main())

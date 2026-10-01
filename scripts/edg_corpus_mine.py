#!/usr/bin/env python3
"""edg_corpus_mine.py — license-aware EDG test-corpus miner for LCCC.

EDG (https://github.com/edgcpp/compiler) open-sourced its front end together
with a large single-file test corpus.  Two subtrees matter to LCCC:

  tests/tests/imported/clang/c/**   1 529 C tests, Apache-2.0 WITH LLVM
      exception  ->  LEGALLY SAFE to adapt into the LCCC tree with
      attribution (SPDX headers are preserved on every copied file).
      Coverage: C/, Parser/, Preprocessor/, Sema/ — WG14 N-papers,
      unsequenced-modification semantics, preprocessor corner cases.

  tests/tests/imported/gnu/c/**     18 188 GCC execution tests, GPL-3.0
      ->  NEVER copied into the LCCC tree.  We only emit a *manifest of
      testcase names* (facts, not copyrightable expression) so a locally
      checked-out GCC testsuite can supply the originals at run time.

Modes
-----
  clang-c    mine the Apache-licensed Clang C corpus into
             tests/corpus/clang-c/ (metadata consolidated in
             corpus-index.json; per-file sidecars only with --emit-sidecars)
  gnu-manifest  emit tests/corpus/gnu-torture-manifest.jsonl (names +
             directives only; zero source text) plus a summary JSON
  summary    print corpus statistics (no files written)
  selftest   parser unit tests over synthetic .sft content (no EDG needed)

Design notes (PR #719 review findings F3-F6 are documented in
docs/REVIEW_719_ADJUDICATION.md):

* Directive scanning is limited to the leading comment block (blank lines,
  // and /* */ comments).  Preprocessor directives are CODE under test, not
  .sft headers; a mid-file `//type:` comment must not reclassify a test.
* Diagnostic expectations (Clang -verify style) are extracted from EVERY
  line, including header comments (`expected-no-diagnostics`) and
  preprocessor lines.  Full -verify syntax is understood: custom prefixes
  (`c-error`, `-verify=expected,c,c11,c23`), `-re` regex form, `@±N`/`@*`
  line offsets, repeat counts, nested single braces in `{{...}}`, and
  multiple annotations per line.  Each expectation records its source line.
* `//options:` is a list of SEPARATE INVOCATIONS (EDG runs the test once per
  `:`-separated option set) — recorded as `option_sets`, never conflated.
  Each set is additionally translated to suggested GCC/lccc flags
  (`gcc_flags`) so a runner can execute the corpus without re-deriving EDG's
  dialect switches.
* The full EDG-relative subdirectory (C/C89/, C/C11/, C/DR/, ...) is
  preserved so standard-dialect grouping survives mining.
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

# Clang -verify diagnostics.  Supported forms:
#   expected-warning {{msg}}     expected-error-re {{regex}}
#   expected-error@-1 {{msg}}    expected-warning 2 {{msg}}
#   c23-warning {{msg}}          // expected-no-diagnostics
# The annotation token is matched here; the {{...}} message is consumed
# separately by _verify_message() with BALANCED-brace scanning so nested
# single braces work ({{outer ({{[0-9]+}})}}) without swallowing the next
# annotation on the same line ({{a}} expected-warning {{b}}).
EXPECT_RE = re.compile(
    r"\b(?P<prefix>[A-Za-z0-9_]+)-(?P<kind>no-diagnostics|warning|error|note)"
    r"(?P<re>-re)?\b"
    r"(?:\s*@(?P<loc>[+-]?\d+|\*|#\S+))?"
    r"(?:\s+(?P<count>\d+(?:-\d+|\+)?))?"
)


def _verify_message(line: str, pos: int) -> tuple[str, int]:
    """Consume a balanced {{...}} message starting at/after pos.

    The {{ must follow the annotation with only whitespace between
    (anything else is prose, not a message).  Nested {{...}} is kept
    balanced.  Returns (message, end_position).
    """
    rest = line[pos:]
    if not rest.lstrip().startswith("{{"):
        return "", pos
    brace = pos + len(rest) - len(rest.lstrip())
    depth = 1
    i = brace + 2
    while i < len(line) - 1 and depth:
        if line[i:i + 2] == "{{":
            depth += 1
            i += 2
        elif line[i:i + 2] == "}}":
            depth -= 1
            if depth == 0:
                return line[brace + 2:i], i + 2
            i += 2
        else:
            i += 1
    # unbalanced: take the rest of the line
    return line[brace + 2:], len(line)

# -verify=expected,c,c11,c23 custom prefixes on // RUN: lines.
VERIFY_PREFIX_RE = re.compile(r"-verify=([A-Za-z0-9_,]+)")

VALID_TYPES = {"fn", "fp", "fc", "cn", "cp", "cc", "ln", "lp", "rn", "rp",
               "ra", "cppbe", "s"}

SIZE_CAP_BYTES = 256 * 1024
# Generated Arm SME/SVE feature-matrix tests: multi-MB combinatorial giants
# with zero relevance to LCCC's x86-64 C mission; skipped by name pattern.
GIANT_RE = re.compile(r"^(arm_sme|arm_sve|arm_cde|arm_mve|arm_neon)_")

# EDG front-end dialect switches -> suggested GCC/lccc flags.  Unknown EDG
# flags are preserved verbatim in option_sets and noted in `untranslated`.
EDG_TO_GCC = {
    "--c": ["-std=c11"],          # plain C mode; tests are standard-agnostic
    "--c90": ["-std=c90"], "--c89": ["-std=c90"],
    "--c99": ["-std=c99"], "--c11": ["-std=c11"],
    "--c17": ["-std=c17"], "--c18": ["-std=c17"],
    "--c2x": ["-std=c23"], "--c23": ["-std=c23"],
    "--gnu": ["-std=gnu11"], "--gnu_version": [],
    "--strict": ["-pedantic-errors"],
    "--strict_ansi": ["-ansi"],
    "--microsoft": [], "--ms_extensions": [], "--ms_compatibility": [],
    "--no_warnings": ["-w"], "--warnings_are_errors": [],
    "--check_compat": [],
}
CPP_OPTION_RE = re.compile(r"--c\+\+|--cpp|--cp\b")
TARGET_ARCH_RE = re.compile(r"--target\s+(\S+)|--march\s+(\S+)")


def translate_options(option_set: str) -> tuple[list[str], list[str], list[str]]:
    """Map one EDG option set to (gcc_flags, arch_tags, untranslated)."""
    gcc: list[str] = []
    arch: list[str] = []
    untr: list[str] = []
    for arg in option_set.split():
        if arg in EDG_TO_GCC:
            gcc.extend(EDG_TO_GCC[arg])
        else:
            m = TARGET_ARCH_RE.match(arg) or (
                TARGET_ARCH_RE.match("--target " + arg)
                if not arg.startswith("-") else None)
            if m:
                arch.append(m.group(1) or m.group(2) or arg)
            elif arg.startswith(("-D", "-U", "-I")):
                gcc.append(arg)
            else:
                untr.append(arg)
    return gcc, arch, untr


def parse_sft(path: Path) -> dict:
    """Parse directives + Clang -verify expectations of one .sft file.

    Directive scanning stops at the first code line (review finding F3:
    preprocessor directives are code; a mid-file `//type:` must not leak).
    Expectation scanning runs on EVERY line, including header comments and
    preprocessor lines (finding F3b: `expected-no-diagnostics` in headers).
    """
    info: dict = {
        "type": "cp",           # EDG default for compile positive
        "option_sets": [],
        "options_all": "",
        "options_sep": ":",
        "cases": None,
        "match_regex": None,
        "remark": None,
        "directives": {},
        "expected": [],
        "verify_prefixes": [],
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
    in_block_comment = False
    for lineno, line in enumerate(text.splitlines()):
        stripped = line.strip()
        # ---- expectations run on every line (F3b, F4) --------------------
        for m in EXPECT_RE.finditer(line):
            loc = m.group("loc") or ""
            offset = 0
            if loc and loc[0] in "+-" and loc[1:].isdigit():
                offset = int(loc)
            msg, _ = _verify_message(line, m.end())
            rec = {
                "kind": m.group("kind"),
                "prefix": m.group("prefix"),
                "regex_form": bool(m.group("re")),
                "loc": loc,
                "count": m.group("count"),
                "msg": msg.strip(),
                "line": lineno + 1 + offset,
            }
            info["expected"].append(rec)
        vm = VERIFY_PREFIX_RE.search(line)
        if vm:
            for p in vm.group(1).split(","):
                if p and p not in info["verify_prefixes"]:
                    info["verify_prefixes"].append(p)
        # ---- directive header block --------------------------------------
        if in_header:
            if in_block_comment:
                info["header_lines"] = lineno + 1
                if "*/" in stripped:
                    in_block_comment = False
                continue
            if stripped == "" or stripped.startswith("//"):
                pass  # still header; directives checked below
            elif stripped.startswith("/*"):
                if "*/" not in stripped:
                    in_block_comment = True
                info["header_lines"] = lineno + 1
                continue
            elif stripped.startswith("*"):
                pass  # block-comment continuation style
            else:
                # First code line (declarations, #preprocessor, anything
                # else) ends the .sft directive header (F3).
                in_header = False
            if in_header:
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

    # Honour //options_sep (may follow //options in the header) and keep
    # each `:`-separated group as a SEPARATE invocation (finding F6).
    sep = raw_options_sep or ":"
    info["options_sep"] = sep
    if raw_options is not None:
        sets = [o.strip() for o in raw_options.split(sep) if o.strip()]
        info["option_sets"] = sets
    # Convenience union of all args across sets (for quick filtering).
    info["options"] = sorted({a for s in info["option_sets"] for a in s.split()})
    is_cpp = any(CPP_OPTION_RE.search(s) for s in info["option_sets"])
    info["language"] = "c++" if is_cpp else "c"
    archs: set[str] = set()
    gcc_sets: list[list[str]] = []
    untranslated: set[str] = set()
    for s in info["option_sets"]:
        g, a, u = translate_options(s)
        gcc_sets.append(g)
        archs.update(a)
        untranslated.update(u)
    info["gcc_flags_per_set"] = gcc_sets
    info["arch_tags"] = sorted(archs)
    info["untranslated_edg_flags"] = sorted(untranslated)

    info["has_main"] = re.search(r"\bmain\s*\(", text) is not None
    info["test_number_uses"] = text.count("TEST_NUMBER")
    return info


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
        ("hash-line-ends-header",
         "//type: fp\n#include <x.h>\n//options: --c99\nint f(){}\n",
         lambda i: i["type"] == "fp" and i["option_sets"] == []),
        ("options-sep-after-options",
         "//options: -a!-b=xx\n//options_sep: !\n",
         lambda i: i["option_sets"] == ["-a", "-b=xx"]),
        ("options-default-sep-are-separate-runs",
         "//options: --c99: --c11\n",
         lambda i: i["option_sets"] == ["--c99", "--c11"]),
        ("options-stripped",
         "//options:  --c11 : --gnu \n",
         lambda i: i["option_sets"] == ["--c11", "--gnu"]),
        ("expected-diag-basic",
         "// RUN: x -verify %s\nf(); // expected-warning {{boom}}\n",
         lambda i: len(i["expected"]) == 1
         and i["expected"][0]["msg"] == "boom"
         and i["expected"][0]["line"] == 2),
        ("expected-in-header-block",
         "// expected-no-diagnostics\nint x;\n",
         lambda i: len(i["expected"]) == 1
         and i["expected"][0]["kind"] == "no-diagnostics"),
        ("expected-on-preprocessor-line",
         "#error boom // expected-error {{boom}}\n",
         lambda i: len(i["expected"]) == 1
         and i["expected"][0]["kind"] == "error"),
        ("expected-line-offset",
         "int y;\n// expected-error@-1 {{early}}\n",
         lambda i: i["expected"][0]["line"] == 1),
        ("expected-regex-nested-count",
         "// expected-error-re 2 {{outer ({{[0-9]+}})}}\n",
         lambda i: i["expected"][0]["regex_form"]
         and i["expected"][0]["count"] == "2"
         and "{{" in i["expected"][0]["msg"]),
        ("expected-two-per-line",
         "// expected-error {{a}} expected-warning {{b}}\n",
         lambda i: [e["kind"] for e in i["expected"]] == ["error", "warning"]),
        ("expected-custom-prefix",
         "// RUN: %clang_cc1 -verify=expected,c,c23 %s\n"
         "x; // c23-warning {{std}}\n",
         lambda i: i["expected"][0]["prefix"] == "c23"
         and i["verify_prefixes"] == ["expected", "c", "c23"]),
        ("cases-and-main",
         "//cases: 3\nint main(void){return 0;}\n",
         lambda i: i["cases"] == 3 and i["has_main"]),
        ("skip-type",
         "//type: s\n",
         lambda i: i["type"] == "s"),
        ("language-cpp",
         "//options: --c++\n",
         lambda i: i["language"] == "c++"),
        ("gcc-flag-translation",
         "//options: --c99\n",
         lambda i: i["gcc_flags_per_set"] == [["-std=c99"]]),
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


def mine_clang_c(edg_root: Path, out_root: Path, force: bool,
                 index_only: bool, include_giants: bool,
                 emit_sidecars: bool) -> int:
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
        # Preserve the full relative subdirectory (C/C89/, C/C11/, C/DR/,
        # Sema/...) so standard-dialect grouping survives mining (F6).
        dest = dest_root / rel.parent / f"{stem}.c"
        dest.parent.mkdir(parents=True, exist_ok=True)
        body = sft.read_text(encoding="utf-8", errors="replace")
        # SPDX attribution first; keep the original directives for provenance.
        dest.write_text(SPDX + body, encoding="utf-8")
        record["license"] = "Apache-2.0 WITH LLVM-exception"
        record["sha256"] = hashlib.sha256(
            dest.read_bytes()).hexdigest()
        if emit_sidecars:  # opt-in: corpus-index.json is the canonical store
            dest.with_suffix(".c.meta.json").write_text(
                json.dumps(record, indent=2, sort_keys=True) + "\n",
                encoding="utf-8")
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
        "carries an SPDX attribution header.  Full license text:\n"
        "../../../third_party_licenses/EDG-Clang-Apache-2.0-LLVM.txt\n"
        "No GPL-licensed GNU testsuite material is present here; those are\n"
        "referenced by name only in ../gnu-torture-manifest.jsonl.\n",
        encoding="utf-8")
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
            "option_sets": info["option_sets"],
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
    ap.add_argument("--emit-sidecars", action="store_true",
                    help="clang-c: also write per-file .c.meta.json "
                         "(default: metadata lives only in corpus-index.json)")
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
                            args.index_only, args.include_giants,
                            args.emit_sidecars)
    return mine_gnu_manifest(edg_root, out_root)


if __name__ == "__main__":
    sys.exit(main())

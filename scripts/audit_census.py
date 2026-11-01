#!/usr/bin/env python3
"""Whole-codebase audit census and ratchet (AUD-RATCHET-1).

The AUDIT-2026-10-13 findings are all "a class of site exists, and its count
must not grow while it is being paid down".  This script measures those classes
from the source tree, so the claim is checkable instead of asserted, and so a
future change that adds a panic to the lowering pipeline, an `unsafe`, an
undeclared environment knob or an unannotated `#[ignore]` fails CI at review
time rather than at the next audit.

Metric definitions (this docstring is the contract):

* Scanning.  Every ``src/**/*.rs`` file is read, then
  (1) comments (``//``, ``/* */`` incl. nesting), string literals (``"..."``,
  ``b"..."``, ``c"..."``, ``r"..."``, ``r#"..."#``, multi-line) and character
  literals (``'x'``, ``'é'``, ``'\\x7f'``, ``'\\u{1F600}'``, ``b'\\''``; never
  lifetimes or labels) are replaced by whitespace, and
  (2) test-only *items* are blanked: an item carrying ``#[test]`` or a
  ``#[cfg(...)]`` whose predicate requires ``test`` (``test``, or ``all(..)``
  with a test-requiring member, or ``any(..)`` whose members all require it;
  ``not(..)`` never does) is blanked from that attribute through the end of
  the item (its matching ``}`` or terminating ``;``).  Production code after a
  test module is still counted.  A test-only out-of-line module
  (``#[cfg(test)] mod name;``, honouring ``#[path = "..."]``) makes its file,
  and every module that file declares, test-only as a whole.
  Both steps replace characters one for one: offsets and line numbers (and
  therefore function lengths) are preserved.
* ``panic_sites`` -- occurrences of ``panic!``, ``unreachable!``,
  ``unimplemented!``, ``todo!``, ``.expect(`` and ``.unwrap(``.
  ``debug_assert*``/``assert*`` are deliberately excluded: those are contract
  checks, not user-input-reachable abort paths.
* ``panic_sites_hot`` -- the same count restricted to ``src/frontend/``,
  ``src/ir/lowering/`` and ``src/driver/``, i.e. the code that parses and
  lowers *user input* (AUD-ICE-1's actual target).
* ``unsafe_sites`` -- occurrences of the ``unsafe`` keyword (blocks, fns,
  impls, traits).
* ``fns_over_300`` / ``fns_over_500`` / ``fns_over_1000`` -- functions whose
  body (from the ``fn`` keyword through its closing brace, in the *stripped*
  text) exceeds the threshold in lines.  A bracket-matching scan, not a
  parser: nested generics (``<T: Into<U>>``, ``F: Fn() -> T``) and bracketed
  ``;`` in return types are handled, and an unclosed signature or body is a
  hard failure (exit 1) rather than a span measured to end of file.
* ``knob_literals`` -- distinct ``CCC_*``/``LCCC_*`` identifiers appearing in
  ``src/`` (a name ending in ``_`` is a prefix, not a knob); internal assembly
  annotation ``LCCC_TAIL_CALL`` is excluded because it is not a configuration
  option. ``knobs_undocumented`` -- those not named in any ``*.md`` in git's
  index (``git ls-files``; untracked notes never count, and a tree without git
  metadata fails). Rust sources are scanned by directory walk, so an untracked
  ``.rs`` can only add sites, never hide them.
* ``ignored_tests`` -- occurrences of ``#[ignore`` in code (comments and
  literals excluded), *including* test code, which is where they live
  (AUD-ARM-GSF-1 requires each one to carry a backlog id or capability gate).

Usage::

    python3 scripts/audit_census.py                  # census table
    python3 scripts/audit_census.py --check          # ratchet vs. baseline
    python3 scripts/audit_census.py --update         # rewrite the baseline
    python3 scripts/audit_census.py --selftest       # scanner self-test
    python3 scripts/audit_census.py --top-fns 25     # longest functions
    python3 scripts/audit_census.py --list-undocumented-knobs

Exit codes: 0 = census printed or ratchet held, 1 = a metric grew beyond the
baseline (or a self-test failed), 2 = usage error.
"""

from __future__ import annotations

import argparse
import bisect
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / "scripts" / "audit_census_baseline.json"

SCAN_DIRS = ("src",)
HOT_PREFIXES = ("src/frontend/", "src/ir/lowering/", "src/driver/")

PANIC_PAT = re.compile(
    r"\bpanic!\s*\(|\bunreachable!\s*\(|\bunimplemented!\s*\(|\btodo!\s*\(|\.expect\s*\(|\.unwrap\s*\("
)
UNSAFE_PAT = re.compile(r"\bunsafe\b")
IGNORE_PAT = re.compile(r"#\[ignore")
# A knob name ends in a letter or digit: `CCC_NO_` (a `starts_with` prefix) or
# `CCC_NO_INLINE_*` (prose for a family) names no variable.
KNOB_PAT = re.compile(r"\b(?:CCC|LCCC)_[A-Z0-9_]*[A-Z0-9]\b")
NON_KNOB_SOURCE_MARKERS = frozenset({"LCCC_TAIL_CALL"})


def source_knob_literals(text: str) -> list[str]:
    return [
        name for name in KNOB_PAT.findall(text) if name not in NON_KNOB_SOURCE_MARKERS
    ]


FN_PAT = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
# One character literal: a plain character (any Unicode scalar), a simple
# escape, `\x7f`, or `\u{...}`.  Anything else after a quote is a lifetime or
# label and stays code.
CHAR_PAT = re.compile(r"'(?:\\(?:u\{[0-9A-Fa-f_]{1,8}\}|x[0-9A-Fa-f]{2}|[^\n])|[^\\'\n])'")


class CensusParseError(ValueError):
    """The stripped text is not shaped like Rust; the census refuses to guess."""


CANDIDATE = re.compile(r'//|/\*|b?"|b?r(#*)"|\'')


def _blank(text: str) -> str:
    """Replace the non-newline characters of ``text`` with spaces."""
    return re.sub(r"[^\n]", " ", text)


def strip_rust(src: str) -> str:
    """Blank out comments and literals, preserving every newline.

    The scan is driven by a regex that finds the *next* interesting token and
    then jumps to its end with ``str.find``; nothing is walked
    character-by-character in Python, so a 700k-line tree is stripped in a few
    seconds.  Replacing (rather than deleting) keeps the newline count -- and
    hence every line number -- identical to the original file.
    """
    out: list[str] = []
    i, n = 0, len(src)
    while True:
        m = CANDIDATE.search(src, i)
        if m is None:
            out.append(src[i:])
            break
        out.append(src[i : m.start()])
        tok = m.group(0)
        if tok == "//":
            j = src.find("\n", m.end())
            j = n if j < 0 else j
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        if tok == "/*":
            # Rust block comments nest.
            depth, j = 1, m.end()
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        if tok.startswith(("r", "br")):  # r"..." / r#"..."# / br#"..."#
            hashes = m.group(1) or ""
            terminator = '"' + hashes
            j = src.find(terminator, m.end())
            j = n if j < 0 else j + len(terminator)
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        if tok.endswith('"'):
            j = m.end()
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1  # Rust string literals may span lines.
            out.append(_blank(src[m.start() : j]))
            i = j
            continue
        # A quote that opens a lifetime ("'a") rather than a character literal.
        cm = CHAR_PAT.match(src, m.start())
        if cm:
            out.append(_blank(src[cm.start() : cm.end()]))
            i = cm.end()
        else:
            out.append(src[m.start() : m.end()])
            i = m.end()
    return "".join(out)


def line_index(text: str) -> list[int]:
    """Offsets of every newline, for O(log n) offset -> line conversion."""
    return [i for i, ch in enumerate(text) if ch == "\n"]


ATTR_START = re.compile(r"#\[")


def _bracket_end(text: str, i: int) -> int:
    """Index just past the bracket group opening at ``text[i]``; len on EOF."""
    pairs = {"(": ")", "[": "]", "{": "}"}
    stack: list[str] = []
    n = len(text)
    while i < n:
        ch = text[i]
        if ch in pairs:
            stack.append(pairs[ch])
        elif stack and ch == stack[-1]:
            stack.pop()
            if not stack:
                return i + 1
        i += 1
    return n


def _cfg_requires_test(pred: str) -> bool:
    """Whether a cfg predicate can only hold in a test build."""
    pred = pred.strip()
    if pred == "test":
        return True
    m = re.fullmatch(r"(all|any|not)\s*\((.*)\)", pred, re.S)
    if not m:
        return False
    args, depth, start = [], 0, 0
    body = m.group(2)
    for k, ch in enumerate(body):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        elif ch == "," and depth == 0:
            args.append(body[start:k])
            start = k + 1
    args.append(body[start:])
    args = [arg for arg in args if arg.strip()]
    if m.group(1) == "all":
        return any(_cfg_requires_test(arg) for arg in args)
    if m.group(1) == "any":
        return bool(args) and all(_cfg_requires_test(arg) for arg in args)
    return False


def _attribute_is_test_only(attr_body: str) -> bool:
    body = attr_body.strip()
    if body == "test":
        return True
    m = re.fullmatch(r"cfg\s*\((.*)\)", body, re.S)
    return bool(m) and _cfg_requires_test(m.group(1))


def test_item_spans(stripped: str) -> list[tuple[int, int, int]]:
    """(attribute start, item start, item end) of every test-only item.

    ``stripped`` must already have comments and literals blanked.  The item
    begins after the attribute group and ends at its first top-level ``;``
    (``use``/``mod x;``/unit items) or at the ``}`` matching its first
    top-level ``{``.  Nested spans inside an excluded item are not reported.
    """
    spans: list[tuple[int, int, int]] = []
    n, i = len(stripped), 0
    while True:
        m = ATTR_START.search(stripped, i)
        if m is None:
            return spans
        attr_end = _bracket_end(stripped, m.start() + 1)
        if not _attribute_is_test_only(stripped[m.start() + 2 : attr_end - 1]):
            i = attr_end
            continue
        # Skip further outer attributes and whitespace up to the item itself.
        j = attr_end
        while True:
            while j < n and stripped[j].isspace():
                j += 1
            if stripped.startswith("#[", j):
                j = _bracket_end(stripped, j + 1)
                continue
            break
        item_start, k = j, j
        while k < n and stripped[k] not in ";{":
            if stripped[k] in "([":
                k = _bracket_end(stripped, k)
                continue
            k += 1
        end = n if k >= n else (k + 1 if stripped[k] == ";" else _bracket_end(stripped, k))
        spans.append((m.start(), item_start, end))
        i = end


def blank_spans(text: str, spans: list[tuple[int, int, int]]) -> str:
    """Blank each (start, _, end) range, keeping every newline."""
    out, last = [], 0
    for start, _item, end in spans:
        out.append(text[last:start])
        out.append(_blank(text[start:end]))
        last = end
    out.append(text[last:])
    return "".join(out)


def strip_tests(stripped: str) -> str:
    """Blank test-only items of comment/literal-stripped text, item-exactly."""
    return blank_spans(stripped, test_item_spans(stripped))


MOD_DECL = re.compile(r"\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
PATH_ATTR = re.compile(r'#\[\s*path\s*=\s*"([^"]+)"\s*\]')


def _module_dir(path: pathlib.Path) -> pathlib.Path:
    if path.name in ("mod.rs", "lib.rs", "main.rs"):
        return path.parent
    return path.parent / path.stem


def _preceding_attributes(stripped: str, pos: int) -> list[tuple[int, int]]:
    """(start, end) of the outer attributes immediately before ``pos``."""
    attrs: list[tuple[int, int]] = []
    j = pos
    while True:
        while j > 0 and stripped[j - 1].isspace():
            j -= 1
        if j == 0 or stripped[j - 1] != "]":
            return attrs
        depth, k = 0, j - 1
        while k >= 0:
            if stripped[k] == "]":
                depth += 1
            elif stripped[k] == "[":
                depth -= 1
                if depth == 0:
                    break
            k -= 1
        if k <= 0 or stripped[k - 1] != "#":
            return attrs
        attrs.append((k - 1, j))
        j = k - 1


def declared_module_files(path: pathlib.Path, raw: str, spans: list[tuple[int, int, int]] | None,
                          stripped: str) -> list[pathlib.Path]:
    """Files of out-of-line modules declared by ``path``.

    With ``spans`` given, only declarations that ARE one of those (test-only)
    items, or lie inside one, are returned; with None, every declaration is.
    ``#[path]`` values are read from ``raw`` at the attribute offsets
    (stripping preserves offsets, but blanks the string itself).
    """
    regions = [(0, len(stripped))] if spans is None else [(a, e) for a, _i, e in spans]
    found: list[pathlib.Path] = []
    for start, end in regions:
        for m in MOD_DECL.finditer(stripped, start, end):
            explicit = [value for a, e in _preceding_attributes(stripped, m.start())
                        for value in PATH_ATTR.findall(raw[a:e])]
            if explicit:
                found.append((path.parent / explicit[0]).resolve())
                continue
            base = _module_dir(path)
            for candidate in (base / f"{m.group(1)}.rs", base / m.group(1) / "mod.rs"):
                if candidate.exists():
                    found.append(candidate.resolve())
                    break
    return found


def test_only_files(sources: dict[pathlib.Path, tuple[str, str]]) -> set[pathlib.Path]:
    """Resolved paths of files reachable only through test-only modules."""
    pending: list[pathlib.Path] = []
    for path, (raw, stripped) in sources.items():
        pending += declared_module_files(path, raw, test_item_spans(stripped), stripped)
    result: set[pathlib.Path] = set()
    while pending:
        path = pending.pop()
        if path in result or path not in sources:
            continue
        result.add(path)
        raw, stripped = sources[path]
        pending += declared_module_files(path, raw, None, stripped)
    return result


def _skip_balanced(text: str, i: int, open_ch: str, close_ch: str, what: str) -> int:
    """Index just past the ``close_ch`` matching ``text[i] == open_ch``."""
    depth, n = 0, len(text)
    while i < n:
        if text[i] == open_ch:
            depth += 1
        elif text[i] == close_ch:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise CensusParseError(f"unclosed {what}")


def _skip_generics(text: str, i: int) -> int:
    """Index just past the generic list opened at ``text[i] == '<'``.

    ``->`` (in ``F: Fn() -> T`` bounds) is not a closing angle bracket, and a
    const-generic ``{ ... }`` argument is skipped as a unit.  ``=>`` is skipped
    the same way: macro inputs such as ``fn f(&self) -> T => f_impl;`` reuse
    the ``fn`` token.
    """
    depth, n = 0, len(text)
    while i < n:
        if text.startswith(("->", "=>"), i):
            i += 2
            continue
        ch = text[i]
        if ch == "{":
            i = _skip_balanced(text, i, "{", "}", "const-generic block")
            continue
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
            if depth == 0:
                return i + 1
        elif ch == ";":
            break
        i += 1
    raise CensusParseError("unclosed generic parameter list")


def _body_start(text: str, i: int) -> int | None:
    """From the end of the parameter list, the index of the body's ``{``, or
    None for a bodiless declaration.  ``;``/``{`` only count outside brackets:
    ``-> [u8; 4]`` is a return type, ``-> Foo<{ N }>`` a const argument."""
    depth, n = 0, len(text)
    while i < n:
        if text.startswith(("->", "=>"), i):
            i += 2
            continue
        ch = text[i]
        if ch in "([<":
            depth += 1
        elif ch in ")]>":
            depth -= 1
            if depth < 0:
                raise CensusParseError("unbalanced signature")
        elif ch == "{":
            if depth == 0:
                return i
            i = _skip_balanced(text, i, "{", "}", "const-generic block")
            continue
        elif ch == ";" and depth == 0:
            return None
        i += 1
    raise CensusParseError("signature runs to end of file")


def scan_functions(
    text: str, newlines: list[int] | None = None
) -> list[tuple[str, int, int]]:
    """Return (name, start_line, end_line) for every brace-delimited ``fn``.

    Generic parameter lists may nest and contain ``->``; return types may
    contain ``;`` inside brackets.  A signature or body that never closes
    raises :class:`CensusParseError` instead of being silently measured to
    end of file or dropped.
    """
    if newlines is None:
        newlines = line_index(text)
    fns: list[tuple[str, int, int]] = []
    n = len(text)
    for m in FN_PAT.finditer(text):
        name = m.group(1)
        start_line = bisect.bisect_right(newlines, m.start()) + 1
        try:
            i = m.end()
            while i < n and text[i].isspace():
                i += 1
            if i < n and text[i] == "<":
                i = _skip_generics(text, i)
                while i < n and text[i].isspace():
                    i += 1
            if i >= n or text[i] != "(":
                raise CensusParseError("no parameter list")
            i = _skip_balanced(text, i, "(", ")", "parameter list")
            body = _body_start(text, i)
            if body is None:
                continue
            end = _skip_balanced(text, body, "{", "}", "function body") - 1
        except CensusParseError as error:
            raise CensusParseError(f"fn {name} at line {start_line}: {error}") from None
        end_line = bisect.bisect_right(newlines, end) + 1
        fns.append((name, start_line, end_line))
    return fns


def strip_text(text: str) -> str:
    return strip_tests(strip_rust(text))


def load_sources() -> dict[pathlib.Path, tuple[str, str]]:
    """Resolved path -> (raw, comment/literal-stripped) for every scanned file."""
    sources: dict[pathlib.Path, tuple[str, str]] = {}
    for directory in SCAN_DIRS:
        for path in sorted((ROOT / directory).rglob("*.rs")):
            raw = path.read_text(errors="ignore")
            sources[path.resolve()] = (raw, strip_rust(raw))
    return sources


def tracked_markdown(root: pathlib.Path) -> list[pathlib.Path]:
    """Markdown files in git's index: only tracked documentation can discharge
    an undocumented-knob debt (an untracked scratch note never ships).  A tree
    without git metadata is a usage error, not an empty documentation set."""
    import subprocess

    try:
        top = subprocess.run(["git", "-C", str(root), "rev-parse", "--show-toplevel"],
                             check=True, capture_output=True, text=True).stdout.strip()
        listed = subprocess.run(["git", "-C", str(root), "ls-files", "-z", "--", "*.md"],
                                check=True, capture_output=True).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        raise CensusParseError(f"documentation scope needs a git checkout: {error}") from None
    # An enclosing repository is not this tree's documentation scope.
    if pathlib.Path(top).resolve() != root.resolve():
        raise CensusParseError(f"{root} is not a git work-tree root (enclosing root: {top})")
    return [root / name for name in listed.decode().split("\0") if name and (root / name).is_file()]


def census() -> dict[str, object]:
    metrics: dict[str, int] = {
        "panic_sites": 0,
        "panic_sites_hot": 0,
        "unsafe_sites": 0,
        "ignored_tests": 0,
        "fns_over_300": 0,
        "fns_over_500": 0,
        "fns_over_1000": 0,
    }
    knobs: set[str] = set()
    knob_sites: dict[str, int] = {}
    files = 0

    sources = load_sources()
    test_files = test_only_files(sources)
    for path, (raw, code) in sources.items():
        files += 1
        rel = path.relative_to(ROOT.resolve()).as_posix()
        # `#[ignore]` lives in test code: count it before tests are removed.
        metrics["ignored_tests"] += len(IGNORE_PAT.findall(code))
        # Knobs name string literals (`env::var("CCC_...")`), so they must be
        # counted on the raw text, not the stripped one.
        for knob in source_knob_literals(raw):
            knobs.add(knob)
            knob_sites[knob] = knob_sites.get(knob, 0) + 1
        if path in test_files:
            continue
        text = strip_tests(code)
        panics = len(PANIC_PAT.findall(text))
        metrics["panic_sites"] += panics
        if rel.startswith(HOT_PREFIXES):
            metrics["panic_sites_hot"] += panics
        metrics["unsafe_sites"] += len(UNSAFE_PAT.findall(text))
        try:
            spans = scan_functions(text)
        except CensusParseError as error:
            raise CensusParseError(f"{rel}: {error}") from None
        for _name, start, end in spans:
            length = end - start + 1
            if length > 1000:
                metrics["fns_over_1000"] += 1
            if length > 500:
                metrics["fns_over_500"] += 1
            if length > 300:
                metrics["fns_over_300"] += 1

    documented: set[str] = set()
    for path in tracked_markdown(ROOT):
        documented |= set(KNOB_PAT.findall(path.read_text(errors="ignore")))

    result: dict[str, object] = dict(metrics)
    result.update(
        {
            "files": files,
            "knob_literals": len(knobs),
            "knob_names": sorted(knobs),
            "knobs_undocumented": len(knobs - documented),
            "knob_undocumented_names": sorted(knobs - documented),
        }
    )
    return result


def top_functions(limit: int) -> list[tuple[int, str, str, int, int]]:
    rows: list[tuple[int, str, str, int, int]] = []
    sources = load_sources()
    test_files = test_only_files(sources)
    for path, (_raw, code) in sources.items():
        if path in test_files:
            continue
        text = strip_tests(code)
        rel = path.relative_to(ROOT.resolve()).as_posix()
        try:
            spans = scan_functions(text, line_index(text))
        except CensusParseError as error:
            raise CensusParseError(f"{rel}: {error}") from None
        for name, start, end in spans:
            rows.append((end - start + 1, rel, name, start, end))
    rows.sort(reverse=True)
    return rows[:limit]


def baseline_path() -> pathlib.Path:
    return BASELINE


def load_baseline() -> dict[str, object]:
    if not BASELINE.exists():
        return {}
    return json.loads(BASELINE.read_text())


RATCHETED = (
    "panic_sites",
    "panic_sites_hot",
    "unsafe_sites",
    "ignored_tests",
    "fns_over_300",
    "fns_over_500",
    "fns_over_1000",
    "knob_literals",
    "knobs_undocumented",
)


def check_self_consistency(report: dict[str, object]) -> str | None:
    """`knob_literals` and `knob_names` are two views of one set.

    A report whose count disagrees with its own name list is a corrupted
    artefact, and it fails the ratchet in a way no code change can explain: the
    count grows, the name sets are identical, and the "new knob" diagnostics
    stay silent. That combination cost a full CI cycle to recognise, so it is
    named explicitly instead of looking like a regression in code that did not
    change.
    """
    names = report.get("knob_names")
    literals = report.get("knob_literals")
    if isinstance(names, list) and isinstance(literals, int) and len(names) != literals:
        return (f"knob_literals ({literals}) disagrees with knob_names "
                f"({len(names)} entries)")
    return None


def describe_growth(key: str, baseline: dict[str, object],
                    current: dict[str, object]) -> str:
    """Name what grew, not just by how much.

    "knob_literals: 550 -> 551" in a CI annotation is unactionable: the reader
    has no tree to diff against and no way to tell an intentional new knob from
    a generated file leaking into the walk. The names already travel in the
    report for this purpose, so they go on the same line as the count.
    """
    if not key.endswith(("_literals", "_sites")):
        return ""
    names = sorted(set(current.get("knob_names", ())) - set(baseline.get("knob_names", ())))
    if not names:
        return ""
    shown = ", ".join(names[:8]) + (", ..." if len(names) > 8 else "")
    return f" [{shown}]"


def baseline_label() -> str:
    """`scripts/...` when the baseline lives in the tree, the whole path otherwise.

    `relative_to` raises when the baseline is outside the repository, and a
    diagnostic path must never raise: the caller is already reporting a failure.
    """
    try:
        return str(BASELINE.relative_to(ROOT))
    except ValueError:
        return str(BASELINE)


def cmd_check(current: dict[str, object]) -> int:
    baseline = load_baseline()
    problem = check_self_consistency(baseline) if baseline else None
    if problem:
        print(f"FAIL: {baseline_label()} is self-inconsistent: {problem}"
              f" (re-run --update)", file=sys.stderr)
        return 1
    if not baseline:
        print(f"FAIL: {baseline_label()} is missing (run --update)", file=sys.stderr)
        return 1
    grown = []
    for key in RATCHETED:
        was, now = baseline.get(key), current.get(key)
        if was is None or now is None:
            print(f"FAIL: metric {key} missing from the baseline", file=sys.stderr)
            return 1
        if now > was:
            grown.append(f"{key}: {was} -> {now} (+{now - was})"
                         f"{describe_growth(key, baseline, current)}")
    for key in RATCHETED:
        print(f"  {key:22} {current[key]:6}   baseline {baseline[key]:6}")
    if grown:
        print("FAIL: audit census grew:", file=sys.stderr)
        for line in grown:
            print(f"  {line}", file=sys.stderr)
        known = baseline.get("knob_names")
        if known is not None and current.get("knob_names") is not None:
            for name in sorted(set(current["knob_names"]) - set(known)):
                print(f"  new knob: {name}", file=sys.stderr)
            for name in sorted(set(known) - set(current["knob_names"])):
                print(f"  knob gone: {name}", file=sys.stderr)
        print(
            "Lower the counts, or justify and re-baseline in the commit that adds them.",
            file=sys.stderr,
        )
        return 1
    print("PASS: audit census held (no metric grew)")
    return 0


def cmd_update(current: dict[str, object]) -> int:
    problem = check_self_consistency(current)
    if problem:
        print(f"FAIL: refusing to write an inconsistent baseline: {problem}",
              file=sys.stderr)
        return 1
    baseline = load_baseline()
    payload = {key: current[key] for key in RATCHETED}
    # Diagnostic only (never ratcheted): when a later --check sees the knob
    # count move, it can name the exact knobs instead of just the delta.
    payload["knob_names"] = current["knob_names"]
    old = {key: baseline.get(key) for key in RATCHETED}
    increased = [k for k in RATCHETED if old.get(k) is not None and payload[k] > old[k]]
    BASELINE.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    print(f"baseline written: {baseline_label()}")
    for key in RATCHETED:
        before = old.get(key)
        arrow = "=" if before == payload[key] else f"{before} -> {payload[key]}"
        print(f"  {key:22} {arrow}")
    if increased:
        print(
            "WARNING: "
            + ", ".join(increased)
            + " increased; a ratchet may only be raised in the commit that adds the sites,"
            " and lowering it in the commit that removes them.",
            file=sys.stderr,
        )
    return 0


SELFTEST_SOURCE = r'''
// panic! in a comment must not count
/* block: unreachable!(x) */
fn normal(x: Option<u32>) -> u32 {
    let s = "unwrap() and panic! inside a string";
    let raw = r#"expect( "#;
    let multi = "first line
panic!() on a continuation line";
    x.unwrap() + 1
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "needs capability X"]
    fn t() {
        panic!("test-only panic must not count");
        None::<u32>.expect("me too");
    }
}

fn after_tests(x: Option<u8>) -> u8 { x.expect("production after tests") }

#[cfg(all(test, target_arch = "x86_64"))]
fn all_test() { todo!() }

#[cfg(any(test, feature = "probe"))]
fn any_test(x: Option<u8>) -> u8 { x.unwrap() }

#[cfg(not(test))]
fn not_test(x: Option<u8>) -> u8 { x.unwrap() }

#[test]
fn bare_test() { unreachable!() }

#[cfg(test)]
use std::cell::Cell;
fn after_use() { unimplemented!() }
'''


def _selftest_module_files() -> list[tuple[str, bool]]:
    import tempfile

    with tempfile.TemporaryDirectory(prefix="audit-census-") as tmp:
        root = pathlib.Path(tmp).resolve()
        (root / "parent").mkdir()
        files = {
            root / "lib.rs": "#[cfg(test)]\nmod unit;\nmod parent;\n"
            "#[cfg(test)]\n#[path = \"elsewhere/renamed.rs\"]\nmod moved;\n",
            root / "unit.rs": "mod nested;\nfn t() { panic!() }\n",
            root / "unit" / "nested.rs": "fn n() { panic!() }\n",
            root / "parent.rs": "#[cfg(test)]\nmod parent_tests;\nfn p() {}\n",
            root / "parent" / "parent_tests.rs": "fn q() { panic!() }\n",
            root / "elsewhere" / "renamed.rs": "fn r() { panic!() }\n",
        }
        for path, text in files.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        sources = {path: (text, strip_rust(text)) for path, text in files.items()}
        excluded = test_only_files(sources)
        expected = {root / "unit.rs", root / "unit" / "nested.rs",
                    root / "parent" / "parent_tests.rs", root / "elsewhere" / "renamed.rs"}
        return [
            ("test-only module files (incl. nested, #[path], non-mod.rs parent) excluded",
             excluded == expected),
            ("production module files kept", not ({root / "lib.rs", root / "parent.rs"} & excluded)),
        ]


LEXER_SELFTEST_SOURCE = r'''
fn lexemes<'a>(s: &'a str) -> char {
    let smile = '\u{1F600}'; let accent = 'é'; let del = '\x7f'; let q = b'\'';
    let brace = '{'; let c = c"panic!()"; let cr = cr#"todo!()"#; let b = br"x.unwrap()";
    'outer: loop { break 'outer; }
    s.chars().next().unwrap()
}
'''

TWO_MODULE_SELFTEST_SOURCE = r'''
fn before() { panic!() }
#[cfg(test)]
mod first { fn a() { panic!() } }
fn between() { panic!() }
#[cfg(test)]
mod second { fn b() { panic!() } }
fn after() { panic!() }
'''

SIGNATURE_SELFTEST_SOURCE = r'''
fn nested<T: Into<Vec<u8>>, F: Fn(u8) -> u8>(t: T, f: F) -> u8 {
    f(t.into()[0])
}
fn array_ret<const N: usize>() -> [u8; N] { [0; N] }
fn const_arg() -> Wrap<{ 1 + 2 }> { Wrap }
trait Decl { fn decl(&self) -> [u8; 4]; }
'''


def _selftest_lexer() -> list[tuple[str, bool]]:
    code = strip_rust(LEXER_SELFTEST_SOURCE)
    two = strip_tests(strip_rust(TWO_MODULE_SELFTEST_SOURCE))
    two_lines = [two[: m.start()].count("\n") for m in PANIC_PAT.finditer(two)]
    signatures = strip_tests(strip_rust(SIGNATURE_SELFTEST_SOURCE))
    raised = []
    for broken in ("fn open_body() { if x {\n", "fn open_params(a: u8\n", "fn open_generic<T: Into<u8>(\n"):
        try:
            scan_functions(broken)
            raised.append(False)
        except CensusParseError:
            raised.append(True)
    return [
        ("unicode/escaped/byte char literals and c/cr/br strings blanked; lifetimes and labels kept",
         PANIC_PAT.findall(code) == [".unwrap("] and "1F600" not in code and "x7f" not in code and "{" not in code.split("let brace", 1)[1].split(";", 1)[0]
         and "'a str" in code and "'outer: loop" in code and len(code) == len(LEXER_SELFTEST_SOURCE)),
        ("production before, between and after two test modules counts",
         two_lines == [1, 4, 7]),
        ("nested generics, Fn() -> T bounds, bracketed ; and const-generic braces",
         [name for name, _s, _e in scan_functions(signatures)] == ["nested", "array_ret", "const_arg"]
         and scan_functions(signatures)[0][1:] == (2, 4)),
        ("unclosed body, parameter list and generics are hard failures", raised == [True, True, True]),
        ("knob names exclude prefixes and families",
         source_knob_literals(
             'starts_with("CCC_NO_") CCC_NO_INLINE_* CCC_NO_X LCCC_A1 CCC_ LCCC_TAIL_CALL'
         )
         == ["CCC_NO_X", "LCCC_A1"]),
    ]


def _selftest_documentation_scope() -> list[tuple[str, bool]]:
    import subprocess
    import tempfile

    with tempfile.TemporaryDirectory(prefix="audit-census-docs-") as tmp:
        root = pathlib.Path(tmp)
        (root / "tracked.md").write_text("CCC_TRACKED\n")
        (root / "untracked.md").write_text("CCC_UNTRACKED\n")
        missing_git = False
        try:
            tracked_markdown(root)
        except CensusParseError:
            missing_git = True
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        subprocess.run(["git", "-C", str(root), "add", "tracked.md"], check=True)
        names = [path.name for path in tracked_markdown(root)]
    return [
        ("only git-tracked markdown documents a knob", names == ["tracked.md"]),
        ("a tree without git metadata fails instead of documenting nothing", missing_git),
    ]


def cmd_selftest() -> int:
    code = strip_rust(SELFTEST_SOURCE)
    stripped = strip_tests(code)
    panic_lines = sorted(stripped[: m.start()].count("\n") + 1 for m in PANIC_PAT.finditer(stripped))
    expected_lines = sorted(
        number for number, line in enumerate(SELFTEST_SOURCE.split("\n"), 1)
        if re.match(r"\s*x\.unwrap\(\) \+ 1|fn (after_tests|any_test|not_test|after_use)", line)
    )
    checks = [
        ("comments, strings and raw strings never count",
         "inside a string" not in code and "block:" not in code and "panic! in" not in code
         and "continuation" not in code
         and all("expect" not in line for line in code.split("\n") if "let raw" in line)),
        ("production after a test module, any(test, ..) and not(test) items count",
         panic_lines == expected_lines and len(expected_lines) == 5),
        ("test module, all(test, ..) item, bare #[test] fn and test-only use blanked",
         "test-only" not in stripped and "todo" not in stripped
         and "bare_test" not in stripped and "Cell" not in stripped),
        ("#[ignore] inside test code is counted", len(IGNORE_PAT.findall(code)) == 1),
        ("line count preserved", stripped.count("\n") == SELFTEST_SOURCE.count("\n")),
        ("offsets preserved", len(stripped) == len(SELFTEST_SOURCE)),
        ("long function detection",
         [name for name, _s, _e in scan_functions(stripped)]
         == ["normal", "after_tests", "any_test", "not_test", "after_use"]),
        *_selftest_module_files(),
        *_selftest_lexer(),
        *_selftest_documentation_scope(),
    ]
    failed = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(f"  {'ok  ' if ok else 'FAIL'} {name}")
    if failed:
        print(f"FAIL: audit_census selftest: {len(failed)} check(s) failed", file=sys.stderr)
        return 1
    print("PASS: audit_census selftest")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="ratchet against the baseline (exit 1 if a metric grew)")
    parser.add_argument("--update", action="store_true", help="rewrite the baseline from this tree")
    parser.add_argument("--selftest", action="store_true", help="self-test the scanner")
    parser.add_argument("--top-fns", type=int, metavar="N", default=0, help="list the N longest functions")
    parser.add_argument(
        "--list-undocumented-knobs",
        action="store_true",
        help="print the CCC_*/LCCC_* literals that no markdown file names",
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return cmd_selftest()

    try:
        current = census()
    except CensusParseError as error:
        print(f"FAIL: audit_census: {error}", file=sys.stderr)
        return 1

    if args.list_undocumented_knobs:
        for name in current["knob_undocumented_names"]:
            print(name)
        return 0

    if args.top_fns:
        print(f"{'lines':>7}  {'function':<44} file:line")
        for length, rel, name, start, _ in top_functions(args.top_fns):
            print(f"{length:7}  {name:<44} {rel}:{start}")
        return 0

    if args.update:
        return cmd_update(current)
    if args.check:
        return cmd_check(current)

    for key in (
        "files",
        "panic_sites",
        "panic_sites_hot",
        "unsafe_sites",
        "ignored_tests",
        "fns_over_300",
        "fns_over_500",
        "fns_over_1000",
        "knob_literals",
        "knobs_undocumented",
    ):
        print(f"{key:22} {current[key]}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

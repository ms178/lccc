#!/usr/bin/env python3
"""Fail when a standalone ci_local gate is absent from hosted workflows.

The local mirror and GitHub workflows intentionally use different orchestration,
but every directly invoked regression shell/Python program in ci_local.sh must
also be EXECUTED by at least one hosted workflow step.

Execution semantics: a command counts only when it appears inside a `run:`
script body of a workflow (single-line or block scalar). Mentioning a gate in
a comment, a step name, or any non-run YAML field does not run anything and
must not satisfy parity -- a workflow that referenced
`tests/regression/check_x.sh` in prose while the step itself was removed would
otherwise pass this check while silently losing coverage.
"""
from __future__ import annotations

import re
import shlex
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / "scripts" / "ci_local.sh"
WORKFLOWS = ROOT / ".github" / "workflows"

COMMAND = re.compile(
    r"(?:tests/regression/[A-Za-z0-9_./-]+\.sh|"
    r"tests/linker/[A-Za-z0-9_./-]+\.(?:py|sh)|"
    r"scripts/[A-Za-z0-9_./-]+\.py|"
    r"\.github/scripts/[A-Za-z0-9_./-]+\.py)"
)

REGRESSION_DIR = ROOT / "tests" / "regression"
ALLOWLIST = ROOT / "scripts" / "ci_gate_allowlist.txt"

# `. path` / `source path` — how one shell gate pulls in a shared library.
SOURCE_RE = re.compile(r"^\s*(?:\.|source)\s+(?:\"([^\"]+)\"|'([^']+)'|(\S+))")
# `# shellcheck source=<repo-relative path>` — the repo-relative spelling of
# the same file, which the `$here/...` form cannot give us.
SHELLCHECK_SOURCE_RE = re.compile(
    r"^\s*#\s*shellcheck\s+source=(\S+)"
)


def sourced_libraries() -> set[str]:
    """Repo-relative paths that some gate script `source`s.

    A sourced file is a library, not a gate: nothing can execute it on its own,
    so requiring a hosted workflow to invoke it directly would be requiring a
    no-op.  It is covered transitively -- the gate that sources it runs in
    hosted CI, and cannot pass without the library.

    Derived by scanning, not by a naming convention.  A `lib_` prefix would
    work today and silently stop working the first time someone sources a
    helper that is not called `lib_*`, which is exactly the kind of gap this
    script exists to close.
    """
    found: set[str] = set()
    candidates: list[Path] = [LOCAL]
    candidates.extend(sorted(WORKFLOWS.glob("*.yml")))
    for directory in (REGRESSION_DIR, ROOT / "scripts", ROOT / ".github" / "scripts"):
        if directory.is_dir():
            candidates.extend(sorted(directory.glob("*.sh")))
            candidates.extend(sorted(directory.glob("*.py")))
    for path in candidates:
        try:
            text = path.read_text()
        except OSError:
            continue
        pending_directive: str | None = None
        for line in text.splitlines():
            d = SHELLCHECK_SOURCE_RE.match(line)
            if d:
                pending_directive = d.group(1)
                continue
            m = SOURCE_RE.match(line)
            if not m:
                # A directive applies only to the source statement it
                # immediately precedes; anything else (including a blank line)
                # breaks the pairing, so a stale directive can never be
                # attached to an unrelated file.
                if line.strip():
                    pending_directive = None
                continue
            raw = next((g for g in m.groups() if g), "")
            # `$here/...` and `$root/...` do not tell us where the library
            # lives, so the path cannot be recovered from the source line
            # alone.  The `# shellcheck source=<repo-relative path>` directive
            # that shellcheck already requires states it exactly, so pair the
            # two: remember the path from the nearest preceding directive and
            # attach it to the source statement that follows.
            if pending_directive:
                found.add(pending_directive)
                pending_directive = None
                continue
            tail = re.sub(r"^\$\{?[A-Za-z_][A-Za-z0-9_]*\}?/", "", raw)
            if "/" in tail:
                found.add(tail)
            elif raw.startswith(("tests/", "scripts/", ".github/")):
                found.add(raw)
    return found



# A step `if:` condition that still runs the step on the ordinary
# (green-path) PR run. Anything else — `failure()`, `runner.os == ...`,
# an expression gate — means the step does not execute on the path the
# parity contract is about, so its body is not coverage.
_ACTIVE_IF = {"always()", "${{ always() }}", "success()", "${{ success() }}"}


def _env_names(env) -> set[str]:
    """LCCC*-tool override names declared by one `env:` mapping."""
    names: set[str] = set()
    if isinstance(env, dict):
        names |= {str(k) for k in env}
    elif isinstance(env, list):
        for item in env:
            if isinstance(item, dict):
                names |= {str(k) for k in item}
    # Exactly the argparse-default override channels of the differential
    # tools under contract (asmdiff/encdiff: LCCC, LCCC_GAS, LCCC_OBJCOPY,
    # LCCC_OBJDUMP). Other LCCC_* variables (e.g. LCCC_RELOCS_TOOL for the
    # linker suite, which passes it on the command line by contract) are
    # not hidden channels for THESE programs.
    return {n for n in names if n in
            {"LCCC", "LCCC_GAS", "LCCC_OBJCOPY", "LCCC_OBJDUMP"}}


def _strip_run_comments(run: str) -> str:
    """One `run:` body, comment-stripped, as joined text lines."""
    pieces: list[str] = []
    for line in run.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        # Cut a trailing comment; `#` can never be part of a matched
        # path (not in the command charset).
        cut = re.search(r"\s#", line)
        pieces.append(line[: cut.start()] if cut else line)
    return "\n".join(pieces)


def workflow_run_steps(path: Path):
    """Yield (active, soft, lccc_env, body) for every `run:` step.

    `active` is False when a step-level `if:` keeps the step off the
    green-path run (its body executes nothing on the PR path this
    checker reasons about); `soft` is True under `continue-on-error`
    (the step runs, but its failure cannot fail the build — a gate
    there is not a gate); `lccc_env` is the set of hidden LCCC* tool
    overrides the step inherits (step `env:` merged over job `env:`).
    """
    try:
        import yaml
    except ImportError:
        print(
            "error: PyYAML is required to verify gate parity with execution "
            "semantics (python3 -m pip install pyyaml)",
            file=sys.stderr,
        )
        raise

    doc = yaml.safe_load(path.read_text())
    if not isinstance(doc, dict):
        return
    jobs = doc.get("jobs") or {}
    if not isinstance(jobs, dict):
        return
    for job in jobs.values():
        if not isinstance(job, dict):
            continue
        # `steps` is the normal location; composite action files would
        # use `runs.steps`, accepted for future-proofing.
        containers = []
        if isinstance(job.get("steps"), list):
            containers.append(job["steps"])
        runs = job.get("runs") or {}
        if isinstance(runs, dict) and isinstance(runs.get("steps"), list):
            containers.append(runs["steps"])
        job_env = _env_names(job.get("env"))
        for steps in containers:
            for step in steps:
                if isinstance(step, dict) and isinstance(step.get("run"), str):
                    cond = step.get("if")
                    active = cond is None or str(cond).strip() in _ACTIVE_IF
                    soft = bool(step.get("continue-on-error"))
                    env = job_env | _env_names(step.get("env"))
                    yield active, soft, env, _strip_run_comments(step["run"])


def run_script_bodies(path: Path) -> str:
    """All ACTIVE `run:` script text of one workflow, comment-stripped.

    Handles the single-line (`run: make check`) and block scalar
    (`run: |`/`run: >`) forms through the YAML parser, so quoting,
    folding, and comment placement cannot fake an execution. Shell
    comment lines and trailing comments inside the script body are
    removed before matching: a gate commented out of its own run block
    executes nothing and must fail parity.

    Execution semantics go one level deeper: a step whose `if:` keeps it
    off the green-path run (anything but always()/success()) does not
    execute on the PR path, so its body is not coverage and is excluded
    here — a workflow that referenced a gate only from a conditional
    step used to pass parity while silently losing coverage.
    """
    return "\n".join(
        body for active, _soft, _env, body in workflow_run_steps(path)
        if active
    )


def allowlist_entries() -> set[str]:
    """Paths named in the orphan allowlist (comments and blanks stripped).

    Each entry is a gate script that is KNOWN to be unwired; the list is a
    reviewed, shrinking record of coverage debt, never a place to park a new
    gate (wiring it into ci_local.sh or a workflow deletes its entry).
    """
    if not ALLOWLIST.exists():
        return set()
    entries = set()
    for line in ALLOWLIST.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        # A trailing "  # reason" annotates the entry; only the path
        # matters for matching (a reason can never contain whitespace-free
        # `tests/...sh` shape that would alias another path).
        path = line.split("#", 1)[0].strip()
        if path:
            entries.add(path)
    return entries


def check_orphaned_gates(local_text: str, hosted: str) -> int:
    """Fail on any tests/regression/check_*.sh executed by nothing.

    The parity check above only sees scripts that ci_local.sh already
    references; a gate script wired into NEITHER mirror is invisible to it.
    That blind spot let a whole batch of codegen-contract gates rot
    unexecuted. Every gate script must therefore be referenced by
    ci_local.sh, by a hosted workflow, or be explicitly recorded in the
    allowlist -- and the allowlist must not carry entries that are no
    longer orphans, so it can only shrink.
    """
    if not REGRESSION_DIR.is_dir():
        return 0
    allow = allowlist_entries()
    orphans = []
    for path in sorted(REGRESSION_DIR.glob("check_*.sh")):
        rel = str(path.relative_to(ROOT))
        if rel in local_text or rel in hosted:
            continue
        if rel not in allow:
            orphans.append(rel)
    stale = sorted(p for p in allow if p in local_text or p in hosted)
    ok = True
    if orphans:
        ok = False
        print("gate scripts are wired into nothing and not allowlisted:", file=sys.stderr)
        for p in orphans:
            print(f"  {p}", file=sys.stderr)
        print(
            "  wire the gate into scripts/ci_local.sh and/or a workflow",
            file=sys.stderr,
        )
    if stale:
        ok = False
        print("allowlist entries that are no longer orphans (delete them):", file=sys.stderr)
        for p in stale:
            print(f"  {p}", file=sys.stderr)
    if not ok:
        return 1
    return 0


def _logical_command_lines(script: str) -> list[list[str]]:
    """Every logical line of a shell script as a shlex token list.

    `\\` continuations are joined first (a wrapped invocation is ONE
    command), comments are shlex-stripped, and lines that do not tokenize
    are dropped — the same conventions the mirrors' own parsers use.
    """
    tokens_per_line: list[list[str]] = []
    lines = script.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        while line.rstrip().endswith("\\") and i < len(lines):
            line = line.rstrip()[:-1] + " " + lines[i]
            i += 1
        if not line.strip():
            continue
        try:
            tokens = shlex.split(line, comments=True)
        except ValueError:
            continue
        if tokens:
            tokens_per_line.append(tokens)
    return tokens_per_line


def program_args(tokens: list[str], program: str) -> list[str] | None:
    """Args after `program` when it stands in COMMAND position, else None.

    Command position: after a `gate NAME fast` wrapper, `env`, and
    VAR=value assignments, optionally behind the python3/bash/sh
    interpreter — exactly the prefixes the mirrors' orchestration uses.
    Anything else (echo, false, `&&`, `if`, prose) is NOT a command
    position: a decoy line that merely CONTAINS the program never
    satisfies a contract. This closes the whole decoy class at once —
    `echo bash scripts/ensure_gas_247.sh x86_64-linux-gnu' provisioned
    nothing but passed a token-pair scan, and a `python3 ...' line that
    was the ARGUMENT of an echo passed a starts-with scan.
    """
    if program not in tokens:
        return None
    head = tokens[: tokens.index(program)]
    if head[:1] == ["gate"]:
        head = head[3:]  # gate NAME fast|slow
    if head[:1] == ["env"]:
        head = head[1:]
    if head and head[-1] in ("python3", "bash", "sh"):
        head = head[:-1]
    if not all(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", t) for t in head):
        return None
    return tokens[tokens.index(program) + 1 :]


def direct_asmdiff_commands(script: str) -> list[list[str]]:
    """Parse directly executed asm-diff commands, preserving mode and corpus.

    Path-only parity conflates x86-64 and i686 invocations of asmdiff.py.
    Only a real command-position `python3 scripts/asmdiff.py ...`
    qualifies: prose, comments, `echo python3 ...', and python3 lines
    that are arguments of another command are not executable
    differential gates.
    """
    return direct_diff_commands(script, "scripts/asmdiff.py")


def direct_diff_commands(script: str, program: str) -> list[list[str]]:
    """Args of every command-position `python3 <program> ...` invocation.

    The asmdiff and encdiff parity checks share this parser: `\\`
    continuations are joined into one logical line (so a gate-prefixed
    invocation yields its argument list with the wrapper resolved away),
    and command position is decided by `program_args` — a `python3 ...'
    that sits inside an echo, behind `false &&', or in prose never
    qualifies.
    """
    commands = []
    for tokens in _logical_command_lines(script):
        args = program_args(tokens, program)
        if args is not None:
            commands.append(args)
    return commands


def asm_option(tokens: list[str], name: str) -> str | None:
    """The value of `name` when the option appears EXACTLY once, else None.

    argparse's plain `store' action is LAST-occurrence-wins, so a command
    with two `--objdump's runs with the second value while a
    first-occurrence reader sees the first — the exact duplicate-override
    false pass. The contract layer therefore never reads single options
    out of a command; this helper (used for diagnostics only) refuses to
    guess when an option is repeated or dangling.
    """
    values: list[str] = []
    for i, token in enumerate(tokens):
        if token == name:
            if i + 1 >= len(tokens):
                return None
            values.append(tokens[i + 1])
        elif token.startswith(name + "="):
            values.append(token[len(name) + 1 :])
    return values[0] if len(values) == 1 else None


# The pinned-oracle PAIR contract, as EXACT tokens: the mirrors must
# spell the pin paths verbatim (shlex-quoted in the scripts; the token
# the parser sees is the unquoted value). Substring containment accepted
# lookalikes — .../bin/objdump-untrusted, /untrusted/$HOME/.../bin/objdump,
# .../bin/objdumps — which is precisely the unpinned-oracle class the pin
# exists to prevent, so the check is equality, not containment.
PINNED_AS = "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"
PINNED_OBJDUMP = "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"


def ensure_gas_invoked(script: str) -> bool:
    """True iff the script runs the exact provisioning command.

    `bash scripts/ensure_gas_247.sh x86_64-linux-gnu' in COMMAND position
    (gate/env/VAR= wrappers and the interpreter resolved away), with the
    target as the one and only argument. The old any-position token scan
    accepted `echo bash scripts/ensure_gas_247.sh x86_64-linux-gnu' and
    `false && bash ...' — lines that provision nothing — and a substring
    scan before that accepted lookalikes outright. Command position plus
    exact args closes the whole decoy class: a mutated target, extra
    arguments, and the wrong target all fail, and so does any spelling
    where the installer is not the command being run.
    """
    for tokens in _logical_command_lines(script):
        args = program_args(tokens, "scripts/ensure_gas_247.sh")
        if args == ["x86_64-linux-gnu"]:
            return True
    return False


def _contract_status(cmd: list[str], option_spec: dict[str, list[str]],
                     rest_spec: list[str]) -> tuple[bool, list[str]]:
    """(matches, human diff) of one command against the token contract.

    The contract has two layers, because argparse semantics have two
    layers:

    - OPTION/VALUE PAIRS: every option the contract names must appear
      with exactly its expected values (`--as' once with the pinned
      path; `--file' twice with the two corpora — append options carry
      a value LIST, compared as multisets). A duplicate value-taking
      option (argparse `store' runs the LAST occurrence while a
      first-occurrence reader validated the first), an injected value,
      and a value SWAPPED between two pinned options (same flat token
      bag, different pairing: `--objdump <as-path> --as <objdump-path>')
      all fail HERE.
    - REST TOKENS: everything else — flags (`--32', `--offline') and
      positionals — as an exact multiset. An unregistered option
      (`--objcopy /evil'), an extra corpus, a dropped flag: all fail
      here. The 64-bit encdiff contract's rest is exactly
      [--offline, --quiet], so `--32' in it is a hard mismatch.
    """
    actual: dict[str, list[str]] = {}
    rest: Counter = Counter()
    i = 0
    while i < len(cmd):
        tok = cmd[i]
        name, eq, inline = tok.partition("=")
        if not tok.startswith("--"):
            rest[tok] += 1
            i += 1
            continue
        if name not in option_spec or (not eq and (
                i + 1 >= len(cmd) or cmd[i + 1].startswith("--"))):
            # Not a contract option (injected flags/tools land here), or
            # a contract option in a shape the contract does not spell
            # (dangling, or its value looks like a flag).
            rest[tok] += 1
            i += 1
            continue
        value = inline if eq else cmd[i + 1]
        i += 1 if eq else 2
        actual.setdefault(name, []).append(value)
    diff: list[str] = []
    for name in sorted(option_spec):
        want, got = Counter(option_spec[name]), Counter(actual.get(name, []))
        if want != got:
            for tok in sorted((want - got).elements()):
                diff.append(f"missing {name} {tok!r}")
            for tok in sorted((got - want).elements()):
                diff.append(f"unexpected {name} {tok!r}")
    want_rest, got_rest = Counter(rest_spec), rest
    if want_rest != got_rest:
        for tok in sorted((want_rest - got_rest).elements()):
            diff.append(f"missing {tok!r}")
        for tok in sorted((got_rest - want_rest).elements()):
            diff.append(f"unexpected {tok!r}")
    return (not diff), diff


def check_asmdiff_gate_parity(local_text: str, hosted: str) -> int:
    """Require the *specific mode, compiler, corpus and oracle PAIR*.

    Each asm-diff gate's invocation is pinned as an exact pair/rest
    contract (see _contract_status): `--jobs 2', the pinned `as' AND
    `objdump' of the 2.47 pair (each exactly once, each with its own
    pinned value), the mode compiler, `--32' present exactly for i686
    — and NOTHING else: no pinned casefile subset (asmdiff.py defaults
    to every tests/asm-diff/*.casefile, so new corpora cannot land
    ungated) and no option or flag the contract does not name.
    """
    specs = (
        ("x86-asm-diff",
         {"--jobs": ["2"], "--as": [PINNED_AS],
          "--objdump": [PINNED_OBJDUMP],
          "--lccc": ["target/fastbuild/lccc-x86"]},
         []),
        ("i686-asm-diff",
         {"--jobs": ["2"], "--as": [PINNED_AS],
          "--objdump": [PINNED_OBJDUMP],
          "--lccc": ["target/fastbuild/lccc-i686"]},
         ["--32"]),
    )
    missing = []
    for where, text in (("local", local_text), ("hosted", hosted)):
        commands = direct_asmdiff_commands(text)
        for gate, option_spec, rest_spec in specs:
            best: list[str] | None = None
            for cmd in commands:
                ok, diff = _contract_status(cmd, option_spec, rest_spec)
                if ok:
                    best = []
                    break
                if ("--as" in cmd or "--objdump" in cmd or "--lccc" in cmd
                        or "--32" in cmd):
                    best = diff
            if best is None:
                best = ["no asm-diff invocation of this gate found"]
            if best:
                missing.append(f"{where}: {gate} (exact token contract; "
                               + "; ".join(best) + ")")
    for where, text in (("local", local_text), ("hosted", hosted)):
        if not ensure_gas_invoked(text):
            missing.append(f"{where}: provision the pinned 2.47 oracle pair "
                           "(exact command, in command position: "
                           "bash scripts/ensure_gas_247.sh x86_64-linux-gnu)")
    if missing:
        print("missing mode/corpus-specific assembly gates:", file=sys.stderr)
        for item in missing:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


# The encdiff corpus gate contract: the exact invocation both mirrors must
# run. The corpus set is an INVARIANT, not a default — a third corpus file
# is a real coverage change that must update this contract consciously
# (same discipline as the pinned-oracle count in the parity tests). So
# are the two pins beyond the assembler: the 2.47 objdump (the disassembler decides BEATS/ok
# verdicts — an unpinned objdump is an unpinned oracle, whatever binutils
# the runner image ships) and the checked-in verdict histogram (the
# aggregate record: BEATS -> ok-best drift, new rows, deleted rows and
# brand-new verdict classes all fail until the baseline is re-recorded).
ENCDIFF_CORPUS_FILES = (
    "tests/encdiff-corpus/index-fold-64.insn",
    "tests/encdiff-corpus/data16-branches-64.insn",
)
ENCDIFF_HISTOGRAM = "tests/encdiff-corpus/expected-verdicts.txt"


def check_encdiff_gate_parity(local_text: str, hosted: str) -> int:
    """Require the IDENTICAL encdiff corpus invocation on both mirrors.

    The encdiff-corpus gate is byte-truth-dependent exactly like the
    asm-diff gates (GAS 2.47 pinned oracle; --offline so the verdicts are
    network-independent; the 64-bit law corpora, not their 32-bit
    sibling). The invocation is pinned as an exact pair/rest contract
    (see _contract_status): the pinned oracle PAIR (each half exactly
    once with its own value — a swap of the two paths is a mismatch,
    not a spelling), the one checked-in verdict baseline, both 64-bit
    law corpora through `--file' (an append option: twice, with exactly
    those two values), and NOTHING else — a duplicate option override,
    an injected `--32' (argparse would run the 32-bit corpus while the
    checker validated the 64-bit one), an unregistered extra tool, or a
    duplicated `--file' all fail here. Path-level mirroring only proves
    the script path appears somewhere on the other side: a hosted-only
    edit — depinned --as, dropped --offline, a swapped corpus file, the
    wrong compiler mode, or removing the step while ci_local keeps its
    copy — passed every other check in this module before these
    contracts existed.
    """
    option_spec = {
        "--lccc": ["target/fastbuild/lccc-x86"],
        "--as": [PINNED_AS],
        "--objdump": [PINNED_OBJDUMP],
        "--expect-histogram": [ENCDIFF_HISTOGRAM],
        "--file": list(ENCDIFF_CORPUS_FILES),
    }
    rest_spec = ["--offline", "--quiet"]
    missing = []
    for where, text in (("local", local_text), ("hosted", hosted)):
        commands = direct_diff_commands(text, "scripts/encdiff.py")
        best: list[str] | None = None
        for cmd in commands:
            ok, diff = _contract_status(cmd, option_spec, rest_spec)
            if ok:
                best = []
                break
            best = diff if best is None else best
        if best is None:
            best = ["no encdiff.py invocation found"]
        if best:
            missing.append(
                f"{where}: encdiff-corpus gate "
                "(exact token contract; " + "; ".join(best) + ")"
            )
    if 'gate "encdiff-corpus" fast' not in local_text:
        missing.append('local: fast gate registration for "encdiff-corpus"')
    if missing:
        print("encdiff corpus gate parity:", file=sys.stderr)
        for item in missing:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


def shell_commands(script: str, program: str) -> list[list[str]]:
    """Tokens of every logical shell line that executes `program`.

    `\\` continuations are joined first, so an environment assignment on a
    line of its own still belongs to the command it prefixes.  Only a
    command position counts: leading `env`/`gate NAME fast env` wrappers,
    VAR=value assignments and the interpreter may precede it, prose may not.
    """
    commands = []
    lines = script.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        while line.rstrip().endswith("\\") and i < len(lines):
            line = line.rstrip()[:-1] + " " + lines[i]
            i += 1
        if program not in line:
            continue
        try:
            tokens = shlex.split(line, comments=True)
        except ValueError:
            continue
        if program not in tokens:
            continue
        head = tokens[: tokens.index(program)]
        if head[:1] == ["gate"]:
            head = head[3:]  # gate NAME fast|slow
        if head[:1] == ["env"]:
            head = head[1:]
        if head and head[-1] in ("python3", "bash"):
            head = head[:-1]
        if all(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", t) for t in head):
            commands.append(tokens)
    return commands


def check_linker_suite_parity(local_text: str, hosted: str) -> int:
    """Both sides run the WHOLE linker suite, strictly, i386 included.

    Path parity alone accepted `run_linker_tests.py --filter i386_` -- a
    handful of the suite's tests -- as "the linker suite runs in CI".
    Require on each side a real invocation with --strict (SKIP/WARN fail),
    no --filter/--tag/--list, LCCC_REQUIRE_I386=1, the kernel relocs tool
    and the pinned GNU as 2.47 first in PATH.
    """
    program = "tests/linker/run_linker_tests.py"
    missing = []
    for where, text in (("local", local_text), ("hosted", hosted)):
        ok = False
        for tokens in shell_commands(text, program):
            args = tokens[tokens.index(program) + 1 :]
            env = tokens[: tokens.index(program)]
            if (
                "--strict" in args
                and not any(a.split("=", 1)[0] in ("--filter", "--tag", "--list") for a in args)
                and "LCCC_REQUIRE_I386=1" in env
                and any(t.startswith("LCCC_RELOCS_TOOL=") for t in env)
                and any(
                    t.startswith("PATH=") and "gas-2.47-x86_64-linux-gnu/bin:" in t for t in env
                )
            ):
                ok = True
        if not ok:
            missing.append(
                f"{where}: full-suite `{program} --strict` with LCCC_REQUIRE_I386=1, "
                "LCCC_RELOCS_TOOL and the GNU as 2.47 PATH (no --filter/--tag)"
            )
        if not shell_commands(text, "tests/linker/setup_kernel_tools.sh"):
            missing.append(f"{where}: bash tests/linker/setup_kernel_tools.sh")
    if missing:
        print("linker suite is not run in full on both sides:", file=sys.stderr)
        for item in missing:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


HOSTED_ONLY = ROOT / "scripts" / "ci_hosted_only.txt"
CARGO_SUBCOMMAND = re.compile(r"\bcargo\s+(?:\+\S+\s+)?([a-z][a-z-]*)")
CARGO_CONFIG = re.compile(r"""--config[\s=]+(['"]?)([A-Za-z0-9_.-]+=[^'"\s]+)\1""")


# The programs whose INVOCATION is under exact contract elsewhere in this
# module (differential gates, the pinned-oracle provisioner, and their
# compiler-free contract suites). check_step_guards holds every one of
# them to the workflow-level execution semantics: active, not
# continue-on-error, and free of hidden LCCC* tool overrides.
GUARDED_PROGRAMS = (
    "scripts/asmdiff.py",
    "scripts/encdiff.py",
    "scripts/ensure_gas_247.sh",
    "scripts/test_asmdiff.py",
)


def check_step_guards(paths) -> int:
    """Gate commands must be real, failing, unperturbed workflow steps.

    Three ways a workflow can display a gate while not running it as a
    gate, none visible to any text-level parity check:

    - an `if:` that keeps the step off the green-path run (the gate
      'exists' but executes on no PR path — run_script_bodies already
      excludes these bodies from parity; this reports them by name);
    - `continue-on-error: true` (the step runs, but its failure cannot
      fail the build — a gate whose red is invisible is not a gate);
    - a step/job `env:` setting LCCC, LCCC_GAS, LCCC_OBJCOPY or
      LCCC_OBJDUMP (the argparse-default override channels of the
      differential tools: a hidden env can swap the tool a command
      line's pins never chose).
    """
    problems: list[str] = []
    for path in paths:
        for active, soft, env, body in workflow_run_steps(path):
            tokens = _logical_command_lines(body)
            guarded = sorted({
                program for program in GUARDED_PROGRAMS
                if any(program_args(t, program) is not None for t in tokens)
            })
            if not guarded:
                continue
            label = ", ".join(guarded)
            if not active:
                problems.append(
                    f"{path.name}: {label} sits in a step whose `if:` keeps "
                    "it off the green-path run — a gate that does not "
                    "execute on the PR path is not coverage")
            if soft:
                problems.append(
                    f"{path.name}: {label} runs under continue-on-error — "
                    "a gate whose failure cannot fail the build is not a "
                    "gate")
            if env:
                problems.append(
                    f"{path.name}: {label} runs with hidden tool-override "
                    f"env {sorted(env)} — the tools under contract come "
                    "from the pinned command line, never an environment "
                    "default")
    if problems:
        print("gate steps violate execution semantics:", file=sys.stderr)
        for item in problems:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


# The compiler-free contract suites of the differential infrastructure:
# asmdiff's parser/oracle unit tests (mocked failure modes + a real
# toolchain leg) and the binutils provisioner's validation matrix. Both
# run without a compiler and without network, exactly like
# test_encdiff.py / test_ci_gate_parity.py, whose registrations set the
# precedent: a suite nothing executes is a contract nothing enforces.
CONTRACT_SUITES = (
    ("scripts/test_asmdiff.py", []),
    ("scripts/ensure_gas_247.sh", ["--self-test"]),
)
CONTRACT_SUITE_GATES = (
    "asmdiff-semantic-validation",
    "gas-oracle-pair-self-test",
)


def check_test_suite_registration(local_text: str, hosted: str) -> int:
    """Both mirrors must RUN the differential contract suites.

    The real differential gates are integration checks; they do not
    replace the synthetic parser/validator regressions (the failure
    modes they pin — mocked failed disassemblers, orphan listings, fake
    tool pairs — are exactly the ones an end-to-end green run cannot
    exhibit). The audit class: test_asmdiff.py and the provisioner's
    --self-test shipped unwired — 19 + 12 pinned contract cases that no
    CI path executed.
    """
    missing = []
    for where, text in (("local", local_text), ("hosted", hosted)):
        for program, args in CONTRACT_SUITES:
            if not any(
                program_args(tokens, program) == args
                for tokens in _logical_command_lines(text)
            ):
                missing.append(
                    f"{where}: {' '.join([program, *args])} "
                    "(compiler-free contract suite)")
    for gate in CONTRACT_SUITE_GATES:
        if f'gate "{gate}" fast' not in local_text:
            missing.append(f'local: fast gate registration for "{gate}"')
    if missing:
        print("differential contract suites are not wired into both mirrors:",
              file=sys.stderr)
        for item in missing:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


def check_hosted_steps_mirrored(local_text: str, hosted: str) -> int:
    """The reverse direction: everything hosted CI executes runs locally too.

    main() proves every ci_local gate is also hosted; nothing proved the
    converse, so a step added to a workflow alone (PR #639's two
    debug-assertions steps, the inline-asm UTF-8 check) left ci_local
    green on a tree CI could fail.  Checked: every gate/helper script a
    workflow executes, every cargo subcommand, and every `--config` build
    mode (a different profile is a different compiler: debug-assertions
    compile in the %rax shadow-epoch validator).  Exceptions live in
    ci_hosted_only.txt with a reason; like the orphan allowlist it can only
    shrink -- an entry that ci_local now mirrors must be deleted.
    """
    # Execution semantics on the local side too: a ci_local comment that
    # names a script or a build mode runs nothing.
    kept = []
    for line in local_text.splitlines():
        if line.lstrip().startswith("#"):
            continue
        cut = re.search(r"\s#", line)
        kept.append(line[: cut.start()] if cut else line)
    local_text = "\n".join(kept)
    allow = set()
    if HOSTED_ONLY.exists():
        for line in HOSTED_ONLY.read_text().splitlines():
            entry = line.split("#", 1)[0].strip()
            if entry:
                allow.add(entry)
    missing = []
    for path in sorted(set(COMMAND.findall(hosted))):
        if path not in local_text and path not in allow:
            missing.append(f"script {path}")
    local_subs = set(CARGO_SUBCOMMAND.findall(local_text))
    for sub in sorted(set(CARGO_SUBCOMMAND.findall(hosted))):
        if sub not in local_subs and f"cargo {sub}" not in allow:
            missing.append(f"cargo {sub}")
    local_cfg = {m[1] for m in CARGO_CONFIG.findall(local_text)}
    for cfg in sorted({m[1] for m in CARGO_CONFIG.findall(hosted)}):
        if cfg not in local_cfg and f"--config {cfg}" not in allow:
            missing.append(f"--config {cfg}")
    stale = sorted(
        e
        for e in allow
        if e in local_text
        or (e.startswith("cargo ") and e[6:] in local_subs)
        or (e.startswith("--config ") and e[9:] in local_cfg)
    )
    if missing:
        print("ci_local.sh does not mirror these hosted CI steps:", file=sys.stderr)
        for item in missing:
            print(f"  {item}", file=sys.stderr)
        print(
            "  mirror them in scripts/ci_local.sh (or, for a measurement that has"
            " no local meaning, record it with a reason in"
            " scripts/ci_hosted_only.txt)",
            file=sys.stderr,
        )
    if stale:
        print("ci_hosted_only.txt entries ci_local.sh now mirrors (delete them):", file=sys.stderr)
        for item in stale:
            print(f"  {item}", file=sys.stderr)
    return 1 if missing or stale else 0


# Corpus metadata: discovery must cover future test modules, not one pinned file.
FUZZ_TEST_COMMAND = ("python3", "-m", "unittest", "discover", "-s", "tests/fuzz", "-p", "test_*.py")


def check_fuzz_test_gate_parity(local_text: str, hosted: str) -> int:
    """Require executable discovery in both mirrors, and a FAST local gate."""
    for where, text, wanted in (
        ("local", local_text, ("gate", "fuzz-harness-tests", "fast", *FUZZ_TEST_COMMAND)),
        ("hosted", hosted, FUZZ_TEST_COMMAND),
    ):
        commands = []
        for line in text.replace("\\\n", " ").splitlines():
            try:
                commands.append(tuple(shlex.split(line, comments=True)))
            except ValueError:
                continue
        if wanted not in commands:
            print(f"missing {where} fuzz-harness discovery gate", file=sys.stderr)
            return 1
    return 0


GATE_REGISTRATION = re.compile(r'^gate\s+"([^"]+)"', re.M)


def check_gate_name_uniqueness(local_text: str) -> int:
    """Every `gate "<name>"` registration must appear exactly once.

    A duplicated registration is not a harmless repeat: it re-runs the whole
    gate, which for the C-compiling gates means recompiling and re-running a
    corpus a second time, and it inflates the PASSED count so a green summary
    overstates how much was actually checked.  Worse, the two copies can drift
    -- one edited, one forgotten -- at which point the summary still says the
    gate passed while the version that was meant to run never did.

    This is the check that was missing when four duplicates shipped: the rest
    of this module reduces gate commands to a `set`, which erases exactly the
    multiplicity a duplicate is made of.  Both a repeated gate *name* and a
    repeated gate *command path* are reported, because they are different
    mistakes with the same symptom.
    """
    names = GATE_REGISTRATION.findall(local_text)
    dup_names = sorted(n for n, c in Counter(names).items() if c > 1)
    if dup_names:
        print("ci_local.sh registers the same gate name more than once:", file=sys.stderr)
        for name in dup_names:
            lines = [
                str(i + 1)
                for i, line in enumerate(local_text.splitlines())
                if line.startswith(f'gate "{name}"')
            ]
            print(f"  {name}: registered {len(lines)}x at line(s) {', '.join(lines)}", file=sys.stderr)
        print(
            "  a duplicate re-runs the gate and inflates the PASSED count; "
            "keep the registration that sits with its rationale",
            file=sys.stderr,
        )
        return 1

    # Deliberately NOT a multiplicity check over command paths.  The same
    # script under different arguments is a legitimate and common shape here:
    # `check_volatile_destructuring.py` runs as --self-test and then for real,
    # and `asmdiff.py` / `fuzz_diff.py` run once per mode because an i686 gate
    # must not stand in for the missing x64 corpus (see
    # test_ci_gate_parity.py).  COMMAND.findall also matches inside comments,
    # so a path named in prose would count as an invocation.  Requiring unique
    # paths would fail the clean tree; requiring unique gate *names* does not,
    # and catches the actual defect -- a re-registered gate.
    return 0


def main() -> int:
    local_text = LOCAL.read_text()
    if check_gate_name_uniqueness(local_text) != 0:
        return 1
    local_paths = set(COMMAND.findall(local_text)) - sourced_libraries()
    workflows = sorted(WORKFLOWS.glob("*.yml"))
    bodies = []
    for path in workflows:
        bodies.append(run_script_bodies(path))
    hosted = "\n".join(bodies)
    missing = sorted(path for path in local_paths if path not in hosted)
    if missing:
        print("hosted CI is missing standalone ci_local gates:", file=sys.stderr)
        for path in missing:
            print(f"  {path}", file=sys.stderr)
        return 1
    rc = check_orphaned_gates(local_text, hosted)
    if rc != 0:
        return rc
    if check_fuzz_test_gate_parity(local_text, hosted) != 0:
        return 1
    if check_asmdiff_gate_parity(local_text, hosted) != 0:
        return 1
    if check_encdiff_gate_parity(local_text, hosted) != 0:
        return 1
    if check_test_suite_registration(local_text, hosted) != 0:
        return 1
    if check_step_guards(workflows) != 0:
        return 1
    if check_hosted_steps_mirrored(local_text, hosted) != 0:
        return 1
    if check_linker_suite_parity(local_text, hosted) != 0:
        return 1
    print(
        f"CI/local standalone gate parity: PASS ({len(local_paths)} commands, "
        "2 mode/corpus-specific asm-diff gates, encdiff corpus gate, "
        "contract suites wired, step semantics guarded, "
        "strict full linker suite, hosted steps mirrored)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

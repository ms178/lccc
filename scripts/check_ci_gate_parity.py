#!/usr/bin/env python3
"""Fail when the CI mirrors' gate invocations drift from their contracts.

The local mirror and GitHub workflows intentionally use different
orchestration, but the CONTRACTS — which programs run, with which pinned
tools, which corpora, which verdict baselines — must hold on both sides.

Three layers, weakest to strongest:

1. Path parity: every directly invoked regression shell/Python program in
   ci_local.sh must also be EXECUTED by at least one hosted workflow step,
   and (converse direction) everything hosted CI executes runs locally too.
   Mentioning a gate in a comment, a step name, or any non-run YAML field
   runs nothing and must not satisfy parity.

2. Invocation contracts (this module's core): every command-position
   invocation of a CONTRACTED program must match exactly one entry in
   INVOCATION_CONTRACTS — an exact multiset of option/value pairs, rest
   tokens and environment-assignment prefix. The check is UNIVERSAL, not
   existential: a second, non-conforming invocation of asmdiff.py next to
   the conforming one is a failure, because the non-conforming one is what
   runs ungated. Proving that *a* conforming line exists proves nothing
   about the lines around it. Each contract must also be MATCHED at least
   once per declared side — a contract nothing satisfies is a gate that no
   longer exists.

3. Execution semantics: a contracted program must sit in an active,
   non-soft step (step/job `if:` and `continue-on-error`, workflow/job/step
   `env:` channels, and the WORKFLOW's own `on:` triggers) — a gate whose
   failure cannot fail the build is not a gate, a hidden environment
   channel is a pin that was never chosen, and a workflow that never runs
   on the push/pull_request path (schedule-only, dispatch-only, no `on:` at
   all) is not coverage no matter what its steps say.

The invocation grammar is the mirrors' own: `gate NAME fast|slow`,
`env`/`/usr/bin/env` with assignments and `-u`, `timeout DUR`, `nice`,
`command`, `setsid`, `time`, `nohup`, assignment prefixes, and a `bash -c`
/ `sh -c` payload (resolved recursively, one quoted-string deep). Anything
else before the program — `echo`, `xargs`, `&&`, `if`, prose — is NOT
command position: a line that mentions a contracted program as a token but
cannot be resolved is reported as an unverifiable mention and fails, because
a gate file has no business naming these programs except to run them.

Shell-fidelity limits, stated honestly: tokens are compared after
canonicalising `${VAR}` to `$VAR` (identical expansions), and any token
whose raw spelling contains a single-quoted or backslash-escaped `$` is
refused outright (a literal `$HOME/...` path can never be the working pin).
Execution channels that hide the program inside other tokens — command
substitution, python -c payloads, encoded strings — are beyond static text
parity and belong to review of the gate files themselves; the checker pins
the grammar the mirrors actually use.
"""
from __future__ import annotations

import re
import shlex
import sys
from collections import Counter
from dataclasses import dataclass, field
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


# ── The shell line model ──────────────────────────────────────────────────

# `${VAR}` and `$VAR` expand identically; the contract compares the expansion
# form, so both spellings of the same path are the same token.
_BRACE_RE = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")


def _canonical(token: str) -> str:
    return _BRACE_RE.sub(r"$\1", token)


class _Cmd:
    """One logical shell line: shlex tokens plus their raw spellings.

    `tokens` are the values a POSIX shell would pass (quotes resolved,
    `${VAR}` canonicalised to `$VAR`); `raw` are the posix=False spellings,
    which keep the quote characters and make single-quoted `$`-paths
    distinguishable from double-quoted ones.  A line whose two tokenisations
    disagree in length, or whose raw spelling hides a `$` behind single
    quotes or a backslash, cannot be verified at all.
    """

    __slots__ = ("lineno", "tokens", "raw")

    def __init__(self, lineno: int, tokens: list[str], raw: list[str]) -> None:
        self.lineno = lineno
        self.tokens = tokens
        self.raw = raw

    def quote_problem(self) -> str | None:
        if len(self.tokens) != len(self.raw):
            return "the line tokenises differently with and without quotes"
        for r in self.raw:
            if "'" in r and "$" in r:
                return f"token {r} carries a $ inside single quotes (a literal path, never the expanding pin)"
            if "\\" in r and "$" in r:
                return f"token {r} escapes a $ (a literal path, never the expanding pin)"
        return None

    def display(self) -> str:
        return " ".join(self.raw)


def _logical_command_lines(script: str) -> list[_Cmd]:
    """Every logical line of a shell script as a _Cmd.

    `\\` continuations are joined first (a wrapped invocation is ONE
    command), comments are shlex-stripped, and lines that do not tokenize
    are dropped — heredoc bodies and other non-command text live there.
    A shell-syntax-error line would fail the script itself at runtime, so
    dropping untokenizable lines cannot hide a working invocation.
    """
    out: list[_Cmd] = []
    lines = script.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        i += 1
        lineno = i
        while line.rstrip().endswith("\\") and i < len(lines):
            line = line.rstrip()[:-1] + " " + lines[i]
            i += 1
        if not line.strip():
            continue
        try:
            tokens = shlex.split(line, comments=True)
            raw = shlex.split(line, comments=True, posix=False)
        except ValueError:
            continue
        if tokens:
            out.append(_Cmd(lineno, [_canonical(t) for t in tokens], raw))
    return out


# ── Command position: the mirrors' wrapper grammar ─────────────────────────

_TIMEOUT_DUR = re.compile(r"\d+(\.\d+)?[smhd]?")
_ASSIGN_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=.*")
_INTERPRETERS = frozenset({
    "python3", "python", "bash", "sh",
    "/bin/bash", "/usr/bin/bash", "/bin/sh", "/usr/bin/sh",
})


def _reduce_head(head: list[str]):
    """Consume the mirrors' wrapper grammar off the front of `head`.

    Returns (env_prefix, tail_kind, tail) where tail_kind is:
      'done'  — head fully consumed (wrappers/assignments only),
      'interp'— head ends with a lone interpreter (the program follows),
      'cc'    — head ends with `bash|sh -c PAYLOAD` (the payload follows),
    or None when the head is anything else — which means the program does
    not stand in command position on this line.

    Accepted wrappers, each of which runs the command unchanged:
      gate NAME fast|slow        the local mirror's registration wrapper
      env|/usr/bin/env [VAR=.. | -u NAME | --unset=NAME | -i]
      timeout DUR                GNU timeout's single duration form
      nice [-]N | nice -n N
      command [-p]
      setsid [-w|-f|-c]
      time, nohup
      VAR=value                  shell assignment prefix (any position)
    Anything else — echo, xargs, `&&`, `if`, prose — is not command
    position, and a spelling outside this list (an unknown flag, a
    two-argument timeout) is refused rather than guessed: extend the
    grammar consciously, never permissively.
    """
    env_prefix: list[str] = []
    if head[:1] == ["gate"]:
        if len(head) < 3 or head[2] not in ("fast", "slow"):
            return None
        head = head[3:]
    i = 0
    while i < len(head):
        tok = head[i]
        if tok in ("env", "/usr/bin/env"):
            i += 1
            while i < len(head):
                t = head[i]
                if _ASSIGN_RE.fullmatch(t):
                    env_prefix.append(t)
                    i += 1
                elif t in ("-i", "--ignore-environment"):
                    i += 1
                elif t in ("-u", "--unset"):
                    if i + 1 >= len(head):
                        return None
                    i += 2
                elif t.startswith("--unset="):
                    i += 1
                elif t.startswith("-"):
                    return None  # an env flag this grammar does not know
                else:
                    break  # the command `env` will run
            continue
        if tok == "timeout":
            if i + 1 >= len(head) or not _TIMEOUT_DUR.fullmatch(head[i + 1]):
                return None
            i += 2
            continue
        if tok == "nice":
            i += 1
            if i < len(head) and head[i] == "-n":
                if i + 1 >= len(head) or not re.fullmatch(r"-?\d+", head[i + 1]):
                    return None
                i += 2
            elif i < len(head) and re.fullmatch(r"-?\d+", head[i]):
                i += 1
            continue
        if tok == "command":
            i += 1
            if i < len(head) and head[i] == "-p":
                i += 1
            continue
        if tok == "setsid":
            i += 1
            while i < len(head) and head[i] in ("-w", "--wait", "-f", "--fork", "-c"):
                i += 1
            continue
        if tok in ("time", "nohup"):
            i += 1
            continue
        if _ASSIGN_RE.fullmatch(tok):
            env_prefix.append(tok)
            i += 1
            continue
        if tok in ("bash", "sh") and head[i + 1:i + 2] == ["-c"] \
                and i + 3 == len(head):
            return env_prefix, "cc", head[i + 2]
        if tok in _INTERPRETERS and i + 1 == len(head):
            return env_prefix, "interp", tok
        return None
    return env_prefix, "done", None


def _find_invocation(cmd: _Cmd, program: str, depth: int = 0):
    """(env_prefix, args, where) when `program` stands in command position.

    `where` is the _Cmd the invocation was resolved ON — the line itself,
    or the `bash -c` payload one level down — so quote fidelity is judged
    where the tokens (and their quoting) actually live.

    Returns the string "mention" when the program appears as a token on the
    line (or inside a `bash -c` payload in command position) but cannot be
    resolved — the caller reports it, because a gate file has no legitimate
    way to name these programs except to run them — and None when the
    program is not on the line at all.
    """
    tokens = cmd.tokens
    if program in tokens:
        head = tokens[: tokens.index(program)]
        reduced = _reduce_head(head)
        if reduced is None or reduced[1] == "cc":
            return "mention"
        env_prefix, _kind, _tail = reduced
        return env_prefix, tokens[tokens.index(program) + 1:], cmd
    # The program may hide one quoted-string deep inside `bash -c '...'`.
    reduced = _reduce_head(tokens)
    if reduced is not None and reduced[1] == "cc" and depth < 3:
        _env, _kind, payload = reduced
        try:
            sub_tokens = shlex.split(payload, comments=True)
            sub_raw = shlex.split(payload, comments=True, posix=False)
        except ValueError:
            return None
        if not sub_tokens:
            return None
        sub = _Cmd(cmd.lineno, [_canonical(t) for t in sub_tokens], sub_raw)
        found = _find_invocation(sub, program, depth + 1)
        if isinstance(found, tuple):
            return reduced[0] + found[0], found[1], found[2]
        return found
    return None


def program_args(tokens: list[str], program: str) -> list[str] | None:
    """Args after `program` when it stands in COMMAND position, else None.

    Convenience wrapper over _find_invocation for callers holding a plain
    token list (the mirrors' own parsers use the same convention).
    """
    found = _find_invocation(_Cmd(0, tokens, tokens), program)
    return found[1] if isinstance(found, tuple) else None


# ── The invocation-contract registry ────────────────────────────────────────
# The pinned-oracle PAIR contract, as EXACT tokens: the mirrors must spell
# the pin paths verbatim (shlex-quoted in the scripts; the token the parser
# sees is the unquoted value). Substring containment accepted lookalikes —
# .../bin/objdump-untrusted, /untrusted/$HOME/.../bin/objdump,
# .../bin/objdumps — which is precisely the unpinned-oracle class the pin
# exists to prevent, so the check is equality, not containment.
PINNED_AS = "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as"
PINNED_OBJDUMP = "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/objdump"


@dataclass(frozen=True)
class InvocationContract:
    """One exact invocation shape a contracted program must run under.

    option_spec: option name -> the exact multiset of values argparse must
      see (`--file' twice with the two corpora; each oracle pin once, with
      its own value — argparse `store' is LAST-occurrence-wins, so a
      duplicate override or a value swap between two pins is a mismatch,
      not a spelling).
    rest_spec: every remaining token (flags, positionals) as an exact
      multiset — an unregistered option, an injected `--32', a dropped
      flag: all mismatches. Exact tokens also close argparse's prefix
      ABBREVIATIONS (`--filt' for `--filter'): the mirrors spell every
      option in full, so an abbreviated spelling is an unregistered token.
    env_spec: the exact multiset of VAR=value assignments that must prefix
      the command (the linker suite's PATH/I386/relocs pins; the contract
      suites' pinned-tool channels).
    sides: which mirrors the contract applies to ("local" = ci_local.sh,
      "hosted" = the workflow run bodies). Per-side contracts pin mirrors
      that legitimately differ in detail (hosted's linker run adds -v and
      --json) without weakening either side.
    local_gate: the (name, speed) registration ci_local.sh must carry for
      this contract — parsed from comment-stripped `gate NAME fast|slow'
      lines, so a comment can never satisfy it and a fast->slow demotion
      (which drops the gate from --fast runs) is a failure.
    """

    program: str
    name: str
    sides: frozenset[str]
    option_spec: dict[str, tuple[str, ...]] = field(default_factory=dict)
    rest_spec: tuple[str, ...] = ()
    env_spec: tuple[str, ...] = ()
    local_gate: tuple[str, str] | None = None


_BOTH = frozenset({"local", "hosted"})

# The encdiff corpus contract's invariant pieces: the corpus set is an
# INVARIANT, not a default — a third corpus file is a real coverage change
# that must update this contract consciously (same discipline as the
# pinned-oracle count in the parity tests). So is the checked-in verdict
# histogram (the aggregate record: BEATS -> ok-best drift, new rows,
# deleted rows and brand-new verdict classes all fail until the baseline is
# re-recorded).
ENCDIFF_CORPUS_FILES = (
    "tests/encdiff-corpus/index-fold-64.insn",
    "tests/encdiff-corpus/data16-branches-64.insn",
)
ENCDIFF_HISTOGRAM = "tests/encdiff-corpus/expected-verdicts.txt"

# The contract suites' pinned-tool channels: the real-toolchain legs must
# exercise the provisioned 2.47 pair, and under LCCC_REQUIRE_PINNED_ORACLE=1
# a missing pin is an error, not a skip — in CI a silently-skipped leg is a
# silently-untested parser.
ASMDIFF_SUITE_ENV = (
    f"ASMDIFF_TEST_AS={PINNED_AS}",
    f"ASMDIFF_TEST_OBJDUMP={PINNED_OBJDUMP}",
    "LCCC_REQUIRE_PINNED_ORACLE=1",
)
ENCDIFF_SUITE_ENV = (
    f"ENCDIFF_TEST_OBJDUMP={PINNED_OBJDUMP}",
    "LCCC_REQUIRE_PINNED_ORACLE=1",
)

# The linker suite's environment pins: the pinned GNU as 2.47 first in PATH
# (fixtures assemble with APX REX2 encodings the runner's older gas cannot),
# the fail-closed i386 multilib contract, and the kernel relocs tool.
LINKER_ENV = (
    f"PATH=$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH",
    "LCCC_REQUIRE_I386=1",
    "LCCC_RELOCS_TOOL=$HOME/.cache/lccc-kernel-tools/bin/relocs",
)

INVOCATION_CONTRACTS: tuple[InvocationContract, ...] = (
    InvocationContract(
        program="scripts/asmdiff.py",
        name="x86-asm-diff",
        sides=_BOTH,
        option_spec={
            "--jobs": ("2",),
            "--as": (PINNED_AS,),
            "--objdump": (PINNED_OBJDUMP,),
            "--lccc": ("target/fastbuild/lccc-x86",),
        },
        local_gate=("x86-asm-diff", "fast"),
    ),
    InvocationContract(
        program="scripts/asmdiff.py",
        name="i686-asm-diff",
        sides=_BOTH,
        option_spec={
            "--jobs": ("2",),
            "--as": (PINNED_AS,),
            "--objdump": (PINNED_OBJDUMP,),
            "--lccc": ("target/fastbuild/lccc-i686",),
        },
        rest_spec=("--32",),
        local_gate=("i686-asm-diff", "fast"),
    ),
    InvocationContract(
        program="scripts/encdiff.py",
        name="encdiff-corpus",
        sides=_BOTH,
        option_spec={
            "--lccc": ("target/fastbuild/lccc-x86",),
            "--as": (PINNED_AS,),
            "--objdump": (PINNED_OBJDUMP,),
            "--expect-histogram": (ENCDIFF_HISTOGRAM,),
            "--file": ENCDIFF_CORPUS_FILES,
        },
        rest_spec=("--offline", "--quiet"),
        local_gate=("encdiff-corpus", "fast"),
    ),
    InvocationContract(
        program="scripts/ensure_gas_247.sh",
        name="gas-provision",
        sides=_BOTH,
        rest_spec=("x86_64-linux-gnu",),
        local_gate=("asm-diff-oracle-gas-2.47", "fast"),
    ),
    InvocationContract(
        program="scripts/ensure_gas_247.sh",
        name="gas-self-test",
        sides=_BOTH,
        rest_spec=("--self-test",),
        local_gate=("gas-oracle-pair-self-test", "fast"),
    ),
    InvocationContract(
        program="scripts/test_asmdiff.py",
        name="asmdiff-semantic-validation",
        sides=_BOTH,
        env_spec=ASMDIFF_SUITE_ENV,
        local_gate=("asmdiff-semantic-validation", "fast"),
    ),
    InvocationContract(
        program="scripts/test_encdiff.py",
        name="encdiff-semantic-validation",
        sides=_BOTH,
        env_spec=ENCDIFF_SUITE_ENV,
        local_gate=("encdiff-semantic-validation", "fast"),
    ),
    InvocationContract(
        program="scripts/test_ci_gate_parity.py",
        name="ci-parity-self-test",
        sides=_BOTH,
        local_gate=("ci-asm-diff-parity-self-test", "fast"),
    ),
    InvocationContract(
        program="scripts/check_ci_gate_parity.py",
        name="ci-parity",
        sides=_BOTH,
        local_gate=("ci-gate-parity", "fast"),
    ),
    InvocationContract(
        program="tests/linker/run_linker_tests.py",
        name="linker-suite",
        sides=frozenset({"local"}),
        option_spec={"--lccc": ("target/fastbuild/lccc",)},
        rest_spec=("--strict",),
        env_spec=LINKER_ENV,
        local_gate=("linker-suite", "fast"),
    ),
    InvocationContract(
        program="tests/linker/run_linker_tests.py",
        name="linker-suite",
        sides=frozenset({"hosted"}),
        option_spec={
            "--lccc": ("target/fastbuild/lccc",),
            "--json": ("$RUNNER_TEMP/linker-results.json",),
        },
        rest_spec=("--strict", "-v"),
        env_spec=LINKER_ENV,
    ),
    InvocationContract(
        program="tests/linker/setup_kernel_tools.sh",
        name="kernel-relocs-tool",
        sides=_BOTH,
        rest_spec=("--prefix", "$HOME/.cache/lccc-kernel-tools"),
        local_gate=("kernel-relocs-tool", "fast"),
    ),
)

# The programs under invocation contract — derived, never hand-maintained:
# a contract added to the registry is automatically held to the workflow
# execution semantics (check_step_guards) and to the universal rule.
GUARDED_PROGRAMS = tuple(sorted({c.program for c in INVOCATION_CONTRACTS}))

# Hidden environment channels per contracted program: variables whose value
# a step/job/workflow `env:` must NOT set while the program runs, because
# they override what the invocation's own pins chose. The differential
# tools' argparse-default channels (LCCC*), the provisioner's cache
# redirects (GAS_DL_DIR/GAS_CACHE can point provisioning at a doctored
# tarball or source tree), the encdiff suite's import-time objdump default,
# the linker suite's compiler/reference channels — and the suites' own
# pinned-tool channels, which the contracts require on the COMMAND LINE
# (where a shell assignment prefix wins over any exported value) so that a
# step env of the same name can only ever be dead weight.
PROGRAM_ENV_CHANNELS: dict[str, frozenset[str]] = {
    "scripts/asmdiff.py": frozenset(
        {"LCCC", "LCCC_GAS", "LCCC_OBJCOPY", "LCCC_OBJDUMP"}),
    "scripts/encdiff.py": frozenset(
        {"LCCC", "LCCC_GAS", "LCCC_OBJCOPY", "LCCC_OBJDUMP"}),
    "scripts/test_encdiff.py": frozenset(
        {"LCCC_OBJDUMP", "ENCDIFF_TEST_OBJDUMP"}),
    "scripts/test_asmdiff.py": frozenset(
        {"ASMDIFF_TEST_AS", "ASMDIFF_TEST_OBJDUMP"}),
    "scripts/ensure_gas_247.sh": frozenset({"GAS_DL_DIR", "GAS_CACHE"}),
    "tests/linker/run_linker_tests.py": frozenset(
        {"LCCC_BIN", "LINKTEST_CC", "LINKTEST_CXX"}),
}


# ── Workflow walking with execution semantics ──────────────────────────────

# A step `if:` condition that still runs the step on the ordinary
# (green-path) PR run. Anything else — `failure()`, `runner.os == ...`,
# an expression gate — means the step does not execute on the path the
# parity contract is about, so its body is not coverage.
_ACTIVE_IF = {"always()", "${{ always() }}", "success()", "${{ success() }}"}

# The workflow events that exercise the code under review. A workflow
# whose `on:` declares none of them (schedule-only, dispatch-only, a
# release hook, or no `on:` at all — which GitHub refuses to run) never
# executes on the push/pull_request path, so its steps are not coverage
# regardless of what they contain.
_PR_PATH_EVENTS = frozenset({"push", "pull_request"})


def _workflow_triggers(doc: dict) -> set[str]:
    """Event names a workflow's `on:` declares, all spellings.

    `on:` may be a bare event name, a list of names, or a mapping whose
    KEYS are the event names (`on: {pull_request: {branches: [main]}}`).
    PyYAML resolves the unquoted key `on` as the BOOLEAN True (YAML 1.1),
    so the trigger spec lives under `doc[True]` for every workflow written
    the normal way — reading only `doc["on"]` would silently see nothing
    and treat every workflow as trigger-less. Both spellings are read; a
    workflow with neither declares nothing.
    """
    raw = ()
    for key in ("on", True):
        value = doc.get(key)
        if value is None:
            continue
        if isinstance(value, (str, int, bool)):
            raw += (value,)
        elif isinstance(value, (list, dict)):
            raw += tuple(value)
    return {str(event) for event in raw}


def _env_names(env) -> set[str]:
    """Environment variable names declared by one `env:` mapping."""
    names: set[str] = set()
    if isinstance(env, dict):
        names |= {str(k) for k in env}
    elif isinstance(env, list):
        for item in env:
            if isinstance(item, dict):
                names |= {str(k) for k in item}
    return names


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


class _WorkflowError(ValueError):
    """A workflow file this checker refuses to interpret."""


def _unique_keys(loader, node, deep=False):
    """YAML mapping constructor that refuses duplicate keys.

    PyYAML's default loader silently keeps the LAST of two same-named keys,
    so a workflow edit that (accidentally or not) adds a second `run:` to a
    step parses as something other than what a reader sees in the file.
    A checker built on a silently-collapsing parse is a checker that can be
    fed a different file than the one it printed — refuse instead.
    """
    import yaml
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise _WorkflowError(f"duplicate YAML key {key!r}")
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


def _load_workflow(path: Path) -> dict:
    try:
        import yaml
    except ImportError:
        print(
            "error: PyYAML is required to verify gate parity with execution "
            "semantics (python3 -m pip install pyyaml)",
            file=sys.stderr,
        )
        raise

    class _UniqueKeyLoader(yaml.SafeLoader):
        # A subclass, so the refusing constructor is THIS checker's, never
        # a mutation of the process-global SafeLoader everyone else uses.
        pass

    _UniqueKeyLoader.add_constructor(
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _unique_keys)
    try:
        doc = yaml.load(path.read_text(), Loader=_UniqueKeyLoader)
    except _WorkflowError as e:
        raise _WorkflowError(f"{path.name}: {e}") from None
    except yaml.YAMLError as e:
        raise _WorkflowError(f"{path.name}: invalid YAML: {e}") from None
    return doc if isinstance(doc, dict) else {}


def workflow_run_steps(path: Path):
    """Yield (active, soft, env_names, body, why) for every `run:` step.

    `active` is False when the WORKFLOW's `on:` triggers never run it on
    the push/pull_request path, or a step- OR JOB-level `if:` keeps the
    step off the green-path run; `soft` is True under step- OR job-level
    `continue-on-error` (the step runs, but its failure cannot fail the
    build — a gate there is not a gate); `env_names` is the union of the
    workflow-, job- and step-level `env:` names the step inherits; `why`
    explains a non-active/non-soft verdict for diagnostics.
    """
    doc = _load_workflow(path)
    triggers = _workflow_triggers(doc)
    on_pr_path = bool(triggers & _PR_PATH_EVENTS)
    jobs = doc.get("jobs") or {}
    if not isinstance(jobs, dict):
        return
    wf_env = _env_names(doc.get("env"))
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
        job_env = wf_env | _env_names(job.get("env"))
        job_cond = job.get("if")
        job_active = job_cond is None or str(job_cond).strip() in _ACTIVE_IF
        job_soft = bool(job.get("continue-on-error"))
        for steps in containers:
            for step in steps:
                if isinstance(step, dict) and isinstance(step.get("run"), str):
                    cond = step.get("if")
                    active = cond is None or str(cond).strip() in _ACTIVE_IF
                    soft = bool(step.get("continue-on-error"))
                    env = job_env | _env_names(step.get("env"))
                    why = ""
                    if not on_pr_path:
                        names = ", ".join(sorted(triggers)) or "nothing declared"
                        why = (f"workflow `on:' ({names}) never runs on"
                               " the push/pull_request path")
                    elif not job_active:
                        why = f"job `if: {job_cond}` keeps the whole job off the green-path run"
                    elif not active:
                        why = f"step `if: {cond}` keeps it off the green-path run"
                    elif job_soft:
                        why = "job-level continue-on-error swallows its red"
                    elif soft:
                        why = "continue-on-error swallows its red"
                    yield (active and job_active and on_pr_path), \
                        (soft or job_soft), env, \
                        _strip_run_comments(step["run"]), why


def run_script_bodies(path: Path) -> str:
    """All ACTIVE `run:` script text of one workflow, comment-stripped.

    Handles the single-line (`run: make check`) and block scalar
    (`run: |`/`run: >`) forms through the YAML parser, so quoting,
    folding, and comment placement cannot fake an execution. Shell
    comment lines and trailing comments inside the script body are
    removed before matching: a gate commented out of its own run block
    executes nothing and must fail parity.

    Execution semantics go one level deeper: a step (or job) whose `if:`
    keeps it off the green-path run, or a WORKFLOW whose `on:` triggers
    never fire on the push/pull_request path (schedule-only,
    dispatch-only, no `on:` at all), does not execute on the PR path, so
    its body is not coverage and is excluded here — a workflow that
    referenced a gate only from a conditional step used to pass parity
    while silently losing coverage. Raises _WorkflowError for files the
    duplicate-key-refusing loader rejects.
    """
    return "\n".join(
        body for active, _soft, _env, body, _why in workflow_run_steps(path)
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


# ── Contract evaluation ────────────────────────────────────────────────────

def _contract_status(cmd: list[str], option_spec: dict[str, tuple[str, ...]],
                     rest_spec: tuple[str, ...]) -> list[str]:
    """Diff of one command against the option/rest token contract.

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
      (`--objcopy /evil'), an extra corpus, a dropped flag, and an
      abbreviated option (`--filt', which argparse would resolve to
      `--filter' while the denylist spelled the full word): all fail
      here.
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
    return diff


def _contract_diff(env: list[str], args: list[str],
                   contract: InvocationContract) -> list[str]:
    """Full diff (env prefix + options + rest) of one invocation."""
    diff: list[str] = []
    want_env, got_env = Counter(contract.env_spec), Counter(env)
    if want_env != got_env:
        for tok in sorted((want_env - got_env).elements()):
            diff.append(f"missing env assignment {tok!r}")
        for tok in sorted((got_env - want_env).elements()):
            diff.append(f"unexpected env assignment {tok!r}")
    return diff + _contract_status(args, contract.option_spec,
                                   contract.rest_spec)


def check_invocation_contracts(local_text: str, hosted: str) -> int:
    """UNIVERSAL invocation parity: every invocation matches a contract.

    For each side (ci_local.sh, hosted run bodies), every command-position
    invocation of a contracted program must conform to exactly one
    registered contract — a second, non-conforming invocation next to a
    conforming one is a failure, because it is precisely the line that
    runs ungated (the old existential check proved *a* conforming line
    exists, which says nothing about the lines around it). A line that
    mentions a contracted program as a token but cannot be resolved to
    command position is an unverifiable mention and fails: gate files
    name these programs to run them, not to discuss them. Every contract
    must also be matched at least once on each declared side — a contract
    nothing satisfies is a gate that no longer exists — and the local
    gate registrations (parsed from comment-stripped `gate NAME fast'
    lines) must carry each contract's name and speed class.
    """
    problems: list[str] = []
    # Keyed by contract index: InvocationContract carries a dict (the
    # option spec) and is deliberately not hashable.
    matched = {(i, side): 0 for i, c in enumerate(INVOCATION_CONTRACTS)
               for side in ("local", "hosted") if side in c.sides}
    for side, text in (("local", local_text), ("hosted", hosted)):
        for cmd in _logical_command_lines(text):
            for program in GUARDED_PROGRAMS:
                found = _find_invocation(cmd, program)
                if found is None:
                    continue
                if found == "mention":
                    problems.append(
                        f"{side} line {cmd.lineno}: {program} appears outside "
                        "the command-position grammar — if the line runs it, "
                        "spell it as one command and register it; if it is "
                        "prose, it does not belong in a run body: "
                        f"{cmd.display()}")
                    continue
                quote = found[2].quote_problem()
                if quote is not None:
                    problems.append(
                        f"{side} line {cmd.lineno}: {program} cannot be "
                        f"verified ({quote}) — if the line runs it, spell it "
                        "as one command and register it; if it is prose, it "
                        f"does not belong in a run body: {cmd.display()}")
                    continue
                env, args = found[0], found[1]
                candidates = [(i, c) for i, c in enumerate(INVOCATION_CONTRACTS)
                              if c.program == program and side in c.sides]
                best: tuple[int, InvocationContract, list[str]] | None = None
                for i, contract in candidates:
                    diff = _contract_diff(env, args, contract)
                    if not diff:
                        matched[(i, side)] += 1
                        best = None
                        break
                    if best is None or len(diff) < len(best[2]):
                        best = (i, contract, diff)
                if best is not None:
                    _i, contract, diff = best
                    problems.append(
                        f"{side} line {cmd.lineno}: {program} invocation "
                        f"matches no registered contract (nearest: "
                        f"{contract.name}) — " + "; ".join(diff) +
                        f" | invocation: {cmd.display()}")
    for i, contract in enumerate(INVOCATION_CONTRACTS):
        for side in sorted(contract.sides):
            if matched[(i, side)] == 0:
                problems.append(
                    f"{side}: no invocation matches the {contract.name} "
                    f"contract of {contract.program} — the gate is gone; "
                    "restore it (exact invocation in INVOCATION_CONTRACTS)")
    registrations, malformed = gate_registrations(local_text)
    for contract in INVOCATION_CONTRACTS:
        if contract.local_gate is None:
            continue
        name, speed = contract.local_gate
        got = registrations.get(name)
        if got != speed:
            problems.append(
                f'local: gate "{name}" must be registered with speed '
                f'"{speed}" in ci_local.sh (found: {got!r})')
    if malformed:
        for lineno in malformed:
            problems.append(
                f"local line {lineno}: malformed gate registration — "
                '`gate NAME fast|slow\' is the only accepted shape')
    if problems:
        print("invocation contracts violated (every command-position "
              "invocation of a contracted program must match exactly one "
              "registered contract):", file=sys.stderr)
        for item in problems:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


# ── Gate registrations, parsed (never substring-matched) ───────────────────

def gate_registrations(text: str) -> tuple[dict[str, str], list[int]]:
    """(`gate NAME fast|slow' registrations, malformed line numbers).

    Parsed from the comment-stripped logical lines — the same grammar the
    runner uses — so a registration named in a comment, a step name, or
    prose can never satisfy a contract, and a `gate NAME <anything else>'
    line is reported as malformed instead of being silently ignored.
    """
    registrations: dict[str, str] = {}
    malformed: list[int] = []
    for cmd in _logical_command_lines(text):
        if cmd.tokens[:1] != ["gate"]:
            continue
        if len(cmd.tokens) >= 3 and cmd.tokens[2] in ("fast", "slow"):
            registrations[cmd.tokens[1]] = cmd.tokens[2]
        else:
            malformed.append(cmd.lineno)
    return registrations, malformed


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
    multiplicity a duplicate is made of.  Malformed registrations (a third
    token that is neither fast nor slow) fail too: the runner would treat any
    non-"slow" value as a fast gate, so a typo silently changes the gate's
    --fast behaviour.
    """
    seen: dict[str, list[int]] = {}
    malformed: list[int] = []
    for cmd in _logical_command_lines(local_text):
        if cmd.tokens[:1] != ["gate"]:
            continue
        if len(cmd.tokens) >= 3 and cmd.tokens[2] in ("fast", "slow"):
            seen.setdefault(cmd.tokens[1], []).append(cmd.lineno)
        else:
            malformed.append(cmd.lineno)
    dup_names = sorted(n for n, lines in seen.items() if len(lines) > 1)
    ok = True
    if dup_names:
        ok = False
        print("ci_local.sh registers the same gate name more than once:", file=sys.stderr)
        for name in dup_names:
            print(f"  {name}: registered {len(seen[name])}x at line(s) "
                  f"{', '.join(str(n) for n in seen[name])}", file=sys.stderr)
        print(
            "  a duplicate re-runs the gate and inflates the PASSED count; "
            "keep the registration that sits with its rationale",
            file=sys.stderr,
        )
    if malformed:
        ok = False
        print("malformed gate registrations (`gate NAME fast|slow' is the "
              "only accepted shape):", file=sys.stderr)
        for lineno in malformed:
            print(f"  line {lineno}", file=sys.stderr)
    if not ok:
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


# ── Execution-semantics guard for contracted programs ──────────────────────

def check_step_guards(paths) -> int:
    """Contracted programs must be real, failing, unperturbed steps.

    Five ways a workflow can display a gate while not running it as one,
    none visible to any text-level parity check:

    - a workflow-level `on:` that never fires on the push/pull_request
      path (schedule-only, dispatch-only, release-only, or no `on:` at
      all): nothing in the file executes on the PR path — the bodies are
      excluded from parity and reported here by name;
    - a step- or job-level `if:` that keeps it off the green-path run
      (the gate 'exists' but executes on no PR path — run_script_bodies
      already excludes these bodies from parity; this reports them by
      name);
    - step- or job-level `continue-on-error: true` (the step runs, but
      its failure cannot fail the build — a gate whose red is invisible
      is not a gate);
    - a workflow/job/step `env:` setting one of the program's hidden
      channels (the argparse-default overrides of the differential
      tools, the provisioner's cache redirects, the suites' pinned-tool
      channels): a hidden env can swap the tool a command line's pins
      never chose;
    - an unverifiable mention of the program in a step body (outside
      the command-position grammar or in quotes hiding a literal path).
    """
    problems: list[str] = []
    for path in paths:
        try:
            steps = list(workflow_run_steps(path))
        except _WorkflowError as e:
            problems.append(f"workflow not parseable under the no-duplicate-"
                            f"keys contract: {e}")
            continue
        for active, soft, env_names, body, why in steps:
            cmds = _logical_command_lines(body)
            present: list[str] = []
            mentions: list[str] = []
            for program in GUARDED_PROGRAMS:
                for cmd in cmds:
                    found = _find_invocation(cmd, program)
                    if isinstance(found, tuple):
                        if found[2].quote_problem() is None:
                            present.append(program)
                        else:
                            mentions.append(program)
                        break
                    if found == "mention":
                        mentions.append(program)
                        break
            if not present and not mentions:
                continue
            label = ", ".join(sorted(set(present) | set(mentions)))
            if not active:
                problems.append(
                    f"{path.name}: {label} sits in a step kept off the "
                    f"green-path run ({why}) — a gate that does not execute "
                    "on the PR path is not coverage")
            if soft:
                problems.append(
                    f"{path.name}: {label} runs under continue-on-error "
                    f"({why}) — a gate whose failure cannot fail the build "
                    "is not a gate")
            for program in sorted(set(present)):
                hit = sorted(PROGRAM_ENV_CHANNELS.get(program, frozenset())
                             & env_names)
                if hit:
                    problems.append(
                        f"{path.name}: {program} runs with hidden env "
                        f"{hit} — a channel that can override what the "
                        "invocation's own pins chose")
            for program in sorted(set(mentions) - set(present)):
                problems.append(
                    f"{path.name}: {program} is mentioned in a run body "
                    "outside the command-position grammar — spell it as one "
                    "command or remove the mention")
    if problems:
        print("gate steps violate execution semantics:", file=sys.stderr)
        for item in problems:
            print(f"  {item}", file=sys.stderr)
        return 1
    return 0


HOSTED_ONLY = ROOT / "scripts" / "ci_hosted_only.txt"
CARGO_SUBCOMMAND = re.compile(r"\bcargo\s+(?:\+\S+\s+)?([a-z][a-z-]*)")
CARGO_CONFIG = re.compile(r"""--config[\s=]+(['"]?)([A-Za-z0-9_.-]+=[^'"\s]+)\1""")


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


def main() -> int:
    local_text = LOCAL.read_text()
    failures: list[str] = []
    if check_gate_name_uniqueness(local_text) != 0:
        failures.append("gate name uniqueness")
    local_paths = set(COMMAND.findall(local_text)) - sourced_libraries()
    workflows = sorted(WORKFLOWS.glob("*.yml"))
    try:
        bodies = [run_script_bodies(path) for path in workflows]
    except _WorkflowError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    hosted = "\n".join(bodies)
    missing = sorted(path for path in local_paths if path not in hosted)
    if missing:
        print("hosted CI is missing standalone ci_local gates:", file=sys.stderr)
        for path in missing:
            print(f"  {path}", file=sys.stderr)
        failures.append("hosted gate presence")
    checks = (
        ("orphaned gate scripts", check_orphaned_gates(local_text, hosted)),
        ("fuzz harness discovery", check_fuzz_test_gate_parity(local_text, hosted)),
        ("invocation contracts", check_invocation_contracts(local_text, hosted)),
        ("workflow step guards", check_step_guards(workflows)),
        ("hosted steps mirrored", check_hosted_steps_mirrored(local_text, hosted)),
    )
    for name, rc in checks:
        if rc != 0:
            failures.append(name)
    if failures:
        # Every check has already printed its own diagnostics above; run
        # them ALL (not first-fail) so one run reports every violation.
        print("ci gate parity: FAILED (" + "; ".join(failures) + ")", file=sys.stderr)
        return 1
    print(
        f"CI/local standalone gate parity: PASS ({len(local_paths)} commands, "
        f"{len(INVOCATION_CONTRACTS)} registered invocation contracts, "
        "2 mode/corpus-specific asm-diff gates, encdiff corpus gate, "
        "contract suites wired to the pinned pair, step semantics guarded, "
        "strict full linker suite, hosted steps mirrored)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

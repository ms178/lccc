#!/usr/bin/env python3
"""Check GCC-compatible warning groups, format security, and suppression.

Beyond the original group/format cases this gate carries the GP-09 warning
registry contract: for every warning in `WARNING_REGISTRY` (one fixture per
kind) it pins the measured GCC 16.2 cells — default state, group membership,
`-Wno-<name>` stickiness against group flags, positive re-enablement,
`-Werror=<name>`/`-Wno-error=<name>` pairs, exact diagnostic message and
`[-W…]` label — and asserts GCC and LCCC produce the same result. It also
runs the MB-01 unknown-option policy rows and a precision sweep over the
benchmark corpus: every LCCC warning line must exist in GCC's set.
"""

from __future__ import annotations

import argparse
import os
import re
import shlex
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The phrases below are GCC's C-locale spelling. Under a UTF-8 locale GCC
# quotes with U+2018/U+2019 instead, and Python itself exports
# LC_CTYPE=C.UTF-8 to children when started in the C locale (PEP 538),
# so every compiler runs with LC_ALL=C.
ENV = {**os.environ, "LC_ALL": "C"}
SOURCE = r'''
extern int printf(const char *, ...) __attribute__((format(printf, 1, 2)));
static int unused_file_scope;

int warning_probe(int signed_value, unsigned int unsigned_value) {
    int unused_local = 0;
    int shadowed = 1;
    [[maybe_unused]] int deliberately_suppressed = 1;
    {
        int shadowed = 2;
        (void)shadowed;
        int used_inner_shadow = 3;
        (void)used_inner_shadow;
    }
    return (signed_value < unsigned_value) + printf("%d", 1L);
}
'''

SIGN_CONTROL_SOURCE = r'''
int warning_suppression_probe(unsigned int u, int s) {
    return (u == 0) + (u < 13) + (s == 0U)
        + (u != (int)(s == 0))
        + (u != (((int)(s == 0) | (u != 13)) & ((u & ~1u) == 42)));
}
'''

SIGN_NEGATIVE_SOURCE = r'''
int warning_negative_constant_probe(unsigned int u) {
    return u < -1;
}
'''

FORMAT_SECURITY_SOURCE = r'''
extern int printf(const char *, ...) __attribute__((format(printf, 1, 2)));
void warning_format_security_probe(const char *user_format) {
    printf(user_format);
    printf(user_format, 1);
    printf("%s", user_format);
}
'''

FEATURES = {
    "unused": "unused variable",
    "format": "format '%d'",
    "sign": "different signedness",
    "format-security": "format not a string literal and no format arguments",
}
CASES = {
    "default": ((), set()),
    "wall": (("-Wall",), {"unused", "format"}),
    "wextra": (("-Wextra",), {"sign"}),
    "wall-extra": (("-Wall", "-Wextra"), {"unused", "format", "sign"}),
    "individual-unused": (("-Wunused-variable",), {"unused"}),
    "individual-format": (("-Wformat",), {"format"}),
    "individual-sign": (("-Wsign-compare",), {"sign"}),
    "disable-unused": (("-Wall", "-Wno-unused-variable"), {"format"}),
    "disable-format": (("-Wall", "-Wno-format"), {"unused"}),
    "disable-sign": (("-Wextra", "-Wno-sign-compare"), set()),
    "disable-sign-wall-extra": (("-Wall", "-Wextra", "-Wno-sign-compare"), {"unused", "format"}),
}

FORMAT_SECURITY_CASES = (
    ("format-security", ("-Wformat", "-Wformat-security"), {"format-security"}),
    ("format-security-without-format", ("-Wformat-security",), set()),
    (
        "format-security-disabled",
        ("-Wformat", "-Wformat-security", "-Wno-format-security"),
        set(),
    ),
    (
        "format-security-format-disabled",
        ("-Wformat", "-Wformat-security", "-Wno-format"),
        set(),
    ),
    (
        "format-security-demoted",
        ("-Wall", "-Wformat-security", "-Werror", "-Wno-error=format-security"),
        {"format-security"},
    ),
)
FORMAT_SECURITY_ERROR_FLAGS = ("-Wformat", "-Werror=format-security")

# --- GP-09: the registry contract ---------------------------------------
# One single-kind fixture per registry name; every phrase and cell below was
# measured against GCC 16.2 with `-std=c23` (the gate re-checks GCC first,
# then requires LCCC to match it, so a GCC upgrade cannot silently pass).
KIND_FIXTURES = {
    "implicit-function-declaration": (
        "int probe_ifd(void) { return undeclared_helper(); }",
        "implicit declaration of function 'undeclared_helper'",
    ),
    "cpp": (
        "#warning kind-probe-cpp\nint probe_cpp(void) { return 0; }",
        "#warning kind-probe-cpp",
    ),
    "return-type": ("int probe_rt(void) { }", "control reaches end of non-void function"),
    "incompatible-pointer-types": (
        "void probe_ipt(int *p) { char *q = p; (void)q; }",
        "from incompatible pointer type",
    ),
    "attributes": (
        "void __attribute__((const)) v(void){}",
        "'const' attribute on function returning 'void'",
    ),
    "unused-variable": ("void probe_uv(void) { int probe_local = 1; }", "unused variable"),
    "sign-compare": ("int probe_sc(unsigned u) { return u < -1; }", "different signedness"),
    "pedantic": (
        "int probe_pedantic(int x) { int y = ({ int t = x; t + 1; }); return y; }",
        "ISO C forbids braced-groups",
    ),
    "vla": (
        # The bound result must be READ: with -Wall the write-only shape
        # also pulls in GCC's unrelated -Wunused-but-set-variable, which is
        # a different kind's fixture problem (that gap is backlog-listed).
        "int probe_vla(int n) { int a[n]; a[0] = n; return a[0]; }",
        "variable length array",
    ),
    "format": (
        'extern int printf(const char *, ...) __attribute__((format(printf, 1, 2)));\n'
        'void probe_fmt(void) { printf("%d", 1L); }',
        "format '%d'",
    ),
    "format-security": (
        'extern int printf(const char *, ...) __attribute__((format(printf, 1, 2)));\n'
        "void probe_fsec(const char *s) { printf(s); }",
        "format not a string literal and no format arguments",
    ),
    "implicit-int": ("probe_ii() { return 0; }", "type defaults to 'int'"),
    "return-mismatch": ("int probe_rm(void) { return; }", "'return' with no value"),
    "int-conversion": (
        "void *probe_ic(void) { int v = 1; return v; }",
        "makes pointer from integer without a cast",
    ),
    "pointer-to-int-cast": (
        "int probe_pic(int *p) { return (int)p; }",
        "cast from pointer to integer of different size",
    ),
    "int-to-pointer-cast": (
        "void *probe_ipc(int i) { return (void *)i; }",
        "cast to pointer from integer of different size",
    ),
    "compare-distinct-pointer-types": (
        "int *probe_a; char *probe_b;\nint probe_cdp(void) { return probe_a == probe_b; }",
        "comparison of distinct pointer types lacks a cast",
    ),
    "discarded-qualifiers": (
        "const char *probe_dq_src;\nchar *probe_dq(void) { return probe_dq_src; }",
        "return discards 'const' qualifier from pointer target type",
    ),
    "discarded-array-qualifiers": (
        "const int probe_daq_src[2][2];\nint (*probe_daq(void))[2] { return probe_daq_src; }",
        "return discards 'const' qualifier from pointer target type",
    ),
    "return-local-addr": (
        "int *probe_rla(void) { int probe_local = 0; return &probe_local; }",
        "function returns address of local variable",
    ),
}

# Measured membership/severity cells, stated INDEPENDENTLY of the LCCC
# registry (GCC first, then LCCC must agree):
C99_DEFAULT_ERROR = {
    "implicit-function-declaration",
    "incompatible-pointer-types",
    "implicit-int",
    "return-mismatch",
    "int-conversion",
}
DEFAULT_ON = {
    "implicit-function-declaration",
    "cpp",
    "incompatible-pointer-types",
    "attributes",
    "implicit-int",
    "return-mismatch",
    "int-conversion",
    "pointer-to-int-cast",
    "int-to-pointer-cast",
    "compare-distinct-pointer-types",
    "discarded-qualifiers",
    "discarded-array-qualifiers",
    "return-local-addr",
}
IN_WALL = {"return-type", "unused-variable", "format"}
IN_WEXTRA = {"sign-compare"}
# Diagnostic-label spelling inside [-W…]: GCC prints `[-Wformat=]` /
# `[-Werror=format=]` (trailing `=`, measured); every other kind equals
# its flag name.
SUFFIX = {name: name for name in KIND_FIXTURES}
SUFFIX["format"] = "format="

# LCCC's MB-01 contract for unknown positive options: a tolerated warning
# naming the option, exit 0. GCC's driver hard-errors instead — that
# divergence is the documented MB-01 policy, pinned here so it cannot
# drift in either direction.
MB01_TOLERATED = "is tolerated but not implemented by LCCC"

DIAG_LINE = re.compile(r"^(.*):\d+:\d+: (warning|error): (.*\[-W[^\]]*\])$")


def diag_lines(stderr: str) -> list[tuple[str, str]]:
    """(severity, message-with-label) for every source-located warning/error.

    Driver summaries (`ccc: error: …`, `cc1: some warnings…`) carry no
    `file:line:col` and drop out; source excerpts and carets never match.
    """
    out: list[tuple[str, str]] = []
    for line in stderr.splitlines():
        match = DIAG_LINE.match(line)
        if match:
            out.append((match.group(2), match.group(3)))
    return out


def kind_state(
    compiler: list[str],
    fixture: Path,
    flags: tuple[str, ...],
    object_dir: Path | None = None,
) -> tuple[bool, list[tuple[str, str]]]:
    # `-Wreturn-type` is a CFG analysis in GCC: it never fires under
    # `-fsyntax-only` even with `-Wreturn-type` (measured — the long
    # standing "fires nowhere" anomaly), so the return-type rows compile
    # to an object file the way the warning's real contract does.
    mode = ["-fsyntax-only"]
    if object_dir is not None:
        mode = ["-c", "-o", str(object_dir / (fixture.stem + ".o"))]
    proc = subprocess.run(
        compiler + ["-std=c23", *mode, *flags, str(fixture)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=30,
        env=ENV,
    )
    return proc.returncode != 0, diag_lines(proc.stderr)


def kind_enabled(name: str, flags: tuple[str, ...]) -> bool:
    """Whether `name` is in its enabled state under `flags`, from the
    measured cells: defaults + group membership + positive/-Werror forms,
    with `-Wno-<name>` sticky against groups and re-enabled only by a
    later positive form (GCC 16.2, measured)."""
    if f"-Wno-{name}" in flags:
        positive_spellings = (f"-W{name}", f"-Werror={name}")
        if name == "pedantic":
            positive_spellings = positive_spellings + ("-pedantic", "-pedantic-errors")
        no_index = flags.index(f"-Wno-{name}")
        return any(sp in flags and flags.index(sp) > no_index for sp in positive_spellings)
    on = name in DEFAULT_ON
    if "-Wall" in flags and name in IN_WALL:
        on = True
    if "-Wextra" in flags and name in IN_WEXTRA:
        on = True
    if f"-W{name}" in flags or f"-Werror={name}" in flags:
        on = True
    if name == "pedantic" and ("-pedantic" in flags or "-pedantic-errors" in flags):
        on = True
    return on


def expected_kind_state(
    name: str, flags: tuple[str, ...]
) -> tuple[bool, str | None, str | None, bool]:
    """(fires, severity, label, rc-nonzero) from the measured cells.

    Derived only from the hardcoded matrices above, never from LCCC's own
    registry — the gate is external truth.
    """
    if name == "format-security":
        # The check needs BOTH switches (measured): `-Wformat-security`
        # alone, and every group, stay silent until -Wformat is effective
        # (the option-consistency complaint GCC issues for the ineffective
        # request is location-less and belongs to the option ledger, not
        # this emission matrix — see `kind_cases`, which skips the
        # `-Werror=<name>` row for this kind for that reason).
        fires = kind_enabled("format-security", flags) and kind_enabled("format", flags)
        if not fires:
            return False, None, None, False
        return True, "warning", "[-Wformat-security]", False
    fires = kind_enabled(name, flags)
    if f"-Werror={name}" in flags:
        if name in C99_DEFAULT_ERROR:
            # The dialect default already errors: GCC keeps the plain label
            # for those kinds even when the request is spelled -Werror=
            # (measured) — the promotion is the default severity, not a
            # separate explicit one.
            return True, "error", f"[-W{SUFFIX[name]}]", True
        return True, "error", f"[-Werror={SUFFIX[name]}]", True
    if not fires:
        return False, None, None, False
    # `-W<name> -Werror -Wno-error=<name>` demotes; so does a bare
    # `-Wno-error=<name>` over the dialect default error.
    demoted = f"-Wno-error={name}" in flags
    if demoted:
        return True, "warning", f"[-W{SUFFIX[name]}]", False
    severity = "error" if name in C99_DEFAULT_ERROR else "warning"
    return True, severity, f"[-W{SUFFIX[name]}]", name in C99_DEFAULT_ERROR


def kind_cases(name: str) -> dict[str, tuple[str, ...]]:
    # Cases exercised for every kind; membership semantics live in
    # expected_kind_state so each cell is asserted against GCC's answer.
    cases: dict[str, tuple[str, ...]] = {
        "default": (),
        "positive": (f"-W{name}",),
        "negative": (f"-Wno-{name}",),
        "negative-group": (f"-Wno-{name}", "-Wall", "-Wextra", "-pedantic"),
        "negative-positive": (f"-Wno-{name}", f"-W{name}"),
        "positive-negative": (f"-W{name}", f"-Wno-{name}"),
        "re-enable-error": (f"-Werror={name}",),
        "demote": (f"-W{name}", "-Werror", f"-Wno-error={name}"),
        "default-error-demoted": (f"-Wno-error={name}",),
        "wall": ("-Wall",),
        "wextra": ("-Wextra",),
        "pedantic": ("-pedantic",),
        "wall-wextra": ("-Wall", "-Wextra"),
    }
    if name == "format-security":
        # `-Werror=format-security` without `-Wformat` makes GCC report its
        # location-less option-ledger error ('-Wformat-security' ignored
        # without '-Wformat'), which LCCC's emission registry does not model
        # yet; that row is the option ledger's (GP-22), not this matrix's.
        del cases["re-enable-error"]
    return cases


def compiler_version(command: list[str]) -> str:
    proc = subprocess.run(
        command + ["-dumpfullversion"], text=True, capture_output=True, env=ENV
    )
    if proc.returncode:
        raise RuntimeError(f"cannot query GCC version: {proc.stderr.strip()}")
    return proc.stdout.strip()


def observed_features(stderr: str) -> set[str]:
    return {feature for feature, phrase in FEATURES.items() if phrase in stderr}


def run_case(label: str, command: list[str], flags: tuple[str, ...], source: Path) -> set[str]:
    proc = subprocess.run(
        command + ["-std=c23", "-fsyntax-only", *flags, str(source)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        env=ENV,
        timeout=30,
    )
    if proc.returncode:
        raise RuntimeError(
            f"{label} {flags} rejected the warning fixture (exit {proc.returncode}):\n"
            f"{proc.stderr[-4000:]}"
        )
    found = observed_features(proc.stderr)
    # The C23 attribute and the referenced inner object must remain silent.
    if "deliberately_suppressed" in proc.stderr or "used_inner_shadow" in proc.stderr:
        raise RuntimeError(f"{label} warned about a suppressed/used object under {flags}:\n{proc.stderr}")
    if "unused" in found and (
        "unused_local" not in proc.stderr or "shadowed" not in proc.stderr
    ):
        raise RuntimeError(f"{label} missed an unused local or the shadowed outer object under {flags}:\n{proc.stderr}")
    return found


def run_format_security_error(
    label: str, command: list[str], flags: tuple[str, ...], source: Path
) -> None:
    proc = subprocess.run(
        command + ["-std=c23", "-fsyntax-only", *flags, str(source)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        env=ENV,
        timeout=30,
    )
    if proc.returncode == 0:
        raise RuntimeError(f"{label} {flags} did not promote format-security to an error")
    if "format not a string literal and no format arguments" not in proc.stderr:
        raise RuntimeError(f"{label} {flags} failed without the expected diagnostic:\n{proc.stderr}")
    if "[-Werror=format-security]" not in proc.stderr:
        raise RuntimeError(f"{label} {flags} omitted the GCC-compatible error suffix:\n{proc.stderr}")


def run_kind_matrix(gcc: list[str], lccc: list[str], temp: Path) -> None:
    """The GP-09 registry gate: pinned cells + GCC/LCCC identity per kind."""
    total_cases = 0
    for name, (source, phrase) in sorted(KIND_FIXTURES.items()):
        fixture = temp / f"kind-{name}.c"
        fixture.write_text(source + "\n", encoding="utf-8")
        for case_id, flags in kind_cases(name).items():
            total_cases += 1
            # Return-type rows need real code generation (see kind_state).
            object_dir = temp if name == "return-type" else None
            want_fires, want_sev, want_label, want_rc = expected_kind_state(name, flags)
            g_rc, g_diags = kind_state(gcc, fixture, flags, object_dir)
            # GCC is the oracle: its answer must equal the pinned matrix,
            # so a stale measurement cannot be papered over by agreement.
            g_fires = any(phrase in msg for _, msg in g_diags)
            problems = []
            if g_fires != want_fires:
                problems.append(f"GCC fires={g_fires}, pinned {want_fires}")
            if g_rc != want_rc:
                problems.append(f"GCC rc-nonzero={g_rc}, pinned {want_rc}")
            if want_fires:
                sevs = {sev for sev, msg in g_diags if phrase in msg}
                if sevs != {want_sev}:
                    problems.append(f"GCC severities={sorted(sevs)}, pinned {want_sev}")
                labels = {msg for _, msg in g_diags if phrase in msg}
                if not labels or not all(want_label in msg for msg in labels):
                    problems.append(f"GCC labels={sorted(labels)}, pinned {want_label}")
            l_rc, l_diags = kind_state(lccc, fixture, flags, object_dir)
            if (l_rc, l_diags) != (g_rc, g_diags):
                problems.append(
                    f"LCCC={l_rc, l_diags!r} diverges from GCC={g_rc, g_diags!r}"
                )
            if problems:
                raise RuntimeError(
                    f"registry {name} [{case_id}] {flags}: " + "; ".join(problems)
                )
        print(f"warning registry {name}: PASS ({len(kind_cases(name))} cases)")
    print(f"warning registry matrix: PASS ({len(KIND_FIXTURES)} kinds, {total_cases} cases)")


def run_format_level_rows(gcc: list[str], lccc: list[str], temp: Path) -> None:
    """`-Wformat=0|1|2` and `-Werror=format=0`: the only levels GCC 16.2
    accepts on any -W option (measured); every other `-W<name>=N` is a
    driver error there and an MB-01 tolerated warning here."""
    fmt = temp / "level-format.c"
    fmt.write_text(KIND_FIXTURES["format"][0] + "\n", encoding="utf-8")
    sec = temp / "level-security.c"
    sec.write_text(KIND_FIXTURES["format-security"][0] + "\n", encoding="utf-8")
    rows = (
        ("format-0", fmt, ("-Wformat=0", "-Wall"), False, KIND_FIXTURES["format"][1]),
        ("format-1", fmt, ("-Wformat=1",), True, KIND_FIXTURES["format"][1]),
        ("format-2", sec, ("-Wformat=2",), True, KIND_FIXTURES["format-security"][1]),
        ("error-format-0", fmt, ("-Wall", "-Werror=format=0"), False, KIND_FIXTURES["format"][1]),
    )
    for label, fixture, flags, want_fires, phrase in rows:
        g_rc, g_diags = kind_state(gcc, fixture, flags)
        l_rc, l_diags = kind_state(lccc, fixture, flags)
        g_fires = any(phrase in msg for _, msg in g_diags)
        if g_fires != want_fires:
            raise RuntimeError(f"level {label}: GCC fires={g_fires}, pinned {want_fires}")
        if (l_rc, l_diags) != (g_rc, g_diags):
            raise RuntimeError(
                f"level {label} {flags}: LCCC={l_rc, l_diags!r} != GCC={g_rc, g_diags!r}"
            )
    # -Wformat labels carry the trailing `=`.
    _, g_diags = kind_state(gcc, fmt, ("-Wformat",))
    _, l_diags = kind_state(lccc, fmt, ("-Wformat",))
    if not any("[-Wformat=]" in msg for _, msg in g_diags + l_diags):
        raise RuntimeError("[-Wformat=] label missing from GCC or LCCC format warning")
    print("warning registry format-levels: PASS (4 rows + [-Wformat=] label)")


def run_mb01_rows(gcc: list[str], lccc: list[str], temp: Path) -> None:
    """MB-01: unknown positive -W names are diagnosed (not silent), unknown
    -Wno- names are silently honoured, and `undeclared` stays the deliberate
    compatibility-only switch (known-registry row in LCCC, still rejected by
    GCC's driver — pinned as a documented divergence)."""
    fixture = temp / "mb01.c"
    fixture.write_text("int mb01_probe(void) { return 0; }\n", encoding="utf-8")

    def run(compiler: list[str], flag: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            compiler + ["-std=c23", "-fsyntax-only", flag, str(fixture)],
            cwd=ROOT,
            text=True,
            capture_output=True,
            timeout=30,
            env=ENV,
        )

    positive_unknowns = ("-Wtotally-bogus", "-Wsign-compare=1")
    for flag in positive_unknowns:
        lccc_run = run(lccc, flag)
        if lccc_run.returncode:
            raise RuntimeError(f"MB-01 {flag}: LCCC must stay tolerant, got {lccc_run.returncode}")
        if MB01_TOLERATED not in lccc_run.stderr or flag not in lccc_run.stderr:
            raise RuntimeError(f"MB-01 {flag}: tolerated diagnostic missing:\n{lccc_run.stderr}")
        gcc_run = run(gcc, flag)
        if not gcc_run.returncode:
            raise RuntimeError(f"MB-01 {flag}: GCC 16.2 must reject the option (measurement moved)")
        if f"unrecognized command-line option '{flag}'" not in gcc_run.stderr:
            raise RuntimeError(f"MB-01 {flag}: GCC rejection wording changed:\n{gcc_run.stderr}")
    # `-Wundeclared` is the one compatibility-only switch the registry keeps:
    # LCCC accepts it silently on both signs while GCC's driver still rejects
    # the positive spelling (documented divergence, registry row).
    undeclared = run(lccc, "-Wundeclared")
    if undeclared.returncode or undeclared.stderr.strip() or MB01_TOLERATED in undeclared.stderr:
        raise RuntimeError(f"-Wundeclared must be silently accepted:\n{undeclared.stderr}")
    gcc_undeclared = run(gcc, "-Wundeclared")
    if not gcc_undeclared.returncode:
        raise RuntimeError("-Wundeclared: GCC 16.2 driver rejection changed")
    for flag in ("-Wno-totally-bogus", "-Wno-undeclared"):
        for compiler, tag in ((gcc, "GCC"), (lccc, "LCCC")):
            result = run(compiler, flag)
            if result.returncode or result.stderr.strip():
                raise RuntimeError(
                    f"MB-01 {flag}: {tag} must be silently honoured:\n{result.stderr}"
                )
    print(
        "warning registry mb01-policy: PASS "
        f"({len(positive_unknowns)} tolerated positives, 2 silent negatives)"
    )


PRECISION_LINE = re.compile(r"^.*:(\d+):\d+: (?:warning|error): .*\[(-W[^\]]*)\]$")


def precision_labels(stderr: str) -> set[tuple[str, str]]:
    out: set[tuple[str, str]] = set()
    for line in stderr.splitlines():
        match = PRECISION_LINE.match(line)
        if match:
            out.add((match.group(1), match.group(2)))
    return out


def run_precision_sweep(gcc: list[str], lccc: list[str]) -> None:
    """Every LCCC warning line over the benchmark corpus must exist in
    GCC's set — a lccc-only (line, flag) pair is by definition a bug."""
    programs = sorted((ROOT / "tests/benchmark" / "programs").glob("*.c"))
    if not programs:
        raise RuntimeError("benchmark corpus missing: tests/benchmark/programs/*.c")
    flags = ("-Wall", "-Wextra")
    checked = 0
    skipped: list[str] = []
    gcc_only = 0
    lccc_only: list[str] = []
    # Real code generation for the same reason `kind_state` uses it:
    # GCC's -Wreturn-type analysis only exists past `-fsyntax-only`.
    with tempfile.TemporaryDirectory(prefix="lccc-precision-") as objects:
        objects_dir = Path(objects)
        for program in programs:
            results = []
            for compiler in (gcc, lccc):
                proc = subprocess.run(
                    compiler
                    + [
                        "-std=c23",
                        "-c",
                        "-o",
                        str(objects_dir / (compiler[0].replace("/", "_") + program.name + ".o")),
                        *flags,
                        str(program),
                    ],
                    cwd=ROOT,
                    text=True,
                    capture_output=True,
                    timeout=120,
                    env=ENV,
                )
                results.append(proc)
            g_run, l_run = results
            if g_run.returncode or l_run.returncode:
                skipped.append(program.name)
                continue
            checked += 1
            g_labels = precision_labels(g_run.stderr)
            l_labels = precision_labels(l_run.stderr)
            for line, label in sorted(l_labels - g_labels):
                lccc_only.append(f"{program.name}:{line} {label}")
            gcc_only += len(g_labels - l_labels)
    if lccc_only:
        shown = "\n  ".join(lccc_only[:40])
        more = "" if len(lccc_only) <= 40 else f"\n  … {len(lccc_only) - 40} more"
        raise RuntimeError(
            f"precision sweep: {len(lccc_only)} LCCC-only warning line(s):\n  {shown}{more}"
        )
    print(
        "warning registry precision-sweep: PASS "
        f"({checked} programs, {len(skipped)} skipped for compile divergence, "
        f"{gcc_only} GCC-only known-gap line(s))"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        assert CASES["wextra"][1] == {"sign"}
        assert CASES["wall"][1] == {"unused", "format"}
        assert CASES["wall-extra"][1] == {"unused", "format", "sign"}
        assert FORMAT_SECURITY_CASES[0][2] == {"format-security"}
        assert FORMAT_SECURITY_CASES[2][2] == set()
        assert FORMAT_SECURITY_ERROR_FLAGS == ("-Wformat", "-Werror=format-security")
        # Registry manifest: one fixture per kind, the measured cells stay
        # self-consistent, and no fixture is missing a pin.
        assert len(KIND_FIXTURES) == 20
        assert C99_DEFAULT_ERROR <= DEFAULT_ON
        assert DEFAULT_ON == set(KIND_FIXTURES) - {
            "return-type",
            "unused-variable",
            "sign-compare",
            "pedantic",
            "vla",
            "format",
            "format-security",
        }
        assert IN_WALL == {"return-type", "unused-variable", "format"}
        assert IN_WEXTRA == {"sign-compare"}
        assert not (IN_WALL & IN_WEXTRA)
        assert SUFFIX["format"] == "format="
        assert all(SUFFIX[name] == name for name in KIND_FIXTURES if name != "format")
        structural = {
            "default",
            "positive",
            "negative",
            "negative-group",
            "negative-positive",
            "positive-negative",
            "re-enable-error",
            "demote",
            "default-error-demoted",
            "wall",
            "wextra",
            "pedantic",
            "wall-wextra",
        }
        for name in sorted(KIND_FIXTURES):
            # Every kind exercises the same structural case set, except the
            # documented format-security option-ledger exception.
            expected_cases = structural - (
                {"re-enable-error"} if name == "format-security" else set()
            )
            assert set(kind_cases(name)) == expected_cases
        # Spot-check the expectation logic against pinned cells.
        assert expected_kind_state("unused-variable", ()) == (False, None, None, False)
        assert expected_kind_state("unused-variable", ("-Wall",)) == (
            True,
            "warning",
            "[-Wunused-variable]",
            False,
        )
        assert expected_kind_state(
            "unused-variable", ("-Wno-unused-variable", "-Wall")
        ) == (False, None, None, False)
        assert expected_kind_state("implicit-int", ()) == (
            True,
            "error",
            "[-Wimplicit-int]",
            True,
        )
        assert expected_kind_state("return-type", ()) == (False, None, None, False)
        assert expected_kind_state("format", ("-Werror=format",)) == (
            True,
            "error",
            "[-Werror=format=]",
            True,
        )
        print("warning compatibility manifest self-test: PASS")
        return 0

    gcc = shlex.split(args.gcc)
    lccc = shlex.split(args.lccc)
    version = compiler_version(gcc)
    if not re.match(r"^16\.2(?:\.|$)", version):
        raise RuntimeError(f"expected GCC 16.2, found {version!r}")
    if lccc and not Path(lccc[0]).is_absolute() and "/" in lccc[0]:
        lccc[0] = str((ROOT / lccc[0]).resolve())

    with tempfile.TemporaryDirectory(prefix="lccc-warning-compat-") as temp:
        source = Path(temp) / "warnings.c"
        source.write_text(SOURCE, encoding="utf-8")
        for label, (flags, expected) in CASES.items():
            gcc_found = run_case("GCC 16.2", gcc, flags, source)
            lccc_found = run_case("LCCC", lccc, flags, source)
            if gcc_found != expected:
                raise RuntimeError(
                    f"GCC 16.2 {label} {flags}: expected {sorted(expected)}, got {sorted(gcc_found)}"
                )
            if lccc_found != gcc_found:
                raise RuntimeError(
                    f"LCCC {label} {flags}: expected GCC warnings {sorted(gcc_found)}, "
                    f"got {sorted(lccc_found)}"
                )
            print(f"warning compatibility {label}: PASS ({', '.join(sorted(lccc_found)) or 'silent'})")

        for label, fixture, expected in (
            ("sign-suppression", SIGN_CONTROL_SOURCE, set()),
            ("sign-negative-constant", SIGN_NEGATIVE_SOURCE, {"sign"}),
        ):
            source.write_text(fixture, encoding="utf-8")
            gcc_found = run_case("GCC 16.2", gcc, ("-Wextra",), source)
            lccc_found = run_case("LCCC", lccc, ("-Wextra",), source)
            if gcc_found != expected or lccc_found != gcc_found:
                raise RuntimeError(
                    f"{label}: expected GCC/LCCC {sorted(expected)}, "
                    f"got GCC {sorted(gcc_found)}, LCCC {sorted(lccc_found)}"
                )
            print(f"warning compatibility {label}: PASS ({', '.join(sorted(lccc_found)) or 'silent'})")

        source.write_text(FORMAT_SECURITY_SOURCE, encoding="utf-8")
        for label, flags, expected in FORMAT_SECURITY_CASES:
            gcc_found = run_case("GCC 16.2", gcc, flags, source)
            lccc_found = run_case("LCCC", lccc, flags, source)
            if gcc_found != expected or lccc_found != gcc_found:
                raise RuntimeError(
                    f"{label}: expected GCC/LCCC {sorted(expected)}, "
                    f"got GCC {sorted(gcc_found)}, LCCC {sorted(lccc_found)}"
                )
            print(f"warning compatibility {label}: PASS ({', '.join(sorted(lccc_found)) or 'silent'})")

        for label, compiler in (("GCC 16.2", gcc), ("LCCC", lccc)):
            run_format_security_error(label, compiler, FORMAT_SECURITY_ERROR_FLAGS, source)
        print("warning compatibility format-security-error: PASS (fatal in GCC and LCCC)")

        run_kind_matrix(gcc, lccc, Path(temp))
        run_format_level_rows(gcc, lccc, Path(temp))
        run_mb01_rows(gcc, lccc, Path(temp))

    run_precision_sweep(gcc, lccc)

    print(f"warning compatibility gate: PASS (GCC {version})")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        raise SystemExit(f"warning compatibility: ERROR: {exc}")

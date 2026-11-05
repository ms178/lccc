#!/usr/bin/env python3
"""Data-driven ratchet for 20 high-ROI backlog items (2026-11-05 sweep).

`engineering/BACKLOG-ROI-2026-11-05.md` records the measurement behind each
item.  This script is the executable half: 20 independent invariants, each
named after the backlog item it protects, each with a negative control in
`--self-test` proving that a plausible regression is actually caught.

Design rules, because a ratchet that cannot fail is decoration:

* every check reads the live tree, never a cached verdict;
* every check returns named problems, never a boolean;
* every check has a mutation in `negative_controls()` that it must reject;
* `--json` emits the per-item verdict so a reviewer can see which of the 20
  fired, not just an aggregate PASS.

Checks are deliberately static (no compiled binary, no network): the corpus
lane runs this on every PR, so it must be fast, deterministic and portable.
Dynamic verification lives in the gates these checks protect
(`option_effect_ledger.py --lccc`, `check_macro_parity.py`, the SSE2 matrix).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from scripts import option_effect_ledger as ledger  # noqa: E402

LEDGER_FILE = ROOT / "src/driver/option_table.rs"
CLI_FILE = ROOT / "src/driver/cli.rs"
DRIVER_GATE = ROOT / "tests/regression/check_driver_option_contract.sh"
MACRO_GATE_PY = ROOT / "tests/regression/check_macro_parity.py"
MACRO_GATE_SH = ROOT / "tests/regression/check_macro_parity.sh"
CAPABILITY_PROBE = ROOT / "scripts/lccc_capability_probe.py"
MATRIX = ROOT / "tests/regression/sse2_codegen_matrix.json"
MATRIX_GATE = ROOT / "tests/regression/check_sse2_codegen_matrix.py"
GODBOLT = ROOT / "scripts/godbolt.py"
AGENT_LOCK = ROOT / "engineering/toolchains/agent-cache-lock.json"
BACKLOG = ROOT / "backlog.md"
CI_LOCAL = ROOT / "scripts/ci_local.sh"
CI_WORKFLOW = ROOT / ".github/workflows/ci.yml"
CLANG_INSTALLER = ROOT / "scripts/ci_install_clang_oracle.sh"
PREDEFINES = ROOT / "src/frontend/preprocessor/predefined_macros.rs"
PUBLISHER = ROOT / "scripts/publish_agent_toolchain_cache.sh"
ISA_DOC = ROOT / "engineering/agent/BACKLOG.md"

OBSOLETE = ("1.97.0", "23.1.4", "11.1.2", "v7.3-x86_64", "2.42.1")


@dataclass
class Context:
    """All inputs read once, so every check sees exactly the same tree."""
    ledger_rows: list[ledger.Row] = field(default_factory=list)
    cli: str = ""
    driver_gate: str = ""
    macro_py: str = ""
    macro_sh: str = ""
    capability: str = ""
    matrix: dict = field(default_factory=dict)
    matrix_gate: str = ""
    godbolt: str = ""
    agent_lock: dict = field(default_factory=dict)
    backlog: str = ""
    ci_local: str = ""
    ci_workflow: str = ""
    clang_pin: str = ""
    predefines: str = ""
    publisher: str = ""
    isa_doc: str = ""

    @classmethod
    def load(cls, root: Path = ROOT) -> "Context":
        def text(rel: str) -> str:
            path = root / rel
            return path.read_text() if path.is_file() else ""

        pin = re.search(r"^VERSION=([0-9.]+)$", text("scripts/ci_install_clang_oracle.sh"),
                        re.MULTILINE)
        return cls(
            ledger_rows=ledger.parse_rows(text("src/driver/option_table.rs")),
            cli=text("src/driver/cli.rs"),
            driver_gate=text("tests/regression/check_driver_option_contract.sh"),
            macro_py=text("tests/regression/check_macro_parity.py"),
            macro_sh=text("tests/regression/check_macro_parity.sh"),
            capability=text("scripts/lccc_capability_probe.py"),
            matrix=json.loads(text("tests/regression/sse2_codegen_matrix.json") or "{}"),
            matrix_gate=text("tests/regression/check_sse2_codegen_matrix.py"),
            godbolt=text("scripts/godbolt.py"),
            agent_lock=json.loads(text("engineering/toolchains/agent-cache-lock.json") or "{}"),
            backlog=text("backlog.md"),
            ci_local=text("scripts/ci_local.sh"),
            ci_workflow=text(".github/workflows/ci.yml"),
            clang_pin=pin.group(1) if pin else "",
            predefines=text("src/frontend/preprocessor/predefined_macros.rs"),
            publisher=text("scripts/publish_agent_toolchain_cache.sh"),
            isa_doc=text("engineering/agent/BACKLOG.md"),
        )


# ---------------------------------------------------------------------------
# 20 checks.  Each returns a list of named problems; [] means PASS.
# ---------------------------------------------------------------------------

def gp22_ledger_static(c: Context) -> list[str]:
    return [f"GP-22/1: {p}" for p in ledger.static_problems(c.ledger_rows, c.cli)]


def gp22_every_accepted_row_probed(c: Context) -> list[str]:
    bad = [r.spelling for r in c.ledger_rows
           if r.cls in ("Inert", "Implemented") and (not r.probe or r.probe not in ledger.PROBES)]
    return [f"GP-22/2: accepted row has no implemented probe: {', '.join(bad)}"] if bad else []


def gp22_registry_exact(c: Context) -> list[str]:
    probes = [r.probe for r in c.ledger_rows if r.probe]
    out: list[str] = []
    if len(probes) != len(set(probes)):
        out.append("GP-22/3: duplicate probe_id in the ledger")
    if set(probes) != set(ledger.PROBES):
        out.append("GP-22/3: probe registry has an orphan or a hole")
    return out


def gp22_priority_set(c: Context) -> list[str]:
    spellings = {r.spelling for r in c.ledger_rows}
    missing = sorted(set(ledger.PRIORITY_SET) - spellings)
    return [f"GP-22/4: unledgered priority options: {', '.join(missing)}"] if missing else []


def gp22_executable_wiring(c: Context) -> list[str]:
    out: list[str] = []
    if "option_effect_ledger.py" not in c.driver_gate:
        out.append("GP-22/5: the driver-contract gate does not execute the ledger")
    if "--lccc" not in c.driver_gate:
        out.append("GP-22/5: the ledger is not run against the built binary")
    return out


def gp22_hosted_local_parity(c: Context) -> list[str]:
    out: list[str] = []
    if "check_driver_option_contract.sh" not in c.ci_local:
        out.append("GP-22/6: driver contract gate absent from ci_local.sh")
    if "check_driver_option_contract.sh" not in c.ci_workflow:
        out.append("GP-22/6: driver contract gate absent from hosted ci.yml")
    return out


def gp23_macro_gate_exists(c: Context) -> list[str]:
    out: list[str] = []
    for needle, name in (("GCC_ONLY", "GCC_ONLY budget"), ("LCCC_ONLY", "LCCC_ONLY budget"),
                         ("VALUE_DIFFS", "VALUE_DIFFS budget")):
        if needle not in c.macro_py:
            out.append(f"GP-23/1: {name} missing from check_macro_parity.py")
    if "check_macro_parity.py" not in c.ci_local or "check_macro_parity.py" not in c.ci_workflow:
        out.append("GP-23/1: macro-parity gate not wired locally and hosted")
    if "check_macro_parity.sh" not in c.ci_local:
        out.append("GP-23/1: dialect/target shell gate not wired locally")
    return out


def gp23_forbidden_budget(c: Context) -> list[str]:
    """The shell gate's FORBIDDEN list is a ratchet: libc/capability macros
    must never be predefined by lccc.  It must exist and stay non-empty as a
    positive control (re-adding BUFSIZ must be noticed)."""
    out: list[str] = []
    if "FORBIDDEN_LIST" not in c.macro_sh:
        out.append("GP-23/2: FORBIDDEN budget table is missing")
    if '"BUFSIZ"' not in c.macro_sh:
        out.append("GP-23/2: BUFSIZ positive control was removed from the budget")
    return out


def gp05_gnuc_major(c: Context) -> list[str]:
    """GP-05's worst drift was `__GNUC__ 14` while the oracle is GCC 16.2.
    Pin the major in the preprocessor source to the same value the parity
    gate measures."""
    # Match the table entry itself, not a loose `__GNUC__` substring: the
    # first attempt could be satisfied by `__GNUC_MINOR__` on a nearby line,
    # or by any `16` within 80 characters. A wrong major must be named.
    match = re.search(r'\("__GNUC__",\s*"(\d+)"\)', c.predefines)
    if match is None:
        return ["GP-05: __GNUC__ is not defined in the preprocessor's macro table"]
    if match.group(1) != "16":
        return [f"GP-05: __GNUC__ is {match.group(1)}, not the GCC 16 major the oracle gate measures"]
    return []


def mb01_noplt_effect(c: Context) -> list[str]:
    if "GOTPCREL" not in c.driver_gate or "-fno-plt" not in c.driver_gate:
        return ["MB-01/1: the -fno-plt GOT-indirect effect probe is missing"]
    return []


def mb01_visibility_effect(c: Context) -> list[str]:
    if ".hidden" not in c.driver_gate or "-fvisibility=hidden" not in c.driver_gate:
        return ["MB-01/2: the -fvisibility=hidden .hidden effect probe is missing"]
    return []


def mb40_no_catch_all(c: Context) -> list[str]:
    code = "\n".join(line.split("//", 1)[0] for line in c.cli.splitlines())
    if re.search(r'arg\s+if\s+arg\.starts_with\("-f"\)\s*=>\s*\{\s*\}', code):
        return ["MB-40: blanket -f catch-all no-op arm is back"]
    return []


_ITEM_HEADING = re.compile(r"^### ([A-Z]+-\d+[a-z]?)\s*·\s*(.+)$", re.MULTILINE)


def mb41_item_headings_parse(c: Context) -> list[str]:
    headings = _ITEM_HEADING.findall(c.backlog)
    if len(headings) < 20:
        return [f"MB-41/1: only {len(headings)} parseable item headings in backlog.md"]
    bad = [ident for ident, _ in headings if not re.fullmatch(r"[A-Z]+-\d+[a-z]?", ident)]
    return [f"MB-41/1: malformed ids: {bad}"] if bad else []


def mb41_reproducers(c: Context) -> list[str]:
    """Every `R` (reproducible) item must carry a fenced reproducer or an
    inline command.  Items without one are measured, not silently blessed."""
    blocks = re.split(r"(?=^### )", c.backlog, flags=re.MULTILINE)
    missing: list[str] = []
    for block in blocks:
        head = _ITEM_HEADING.match(block)
        if not head or "· R" not in head.group(2):
            continue
        if "```" not in block and "`" not in block:
            missing.append(head.group(1))
    return [f"MB-41/2: R items with no reproducer: {', '.join(missing)}"] if missing else []


def mb42_probe_is_evidence_based(c: Context) -> list[str]:
    out: list[str] = []
    if "NEGATIVE_CONTROLS" not in c.capability:
        out.append("MB-42: capability probe lost its negative controls")
    if "returncode" not in c.capability and "exit status" not in c.capability:
        out.append("MB-42: capability probe no longer records exit status")
    return out


def mb45_gap_counts_named(c: Context) -> list[str]:
    """MB-45 asks that 'gap' counts name the oracle they are a gap against."""
    if "GCC 16.2" not in c.isa_doc and "Clang 23.1" not in c.isa_doc:
        return ["MB-45: the ISA/performance backlog does not name its comparison oracle"]
    return []


def followup_simd_rederived(c: Context) -> list[str]:
    pin = c.clang_pin
    provenance = c.matrix.get("provenance", {})
    out: list[str] = []
    if provenance.get("rederived_with", {}).get("clang") != pin:
        out.append(f"FOLLOWUP-7/2: SIMD matrix rederived_with.clang != live pin {pin}")
    banner = provenance.get("recorded_with", {}).get("clang", "")
    if f"clang version {pin}" not in banner:
        out.append(f"FOLLOWUP-7/2: recorded Clang banner is not {pin}")
    if "Measured with Clang 23.1.4" in provenance.get("note", ""):
        out.append("FOLLOWUP-7/2: a stale 23.1.4 measurement was relabelled as current")
    return out


def followup_simd_generator_keeps_provenance(c: Context) -> list[str]:
    missing = [n for n in ('base["provenance"]', '"rederived_with"', '"note"',
                           '"recorded_with"') if n not in c.matrix_gate]
    return [f"FOLLOWUP-7/2b: --update-baseline would drop: {', '.join(missing)}"] if missing else []


def followup_ce_alias_provenance(c: Context) -> list[str]:
    out: list[str] = []
    if "23.1.0" not in c.godbolt or c.clang_pin not in c.godbolt:
        out.append("FOLLOWUP-7/7: godbolt.py does not distinguish CE's Clang from the pinned oracle")
    if "Record which one a table came from" not in c.godbolt and "record" not in c.godbolt.lower():
        out.append("FOLLOWUP-7/7: no instruction to record the producing oracle")
    return out


def cache_no_obsolete_payload(c: Context) -> list[str]:
    lock = c.agent_lock
    out: list[str] = []
    if lock.get("tag") != "harness-cache-8":
        out.append("CACHE-REDTEAM/1: reviewed lock does not name the refreshed tag")
    tools = lock.get("tools", {})
    if "gdb" not in tools:
        out.append("CACHE-REDTEAM/1: cache lock has no GDB payload")
    for name, spec in tools.items():
        if any(token in spec.get("file", "") for token in OBSOLETE):
            out.append(f"CACHE-REDTEAM/1: {name} names an obsolete payload")
    if "refusing to publish obsolete cache payloads" not in c.publisher:
        out.append("CACHE-REDTEAM/1: publisher lost its obsolete-payload guard")
    return out


CHECKS = [
    ("GP-22/1", gp22_ledger_static),
    ("GP-22/2", gp22_every_accepted_row_probed),
    ("GP-22/3", gp22_registry_exact),
    ("GP-22/4", gp22_priority_set),
    ("GP-22/5", gp22_executable_wiring),
    ("GP-22/6", gp22_hosted_local_parity),
    ("GP-23/1", gp23_macro_gate_exists),
    ("GP-23/2", gp23_forbidden_budget),
    ("GP-05", gp05_gnuc_major),
    ("MB-01/1", mb01_noplt_effect),
    ("MB-01/2", mb01_visibility_effect),
    ("MB-40", mb40_no_catch_all),
    ("MB-41/1", mb41_item_headings_parse),
    ("MB-41/2", mb41_reproducers),
    ("MB-42", mb42_probe_is_evidence_based),
    ("MB-45", mb45_gap_counts_named),
    ("FOLLOWUP-7/2", followup_simd_rederived),
    ("FOLLOWUP-7/2b", followup_simd_generator_keeps_provenance),
    ("FOLLOWUP-7/7", followup_ce_alias_provenance),
    ("CACHE-REDTEAM/1", cache_no_obsolete_payload),
]

assert len(CHECKS) == 20, f"exactly 20 items required, got {len(CHECKS)}"


def run_all(ctx: Context) -> dict[str, list[str]]:
    return {name: fn(ctx) for name, fn in CHECKS}


def negative_controls(ctx: Context) -> list[str]:
    """Each check must reject a plausible regression.  Mutate one field at a
    time on a copy of the context so a control cannot mask another."""
    failures: list[str] = []
    mutations: list[tuple[str, str, Context]] = []

    def mutate(name: str, check: str, **kwargs) -> None:
        import copy
        c = copy.copy(ctx)
        for key, value in kwargs.items():
            setattr(c, key, value)
        mutations.append((name, check, c))

    rows = list(ctx.ledger_rows)
    mutate("ledger duplicate", "GP-22/1", ledger_rows=rows + [rows[0]])
    mutate("accepted row loses probe", "GP-22/2",
           ledger_rows=[ledger.Row(r.spelling, r.cls, r.rationale, None) for r in rows])
    mutate("registry orphan", "GP-22/3", ledger_rows=rows[:-1])
    mutate("priority row dropped", "GP-22/4",
           ledger_rows=[r for r in rows if r.spelling != "-pedantic"])
    mutate("ledger unhooked", "GP-22/5", driver_gate="echo pass")
    mutate("hosted gate removed", "GP-22/6", ci_workflow="")
    mutate("macro budgets deleted", "GP-23/1", macro_py="")
    mutate("BUFSIZ control removed", "GP-23/2", macro_sh="FORBIDDEN_LIST=()")
    mutate("__GNUC__ drift", "GP-05",
           predefines=ctx.predefines.replace('("__GNUC__", "16")', '("__GNUC__", "14")'))
    mutate("__GNUC__ removed", "GP-05", predefines="")
    mutate("no-plt probe removed", "MB-01/1", driver_gate=ctx.driver_gate.replace("GOTPCREL", ""))
    mutate("visibility probe removed", "MB-01/2",
           driver_gate=ctx.driver_gate.replace(".hidden", ""))
    mutate("catch-all restored", "MB-40",
           cli=ctx.cli + '\n                arg if arg.starts_with("-f") => {}\n')
    mutate("backlog headings mangled", "MB-41/1", backlog="### no id here\n")
    mutate("R item loses reproducer", "MB-41/2",
           backlog=re.sub(r"```.*?```", "", ctx.backlog, flags=re.S).replace("`", ""))
    mutate("probe stops recording", "MB-42", capability="")
    mutate("ISA oracle unnamed", "MB-45", isa_doc="")
    stale = dict(ctx.matrix); stale["provenance"] = {
        **ctx.matrix.get("provenance", {}),
        "rederived_with": {"clang": "23.1.4"},
    }
    mutate("SIMD stale re-labelled", "FOLLOWUP-7/2", matrix=stale)
    mutate("generator drops provenance", "FOLLOWUP-7/2b", matrix_gate="")
    mutate("CE alias provenance lost", "FOLLOWUP-7/7", godbolt="")
    lock = dict(ctx.agent_lock)
    lock["tools"] = dict(lock.get("tools", {}))
    lock["tools"]["clang"] = {**lock["tools"].get("clang", {}),
                              "file": "clang-23.1.4-linux-x64.tar.xz"}
    mutate("obsolete cache payload", "CACHE-REDTEAM/1", agent_lock=lock,
           publisher=ctx.publisher.replace("refusing to publish obsolete cache payloads", ""))

    by_name = dict(CHECKS)
    for name, check, mutated in mutations:
        problems = by_name[check](mutated)
        if not problems:
            failures.append(f"negative control {name!r} did not make {check} fail")
    controls = {check for _, check, _ in mutations}
    missing = [name for name, _ in CHECKS if name not in controls]
    if missing:
        failures.append(f"checks without a negative control: {', '.join(missing)}")
    return failures


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args(argv)

    ctx = Context.load()
    results = run_all(ctx)
    failures = negative_controls(ctx) if args.self_test else []
    problems = {k: v for k, v in results.items() if v}

    report = {
        "items": len(CHECKS),
        "passed": len(CHECKS) - len(problems),
        "failed": sorted(problems),
        "results": results,
        "self_test": negative_controls(ctx) if args.self_test else [],
    }
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        for name, probs in results.items():
            print(f"  {'PASS' if not probs else 'FAIL':4} {name}")
            for p in probs:
                print(f"        {p}")
        print(f"backlog ROI ratchet: {report['passed']}/{report['items']} items PASS"
              + (f"; self-test: {len(failures)} failure(s)" if args.self_test else ""))
        for f in failures:
            print(f"  SELF-TEST FAIL: {f}", file=sys.stderr)
    return 1 if problems or failures else 0


if __name__ == "__main__":
    raise SystemExit(main())

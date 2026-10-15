#!/usr/bin/env python3
"""tune_oracle.py — cross-check lccc's per-CPU tuning decisions against the
compilers that ship their own cost models (GCC 16.2, Clang 23.1, ICX/ICC).

The x86 tuning model (`src/backend/x86/cpu_model.rs`) stores measured numbers
per microarchitecture and derives decisions from them.  This oracle makes
those decisions *observable* and *comparable*:

  1. `dump`    — print the row lccc uses for one or all `-mtune=` names
                 (via `LCCC_DUMP_TUNE`), optionally as JSON.
  2. `matrix`  — compile the probe kernels for a list of `-mtune=` rows and
                 classify what each function lowered to (rep movsb / vector
                 loop / SHLX / xor-break …), so a reviewer sees the decision
                 table instead of trusting the comment in the row.
  3. `compare` — for the same probe and the same `-march/-mtune`, fetch the
                 GCC / Clang / ICX output through `scripts/godbolt.py` and
                 print the same classification side by side.  Divergences are
                 where lccc either beats the oracle or has a bug; the audit in
                 `docs/CPU_MODEL_AUDIT.md` §5 was produced with this mode.

Examples:

    scripts/tune_oracle.py dump --lccc target/fastbuild/lccc raptorlake
    scripts/tune_oracle.py dump --all --json > /tmp/rows.json
    scripts/tune_oracle.py matrix --lccc target/fastbuild/lccc \\
        --tunes generic,skylake,raptorlake,gracemont,znver3 --flags=-O2 \\
        tests/bench/cpu_model/tune_probe.c
    scripts/tune_oracle.py compare --lccc target/fastbuild/lccc \\
        --flags="-O2 -march=x86-64-v3 -mtune=raptorlake" \\
        --oracles gcc16.2,clang,icx tests/bench/cpu_model/tune_probe.c

No hardware PMU is required: the oracle reasons about *instruction
selection*, which is what the tuning model decides.  Runtime evidence is
collected separately with `scripts/bench_kernels.py`.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import shlex
import tempfile
import sys
from collections import OrderedDict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_PROBE = REPO / "tests" / "bench" / "cpu_model" / "tune_probe.c"

# --------------------------------------------------------------------------
# Classification of a function body: which tuning-relevant forms appear.
# --------------------------------------------------------------------------
CLASSIFIERS: list[tuple[str, re.Pattern[str]]] = [
    ("rep_movsb", re.compile(r"\brep\s+movsb\b")),
    ("rep_stosb", re.compile(r"\brep\s+stosb\b")),
    ("rep_movsq", re.compile(r"\brep\s+movsq\b")),
    ("ymm_copy", re.compile(r"\bvmovdq[au]\s+[^\n]*%ymm")),
    ("xmm_copy", re.compile(r"\b(v?movdq[au]|movups|movaps)\s+[^\n]*%xmm")),
    ("call_memcpy", re.compile(r"\bcall\s+[^\n]*memcpy")),
    ("call_memset", re.compile(r"\bcall\s+[^\n]*memset")),
    ("vzeroupper", re.compile(r"\bvzeroupper\b")),
    ("shlx", re.compile(r"\b(shlx|shrx|sarx)\b")),
    ("shift_cl", re.compile(r"\b(shl|shr|sar|rol|ror)[lq]?\s+%cl,")),
    ("xor_break", re.compile(r"\bxor[lq]?\s+(%e\w\w),\s*\1\s*\n\s*(popcnt|lzcnt|tzcnt)", re.M)),
    ("popcnt", re.compile(r"\bpopcnt")),
    ("tzcnt_lzcnt", re.compile(r"\b(tzcnt|lzcnt)")),
    ("lea3", re.compile(r"\blea[lq]?\s+-?\d+\([^)]*,[^)]*,[^)]*\)")),
    ("imul_imm", re.compile(r"\bimul[lq]?\s+\$")),
    ("cmov", re.compile(r"\bcmov")),
    ("setcc", re.compile(r"\bset[a-z]{1,3}\s")),
    ("branch", re.compile(r"\bj(?!mp)[a-z]{1,3}\s+\.")),
    ("idiv", re.compile(r"\b(i?div)[lq]?\s")),
]


def split_functions(asm: str) -> "OrderedDict[str, str]":
    """Split AT&T assembly into {symbol: body} (global-label granularity)."""
    out: "OrderedDict[str, str]" = OrderedDict()
    cur = None
    buf: list[str] = []
    for line in asm.splitlines():
        m = re.match(r'^\s*"?([A-Za-z_][\w.$]*)"?:\s*(?:[#;].*)?$', line)
        if m and not m.group(1).startswith(".L"):
            if cur is not None:
                out[cur] = "\n".join(buf)
            cur = m.group(1)
            buf = []
            continue
        if cur is not None:
            buf.append(line)
    if cur is not None:
        out[cur] = "\n".join(buf)
    return out


def classify(body: str) -> list[str]:
    labels = [name for name, rx in CLASSIFIERS if rx.search(body)]
    insns = sum(1 for l in body.splitlines()
                if l.strip() and not l.strip().startswith((".", "#")) and not l.strip().endswith(":"))
    labels.append(f"n={insns}")
    return labels


# --------------------------------------------------------------------------
# lccc drivers
# --------------------------------------------------------------------------
def lccc_dump(lccc: str, tune: str | None, all_rows: bool) -> str:
    env = dict(os.environ)
    env["LCCC_DUMP_TUNE"] = "all" if all_rows else "1"
    with tempfile.TemporaryDirectory(prefix="lccc-tune-dump-") as td:
        tmp = Path(td) / "probe.c"
        tmp.write_text("int lccc_tune_probe;\n")
        flags = ["-O2", "-S", "-o", os.devnull, str(tmp)]
        if tune and not all_rows:
            flags.insert(1, f"-mtune={tune}")
        p = subprocess.run([lccc, *flags], text=True, capture_output=True,
                           env=env, timeout=120)
    text = p.stdout + p.stderr
    if p.returncode != 0 or not parse_dump(text):
        raise RuntimeError(f"lccc tuning dump failed or empty: {text}")
    return text


def parse_dump(text: str) -> list[dict]:
    rows: list[dict] = []
    cur: dict | None = None
    for line in text.splitlines():
        line = line.strip()
        if not line or "=" not in line:
            continue
        k, v = line.split("=", 1)
        if k == "tune":
            cur = OrderedDict(tune=v)
            rows.append(cur)
            continue
        if cur is None:
            continue
        if re.fullmatch(r"-?\d+", v):
            cur[k] = int(v)
        elif v in ("true", "false"):
            cur[k] = v == "true"
        else:
            cur[k] = v
    return rows


def lccc_compile(lccc: str, src: Path, flags: str) -> str:
    with tempfile.TemporaryDirectory(prefix="lccc-tune-asm-") as td:
        tmp = Path(td) / "output.s"
        p = subprocess.run([lccc, *shlex.split(flags), "-S", "-o", str(tmp), str(src)],
                           text=True, capture_output=True, timeout=120)
        if p.returncode != 0:
            raise RuntimeError(f"lccc failed for {flags}: {p.stderr}")
        text = tmp.read_text()
        if not text.strip():
            raise RuntimeError("local compiler produced empty assembly")
        return text


# --------------------------------------------------------------------------
# Godbolt oracles (reuse scripts/godbolt.py)
# --------------------------------------------------------------------------
def godbolt_compile(compiler: str, src: Path, flags: str) -> str:
    sys.path.insert(0, str(REPO / "scripts"))
    import godbolt  # type: ignore

    res = godbolt.compile_on_godbolt(compiler, src.read_text(), flags, raise_on_error=True)
    if res is None:
        raise godbolt.GodboltError("missing compiler response")
    text = "\n".join(godbolt.assembly_lines(res))
    if not text.strip():
        raise godbolt.GodboltError("compiler produced empty assembly")
    return text


# --------------------------------------------------------------------------
# Commands
# --------------------------------------------------------------------------
def cmd_dump(a: argparse.Namespace) -> int:
    text = lccc_dump(a.lccc, a.tune, a.all)
    if a.json:
        print(json.dumps(parse_dump(text), indent=2))
    else:
        sys.stdout.write(text)
    return 0


def print_table(title: str, cols: list[str], rows: dict[str, dict[str, list[str]]]) -> None:
    print(f"\n## {title}\n")
    print("| function | " + " | ".join(cols) + " |")
    print("|---|" + "---|" * len(cols))
    for fn, per in rows.items():
        cells = [" ".join(per.get(c, ["-"])) for c in cols]
        print(f"| `{fn}` | " + " | ".join(cells) + " |")


def cmd_matrix(a: argparse.Namespace) -> int:
    tunes = [t for t in a.tunes.split(",") if t]
    src = Path(a.probe)
    table: dict[str, dict[str, list[str]]] = OrderedDict()
    for t in tunes:
        asm = lccc_compile(a.lccc, src, f"{a.flags} -mtune={t}")
        for fn, body in split_functions(asm).items():
            if a.function and fn != a.function:
                continue
            table.setdefault(fn, OrderedDict())[t] = classify(body)
    print_table(f"lccc decision matrix — {src.name} ({a.flags})", tunes, table)
    return int(not table or any(not any(t in row for row in table.values()) for t in tunes))


def cmd_compare(a: argparse.Namespace) -> int:
    src = Path(a.probe)
    cols = ["lccc"] + [o.strip() for o in a.oracles.split(",") if o.strip()]
    if len(set(cols)) != len(cols) or any(not re.fullmatch(r"[A-Za-z0-9_.-]+", c) for c in cols):
        raise ValueError("oracle labels must be unique and path-safe")
    if a.save:
        Path(a.save).mkdir(parents=True, exist_ok=True)
        for c in cols:
            (Path(a.save) / f"{src.stem}.{c}.s").unlink(missing_ok=True)
        # Fail closed even if the local compile aborts before the final report.
        (Path(a.save) / "status.json").write_text(json.dumps(
            {"complete": False, "requested": cols, "flags": a.flags,
             "errors": {"setup": "comparison incomplete"}}, indent=2) + "\n")
    errors = {}
    table: dict[str, dict[str, list[str]]] = OrderedDict()
    outputs = {"lccc": lccc_compile(a.lccc, src, a.flags)}
    for o in cols[1:]:
        try:
            outputs[o] = godbolt_compile(o, src, a.flags)
        except Exception as e:  # network / API drift must not hide lccc's own table
            print(f"error: {o}: {e}", file=sys.stderr)
            errors[o] = str(e)
            outputs[o] = ""
    for c in cols:
        for fn, body in split_functions(outputs[c]).items():
            if a.function and fn != a.function:
                continue
            table.setdefault(fn, OrderedDict())[c] = classify(body)
    print_table(f"lccc vs oracles — {src.name} ({a.flags})", cols, table)
    for c in cols:
        if not any(c in row for row in table.values()):
            errors.setdefault(c, "requested function(s) not emitted")
    if a.save:
        for c, asm in outputs.items():
            if asm:
                (Path(a.save) / f"{src.stem}.{c}.s").write_text(asm)
        (Path(a.save) / "status.json").write_text(json.dumps(
            {"complete": not errors, "requested": cols, "flags": a.flags, "errors": errors},
            indent=2) + "\n")
    return int(bool(errors))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default=str(REPO / "target" / "fastbuild" / "lccc"))
    sub = ap.add_subparsers(dest="cmd", required=True)

    d = sub.add_parser("dump", help="print the tuning row(s) lccc uses")
    d.add_argument("tune", nargs="?", default=None)
    d.add_argument("--all", action="store_true")
    d.add_argument("--json", action="store_true")
    d.set_defaults(fn=cmd_dump)

    m = sub.add_parser("matrix", help="lccc decisions per -mtune row")
    m.add_argument("probe", nargs="?", default=str(DEFAULT_PROBE))
    m.add_argument("--tunes", default="generic,sandybridge,haswell,skylake,icelake-client,alderlake,raptorlake,gracemont,znver1,znver3,znver4")
    m.add_argument("--flags", default="-O2")
    m.add_argument("--function", default=None)
    m.set_defaults(fn=cmd_matrix)

    c = sub.add_parser("compare", help="lccc vs GCC/Clang/ICX for one flag set")
    c.add_argument("probe", nargs="?", default=str(DEFAULT_PROBE))
    c.add_argument("--flags", default="-O2 -march=x86-64-v3 -mtune=raptorlake")
    c.add_argument("--oracles", default="gcc16.2,clang,icx")
    c.add_argument("--function", default=None)
    c.add_argument("--save", default=None, help="directory for the raw .s files")
    c.set_defaults(fn=cmd_compare)

    import godbolt
    a = ap.parse_args(godbolt.join_dash_values(list(sys.argv[1:] if argv is None else argv)))
    try:
        return a.fn(a)
    except (OSError, RuntimeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

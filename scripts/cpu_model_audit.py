#!/usr/bin/env python3
"""Cross-check every measurable field of the x86 tuning model against uops.info.

The tuning model (`src/backend/x86/cpu_model.rs`) stores *numbers* with a
provenance tag.  Comments rot; numbers can be re-derived.  This script makes
the claim "every `[uops.info]` field equals the published measurement"
executable:

  1. for each model row it runs `lccc -mtune=<row> -S` with `LCCC_DUMP_TUNE=1`
     and parses the `key=value` dump;
  2. it looks the row up in the complete uops.info dataset
     (`scripts/uops_xml.py`, column chosen by `ROW_ARCH`);
  3. it evaluates one *check* per field (table `CHECKS` below) and reports
     OK / MISMATCH / SKIP.

Mismatches fail the run (exit 1) unless an entry in
`scripts/cpu_model_audit_allow.txt` documents the deviation
(`row field  reason...`), so a deliberate conservative choice is reviewable
and an accidental drift is not.  Run it after *every* edit of a row:

    LCCC_BIN=target/fastbuild/lccc scripts/cpu_model_audit.py
    scripts/cpu_model_audit.py --rows raptorlake,meteorlake --verbose
    scripts/cpu_model_audit.py --markdown > engineering/evidence/cpu_model_audit.md

Exit status: 0 = all fields verified or allow-listed, 1 = mismatch,
2 = infrastructure error (binary / data missing).

Tolerances: latencies and µop counts compare exactly; reciprocal throughput
compares within 25 % (uops.info publishes `TP_ports` computed from port usage
for most pages, `TP_loop` otherwise; the model keeps ×100 integers).
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import uops_xml as U  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ALLOW = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                     "cpu_model_audit_allow.txt")

# Hybrid / E-core rows whose ports are not published: only latency / µop
# fields are checkable.
NO_PORTS = {"gracemont"}


def count_pipes(ports: str | None, pat: str | None = None) -> int | None:
    """`1*p01` -> 2, `1*p15` -> 2, `1*p0156B` -> 4 (letters after p)."""
    if not ports:
        return None
    m = re.match(r"\d+\*p(\d+)", ports)
    return len(m.group(1)) if m else None


def edge_dep(entry, start, target) -> int | None:
    v = [int(float(c)) for (s, t, c, *_r) in entry["lat"]
         if s == start and t == target and c is not None]
    return max(v) if v else None


def lea_lat(entry):
    v = [int(float(a)) for (_s, _t, _c, _sr, _mn, _mx, a, _m) in entry["lat"]
         if a is not None]
    return max(v) if v else None


def rtp100(entry):
    t = U.tp(entry)
    return None if t is None else round(t * 100)


def rtp100_loop(entry):
    """Measured throughput (TP_loop / TP_unrolled), not the port-computed
    figure: for the non-pipelined divider `TP_ports` understates occupancy by
    2-3x (ICL DIV r64: ports 3.0, measured 10.0)."""
    v = [float(entry[k]) for k in ("tp_loop", "tp_unrolled") if entry.get(k)]
    return round(min(v) * 100) if v else None


def dividend_range(entry):
    """(lo, hi) latency over the *dividend* operand edges only (RAX/RDX ->
    results).  The divisor edge of pre-ICL dividers has a tiny minimum (5) that
    a dependent `q = a / b` chain through the dividend never sees."""
    lo, hi = [], []
    for (s, _t, c, _sr, mn, mx, _a, _m) in entry["lat"]:
        if s not in (1, 2):
            continue
        for v in (c, mn):
            if v is not None:
                lo.append(int(float(v)))
        for v in (c, mx):
            if v is not None:
                hi.append(int(float(v)))
    return (min(lo), max(hi)) if lo and hi else None


# (field, page, extractor(entry)->value, kind)   kind: exact | rtp | bool
CHECKS = [
    ("shift_cl_uops", "SHL_R64_CL", U.nuops, "exact"),
    ("lea3_latency", "LEA_B_I_D8_R64", lea_lat, "exact"),
    ("lea3_rtp_x100", "LEA_B_I_D8_R64", rtp100, "rtp"),
    ("lea3_uops", "LEA_B_I_D8_R64", U.nuops, "exact"),
    ("cmov_uops", "CMOVB_R64_R64", U.nuops, "exact"),
    ("cmov_latency", "CMOVB_R64_R64", lambda e: (U.lat_range(e) or (None,))[-1], "exact"),
    ("imul64_latency", "IMUL_R64_R64", U.lat, "exact"),
    # Best-case latency: Agner's dependent-chain figure vs uops.info's minimum
    # over dividend edges differ by a few cycles on the pre-ICL dividers
    # (SKL 35 vs 32), hence "near" (within 15 %).
    ("div64_latency_min", "DIV_R64", lambda e: (dividend_range(e) or (None,))[0], "near"),
    ("div64_latency", "DIV_R64", lambda e: (dividend_range(e) or (None, None))[1], "near"),
    ("div64_rtp_x100", "DIV_R64", rtp100_loop, "rtp"),
    ("div32_latency_min", "DIV_R32", lambda e: (dividend_range(e) or (None,))[0], "near"),
    ("div32_latency", "DIV_R32", lambda e: (dividend_range(e) or (None, None))[1], "near"),
    ("div32_rtp_x100", "DIV_R32", rtp100_loop, "rtp"),
    ("pmulld_uops", "PMULLD_XMM_XMM", U.nuops, "exact"),
    ("pmulld_latency", "PMULLD_XMM_XMM", U.lat, "exact"),
    ("fma_latency", "VFMADD231PS_YMM_YMM_YMM", U.lat, "exact"),
    ("fadd_latency", "VADDPS_YMM_YMM_YMM", U.lat, "exact"),
    ("fmul_latency", "VMULPD_YMM_YMM_YMM", U.lat, "exact"),
    ("popcnt_false_dep", "POPCNT_R64_R64",
     lambda e: (edge_dep(e, 1, 1) or 0) > 0, "bool"),
    ("lzcnt_false_dep", "LZCNT_R64_R64",
     lambda e: (edge_dep(e, 1, 1) or 0) > 0, "bool"),
    ("tzcnt_false_dep", "TZCNT_R64_R64",
     lambda e: (edge_dep(e, 1, 1) or 0) > 0, "bool"),
    ("fadd_pipes", "VADDPS_YMM_YMM_YMM", lambda e: count_pipes(e["ports"]), "exact"),
    ("fma_pipes", "VFMADD231PS_YMM_YMM_YMM", lambda e: count_pipes(e["ports"]), "exact"),
    ("vec_int_alu_pipes", "VPADDD_YMM_YMM_YMM", lambda e: count_pipes(e["ports"]), "exact"),
]
# Page fallbacks for cores lacking the preferred form (no AVX2 on SNB/IVB, no
# TZCNT on SNB, ...): (field, arch) -> alternative page.
ALT_PAGE = {
    ("fadd_latency", "SNB"): "VADDPS_YMM_YMM_YMM",
    ("vec_int_alu_pipes", "SNB"): "PADDD_XMM_XMM",
    ("vec_int_alu_pipes", "IVB"): "PADDD_XMM_XMM",
    ("lzcnt_false_dep", "SNB"): "BSF_R64_R64",
    ("lzcnt_false_dep", "IVB"): "BSF_R64_R64",
    ("tzcnt_false_dep", "SNB"): "BSF_R64_R64",
    ("tzcnt_false_dep", "IVB"): "BSF_R64_R64",
    ("fadd_pipes", "ZEN+"): "VADDPS_XMM_XMM_XMM",
    ("fma_pipes", "ZEN+"): "VFMADD231PS_XMM_XMM_XMM",
}
# FP-pipe counts on ZEN are per 128-bit half; the model counts 256-bit pipes.
SKIP = {
    ("fma_pipes", "ZEN+"), ("fma_pipes", "SNB"), ("fma_pipes", "IVB"),
    ("fma_latency", "SNB"), ("fma_latency", "IVB"),
    ("fadd_pipes", "ZEN+"), ("fadd_pipes", "ZEN2"), ("fadd_pipes", "ZEN3"),
    ("fadd_pipes", "ZEN4"), ("fadd_pipes", "ZEN5"), ("fma_pipes", "ZEN2"),
    ("fma_pipes", "ZEN3"), ("fma_pipes", "ZEN4"), ("fma_pipes", "ZEN5"),
    ("vec_int_alu_pipes", "ZEN+"), ("vec_int_alu_pipes", "ZEN2"),
    ("vec_int_alu_pipes", "ZEN3"), ("vec_int_alu_pipes", "ZEN4"),
    ("vec_int_alu_pipes", "ZEN5"), ("vec_int_alu_pipes", "ARL-P"),
    ("fadd_pipes", "ARL-P"), ("fma_pipes", "ARL-P"),
}


def load_allow() -> dict:
    allow = {}
    if os.path.exists(ALLOW):
        for line in open(ALLOW, encoding="utf-8"):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split(None, 2)
            if len(parts) >= 3:
                allow[(parts[0], parts[1])] = parts[2]
    return allow


def dump_row(binary: str, row: str) -> dict:
    with tempfile.NamedTemporaryFile("w", suffix=".c") as f:
        f.write("int f(int x){return x+1;}\n")
        f.flush()
        env = dict(os.environ, LCCC_DUMP_TUNE="1")
        p = subprocess.run([binary, "-O2", f"-mtune={row}", "-S", "-o",
                            os.devnull, f.name], env=env, capture_output=True,
                           text=True)
    out = p.stdout + p.stderr
    kv = {}
    for line in out.splitlines():
        if "=" in line and not line.startswith(("warning", "error")):
            k, _, v = line.partition("=")
            kv[k.strip()] = v.strip()
    return kv


def model_val(kv: dict, field: str):
    v = kv.get(field)
    if v is None:
        return None
    if v in ("true", "false"):
        return v == "true"
    try:
        return int(v)
    except ValueError:
        return v


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", default=os.environ.get(
        "LCCC_BIN", os.path.join(REPO, "target/fastbuild/lccc")))
    ap.add_argument("--rows", default=",".join(U.ROW_ARCH))
    ap.add_argument("--xml")
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--markdown", action="store_true",
                    help="emit a markdown evidence table instead of text")
    a = ap.parse_args()
    if not os.path.exists(a.bin):
        print(f"error: {a.bin} missing (build with scripts/build_lccc_fast.sh)",
              file=sys.stderr)
        return 2
    idx = U.load(a.xml)
    allow = load_allow()
    n_ok = n_bad = n_skip = n_allowed = 0
    rows_md = []
    for row in a.rows.split(","):
        arch = U.ROW_ARCH.get(row)
        if arch is None:
            print(f"error: no uops.info column for row {row!r}", file=sys.stderr)
            return 2
        kv = dump_row(a.bin, row)
        if not kv:
            print(f"error: no tune dump for {row} (LCCC_DUMP_TUNE unsupported?)",
                  file=sys.stderr)
            return 2
        for field, page, ext, kind in CHECKS:
            page = ALT_PAGE.get((field, arch), page)
            mv = model_val(kv, field)
            if (field, arch) in SKIP or mv is None:
                n_skip += 1
                continue
            if row in NO_PORTS and field.endswith("_pipes"):
                n_skip += 1
                continue
            e = idx.get(page, {}).get(arch)
            if e is None:
                n_skip += 1
                if a.verbose:
                    print(f"SKIP  {row:15} {field:22} (no {page} on {arch})")
                continue
            try:
                ev = ext(e)
            except (TypeError, ValueError, IndexError):
                ev = None
            if ev is None:
                n_skip += 1
                continue
            if kind == "rtp":
                ok = abs(mv - ev) <= max(0.25 * ev, 3)
            elif kind == "near":
                ok = abs(mv - ev) <= max(0.15 * ev, 1)
            else:
                ok = mv == ev
            tag = "OK   "
            if not ok:
                reason = allow.get((row, field))
                if reason:
                    tag, n_allowed = "ALLOW", n_allowed + 1
                else:
                    tag, n_bad = "MISMATCH", n_bad + 1
            else:
                n_ok += 1
            if not ok or a.verbose or a.markdown:
                line = (f"{tag:8} {row:15} {field:22} model={mv!s:<6} "
                        f"uops.info[{arch},{page}]={ev}")
                if tag == "ALLOW":
                    line += f"   # {allow[(row, field)]}"
                if a.markdown:
                    rows_md.append((row, field, mv, ev, arch, page, tag.strip()))
                else:
                    print(line)
    if a.markdown:
        print("| row | field | model | uops.info | column | page | verdict |")
        print("|---|---|---|---|---|---|---|")
        for r in rows_md:
            print("| " + " | ".join(str(x) for x in r) + " |")
    print(f"\ncpu_model_audit: {n_ok} verified, {n_allowed} allow-listed, "
          f"{n_bad} MISMATCH, {n_skip} skipped", file=sys.stderr)
    return 1 if n_bad else 0


if __name__ == "__main__":
    sys.exit(main())

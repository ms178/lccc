#!/usr/bin/env python3
"""Executable high-ROI backlog sweep against a live LCCC toolchain.

`backlog.md` ranks work by ROI, but a prose status is not evidence.  This
script probes 16 measurable, already-landed sub-items from that ranking and
from the §12 GP table.  Every check:

* runs the built compiler (or the gate that examines its live source),
* compares an OPTION-ON observable with an OPTION-OFF control where that is
  the only way to prove the option is not inert,
* returns named problems, never a boolean, and
* has a mutation in ``negative_controls()`` that the check must reject.

Use ``--offline --self-test`` for the portable CI verdict-machinery contract;
it uses synthetic pass observations and never invokes a compiler.  Use
``--lccc`` without ``--offline`` for the full data-driven sweep, and ``--out``
to save machine-readable evidence.

The script deliberately does not claim to close the large open features
MB-04 (default PIE), MB-05 (stack-clash probes), MB-06 (canaries), SV-06
(general loop vectoriser) or SV-09 (AVX-512 opmasks).  It pins the *measured*
effects and honest diagnostics that are already shippable; the documentation
names the remaining work explicitly.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (ID, root-backlog ROI, source section, short contract)
ITEMS = [
    ("MB-20/3", 4.8, "§4 rank 3", "peephole never heap-copies a line before a rejecting filter"),
    ("MB-01/1", 4.6, "§4 rank 1", "-fno-plt emits a direct GOTPCREL call"),
    ("MB-01/2", 4.6, "§4 rank 1", "-fvisibility=hidden emits .hidden"),
    ("MB-02/1", 4.5, "§4 rank 2", "-z execstack / noexecstack control PT_GNU_STACK"),
    ("MB-02/2", 4.5, "§4 rank 2", "driver --oformat=binary matches objcopy of the same ELF"),
    ("GP-22/5", 4.5, "§12.1 rank 1", "the option-effect ledger executes on all four drivers"),
    ("GP-01/1", 4.6, "§12.1 rank 2", "an unknown attribute warns under -Wattributes"),
    ("GP-23/1", 4.0, "§12.1 rank 5", "-dM -E macro parity stays within measured budgets"),
    ("GP-05/1", 4.4, "§12.1 rank 3", "__GNUC__ matches the pinned GCC 16.2 oracle"),
    ("MB-12/1", 3.4, "§4 rank 10", "-Wformat warns and -Werror=format fails"),
    ("GP-07/1", 3.4, "§12.1 rank 17", "C digraphs compile without diagnostics"),
    ("GP-08/1", 3.0, "§12.1 rank 22", "default C23 rejects an unprototyped extra argument"),
    ("GP-21/1", 3.2, "§12.1 rank 20", "driver query flags answer without diagnostics"),
    ("MB-13/1", 3.6, "§4 rank 8", "the i686 driver emits a real ELF32 object"),
    ("SV-08/1", 0.0, "§9b.3 SV-08 (unranked, effort S)", "a ymm-using function ends with vzeroupper"),
    ("MB-04/3", 4.2, "§4 rank 4", "explicit -pie and --build-id=sha1 produce DYN + Build ID"),
]

MAIN = "int main(void){return 0;}\n"
EXTERN = "extern int g(int); int f(int x){return g(x)+1;}\n"
FORMAT = 'int printf(const char*, ...); int f(void){return printf("%d\\n", "x");}\n'
DIGRAPH = "int main(void) <% return 0; %>\n"
PROTO = "static int no_args() { return 42; } int probe(void) { return no_args(1); }\n"
YMM = "void f(float *a){for(int i=0;i<8;i++) a[i]=a[i]*2.0f;}\n"


@dataclass(frozen=True)
class Tools:
    root: Path
    lccc: str
    gcc: str
    clang: str
    readelf: str
    objcopy: str
    python: str = sys.executable

    @classmethod
    def resolve(cls, root: Path, args: argparse.Namespace) -> "Tools":
        def binary(value: str | None, default: str, repo_fallback: str | None = None) -> str:
            candidate = value or default
            found = shutil.which(candidate)
            if found:
                return found
            path = Path(candidate)
            if path.is_file() and os.access(path, os.X_OK):
                return str(path.resolve())
            if repo_fallback:
                path = root / repo_fallback
                if path.is_file() and os.access(path, os.X_OK):
                    return str(path.resolve())
            raise SystemExit(f"missing required tool: {candidate}")

        return cls(
            root=root,
            lccc=binary(args.lccc, str(root / "target/fastbuild/lccc")),
            gcc=binary(args.gcc, "gcc-16"),
            clang=binary(args.clang, "clang-23"),
            readelf=binary(args.readelf, "readelf"),
            objcopy=binary(args.objcopy, "objcopy"),
        )


def _run(cmd: list[str], td: Path, timeout: int = 90) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(x) for x in cmd], cwd=td, capture_output=True, text=True,
        timeout=timeout, env={**os.environ, "LC_ALL": "C"},
    )


def _source(td: Path, name: str, text: str) -> Path:
    path = td / name
    path.write_text(text)
    return path


def _compile(tools: Tools, td: Path, src: Path, flags: list[str], out: str) -> subprocess.CompletedProcess[str]:
    return _run([tools.lccc, *flags, str(src), "-o", str(td / out)], td)


def _text(path: Path) -> str:
    return path.read_text(errors="replace") if path.is_file() else ""


def _magic(path: Path) -> bytes:
    return path.read_bytes()[:4] if path.is_file() else b""


def _gnu_stack_flags(readelf: str, td: Path, image: Path) -> str:
    proc = _run([readelf, "-lW", str(image)], td)
    for line in proc.stdout.splitlines():
        if "GNU_STACK" in line:
            match = re.search(r"\b(RWE|RW|R E|R)\b", line)
            return match.group(1).replace(" ", "") if match else "UNKNOWN"
    return "MISSING"


def _macro_major(proc: subprocess.CompletedProcess[str]) -> str:
    match = re.search(r"^#define __GNUC__ (\d+)\s*$", proc.stdout, re.MULTILINE)
    return match.group(1) if match else "MISSING"


def collect(tools: Tools) -> tuple[dict[str, dict], dict]:
    """Probe the live tree.  No cached verdicts and no network access."""
    obs: dict[str, dict] = {}
    with tempfile.TemporaryDirectory(prefix="lccc-backlog-roi-") as raw_td:
        td = Path(raw_td)

        # MB-20/3: the source contract that removed the quadratic peephole
        # allocation pathology.  Also execute its own unittest module so a
        # green scanner with dead negative controls cannot pass.
        scanner = _run([tools.python, str(tools.root / "tests/regression/check_peephole_alloc_before_filter.py"),
                        "--root", str(tools.root / "src/backend")], td)
        tests = _run([tools.python, "-m", "unittest", "discover", "-s", str(tools.root / "tests/regression"),
                      "-p", "test_check_peephole_alloc_before_filter.py"], td)
        count = re.search(r"PASS \((\d+) files", scanner.stdout)
        obs["MB-20/3"] = {
            "rc": scanner.returncode, "tests_rc": tests.returncode,
            "pass": "PASS" in scanner.stdout,
            "files": int(count.group(1)) if count else 0,
            "tests_tail": (tests.stdout + tests.stderr).strip()[-200:],
        }

        # MB-01/1: GOTPCREL must appear only with -fno-plt.
        src = _source(td, "extern.c", EXTERN)
        base = _compile(tools, td, src, ["-fPIC", "-O2", "-S"], "plt.s")
        flag = _compile(tools, td, src, ["-fPIC", "-O2", "-fno-plt", "-S"], "noplt.s")
        obs["MB-01/1"] = {
            "base_rc": base.returncode, "flag_rc": flag.returncode,
            "base_gotpcrel": _text(td / "plt.s").count("GOTPCREL"),
            "flag_gotpcrel": _text(td / "noplt.s").count("GOTPCREL"),
        }

        # MB-01/2: .hidden must appear only with -fvisibility=hidden.
        src = _source(td, "visible.c", "int f(void){return 42;}\n")
        base = _compile(tools, td, src, ["-O2", "-S"], "visible.s")
        flag = _compile(tools, td, src, ["-O2", "-fvisibility=hidden", "-S"], "hidden.s")
        obs["MB-01/2"] = {
            "base_rc": base.returncode, "flag_rc": flag.returncode,
            "base_hidden": _text(td / "visible.s").count(".hidden"),
            "flag_hidden": _text(td / "hidden.s").count(".hidden"),
        }

        # MB-02/1: security-relevant PT_GNU_STACK flags.
        src = _source(td, "main.c", MAIN)
        exec_link = _compile(tools, td, src, ["-Wl,-z,execstack"], "execstack.elf")
        noexec_link = _compile(tools, td, src, ["-Wl,-z,noexecstack"], "noexecstack.elf")
        obs["MB-02/1"] = {
            "exec_rc": exec_link.returncode, "noexec_rc": noexec_link.returncode,
            "exec_flags": _gnu_stack_flags(tools.readelf, td, td / "execstack.elf"),
            "noexec_flags": _gnu_stack_flags(tools.readelf, td, td / "noexecstack.elf"),
        }

        # MB-02/2: driver flat-image conversion must be byte-exact.
        flat = _compile(tools, td, src, ["-Wl,--oformat=binary"], "flat.bin")
        elf = _compile(tools, td, src, [], "normal.elf")
        objcopy = _run([tools.objcopy, "-O", "binary", str(td / "normal.elf"), str(td / "objcopy.bin")], td)
        expected = (td / "objcopy.bin").read_bytes() if (td / "objcopy.bin").is_file() else b""
        actual = (td / "flat.bin").read_bytes() if (td / "flat.bin").is_file() else b""
        obs["MB-02/2"] = {
            "flat_rc": flat.returncode, "elf_rc": elf.returncode, "objcopy_rc": objcopy.returncode,
            "flat_magic": _magic(td / "flat.bin"), "elf_magic": _magic(td / "normal.elf"),
            "flat_equals_objcopy": bool(actual) and actual == expected,
            "flat_sha256": hashlib.sha256(actual).hexdigest(),
        }

        # GP-22/5: execute the ledger on every shipped driver, not only x86-64.
        drivers = {}
        for name in ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv"):
            path = tools.root / "target/fastbuild" / name
            proc = _run([tools.python, str(tools.root / "scripts/option_effect_ledger.py"),
                         "--lccc", str(path)], td, timeout=180) if path.is_file() else None
            drivers[name] = {
                "exists": path.is_file(), "rc": proc.returncode if proc else 1,
                "pass": bool(proc and "PASS" in proc.stdout),
                "tail": ((proc.stdout + proc.stderr).strip()[-160:] if proc else "binary absent"),
            }
        obs["GP-22/5"] = {"drivers": drivers}

        # GP-01/1: unknown attribute must be named and tagged -Wattributes.
        src = _source(td, "attr.c", "int __attribute__((frobnicate)) x; int main(void){return x;}\n")
        attr = _compile(tools, td, src, ["-Wattributes", "-c"], "attr.o")
        obs["GP-01/1"] = {
            "rc": attr.returncode,
            "names_attribute": "frobnicate" in attr.stderr,
            "names_warning": "Wattributes" in attr.stderr,
        }

        # GP-23/1: macro parity gate with the pinned GCC 16.2 oracle.
        macro = _run([tools.python, str(tools.root / "tests/regression/check_macro_parity.py"),
                      "--lccc", tools.lccc, "--gcc", tools.gcc], td, timeout=180)
        obs["GP-23/1"] = {
            "rc": macro.returncode, "pass": "macro parity: PASS" in macro.stdout,
            "tail": (macro.stdout + macro.stderr).strip()[-200:],
        }

        # GP-05/1: exact GCC 16 major, with all three pinned tool banners recorded.
        src = _source(td, "macros.c", MAIN)
        lccc_m = _run([tools.lccc, "-dM", "-E", str(src)], td)
        gcc_m = _run([tools.gcc, "-dM", "-E", str(src)], td)
        gcc_v = _run([tools.gcc, "--version"], td)
        clang_v = _run([tools.clang, "--version"], td)
        icx_hosted = "ICX 2026.2 is supplied by the hosted authenticated CI lane, not this sandbox"
        obs["GP-05/1"] = {
            "lccc_rc": lccc_m.returncode, "gcc_rc": gcc_m.returncode,
            "lccc_gnuc": _macro_major(lccc_m), "gcc_gnuc": _macro_major(gcc_m),
            "gcc_banner": gcc_v.stdout.splitlines()[0] if gcc_v.stdout else "",
            "clang_banner": clang_v.stdout.splitlines()[0] if clang_v.stdout else "",
            "icx_note": icx_hosted,
        }

        # MB-12/1: format warning and its -Werror promotion.
        src = _source(td, "format.c", FORMAT)
        warn = _compile(tools, td, src, ["-Wformat", "-c"], "warn.o")
        werror = _compile(tools, td, src, ["-Werror=format", "-c"], "werror.o")
        obs["MB-12/1"] = {
            "warn_rc": warn.returncode, "werror_rc": werror.returncode,
            "warn_has_format": "-Wformat=" in warn.stderr,
            "werror_has_error": "error:" in werror.stderr,
        }

        # GP-07/1: digraph spelling is real syntax, not a tolerated token.
        src = _source(td, "digraph.c", DIGRAPH)
        digraph = _compile(tools, td, src, ["-fsyntax-only"], "digraph.o")
        obs["GP-07/1"] = {"rc": digraph.returncode, "stderr": digraph.stderr.strip()}

        # GP-08/1: default GNU C23 versus the explicit GNU17 escape hatch.
        src = _source(td, "proto.c", PROTO)
        default = _compile(tools, td, src, ["-fsyntax-only"], "proto.o")
        gnu17 = _compile(tools, td, src, ["-std=gnu17", "-fsyntax-only"], "proto17.o")
        obs["GP-08/1"] = {"default_rc": default.returncode, "gnu17_rc": gnu17.returncode}

        # GP-21/1: driver query answers.
        queries = {}
        for flag in ("-dumpfullversion", "-print-multi-directory", "-print-sysroot",
                     "-print-multi-lib", "-print-search-dirs"):
            proc = _run([tools.lccc, flag], td)
            queries[flag] = {"rc": proc.returncode, "stdout": proc.stdout.strip()}
        obs["GP-21/1"] = {"queries": queries}

        # MB-13/1: i686 object identity, with no 32-bit libc dependency.
        i686 = tools.root / "target/fastbuild/lccc-i686"
        src = _source(td, "i686.c", "int f(void){return 42;}\n")
        proc = _run([str(i686), "-c", str(src), "-o", str(td / "i686.o")], td) if i686.is_file() else None
        header = _run([tools.readelf, "-hW", str(td / "i686.o")], td) if proc and proc.returncode == 0 else None
        klass = re.search(r"Class:\s+(\S+)", header.stdout).group(1) if header and re.search(r"Class:\s+(\S+)", header.stdout) else "MISSING"
        machine = "Intel 80386" in header.stdout if header else False
        obs["MB-13/1"] = {
            "driver_exists": i686.is_file(), "rc": proc.returncode if proc else 1,
            "elf_class": klass, "i386_machine": machine,
        }

        # SV-08/1: AVX upper-state hygiene on a ymm-using function.
        src = _source(td, "ymm.c", YMM)
        ymm = _compile(tools, td, src, ["-O2", "-mavx2", "-S"], "ymm.s")
        text = _text(td / "ymm.s")
        obs["SV-08/1"] = {
            "rc": ymm.returncode, "ymm": text.count("ymm"),
            "vzeroupper": text.count("vzeroupper"),
        }

        # MB-04/3: explicit PIE + build-id effect.  Default PIE remains an
        # OPEN backlog item; this check must not be read as closing it.
        src = _source(td, "pie.c", MAIN)
        pie = _compile(tools, td, src, ["-pie", "-Wl,--build-id=sha1"], "pie.elf")
        header = _run([tools.readelf, "-hW", str(td / "pie.elf")], td)
        notes = _run([tools.readelf, "-nW", str(td / "pie.elf")], td)
        ptype = re.search(r"Type:\s+(\S+)", header.stdout)
        obs["MB-04/3"] = {
            "rc": pie.returncode, "type": ptype.group(1) if ptype else "MISSING",
            "build_id": "Build ID:" in notes.stdout,
        }

    tool_meta = {
        "lccc": tools.lccc, "gcc": tools.gcc, "clang": tools.clang,
        "readelf": tools.readelf, "objcopy": tools.objcopy,
        "mode": "live dynamic probes",
    }
    return obs, tool_meta


# ---------------------------------------------------------------------------
# One check per item.  Each returns a list of named problems; [] means PASS.
# ---------------------------------------------------------------------------

def mb20(o: dict) -> list[str]:
    x = o["MB-20/3"]
    out = []
    if x["rc"] != 0 or x["tests_rc"] != 0 or not x["pass"]:
        out.append("MB-20/3: allocation-before-filter gate or its unit tests did not PASS")
    if x["files"] < 1:
        out.append("MB-20/3: scanner examined zero backend files")
    return out


def mb01_noplt(o: dict) -> list[str]:
    x = o["MB-01/1"]
    out = []
    if x["base_rc"] or x["flag_rc"]:
        out.append("MB-01/1: one of the -fno-plt control compiles failed")
    if x["flag_gotpcrel"] < 1:
        out.append("MB-01/1: -fno-plt emitted no GOTPCREL call")
    if x["base_gotpcrel"] != 0:
        out.append("MB-01/1: the option-off control already emitted GOTPCREL; the probe is not discriminating")
    return out


def mb01_visibility(o: dict) -> list[str]:
    x = o["MB-01/2"]
    out = []
    if x["base_rc"] or x["flag_rc"]:
        out.append("MB-01/2: one of the visibility control compiles failed")
    if x["flag_hidden"] < 1:
        out.append("MB-01/2: -fvisibility=hidden emitted no .hidden symbol")
    if x["base_hidden"] != 0:
        out.append("MB-01/2: the option-off control already emitted .hidden; the probe is not discriminating")
    return out


def mb02_stack(o: dict) -> list[str]:
    x = o["MB-02/1"]
    out = []
    if x["exec_rc"] or x["noexec_rc"]:
        out.append("MB-02/1: one of the PT_GNU_STACK links failed")
    if x["exec_flags"] != "RWE":
        out.append(f"MB-02/1: -z execstack produced {x['exec_flags']}, expected RWE")
    if x["noexec_flags"] != "RW":
        out.append(f"MB-02/1: -z noexecstack produced {x['noexec_flags']}, expected RW")
    return out


def mb02_flat(o: dict) -> list[str]:
    x = o["MB-02/2"]
    out = []
    if x["flat_rc"] or x["elf_rc"] or x["objcopy_rc"]:
        out.append("MB-02/2: a link, ELF control or objcopy conversion failed")
    if x["elf_magic"] != b"\x7fELF":
        out.append("MB-02/2: the control output is not ELF")
    if x["flat_magic"] == b"\x7fELF":
        out.append("MB-02/2: --oformat=binary silently left an ELF container")
    if not x["flat_equals_objcopy"]:
        out.append("MB-02/2: driver flat image does not byte-match objcopy of its own ELF")
    return out


def gp22_ledger(o: dict) -> list[str]:
    drivers = o["GP-22/5"]["drivers"]
    out = []
    if set(drivers) != {"lccc", "lccc-i686", "lccc-arm", "lccc-riscv"}:
        out.append("GP-22/5: ledger driver set changed without extending the gate")
    for name, row in drivers.items():
        if not row["exists"] or row["rc"] != 0 or not row["pass"]:
            out.append(f"GP-22/5: ledger did not PASS on {name}: {row['tail']}")
    return out


def gp01_attributes(o: dict) -> list[str]:
    x = o["GP-01/1"]
    out = []
    if x["rc"] != 0:
        out.append("GP-01/1: unknown-attribute probe did not compile")
    if not x["names_attribute"] or not x["names_warning"]:
        out.append("GP-01/1: diagnostic did not name frobnicate and -Wattributes")
    return out


def gp23_macro(o: dict) -> list[str]:
    x = o["GP-23/1"]
    return [] if x["rc"] == 0 and x["pass"] else [f"GP-23/1: {x['tail']}"]


def gp05_gnuc(o: dict) -> list[str]:
    x = o["GP-05/1"]
    out = []
    if x["lccc_rc"] or x["gcc_rc"]:
        out.append("GP-05/1: a -dM -E probe failed")
    if x["lccc_gnuc"] != "16" or x["gcc_gnuc"] != "16":
        out.append(f"GP-05/1: LCCC={x['lccc_gnuc']}, GCC={x['gcc_gnuc']}; expected 16/16")
    if "16.2.0" not in x["gcc_banner"]:
        out.append("GP-05/1: GCC oracle is not the pinned 16.2.0")
    if "23.1.5" not in x["clang_banner"]:
        out.append("GP-05/1: Clang oracle is not the pinned 23.1.5")
    return out


def mb12_wformat(o: dict) -> list[str]:
    x = o["MB-12/1"]
    out = []
    if x["warn_rc"] != 0 or not x["warn_has_format"]:
        out.append("MB-12/1: -Wformat did not warn and continue")
    if x["werror_rc"] == 0 or not x["werror_has_error"]:
        out.append("MB-12/1: -Werror=format did not promote the warning to an error")
    return out


def gp07_digraph(o: dict) -> list[str]:
    x = o["GP-07/1"]
    return [] if x["rc"] == 0 and not x["stderr"] else [f"GP-07/1: rc={x['rc']} stderr={x['stderr']!r}"]


def gp08_c23(o: dict) -> list[str]:
    x = o["GP-08/1"]
    out = []
    if x["default_rc"] == 0:
        out.append("GP-08/1: default C23 accepted an extra argument to an empty-prototype function")
    if x["gnu17_rc"] != 0:
        out.append("GP-08/1: explicit -std=gnu17 escape hatch was rejected")
    return out


def gp21_queries(o: dict) -> list[str]:
    q = o["GP-21/1"]["queries"]
    out = []
    for flag in ("-dumpfullversion", "-print-multi-directory", "-print-sysroot",
                 "-print-multi-lib", "-print-search-dirs"):
        if flag not in q or q[flag]["rc"] != 0:
            out.append(f"GP-21/1: {flag} failed or is missing")
    if q.get("-dumpfullversion", {}).get("stdout") != "16.2.0":
        out.append("GP-21/1: -dumpfullversion no longer reports the pinned GCC compatibility version")
    if "32;@m32" not in q.get("-print-multi-lib", {}).get("stdout", ""):
        out.append("GP-21/1: -print-multi-lib no longer names the i686 multilib")
    return out


def mb13_i686(o: dict) -> list[str]:
    x = o["MB-13/1"]
    out = []
    if not x["driver_exists"] or x["rc"] != 0:
        out.append("MB-13/1: lccc-i686 object compile failed")
    if x["elf_class"] != "ELF32" or not x["i386_machine"]:
        out.append("MB-13/1: object is not an Intel 80386 ELF32 object")
    return out


def sv08_vzeroupper(o: dict) -> list[str]:
    x = o["SV-08/1"]
    out = []
    if x["rc"] != 0:
        out.append("SV-08/1: ymm fixture did not compile")
    if x["ymm"] < 1:
        out.append("SV-08/1: fixture no longer uses ymm; the gate would be vacuous")
    if x["vzeroupper"] < 1:
        out.append("SV-08/1: ymm-using function has no vzeroupper")
    return out


def mb04_explicit(o: dict) -> list[str]:
    x = o["MB-04/3"]
    out = []
    if x["rc"] != 0:
        out.append("MB-04/3: explicit -pie link failed")
    if x["type"] != "DYN":
        out.append(f"MB-04/3: explicit -pie produced {x['type']}, expected DYN")
    if not x["build_id"]:
        out.append("MB-04/3: --build-id=sha1 emitted no Build ID note")
    return out


CHECKS = [
    ("MB-20/3", mb20),
    ("MB-01/1", mb01_noplt),
    ("MB-01/2", mb01_visibility),
    ("MB-02/1", mb02_stack),
    ("MB-02/2", mb02_flat),
    ("GP-22/5", gp22_ledger),
    ("GP-01/1", gp01_attributes),
    ("GP-23/1", gp23_macro),
    ("GP-05/1", gp05_gnuc),
    ("MB-12/1", mb12_wformat),
    ("GP-07/1", gp07_digraph),
    ("GP-08/1", gp08_c23),
    ("GP-21/1", gp21_queries),
    ("MB-13/1", mb13_i686),
    ("SV-08/1", sv08_vzeroupper),
    ("MB-04/3", mb04_explicit),
]
assert len(CHECKS) == len(ITEMS) == 16
assert [name for name, _ in CHECKS] == [item[0] for item in ITEMS]


def run_checks(observations: dict[str, dict]) -> dict[str, list[str]]:
    return {name: fn(observations) for name, fn in CHECKS}


def synthetic() -> dict[str, dict]:
    """A fully passing observation set for portable, compiler-free CI tests."""
    return {
        "MB-20/3": {"rc": 0, "tests_rc": 0, "pass": True, "files": 373, "tests_tail": "OK"},
        "MB-01/1": {"base_rc": 0, "flag_rc": 0, "base_gotpcrel": 0, "flag_gotpcrel": 1},
        "MB-01/2": {"base_rc": 0, "flag_rc": 0, "base_hidden": 0, "flag_hidden": 1},
        "MB-02/1": {"exec_rc": 0, "noexec_rc": 0, "exec_flags": "RWE", "noexec_flags": "RW"},
        "MB-02/2": {"flat_rc": 0, "elf_rc": 0, "objcopy_rc": 0,
                    "flat_magic": b"/lib", "elf_magic": b"\x7fELF",
                    "flat_equals_objcopy": True, "flat_sha256": "0" * 64},
        "GP-22/5": {"drivers": {name: {"exists": True, "rc": 0, "pass": True, "tail": "PASS"}
                                for name in ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv")}},
        "GP-01/1": {"rc": 0, "names_attribute": True, "names_warning": True},
        "GP-23/1": {"rc": 0, "pass": True, "tail": "macro parity: PASS"},
        "GP-05/1": {"lccc_rc": 0, "gcc_rc": 0, "lccc_gnuc": "16", "gcc_gnuc": "16",
                    "gcc_banner": "gcc-16 (GCC) 16.2.0", "clang_banner": "clang version 23.1.5",
                    "icx_note": "hosted authenticated ICX 2026.2 lane"},
        "MB-12/1": {"warn_rc": 0, "werror_rc": 1, "warn_has_format": True, "werror_has_error": True},
        "GP-07/1": {"rc": 0, "stderr": ""},
        "GP-08/1": {"default_rc": 1, "gnu17_rc": 0},
        "GP-21/1": {"queries": {
            "-dumpfullversion": {"rc": 0, "stdout": "16.2.0"},
            "-print-multi-directory": {"rc": 0, "stdout": "."},
            "-print-sysroot": {"rc": 0, "stdout": ""},
            "-print-multi-lib": {"rc": 0, "stdout": ".;\n32;@m32"},
            "-print-search-dirs": {"rc": 0, "stdout": "install: /usr/lib/gcc"},
        }},
        "MB-13/1": {"driver_exists": True, "rc": 0, "elf_class": "ELF32", "i386_machine": True},
        "SV-08/1": {"rc": 0, "ymm": 4, "vzeroupper": 1},
        "MB-04/3": {"rc": 0, "type": "DYN", "build_id": True},
    }


def negative_controls(observations: dict[str, dict]) -> list[str]:
    """Each check must reject one plausible, isolated regression."""
    mutations = [
        ("MB-20/3", lambda o: o["MB-20/3"].update(rc=1)),
        ("MB-01/1", lambda o: o["MB-01/1"].update(flag_gotpcrel=0)),
        ("MB-01/2", lambda o: o["MB-01/2"].update(flag_hidden=0)),
        ("MB-02/1", lambda o: o["MB-02/1"].update(exec_flags="RW")),
        ("MB-02/2", lambda o: o["MB-02/2"].update(flat_equals_objcopy=False)),
        ("GP-22/5", lambda o: o["GP-22/5"]["drivers"]["lccc-arm"].update(rc=1)),
        ("GP-01/1", lambda o: o["GP-01/1"].update(names_warning=False)),
        ("GP-23/1", lambda o: o["GP-23/1"].update(rc=1)),
        ("GP-05/1", lambda o: o["GP-05/1"].update(lccc_gnuc="14")),
        ("MB-12/1", lambda o: o["MB-12/1"].update(werror_rc=0)),
        ("GP-07/1", lambda o: o["GP-07/1"].update(rc=1)),
        ("GP-08/1", lambda o: o["GP-08/1"].update(default_rc=0)),
        ("GP-21/1", lambda o: o["GP-21/1"]["queries"]["-dumpfullversion"].update(stdout="15.2.0")),
        ("MB-13/1", lambda o: o["MB-13/1"].update(elf_class="ELF64")),
        ("SV-08/1", lambda o: o["SV-08/1"].update(vzeroupper=0)),
        ("MB-04/3", lambda o: o["MB-04/3"].update(type="EXEC")),
    ]
    by_name = dict(CHECKS)
    failures = []
    for name, mutate in mutations:
        copy_obs = copy.deepcopy(observations)
        mutate(copy_obs)
        if not by_name[name](copy_obs):
            failures.append(f"negative control for {name} did not make its check fail")
    missing = {name for name, _ in CHECKS} - {name for name, _ in mutations}
    if missing:
        failures.append(f"checks without a negative control: {', '.join(sorted(missing))}")
    return failures


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lccc", help="built LCCC driver (default target/fastbuild/lccc)")
    ap.add_argument("--gcc", help="GCC 16.2 oracle (default gcc-16)")
    ap.add_argument("--clang", help="Clang 23.1.5 oracle (default clang-23)")
    ap.add_argument("--readelf", help="readelf binary (default readelf)")
    ap.add_argument("--objcopy", help="objcopy binary (default objcopy)")
    ap.add_argument("--offline", action="store_true",
                    help="use synthetic pass observations; no compiler is invoked")
    ap.add_argument("--self-test", action="store_true", help="execute the 16 negative controls")
    ap.add_argument("--json", action="store_true", help="emit machine-readable verdicts")
    ap.add_argument("--out", type=Path, help="also write the JSON report to this path")
    args = ap.parse_args(argv)

    if args.offline:
        observations = synthetic()
        tool_meta = {"mode": "offline synthetic verdict machinery"}
    else:
        tools = Tools.resolve(ROOT, args)
        observations, tool_meta = collect(tools)

    results = run_checks(observations)
    problems = {k: v for k, v in results.items() if v}
    controls = negative_controls(observations) if args.self_test else []
    report = {
        "items": len(CHECKS),
        "passed": len(CHECKS) - len(problems),
        "failed": sorted(problems),
        "results": results,
        "self_test": controls,
        "tools": tool_meta,
        "items_registry": [
            {"id": i, "roi": roi, "source": source, "contract": contract}
            for i, roi, source, contract in ITEMS
        ],
    }
    raw = json.dumps(report, indent=2, sort_keys=True, default=str)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(raw + "\n")
    if args.json:
        print(raw)
    else:
        for name, probs in results.items():
            print(f"  {'PASS' if not probs else 'FAIL':4} {name}")
            for p in probs:
                print(f"        {p}")
        print(f"backlog ROI dynamic sweep: {report['passed']}/{report['items']} items PASS"
              + (f"; self-test: {len(controls)} failure(s)" if args.self_test else ""))
        for f in controls:
            print(f"  SELF-TEST FAIL: {f}", file=sys.stderr)
    return 1 if problems or controls else 0


if __name__ == "__main__":
    raise SystemExit(main())

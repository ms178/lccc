#!/usr/bin/env python3
"""Endian-load footprints, affine bounds and rotated wraparound, executed.

GCC 16.2 supplies an independent native reference. Other targets execute the
same fixed-width, self-checking C via their Linux syscall entry, without a
cross-libc or a silent architecture skip. No timing claims are made here.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCES = [ROOT / "tests/regression" / name for name in
           ("byte_load_preinline.c", "affine_exit_bound.c", "rotation_signed_wrap.c", "affine_optional_exit.c",
            "wrapv_byte_address.c", "fenv_rounding_freestanding.c", "wrapv_masked_index.c",
            "integer_pointer_width.c")]
SOURCE_FLAGS = {"fenv_rounding_freestanding.c": ["-frounding-math"],
                "integer_pointer_width.c": ["-DLCCC_POINTER_WIDTH_FREESTANDING=1", "-ffreestanding",
                                            "-Wno-int-to-pointer-cast", "-Wno-pointer-to-int-cast"]}
ENTRY = {
    "i686": ".text\n.globl _start\n_start:\ncall main\nmovl %eax,%ebx\nmovl $1,%eax\nint $0x80\n",
    "arm": ".text\n.globl _start\n_start:\nbl main\nmov x8,#93\nsvc #0\n",
    "riscv": ".text\n.globl _start\n_start:\ncall main\nli a7,93\necall\n",
}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    ap.add_argument("--cross", action="store_true")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    if not __debug__:
        ap.error("optimized Python disables assertions")
    clean = {k: v for k, v in os.environ.items() if not k.startswith(("CCC_", "LCCC_"))}
    clean["CCC_VERIFY_IR"] = "abort"
    records = []

    def run(command, env=None, timeout=30):
        result = subprocess.run(command, env=env or clean, capture_output=True,
                                text=True, timeout=timeout)
        assert result.returncode == 0, (command, result.returncode, result.stdout[-3000:], result.stderr[-3000:])
        return result

    error = None
    try:
        version = run([args.gcc, "-dumpfullversion"]).stdout.strip()
        assert version == "16.2.0", f"expected GCC 16.2.0, got {version}"
        with tempfile.TemporaryDirectory(prefix="lccc-pr824-boundaries-") as tmp:
            tmp = Path(tmp)
            for source in SOURCES:
                output = tmp / "reference"
                command = [args.gcc, "-O2", "-fwrapv", "-Wall", "-Wextra", "-Werror", *SOURCE_FLAGS.get(source.name, []), str(source), "-o", str(output)]
                run(command); run([str(output)])
                records.append({"reference": str(source.relative_to(ROOT)), "command": command, "passed": True})
            targets = [("x86-64", args.lccc, [], [])]
            if args.cross:
                for arch, runner in [("i686", []), ("arm", ["qemu-aarch64"]), ("riscv", ["qemu-riscv64"])]:
                    entry = tmp / (arch + ".s")
                    entry.write_text(ENTRY[arch])
                    compiler = str(Path(args.lccc).with_name("lccc-" + arch))
                    targets.append((arch, compiler, ["-nostdlib", "-static", str(entry)], runner))
            modes = [(opt, {}) for opt in ("-O0", "-O1", "-O2", "-O3", "-Os")]
            modes += [("-O2", {"CCC_LOOP_ROTATE": "1"}),
                      ("-O2", {"CCC_NO_AFFINE_EXIT_FOLD": "1"}),
                      ("-O2", {"CCC_LOOP_ROTATE": "1", "CCC_NO_AFFINE_EXIT_FOLD": "1"}),
                      ("-O2", {"CCC_DISABLE_PASSES": "loop_bound"}),
                      ("-O2", {"CCC_DISABLE_PASSES": "loop_bound", "CCC_NO_AFFINE_EXIT_FOLD": "1"}),
                      ("-O2", {"CCC_DISABLE_PASSES": "byte_load"})]
            for arch, compiler, extra, runner in targets:
                for source in SOURCES:
                    for opt, overrides in modes:
                        output = tmp / "program"
                        command = [compiler, opt, "-fwrapv", *SOURCE_FLAGS.get(source.name, []), *extra, str(source), "-o", str(output)]
                        run(command, dict(clean, **overrides))
                        run([*runner, str(output)], timeout=10)
                        records.append({"target": arch, "source": source.name, "opt": opt,
                                        "env": overrides, "command": command, "passed": True})
            for opt in ("-O0", "-O2", "-O3", "-Os"):
                output = tmp / "guard"
                command = [args.lccc, opt, "-DGUARD_PAGES", str(SOURCES[0]), "-o", str(output)]
                run(command); run([str(output)], timeout=10)
                records.append({"guard_pages": opt, "command": command, "passed": True})
            asm = tmp / "shape.s"
            run([args.lccc, "-O2", "-S", str(SOURCES[0]), "-o", str(asm)])
            assert not re.search(r"\bcallq?\s+body_le64\b", asm.read_text()), "cheap folded helper stayed out of line"
            records.append({"shape": "folded helper has no call", "passed": True})
            # Both independently scheduled producers can reach this guard.
            # Pin the complete ownership matrix, not just a shared opcode.
            # CCC_NO_AFFINE_EXIT_FOLD is the global owner switch: it kills
            # EVERY affine exit-compare producer. CCC_DISABLE_PASSES=loop_bound
            # only unschedules the early canonicalizer, so the late producer
            # still folds. Pin that settled ownership matrix.
            for name, overrides, expected_early, expected_late in [
                ("both-enabled", {}, True, False),
                ("global-kill", {"CCC_NO_AFFINE_EXIT_FOLD": "1"}, False, False),
                ("early-disabled", {"CCC_DISABLE_PASSES": "loop_bound"}, False, True),
                ("both-disabled", {"CCC_DISABLE_PASSES": "loop_bound",
                                   "CCC_NO_AFFINE_EXIT_FOLD": "1"}, False, False),
            ]:
                command = [args.lccc, "-O2", "-S", str(SOURCES[1]), "-o", str(asm)]
                result = run(command, dict(clean, CCC_DEBUG_AFFINE_FOLD="1", **overrides))
                body = asm.read_text().split("hot_guard_64:\n", 1)[1].split(".size hot_guard_64", 1)[0]
                folded = bool(re.search(r"cmpq\s+\$248,", body))
                reports = [line for line in result.stderr.splitlines()
                           if line.startswith("[AFFOLD] hot_guard_64:")]
                early = any("(early unsigned)" in line for line in reports)
                late = any("(early unsigned)" not in line for line in reports)
                assert (early, late) == (expected_early, expected_late), (name, reports)
                assert folded == (expected_early or expected_late), (name, body)
                records.append({"shape": "independent unsigned fold owners", "mode": name,
                                "env": overrides, "command": command, "early": early,
                                "late": late, "passed": True})
    except (AssertionError, OSError, ValueError, IndexError, subprocess.TimeoutExpired) as failure:
        error = str(failure)
    summary = {"passed": len(records), "failed": int(error is not None), "skipped": 0,
               "error": error, "sources": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in SOURCES},
               "records": records}
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(summary, indent=2) + "\n")
    if error:
        print("FAIL:", error)
        escaped = error.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error title=Cross-target boundary witness failed::{escaped}")
    print(f"PR824 boundaries: {len(records)} PASS / {summary['failed']} FAIL / 0 SKIP")
    return int(error is not None)


if __name__ == "__main__":
    raise SystemExit(main())

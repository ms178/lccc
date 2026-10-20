#!/usr/bin/env python3
"""Batch 4 exact signed division runtime/structure and strict-load legality."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--lccc", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--clang")
    args = parser.parse_args()
    source = ROOT / "tests/regression/exact_signed_pow2.c"
    common = ["-fwrapv", "-ffp-contract=off", "-Wall", "-Wextra", "-Werror"]
    modes = [("gcc", args.gcc, ["-O2"], {})]
    if args.clang:
        modes.append(("clang", args.clang, ["-O2"], {}))
    for level in ["-O1", "-O2", "-O3"]:
        modes.append((level, args.lccc, [level], {}))
    for name, flags, env in [
        ("v3", ["-O2", "-march=x86-64-v3"], {}),
        ("sse2", ["-O2", "-march=x86-64"], {}),
        ("no-avx2", ["-O2", "-mno-avx2"], {}),
        ("no-sse41", ["-O2", "-mno-sse4.1"], {}),
        ("disabled", ["-O2"], {"CCC_DISABLE_PASSES": "exact_signed_div"}),
        ("no-regalloc", ["-O2"], {"CCC_NO_REGALLOC": "1"}),
    ]:
        modes.append((name, args.lccc, flags, env))
    with tempfile.TemporaryDirectory(prefix="exact-signed-div-") as td:
        expected = None
        for name, compiler, flags, extra in modes:
            output = Path(td) / name
            env = dict(os.environ, **extra)
            if compiler == args.lccc:
                env.update(CCC_VERIFY_IR="abort", CCC_VALIDATE_SSA="1")
            subprocess.run(
                [compiler, *flags, *common, str(source), "-o", str(output)],
                env=env,
                check=True,
                capture_output=True,
                timeout=120,
            )
            result = subprocess.run(
                [str(output)], check=True, capture_output=True, timeout=60
            ).stdout
            if expected is None:
                expected = result
            assert result == expected, name
            if compiler == args.lccc:
                assembly = output.with_suffix(".s")
                subprocess.run(
                    [compiler, *flags, *common, "-S", str(source), "-o", str(assembly)],
                    env=env,
                    check=True,
                    capture_output=True,
                    timeout=120,
                )
                text = assembly.read_text()
                body = re.search(
                    r"(?ms)^extra_volatile_load:\n(.*?)^\.size extra_volatile_load,",
                    text,
                )
                assert body and "exact_recip_probe" in body[1], name
                assert "vdivpd" not in body[1], f"{name}: unrelated volatile load lost"
                if name == "-O2":
                    enabled = re.search(
                        r"(?ms)^complement_w64:\n(.*?)^\.size complement_w64,", text
                    )[1]
                if name == "disabled":
                    disabled = re.search(
                        r"(?ms)^complement_w64:\n(.*?)^\.size complement_w64,", text
                    )[1]
            print(f"{name}: PASS 320160 comparisons + volatile-load legality")
        # Exact division must actually shorten the signed-bias sequence, not
        # just pass a runtime test through an already-optimized control.
        count = lambda s: len(re.findall(r"^\s+(?:sar|shr|add|lea)\w*\b", s, re.M))
        assert count(enabled) < count(disabled), (enabled, disabled)
        spectral = ROOT / "tests/benchmark/programs/spectral_norm.c"
        stages = {}
        for name, extra in [
            ("spectral", {}),
            ("spectral-bias", {"CCC_DISABLE_PASSES": "exact_signed_div"}),
        ]:
            assembly = Path(td) / (name + ".s")
            subprocess.run(
                [args.lccc, "-O2", "-S", str(spectral), "-o", str(assembly)],
                env=dict(os.environ, **extra),
                check=True,
                capture_output=True,
                timeout=120,
            )
            stages[name] = assembly.read_text()
        assert "vdivpd" in stages["spectral"]
        assert len(re.findall(r"vpsrad \$31", stages["spectral"])) < len(
            re.findall(r"vpsrad \$31", stages["spectral-bias"])
        )
        print("exact_signed_div ablation: shorter rounding sequence PASS")


if __name__ == "__main__":
    main()

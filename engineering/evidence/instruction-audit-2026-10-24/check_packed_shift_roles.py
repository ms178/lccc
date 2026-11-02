#!/usr/bin/env python3
"""Compare packed-shift acceptance and accepted bytes with GNU as in both modes."""
import argparse
import itertools
import json
import pathlib
import subprocess
import tempfile


def cases():
    banks = ("xmm", "ymm", "zmm")
    for mnemonic, variable in itertools.chain(
        ((m, False) for m in ("vpsllw", "vpslld", "vpsllq", "vpsrlw", "vpsrld", "vpsrlq", "vpsraw", "vpsrad", "vpsraq")),
        ((m, True) for m in ("vpsllvw", "vpsllvd", "vpsllvq", "vpsrlvw", "vpsrlvd", "vpsrlvq", "vpsravw", "vpsravd", "vpsravq")),
    ):
        for count, src, dst in itertools.product(banks, repeat=3):
            yield f"{mnemonic} %{count}1, %{src}2, %{dst}3", src == dst and count == (dst if variable else "xmm")
        for bank in banks:
            yield f"{mnemonic} $3, %{bank}2, %{bank}3", not variable
            yield f"{mnemonic} %eax, %{bank}2, %{bank}3", False
            yield f"{mnemonic} %xmm1, %eax, %{bank}3", False
    for mnemonic in ("vpslldq", "vpsrldq"):
        for src, dst in itertools.product(banks, repeat=2):
            yield f"{mnemonic} $3, %{src}2, %{dst}3", src == dst
            yield f"{mnemonic} %xmm1, %{src}2, %{dst}3", False
    for prefix, bank in itertools.product(("", "{evex} "), banks):
        yield f"{prefix}vpslld (%eax), %{bank}2, %{bank}3", True
        yield f"{prefix}vpslld $3, (%eax), %{bank}3", True
        yield f"{prefix}vpsllvd (%eax), %{bank}2, %{bank}3", True
        yield f"{prefix}vpsllvd %xmm1, (%eax), %{bank}3", False
        yield f"{prefix}vpslld %ymm1, %{bank}2, %{bank}3", False
        yield f"{prefix}vpslld %xmm1, %xmm2, %{bank}3", bank == "xmm"


    for mnemonic, reg in itertools.product(
        ("pslldq", "psrldq"), ("xmm0", "xmm7", "mm0", "eax", "ax", "al", "ymm0", "zmm0"),
    ):
        yield f"{mnemonic} $3, %{reg}", reg.startswith("xmm")
    # Immediate shifts use full-vector data tuples, or dword/qword broadcast
    # tuples. They require EVEX even when the destination is XMM/YMM.
    for mnemonic, bank, prefix in itertools.product(
        ("vpsllw", "vpslld", "vpsllq", "vpsrlw", "vpsrld", "vpsrlq", "vpsraw", "vpsrad", "vpsraq", "vpslldq", "vpsrldq"),
        banks, ("", "{evex} "),
    ):
        yield f"{prefix}{mnemonic} $3, 64(%eax), %{bank}3", True
        yield f"{{vex}} {mnemonic} $3, 64(%eax), %{bank}3", False
    for mnemonic, element in (("vpslld", 4), ("vpsrld", 4), ("vpsrad", 4), ("vpsllq", 8), ("vpsrlq", 8), ("vpsraq", 8)):
        for bank, size in zip(banks, (16, 32, 64)):
            yield f"{mnemonic} $3, 64(%eax){{1to{size // element}}}, %{bank}3", True
            yield f"{mnemonic} $3, 64(%eax){{1to{size // element * 2}}}, %{bank}3", False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("compiler", type=pathlib.Path)
    parser.add_argument("--report", required=True, type=pathlib.Path)
    parser.add_argument("--gas", default="as")
    parser.add_argument("--objcopy", default="objcopy")
    args = parser.parse_args()
    compiler = str(args.compiler.resolve())
    report = {"compiler": compiler, "gas_version": subprocess.check_output([args.gas, "--version"], text=True).splitlines()[0], "decisions": 0, "byte_matches": 0, "mismatches": []}
    with tempfile.TemporaryDirectory(prefix="packed-shift-audit-") as directory:
        p = pathlib.Path(directory)
        source = p / "case.s"
        for mode, (line, expected) in itertools.product((32, 64), cases()):
            source.write_text(".text\n" + line + "\n")
            gas = subprocess.run([args.gas, f"--{mode}", str(source), "-o", str(p / "gas.o")], capture_output=True)
            ours = subprocess.run([compiler, f"-m{mode}", "-c", str(source), "-o", str(p / "ours.o")], capture_output=True)
            report["decisions"] += 1
            if (gas.returncode == 0) != expected or (ours.returncode == 0) != expected:
                report["mismatches"].append({"mode": mode, "assembly": line, "expected": expected, "gas_exit": gas.returncode, "compiler_exit": ours.returncode})
            elif expected:
                for tool in ("gas", "ours"):
                    subprocess.run([args.objcopy, "-O", "binary", "--only-section=.text", str(p / f"{tool}.o"), str(p / f"{tool}.text")], check=True, capture_output=True)
                if (p / "gas.text").read_bytes() == (p / "ours.text").read_bytes():
                    report["byte_matches"] += 1
                else:
                    report["mismatches"].append({"mode": mode, "assembly": line, "error": "different instruction bytes"})
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(f"{report['decisions']} decisions, {report['byte_matches']} exact byte matches, {len(report['mismatches'])} mismatches")
    return bool(report["mismatches"])


if __name__ == "__main__":
    raise SystemExit(main())

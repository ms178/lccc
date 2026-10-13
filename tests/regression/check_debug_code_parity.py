#!/usr/bin/env python3
"""Debug attribution must not change emitted .text (all benchmark programs)."""
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    compiler = os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    failures = []
    sources = sorted((ROOT / "tests/benchmark/programs").glob("*.c"))
    assert sources, "empty parity corpus"
    with tempfile.TemporaryDirectory(prefix="lccc-debug-parity-") as tmp:
        work = Path(tmp)
        for source in sources:
            code = []
            for mode, flags in [("plain", []), ("debug", ["-g"])]:
                obj, raw = work / (mode + ".o"), work / (mode + ".text")
                subprocess.run([compiler, "-O2", "-march=x86-64-v3", "-c", *flags,
                                str(source), "-o", str(obj)], check=True)
                subprocess.run(["objcopy", "-O", "binary", "--only-section=.text",
                                str(obj), str(raw)], check=True)
                code.append(raw.read_bytes())
            if not code[0] or code[0] != code[1]:
                failures.append(source.name)
                print(f"FAIL debug .text parity: {source.name}", flush=True)
        # Size policy is driver-owned, not a heuristic reconstructed by a
        # peephole. On every program, -Os must already disable retargeting;
        # the explicit bisection switch must therefore leave .text unchanged.
        for source in sources:
            code = []
            for mode, skip in [("size", ""), ("size-no-jump", "jump_thread")]:
                obj, raw = work / (mode + ".o"), work / (mode + ".text")
                env = os.environ.copy()
                env["CCC_PEEPHOLE_SKIP"] = skip
                subprocess.run([compiler, "-Os", "-march=x86-64-v3", "-c",
                                str(source), "-o", str(obj)], env=env, check=True)
                subprocess.run(["objcopy", "-O", "binary", "--only-section=.text",
                                str(obj), str(raw)], check=True)
                code.append(raw.read_bytes())
            if not code[0] or code[0] != code[1]:
                failures.append("size-policy:" + source.name)
                print(f"FAIL size threading policy: {source.name}", flush=True)
    assert not failures, f"debug changes machine code: {failures}"
    print(f"PASS debug .text parity and size threading policy: {len(sources)} programs")


if __name__ == "__main__":
    main()

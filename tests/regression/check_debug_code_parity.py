#!/usr/bin/env python3
"""Debug attribution must not change emitted .text (all benchmark programs)."""
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def disassembly_window(obj: Path, start: int, end: int) -> str:
    """Return objdump's .text instructions spanning a byte-difference window."""
    try:
        proc = subprocess.run(
            ["objdump", "-d", "-j", ".text", str(obj)],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        return f"objdump unavailable: {exc}"
    if proc.returncode != 0:
        return f"objdump failed ({proc.returncode})"
    lines = []
    for line in proc.stdout.splitlines():
        match = re.match(r"^\s*([0-9a-fA-F]+):", line)
        if match and start <= int(match.group(1), 16) < end:
            lines.append(line.strip())
    return " | ".join(lines) or "no instructions in byte window"


def report_mismatch(
    title: str,
    source: Path,
    before: bytes,
    after: bytes,
    before_obj: Path,
    after_obj: Path,
) -> None:
    first = next((i for i, pair in enumerate(zip(before, after)) if pair[0] != pair[1]),
                 min(len(before), len(after)))
    start = max(0, first - 12)
    end = min(max(len(before), len(after)), first + 24)
    message = (
        f"{source.name}: before={len(before)} bytes, after={len(after)} bytes, "
        f"first differing .text byte={first}; bytes[{start}:{end}] "
        f"before={before[start:end].hex()} after={after[start:end].hex()}; "
        f"before-insns={disassembly_window(before_obj, start, end)}; "
        f"after-insns={disassembly_window(after_obj, start, end)}"
    )
    # The grouped CI step cannot expose runner logs in every environment;
    # publish the diagnostic as an Actions annotation as well as stdout.
    escaped = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::error title={title}::{escaped}", flush=True)
    print(f"FAIL {title}: {message}", flush=True)


def main():
    compiler = os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    failures = []
    sources = sorted((ROOT / "tests/benchmark/programs").glob("*.c"))
    assert sources, "empty parity corpus"
    with tempfile.TemporaryDirectory(prefix="lccc-debug-parity-") as tmp:
        work = Path(tmp)
        for source in sources:
            code = []
            objects = []
            for mode, flags in [("plain", []), ("debug", ["-g"])]:
                obj, raw = work / (mode + ".o"), work / (mode + ".text")
                subprocess.run([compiler, "-O2", "-march=x86-64-v3", "-c", *flags,
                                str(source), "-o", str(obj)], check=True)
                subprocess.run(["objcopy", "-O", "binary", "--only-section=.text",
                                str(obj), str(raw)], check=True)
                objects.append(obj)
                code.append(raw.read_bytes())
            if not code[0] or code[0] != code[1]:
                failures.append(source.name)
                report_mismatch(
                    "debug .text parity", source, code[0], code[1], objects[0], objects[1]
                )
        # Size policy is driver-owned, not a heuristic reconstructed by a
        # peephole. On every program, -Os must already disable retargeting;
        # the explicit bisection switch must therefore leave .text unchanged.
        for source in sources:
            code = []
            objects = []
            for mode, skip in [("size", ""), ("size-no-jump", "jump_thread")]:
                obj, raw = work / (mode + ".o"), work / (mode + ".text")
                env = os.environ.copy()
                env["CCC_PEEPHOLE_SKIP"] = skip
                subprocess.run([compiler, "-Os", "-march=x86-64-v3", "-c",
                                str(source), "-o", str(obj)], env=env, check=True)
                subprocess.run(["objcopy", "-O", "binary", "--only-section=.text",
                                str(obj), str(raw)], check=True)
                objects.append(obj)
                code.append(raw.read_bytes())
            if not code[0] or code[0] != code[1]:
                failures.append("size-policy:" + source.name)
                report_mismatch(
                    "size threading policy", source, code[0], code[1], objects[0], objects[1]
                )
    assert not failures, f"debug changes machine code: {failures}"
    print(f"PASS debug .text parity and size threading policy: {len(sources)} programs")


if __name__ == "__main__":
    main()

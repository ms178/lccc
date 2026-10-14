#!/usr/bin/env python3
"""Debug attribution must not change emitted .text (all benchmark programs)."""
import difflib
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


def instruction_diff(before_obj: Path, after_obj: Path) -> str:
    """Locate changed instructions, ignoring addresses of relative branches."""
    def instructions(obj: Path):
        try:
            proc = subprocess.run(
                ["objdump", "-d", "-j", ".text", str(obj)],
                check=False,
                capture_output=True,
                text=True,
            )
        except OSError as exc:
            return None, f"objdump unavailable: {exc}"
        if proc.returncode != 0:
            return None, f"objdump failed ({proc.returncode})"
        result = []
        for line in proc.stdout.splitlines():
            fields = line.split("\t")
            if len(fields) < 3:
                continue
            try:
                address = int(fields[0].strip().rstrip(":"), 16)
            except ValueError:
                continue
            encoded = fields[1].split()
            if not encoded or any(not re.fullmatch(r"[0-9a-fA-F]{2}", byte) for byte in encoded):
                continue
            assembly = fields[2].strip()
            if not assembly:
                continue
            branch = re.match(r"^(\S+)\s+([0-9a-fA-F]+)(?:\s+<[^>]+>)?$", assembly)
            normalized = assembly
            if branch and (
                branch.group(1).startswith(("j", "loop"))
                or branch.group(1) in {"call", "callq"}
            ):
                normalized = branch.group(1) + " <relative-target>"
            result.append((address, assembly, normalized))
        return result, None

    before, before_error = instructions(before_obj)
    after, after_error = instructions(after_obj)
    if before_error or after_error:
        return "; ".join(error for error in (before_error, after_error) if error)
    before_norm = [item[2] for item in before]
    after_norm = [item[2] for item in after]
    opcodes = [
        opcode
        for opcode in difflib.SequenceMatcher(
            a=before_norm, b=after_norm, autojunk=False
        ).get_opcodes()
        if opcode[0] != "equal"
    ]
    if not opcodes:
        return "normalized instruction sequences match"

    def render(items, start, end):
        if start == end:
            return "(none)"
        return " | ".join(f"{addr:x}:{asm}" for addr, asm, _ in items[start:end])

    hunks = []
    for tag, i1, i2, j1, j2 in opcodes[:4]:
        before_start, before_end = max(0, i1 - 2), min(len(before), i2 + 3)
        after_start, after_end = max(0, j1 - 2), min(len(after), j2 + 3)
        hunks.append(
            f"{tag}; before={render(before, before_start, before_end)}; "
            f"after={render(after, after_start, after_end)}"
        )
    if len(opcodes) > len(hunks):
        hunks.append(f"and {len(opcodes) - len(hunks)} more instruction hunks")
    return " || ".join(hunks)


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
        f"after-insns={disassembly_window(after_obj, start, end)}; "
        f"instruction-diff={instruction_diff(before_obj, after_obj)}"
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

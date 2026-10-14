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


def assembly_instructions(path: Path):
    """Read machine-instruction text from an assembly or peephole snapshot."""
    try:
        text = path.read_text(errors="replace")
    except OSError as exc:
        return None, f"cannot read {path.name}: {exc}"
    result = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith((".", "#")) or line.endswith(":"):
            continue
        line = line.split("#", 1)[0].rstrip()
        if line:
            result.append(re.sub(r"\s+", " ", line))
    return result, None


def instruction_text_diff(before, after) -> str:
    opcodes = [
        opcode
        for opcode in difflib.SequenceMatcher(a=before, b=after, autojunk=False).get_opcodes()
        if opcode[0] != "equal"
    ]
    if not opcodes:
        return "instruction text matches"
    tag, i1, i2, j1, j2 = opcodes[0]

    def render(lines, start, end):
        if start == end:
            return "(none)"
        return " | ".join(lines[start:end])

    return (
        f"{tag}; before={render(before, max(0, i1 - 2), min(len(before), i2 + 3))}; "
        f"after={render(after, max(0, j1 - 2), min(len(after), j2 + 3))}"
    )


def peephole_trace_diff(compiler: str, source: Path, work: Path) -> str:
    """Rerun only a failing source with pre-peephole and per-pass dumps."""
    results = {}
    for mode, flags in [("plain", []), ("debug", ["-g"])]:
        dump = work / f"{source.stem}.{mode}.pre-peephole.s"
        trace_dir = work / f"{source.stem}.{mode}.peephole-trace"
        obj = work / f"{source.stem}.{mode}.trace.o"
        env = os.environ.copy()
        env["CCC_DUMP_ASM"] = str(dump)
        env["CCC_PEEPHOLE_TRACE"] = str(trace_dir)
        command = [compiler, "-O2", "-march=x86-64-v3", "-c", *flags,
                   str(source), "-o", str(obj)]
        try:
            proc = subprocess.run(command, check=False, capture_output=True, text=True, env=env)
        except OSError as exc:
            return f"trace rerun could not start ({mode}): {exc}"
        if proc.returncode != 0:
            detail = proc.stderr.strip().splitlines()[-1] if proc.stderr.strip() else "no stderr"
            return f"trace rerun failed ({mode}, exit {proc.returncode}): {detail}"
        instructions, error = assembly_instructions(dump)
        if error:
            return f"trace rerun ({mode}): {error}"
        entries = []
        for path in sorted(trace_dir.glob("*.s")):
            match = re.match(r"\d+-p(\d+)-(.+)\.s$", path.name)
            if not match:
                continue
            snapshot, error = assembly_instructions(path)
            if error:
                return f"trace rerun ({mode}): {error}"
            entries.append(((int(match.group(1)), match.group(2)), snapshot))
        results[mode] = (instructions, entries)

    plain_state, plain_entries = results["plain"]
    debug_state, debug_entries = results["debug"]
    if plain_state != debug_state:
        return (
            "pre-peephole codegen differs: "
            + instruction_text_diff(plain_state, debug_state)
        )
    plain_keys = [key for key, _ in plain_entries]
    debug_keys = [key for key, _ in debug_entries]
    if not plain_keys and not debug_keys:
        return "pre-peephole code matches; no phase-1 peephole snapshots"
    matcher = difflib.SequenceMatcher(a=plain_keys, b=debug_keys, autojunk=False)
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            pairs = [(plain_entries[i], debug_entries[j]) for i, j in zip(range(i1, i2), range(j1, j2))]
        elif tag == "delete":
            pairs = [(plain_entries[i], None) for i in range(i1, i2)]
        elif tag == "insert":
            pairs = [(None, debug_entries[j]) for j in range(j1, j2)]
        else:
            count = max(i2 - i1, j2 - j1)
            pairs = [
                (
                    plain_entries[i1 + k] if i1 + k < i2 else None,
                    debug_entries[j1 + k] if j1 + k < j2 else None,
                )
                for k in range(count)
            ]
        for plain_entry, debug_entry in pairs:
            if plain_entry is not None:
                plain_state = plain_entry[1]
            if debug_entry is not None:
                debug_state = debug_entry[1]
            if plain_state != debug_state:
                plain_label = (
                    f"plain p{plain_entry[0][0]} {plain_entry[0][1]}"
                    if plain_entry is not None else "plain unchanged"
                )
                debug_label = (
                    f"debug p{debug_entry[0][0]} {debug_entry[0][1]}"
                    if debug_entry is not None else "debug unchanged"
                )
                return (
                    f"first phase-1 divergence after {plain_label} / {debug_label}: "
                    + instruction_text_diff(plain_state, debug_state)
                )
    return "pre-peephole and phase-1 instruction text match; divergence is in a later stage"


def report_mismatch(
    title: str,
    source: Path,
    before: bytes,
    after: bytes,
    before_obj: Path,
    after_obj: Path,
    trace_info: str = "",
) -> None:
    first = next((i for i, pair in enumerate(zip(before, after)) if pair[0] != pair[1]),
                 min(len(before), len(after)))
    start = max(0, first - 12)
    end = min(max(len(before), len(after)), first + 24)
    trace_suffix = f"; peephole-trace={trace_info}" if trace_info else ""
    message = (
        f"{source.name}: before={len(before)} bytes, after={len(after)} bytes, "
        f"first differing .text byte={first}; bytes[{start}:{end}] "
        f"before={before[start:end].hex()} after={after[start:end].hex()}; "
        f"before-insns={disassembly_window(before_obj, start, end)}; "
        f"after-insns={disassembly_window(after_obj, start, end)}; "
        f"instruction-diff={instruction_diff(before_obj, after_obj)}{trace_suffix}"
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
                trace_info = peephole_trace_diff(compiler, source, work)
                report_mismatch(
                    "debug .text parity",
                    source,
                    code[0],
                    code[1],
                    objects[0],
                    objects[1],
                    trace_info,
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

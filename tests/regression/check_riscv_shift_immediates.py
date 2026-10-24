#!/usr/bin/env python3
"""RV64 shift-immediate acceptance/byte oracle (Clang 23.1.3, no emulation).

Example: python3 tests/regression/check_riscv_shift_immediates.py \
  --compiler target/fastbuild/lccc-riscv --clang clang-23 --out /tmp/rv-shifts.json
Only legal canonical instructions require identical .text. Immediate aliases
are covered by the Rust encoder tests; they are not assumed to be LLVM syntax.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile


def text_section(path: Path) -> bytes:
    data = path.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected little-endian ELF64")
    offset = struct.unpack_from("<Q", data, 40)[0]
    stride, count, names_index = struct.unpack_from("<HHH", data, 58)
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, offset + i * stride)
                for i in range(count)]
    names = sections[names_index]
    strings = data[names[4]:names[4] + names[5]]
    for section in sections:
        start = section[0]
        if strings[start:strings.index(b"\0", start)] == b".text":
            return data[section[4]:section[4] + section[5]]
    raise ValueError("missing .text section")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--before")
    parser.add_argument("--clang", default="clang-23")
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    tools = {"candidate": args.compiler, "clang": args.clang}
    if args.before:
        tools["before"] = args.before
    tools = {key: str(Path(shutil.which(value) or value).absolute())
             for key, value in tools.items()}
    # Do not resolve compiler symlinks: argv[0] selects the backend.
    version = subprocess.check_output([tools["clang"], "--version"], text=True)
    if not re.search(r"\bclang version 23\.1\.3(?:\s|$)", version.splitlines()[0]):
        parser.error("this oracle pins Clang 23.1.3")
    report = {"scope": "RV64 assembler acceptance and exact bytes, not runtime",
              "tools": tools, "clang_version": version.splitlines()[0],
              "binary_sha256": {key: hashlib.sha256(Path(value).read_bytes()).hexdigest()
                                for key, value in tools.items()},
              "valid_byte_matches": 0, "invalid_rejections": 0,
              "before_invalid_acceptances": 0, "failures": [], "cases": []}
    with tempfile.TemporaryDirectory(prefix="rv64-shift-range-") as directory:
        root = Path(directory)
        source = root / "case.s"
        for mnemonic in ("slli", "srli", "srai", "rori", "slliw", "srliw", "sraiw", "roriw"):
            maximum = 31 if mnemonic.endswith("w") else 63
            counts = list(range(maximum + 1)) + [-(1 << 63), -(1 << 32), -65, -1,
                                               maximum + 1, maximum + 2, 1 << 32, (1 << 63) - 1]
            for count in counts:
                valid = 0 <= count <= maximum
                line = f"{mnemonic} a0,a1,{count}"
                source.write_text(f".option norvc\n.text\n{line}\n")
                results = {}
                texts = {}
                for key, tool in tools.items():
                    output = root / f"{key}.o"
                    command = [tool]
                    if key == "clang":
                        command += ["--target=riscv64", "-march=rv64gc_zbb"]
                    command += ["-c", str(source), "-o", str(output)]
                    run = subprocess.run(command, capture_output=True, timeout=30)
                    results[key] = run.returncode
                    if run.returncode == 0 and valid:
                        texts[key] = text_section(output).hex()
                passed = all((results[key] == 0) == valid for key in ("candidate", "clang"))
                if valid:
                    passed = passed and texts.get("candidate") == texts.get("clang")
                    if "before" in tools:
                        passed = passed and results["before"] == 0 and texts.get("before") == texts.get("clang")
                    report["valid_byte_matches"] += int(passed)
                else:
                    report["invalid_rejections"] += int(passed)
                    report["before_invalid_acceptances"] += int(results.get("before") == 0)
                row = {"assembly": line, "valid": valid, "exit_codes": results}
                if texts:
                    row["text_hex"] = texts
                report["cases"].append(row)
                if not passed:
                    report["failures"].append(row)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n")
    print(f"{report['valid_byte_matches']} legal byte matches; "
          f"{report['invalid_rejections']} invalid rejections; "
          f"{len(report['failures'])} failures")
    raise SystemExit(bool(report["failures"]))


if __name__ == "__main__":
    main()

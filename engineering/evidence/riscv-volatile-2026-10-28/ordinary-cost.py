#!/usr/bin/env python3
"""Compare unrelated RV64 functions with and without one volatile function.

The output is static object size, not a runtime benchmark. The input is a
synthetic stress case, not evidence of a broad or real-workload performance
change. No package installation or extraction is performed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def body_lines(path):
    text = path.read_text()
    bodies = {}
    for index in range(80):
        match = re.search(rf"(?ms)^f{index}:\s*$(.*?)^\s*\.size\s+f{index}\b", text)
        if not match:
            raise ValueError(f"missing function f{index}")
        bodies[f"f{index}"] = [line.strip() for line in match.group(1).splitlines()
                                if re.match(r"^\s+[a-z][a-z0-9.]*\b", line)]
    return bodies


def symbol_sizes(path):
    text = subprocess.check_output(["readelf", "-Ws", str(path)], text=True, timeout=30)
    sizes = {match.group(2): int(match.group(1)) for match in re.finditer(
        r"^\s*\d+:\s+[0-9a-f]+\s+(\d+)\s+FUNC\s+\S+\s+\S+\s+\S+\s+(f\d+)$",
        text, re.MULTILINE)}
    if len(sizes) != 80:
        raise ValueError(f"expected 80 function symbols, found {len(sizes)}")
    return sizes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True, help="pre-change lccc-riscv")
    parser.add_argument("--after", type=Path, required=True, help="post-change lccc-riscv")
    parser.add_argument("--output", type=Path, required=True, help="new private output directory")
    args = parser.parse_args()
    before, after = (path.resolve(strict=True) for path in (args.before, args.after))
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    parts = []
    for index in range(80):
        parts.append(f"""int f{index}(int x, int y) {{
    int a[4]; a[0] = x + {index}; a[1] = y ^ {index+1};
    int *p = &a[(x & 1)];
    if (x & 2) {{ *p += y; }} else {{ a[0] -= y; }}
    if (y & 4) return a[0] + *p;
    return a[1] ^ *p;
}}
""")
    plain = "\n".join(parts)
    (output / "plain.c").write_text(plain)
    (output / "mixed.c").write_text(plain + "\nvoid trigger(void) { volatile int v = 2; v; v; }\n")
    result = {"compiler_sha256": {"before": digest(before), "after": digest(after)},
              "source_sha256": {name: digest(output / name) for name in ("plain.c", "mixed.c")},
              "measurement": "sum of function symbol sizes in relocatable objects; no linker relaxation"}
    for opt in ("0", "2"):
        asm = {}
        for version, compiler, source in (("before_plain", before, "plain"),
                                           ("after_plain", after, "plain"),
                                           ("after_mixed", after, "mixed")):
            asm[version] = output / f"{version}-O{opt}.s"
            subprocess.run([str(compiler), f"-O{opt}", "-S", str(output / f"{source}.c"),
                            "-o", str(asm[version])], check=True, timeout=120)
        parsed = {name: body_lines(path) for name, path in asm.items()}
        if parsed["before_plain"] != parsed["after_plain"]:
            raise ValueError(f"-O{opt} unmarked functions changed between compiler binaries")
        sizes = {}
        for source in ("plain", "mixed"):
            obj = output / f"{source}-O{opt}.o"
            subprocess.run([str(after), f"-O{opt}", "-c", str(output / f"{source}.c"),
                            "-o", str(obj)], check=True, timeout=120)
            sizes[source] = symbol_sizes(obj)
        normal, mixed = parsed["after_plain"], parsed["after_mixed"]
        result[f"O{opt}"] = {
            "function_count": len(normal),
            "functions_with_different_assembly_lines": sum(normal[k] != mixed[k] for k in normal),
            "assembly_instruction_lines": {"plain": sum(map(len, normal.values())),
                                           "mixed": sum(map(len, mixed.values()))},
            "object_symbol_bytes": {name: sum(table.values()) for name, table in sizes.items()},
            "functions_with_different_object_sizes": sum(sizes["plain"][k] != sizes["mixed"][k]
                                                          for k in sizes["plain"]),
            "assembly_sha256": {name: digest(path) for name, path in asm.items()},
        }
    (output / "report.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()

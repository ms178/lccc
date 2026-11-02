"""Run from repository root: real RV64 LLVM linked-binary boundary oracle.

Requires network. No assembler-only success is treated as linker evidence.
Force both instructions to four bytes: a compressed JALR is not a CALL_PLT
pair, and would make a manually attached eight-byte relocation corrupt its
neighbor. Validate the actual opcode words, not only the compiler exit code.
"""
import gzip
import json
from pathlib import Path
import sys

sys.path.insert(0, "scripts")
import godbolt as gb

out = Path("engineering/evidence/2026-10-14-auipc-range/oracle")
out.mkdir(parents=True, exist_ok=True)
rows = []
for kind in ("CALL_PLT", "PCREL_HI20"):
    for delta in (-2147485697, -2147485696, 2147481599, 2147481600, 2147483647, 0):
        asm = (".option push\n.option norelax\n.option norvc\n"
               "1: auipc ra, 0\njalr ra, ra, 0\n"
               f".reloc 1b, R_RISCV_{kind}, 1b{delta:+d}\n.option pop\n")
        source = "__attribute__((naked)) void call_probe(void) { __asm__(" + json.dumps(asm) + "); }\n"
        body = {"source": source, "lang": "c", "options": {
            "userArguments": "-O1 -nostdlib -fuse-ld=lld -Wl,-v,--no-relax,-e,call_probe,--build-id=none",
            "filters": {"binary": True, "execute": False, "libraryCode": False,
                        "directives": False, "labels": False},
            "compilerOptions": {"executorRequest": False}, "tools": [], "libraries": []}}
        result = gb._request_json(gb.API + "/compiler/rv64-cclang2310/compile", body=body, timeout=90)
        stem = f"{kind}-{delta}"
        (out / (stem + ".json.gz")).write_bytes(gzip.compress(
            json.dumps({"request": body, "result": result}, indent=2).encode(), mtime=0))
        accepted = -2147485696 <= delta <= 2147481599
        if (result.get("code") == 0) != accepted:
            raise RuntimeError(f"wrong acceptance: {kind} {delta}: {result}")
        if accepted:
            assembly = result["asm"]
            start = next(i for i, row in enumerate(assembly) if row.get("text") == "call_probe:")
            words = [row["opcodes"] for row in assembly[start + 1:] if row.get("opcodes")][:2]
            if len(words) != 2 or any(len(w) != 1 or len(w[0]) != 8 for w in words):
                raise RuntimeError(f"not a fixed-width instruction pair: {words}")
            high = ((delta + 2048) // 4096) & 0xfffff
            expected = [high << 12 | 0x97, ((delta & 4095) << 20 if kind == "CALL_PLT" else 0) | 0x80e7]
            if [int(w[0], 16) for w in words] != expected:
                raise RuntimeError(f"wrong exact encodings: {words}, expected {expected}")
        else:
            diagnostics = "\n".join(x.get("text", "") for x in result.get("stderr", []))
            if f"relocation R_RISCV_{kind} out of range" not in diagnostics:
                raise RuntimeError(f"unrelated rejection: {diagnostics}")
        rows.append({"kind": kind, "delta": delta, "code": result["code"], "passed": True,
                     "artifact": stem + ".json.gz", "stdout": result.get("stdout"),
                     "stderr": result.get("stderr"), "asm": result.get("asm")})
        print(f"PASS {kind} {delta}", flush=True)
(out / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")
print(f"{len(rows)} LLVM linked-binary checks passed")

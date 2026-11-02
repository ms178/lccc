#!/usr/bin/env python3
"""Replay the 2026-10-14 audit contracts; this is NOT a passing CI gate yet.

Default prints JSON observations. --check exits 1 when a known contract still
fails; --selftest tests the probe runner without a compiler. Compiler probes
require an already built fastbuild tree and Clang. Nothing is built/downloaded
implicitly. Tool absence, timeouts and unexpected exceptions are infrastructure
errors, not evidence that a compiler rejected an input. See backlog A14.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import audit_census  # noqa: E402


def run(argv: list[str], *, timeout: int = 30) -> dict:
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return {"argv": argv, "returncode": p.returncode,
                "stdout": p.stdout, "stderr": p.stderr,
                "infrastructure_error": p.returncode < 0}
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"argv": argv, "returncode": None, "stdout": "",
                "stderr": str(exc), "infrastructure_error": True}


def verdict(record: dict, *, accept: bool) -> bool:
    return (not record["infrastructure_error"]
            and record["returncode"] is not None
            and ((record["returncode"] == 0) if accept
                 else record["returncode"] > 0
                 and not re.search(r"panicked at|internal (?:compiler )?error|stack overflow|segmentation fault",
                                   record.get("stderr", ""), re.IGNORECASE)))


def census_probes() -> list[dict]:
    results = []
    source = '#[cfg(test)]\nmod tests {}\nfn production() { panic!("x"); }\n'
    strip_code = getattr(audit_census, "strip_noncode", None) or audit_census.strip_rust
    strip_tests = getattr(audit_census, "strip_tests", None) or audit_census.without_tests
    actual = strip_tests(strip_code(source))
    results.append({"id": "A14-01-production-after-tests", "source": source,
                    "observed": actual, "contract_ok": "production" in actual})
    source = 'fn f() { let s=r#"a" panic!() "#; }'
    actual = strip_code(source)
    results.append({"id": "A14-01-raw-string", "source": source,
                    "observed": actual, "contract_ok": "panic!" not in actual})
    source = 'fn f() { let s="a\nb\nc";\n}'
    actual = strip_code(source)
    results.append({"id": "A14-01-line-preservation", "source": source,
                    "observed": actual,
                    "contract_ok": source.count("\n") == actual.count("\n")})
    source = 'fn f() { let s="a\npanic!()\nc"; }'
    actual = strip_code(source)
    results.append({"id": "A14-01-multiline-string-body", "source": source,
                    "observed": actual, "contract_ok": "panic!" not in actual})
    return results


def compiler_probes(lccc_dir: Path, clang: str) -> list[dict]:
    results = []
    local = str(lccc_dir / "lccc")
    with tempfile.TemporaryDirectory(prefix="lccc-audit-contracts-") as tmp:
        work = Path(tmp)
        vector = work / "vector.c"
        vector.write_text(
            'typedef long v4l __attribute__((ext_vector_type(4)));\n'
            'int main(void) {\n'
            ' return !(sizeof(v4l) == 16 &&\n'
            ' sizeof((long __attribute__((ext_vector_type(4)))){0}) == 16);\n'
            '}\n')
        for opt in ("-O0", "-O2"):
            for name, cc in (("lccc", local), ("clang", clang)):
                exe = work / f"vector-{name}{opt}"
                build = run([cc, "-m32", opt, str(vector), "-o", str(exe)])
                execution = run([str(exe)]) if verdict(build, accept=True) else None
                results.append({"id": "A14-13-vector-size", "compiler": name,
                                "opt": opt, "source": vector.read_text(),
                                "build": build, "execution": execution,
                                "contract_ok": bool(execution and verdict(execution, accept=True))})
        cases = [
            ("A14-14-adr-overflow", "arm", "aarch64-linux-gnu",
             '.text\nadr x0, #1048576\n', False),
            ("A14-14-adr-underflow", "arm", "aarch64-linux-gnu",
             '.text\nadr x0, #-1048577\n', False),
            ("A14-14-adr-max-control", "arm", "aarch64-linux-gnu",
             '.text\nadr x0, #1048575\n', True),
            ("A14-14-adr-min-control", "arm", "aarch64-linux-gnu",
             '.text\nadr x0, #-1048576\n', True),
            ("A14-15-unresolved-product", "riscv", "riscv64-linux-gnu",
             '.data\n.word missing*2\n', False),
            ("A14-15-two-symbol-sum", "riscv", "riscv64-linux-gnu",
             '.data\n.word missing+other\n', False),
            ("A14-15-symbol-difference-control", "riscv", "riscv64-linux-gnu",
             '.data\n.word missing_end-missing_start\n', True),
        ]
        for ident, arch, target, source, accept in cases:
            src = work / (ident + ".s")
            src.write_text(source)
            for name, cc in (("lccc", [str(lccc_dir / ("lccc-" + arch))]),
                             ("clang", [clang, "--target=" + target])):
                obj = work / (ident + name + ".o")
                build = run(cc + ["-c", str(src), "-o", str(obj)])
                record = {"id": ident, "compiler": name, "source": source,
                          "expected_accept": accept, "build": build,
                          "contract_ok": verdict(build, accept=accept)}
                if verdict(build, accept=True) and obj.is_file():
                    record["object_sha256"] = hashlib.sha256(obj.read_bytes()).hexdigest()
                    record["elf"] = run(["readelf", "-r", "-x", ".text", "-x", ".data", str(obj)])
                results.append(record)
        src = work / "flag.c"
        src.write_text('int f(void) { return 42; }\n')
        for name, cc in (("lccc", local), ("clang", clang)):
            build = run([cc, "-fdefinitely-not-a-real-option", "-c", str(src),
                         "-o", str(work / (name + "-flag.o"))])
            results.append({"id": "A14-16-unknown-flag", "compiler": name,
                            "source": src.read_text(), "build": build,
                            "contract_ok": verdict(build, accept=False)})
    return results


def extended_probes(lccc_dir: Path, clang: str, lld: str, strace: str) -> list[dict]:
    """Target flag, malformed relocation and PGO file-lock lifetime contracts."""
    results = []
    with tempfile.TemporaryDirectory(prefix="lccc-audit-extended-") as tmp:
        work = Path(tmp)
        src = work / "popcount.c"
        src.write_text('int pc(unsigned long x) { return __builtin_popcountl(x); }\n')
        for name, cc in (("lccc", [str(lccc_dir / "lccc-arm")]),
                         ("clang", [clang, "--target=aarch64-linux-gnu"])):
            asm = work / (name + ".s")
            build = run(cc + ["-O2", "-mgeneral-regs-only", "-S", str(src), "-o", str(asm)])
            text = asm.read_text() if verdict(build, accept=True) and asm.exists() else ""
            forbidden = bool(re.search(r"\b[vdqsh][0-9]+\b", text))
            results.append({"id": "A14-19-general-regs-only", "compiler": name,
                            "source": src.read_text(), "build": build, "assembly": text,
                            "contract_ok": verdict(build, accept=True) and bool(text) and not forbidden})
        src = work / "reloc.s"
        src.write_text('.data\n.quad missing\n')
        obj = work / "reloc.o"
        build = run([clang, "--target=riscv64-linux-gnu", "-c", str(src), "-o", str(obj)])
        if verdict(build, accept=True):
            data = bytearray(obj.read_bytes())
            if data[:6] != b"\x7fELF\x02\x01" or len(data) < 64:
                raise ValueError("expected little-endian ELF64 fixture")
            shoff = struct.unpack_from("<Q", data, 40)[0]
            entsize, count = struct.unpack_from("<HH", data, 58)
            if entsize != 64 or shoff + entsize * count > len(data):
                raise ValueError("invalid fixture section table")
            patches = []
            for i in range(count):
                header = shoff + i * entsize
                if struct.unpack_from("<I", data, header + 4)[0] == 4:
                    off, size = struct.unpack_from("<QQ", data, header + 24)
                    if size != 24 or off + size > len(data):
                        raise ValueError("expected exactly one ELF64 RELA entry")
                    info = struct.unpack_from("<Q", data, off + 8)[0]
                    struct.pack_into("<Q", data, off + 8, (info & ~0xffffffff) | 255)
                    patches.append(off + 8)
            if len(patches) != 1:
                raise ValueError("expected one relocation section in fixture")
            obj.write_bytes(data)
            for name, cc in (("lccc", [str(lccc_dir / "lccc-riscv"), "-shared", "-nostdlib"]),
                             ("lld", [lld, "-m", "elf64lriscv", "-shared"])):
                out = work / (name + ".so")
                link = run(cc + [str(obj), "-o", str(out)])
                record = {"id": "A14-17-unknown-shared-relocation", "compiler": name,
                          "source": src.read_text(), "fixture_build": build,
                          "mutation": {"r_info_offset": patches[0], "reloc_type": 255},
                          "link": link, "contract_ok": verdict(link, accept=False)}
                if verdict(link, accept=True):
                    record["elf"] = run(["readelf", "-r", "-x", ".data", str(out)])
                results.append(record)
        else:
            results.append({"id": "A14-17-fixture-build", "build": build,
                            "contract_ok": False})
        src = work / "pgo.c"
        src.write_text('int main(void) { return 0; }\n')
        profile = work / "profile"
        profile.mkdir()
        exe = work / "pgo"
        build = run([str(lccc_dir / "lccc"), "-O2", "-fprofile-generate=" + str(profile),
                     str(src), "-o", str(exe)])
        record = {"id": "A14-20-profile-flush-lock", "source": src.read_text(),
                  "build": build, "contract_ok": False}
        if verdict(build, accept=True):
            trace = work / "trace.txt"
            execution = run([strace, "-e", "trace=flock,write,close", "-o", str(trace), str(exe)])
            text = trace.read_text() if trace.exists() else ""
            # The minimal program writes only one profile stream. Check each
            # profile write against the flock state of its own fd, not simply
            # the relative positions of arbitrary close/write syscalls.
            locked = set()
            protected = []
            for line in text.splitlines():
                match = re.match(r"flock\((\d+), (LOCK_EX|LOCK_UN)\)\s*= 0", line)
                if match:
                    if match[2] == "LOCK_EX":
                        locked.add(match[1])
                    else:
                        locked.discard(match[1])
                match = re.match(r'write\((\d+), "lccc-pgo-v1', line)
                if match:
                    protected.append(match[1] in locked)
            record.update(execution=execution, trace=text,
                          contract_ok=verdict(execution, accept=True) and bool(protected) and all(protected))
        results.append(record)
        src = work / "debug.c"
        src.write_text('int f(void) { return 42; }\n')
        obj = work / "debug.o"
        asm = work / "debug.s"
        cc = str(lccc_dir / "lccc")
        assembly_build = run([cc, "-g", "-O0", "-S", str(src), "-o", str(asm)])
        object_build = run([cc, "-g", "-O0", "-c", str(src), "-o", str(obj)])
        sections = run(["readelf", "-SW", str(obj)]) if verdict(object_build, accept=True) else None
        text = asm.read_text() if asm.exists() else ""
        results.append({"id": "A14-21-debug-object", "source": src.read_text(),
                        "assembly_build": assembly_build, "object_build": object_build,
                        "assembly": text, "sections": sections,
                        "contract_ok": bool(sections and verdict(sections, accept=True)
                                            and ".debug_line" in sections["stdout"])})
    return results


class RunnerTests(unittest.TestCase):
    def test_exit_classification(self):
        for rc, infra, accept, expected in [(0, False, True, True),
                                           (1, False, False, True),
                                           (-11, True, False, False),
                                           (None, True, False, False),
                                           (0, False, False, False)]:
            self.assertEqual(verdict({"returncode": rc, "infrastructure_error": infra},
                                     accept=accept), expected)

    def test_caught_compiler_panic_is_not_rejection(self):
        self.assertFalse(verdict({"returncode": 1, "infrastructure_error": False,
                                  "stderr": "internal compiler error: panicked at site"}, accept=False))

    def test_process_success_and_failure(self):
        self.assertTrue(verdict(run([sys.executable, "-c", "pass"]), accept=True))
        self.assertTrue(verdict(run([sys.executable, "-c", "raise SystemExit(2)"]), accept=False))

    def test_absent_tool_is_not_rejection(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = run([str(Path(tmp) / "absent")])
        self.assertTrue(result["infrastructure_error"])
        self.assertFalse(verdict(result, accept=False))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler-probes", action="store_true")
    parser.add_argument("--extended", action="store_true", help="also needs ld.lld, strace")
    parser.add_argument("--lld", default="ld.lld")
    parser.add_argument("--strace", default="strace")
    parser.add_argument("--lccc-dir", type=Path, default=ROOT / "target/fastbuild")
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(RunnerTests)
        return int(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
    records = census_probes()
    if args.compiler_probes:
        records.extend(compiler_probes(args.lccc_dir.resolve(), args.clang))
    if args.extended:
        records.extend(extended_probes(args.lccc_dir.resolve(), args.clang, args.lld, args.strace))
    report = {"schema": "lccc-audit-contracts-v1",
              "note": "Observations, not a CI pass or performance result.",
              "records": records,
              "violations": sum(not r["contract_ok"] for r in records)}
    text = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(text)
    else:
        print(text, end="")
    return int(args.check and report["violations"] > 0)


if __name__ == "__main__":
    raise SystemExit(main())

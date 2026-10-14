#!/usr/bin/env python3
"""PGO buffered-write lock lifetime, deterministic two-writer and error tests."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def run(cmd, env=None, expected=0):
    p = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=60)
    assert p.returncode == expected, (cmd, p.returncode, p.stdout, p.stderr)
    assert not any(marker in p.stderr.lower() for marker in
                   ("panicked at", "ccc: internal error", "internal compiler error")), p.stderr
    return p


def source(count):
    functions = [f"__attribute__((noinline)) int f{i}(int x) {{ return x > {i} ? x + {i} : x - {i}; }}"
                 for i in range(count)]
    calls = "".join(f"s += f{i}(argc);" for i in range(count))
    return "#include <stdio.h>\n" + "\n".join(functions) + f"\nint main(int argc, char **argv) {{ int s=0; (void)argv; {calls} printf(\"%d\\n\",s); return 23; }}\n"


def counters(text):
    """Exact counter sum keyed by record identity, function and edge/site."""
    result = {}
    header = function = None
    for line in text.splitlines():
        fields = line.split()
        assert fields, f"blank profile record: {line!r}"
        if fields[0] == "lccc-pgo-v1":
            assert len(fields) == 5, line
            header = tuple(fields[1:])
            function = None
        elif fields[0] == "func":
            assert len(fields) == 2 and header, line
            function = fields[1]
        elif fields[0] in ("e", "f"):
            assert header and function, line
            assert len(fields) == (2 if fields[0] == "f" else 4), line
            value = int(fields[-1])
            assert 0 <= value <= 0xffffffffffffffff, line
            if fields[0] == "e":
                assert all(0 <= int(x) <= 0xffffffff for x in fields[1:3]), line
            key = (header, function, tuple(fields[:-1]))
            result[key] = result.get(key, 0) + value
        else:
            raise AssertionError(f"unexpected profile line: {line}")
    assert result
    return result


def run_checks(args, checks):
    with tempfile.TemporaryDirectory(prefix="lccc-pgo-io-") as tmp:
        root = Path(tmp)
        so = root / "interpose.so"
        run([args.cc, "-shared", "-fPIC", "-O1", "-Wall", "-Wextra", "-Werror",
             str(ROOT / "tests/regression/fixtures/pgo_io_interpose.c"), "-ldl", "-o", str(so)])
        base_env = {k: v for k, v in os.environ.items() if not k.startswith(("LCCC_PGO_", "PGO_IO_"))
                    and k not in ("LCCC_PROFILE_FILE", "LLVM_PROFILE_FILE", "LD_PRELOAD")}
        for opt in ("-O0", "-O2", "-O3"):
            for count in (1, 96):
                src = root / f"unit-{opt}-{count}.c"
                src.write_text(source(count))
                exe = src.with_suffix(".exe")
                golden = src.with_suffix(".profraw")
                run([args.lccc, opt, "-fprofile-generate=" + str(golden), str(src), "-o", str(exe)])
                expected = run([str(exe)], env=base_env, expected=23)
                assert not expected.stderr, expected.stderr
                expected_sum = count - count * (count - 1) // 2
                assert expected.stdout == f"{expected_sum}\n", expected.stdout
                raw = golden.read_bytes()
                if count == 96:
                    assert len(raw) > 8192, len(raw)
                baseline_counts = counters(raw.decode())
                entries = {key[1]: value for key, value in baseline_counts.items() if key[2] == ("f",)}
                assert len(entries) == count + 1 and all(value == 1 for value in entries.values()), entries
                checks.append(f"{opt}/{count}: single-process profile")
                # Faults must diagnose, preserve application status/output, and
                # never write profile data after failure to acquire the lock.
                for fault, diagnostic in (("fopen", "cannot open profile"),
                        ("fileno", "cannot lock profile"), ("flock", "cannot lock profile"),
                        ("eintr", "cannot lock profile"), ("ferror", "profile write failed"),
                        ("ferror-zero", "profile write failed"), ("ferror-stale", "profile write failed"),
                        ("fclose", "profile close/flush failed")):
                    profile = root / f"fault-{opt}-{count}-{fault}.profraw"
                    env = dict(base_env, LD_PRELOAD=str(so), LCCC_PROFILE_FILE=str(profile), PGO_IO_FAULT=fault)
                    p = run([str(exe)], env=env, expected=23)
                    assert p.stdout == expected.stdout and diagnostic in p.stderr, (fault, p.stdout, p.stderr)
                    if fault.startswith("ferror"):
                        assert "profile write failed:" not in p.stderr, p.stderr
                    if fault in ("fopen", "fileno", "flock", "eintr"):
                        assert not profile.exists() or profile.stat().st_size == 0, fault
                    checks.append(f"{opt}/{count}: injected {fault}")
                p = run([str(exe)], env=dict(base_env, LCCC_PROFILE_FILE="/dev/full"), expected=23)
                assert p.stdout == expected.stdout and "lccc: PGO: profile" in p.stderr, p.stderr
                checks.append(f"{opt}/{count}: ENOSPC")
                # Force the old unlock-before-flush bug, without relying on a
                # lucky scheduling race. B cannot start until A has the lock.
                shared = root / f"shared-{opt}-{count}.profraw"
                sync = root / f"sync-{opt}-{count}"
                sync.mkdir()
                env = dict(base_env, LD_PRELOAD=str(so), LCCC_PROFILE_FILE=str(shared), PGO_IO_SYNC=str(sync))
                processes = []
                try:
                    a = subprocess.Popen([str(exe)], env=dict(env, PGO_IO_ROLE="A"), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                    processes.append(a)
                    deadline = time.monotonic() + 15
                    while not (sync / "A-held").exists():
                        assert a.poll() is None, "A exited before acquiring profile lock"
                        assert time.monotonic() < deadline, "A lock acquisition timed out"
                        time.sleep(.001)
                    b = subprocess.Popen([str(exe)], env=dict(env, PGO_IO_ROLE="B"), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                    processes.append(b)
                    for p in processes:
                        stdout, stderr = p.communicate(timeout=20)
                        assert p.returncode == 23 and stdout.decode() == expected.stdout and not stderr, (p.returncode, stdout, stderr)
                finally:
                    for p in processes:
                        if p.poll() is None:
                            p.kill()
                        p.wait()
                assert shared.read_bytes() == raw * 2, "interleaved profile records"
                assert not (sync / "unlocked-close").exists(), "final flush outside lock"
                assert counters(shared.read_text()) == {key: value * 2 for key, value in baseline_counts.items()}
                checks.append(f"{opt}/{count}: deterministic two writers, exact counts")
                # Exercise the production reader and optimization pipeline,
                # not just the independent raw-byte/counter parser above.
                ingested = root / "ingested.txt"
                use = root / "use.exe"
                p = run([args.lccc, opt, "-fprofile-use=" + str(shared), str(src), "-o", str(use)],
                        env=dict(base_env, LCCC_PGO_DUMP_TEXT=str(ingested)))
                assert f"loaded {count + 1} functions" in p.stderr, p.stderr
                assert counters(ingested.read_text()) == {key: value * 2 for key, value in baseline_counts.items()}
                assert run([str(use)], env=base_env, expected=23).stdout == expected.stdout
                checks.append(f"{opt}/{count}: production ingestion and use")

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lccc", required=True)
    ap.add_argument("--cc", default="gcc", help="host compiler for LD_PRELOAD oracle only")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    if not __debug__:
        ap.error("optimized Python disables test assertions")
    checks = []
    error = None
    try:
        run_checks(args, checks)
    except (AssertionError, OSError, subprocess.TimeoutExpired, ValueError, IndexError) as failure:
        error = str(failure)
    summary = dict(passed=len(checks), failed=int(error is not None), skipped=0, checks=checks, error=error)
    if args.json:
        args.json.parent.mkdir(parents=True, exist_ok=True)
        args.json.write_text(json.dumps(summary, indent=2) + "\n")
    if error is not None:
        print("FAIL", error)
    print(f"PGO I/O: {len(checks)} PASS / {summary['failed']} FAIL / 0 SKIP")
    return int(error is not None)


if __name__ == "__main__":
    raise SystemExit(main())

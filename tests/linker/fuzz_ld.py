#!/usr/bin/env python3
"""Structure-aware mutation fuzzer for the LCCC linker front-end.

Goal: find inputs where lccc-ld *panics* (Rust unwind / abort / signal)
instead of producing a clean diagnostic.  A linker may reject malformed
input, but it must never panic: panics leak internal state, produce
useless messages, and in a `catch_unwind`-less driver yield exit code 101
which build systems misreport.

Oracle: bfd/mold/wild all exit with a normal error status (1) on the same
inputs.  Any lccc exit code >= 101 or killed-by-signal is a defect.
"""
import os, random, subprocess, sys, tempfile, shutil

_HERE = os.path.dirname(os.path.abspath(__file__))
_REPO = os.path.dirname(os.path.dirname(_HERE))
LD = os.environ.get("LCCC_LD",
                    os.path.join(_REPO, "target", "fastbuild", "lccc-ld"))
SEED_DIR = os.environ.get("SEED_DIR", os.path.join(_HERE, "fuzz_seeds"))
N = int(os.environ.get("FUZZ_N", "400"))

def run(cmd, timeout=20):
    try:
        p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=timeout)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return "TIMEOUT", b""

def classify(rc, out):
    """Return None if acceptable, else a defect description."""
    if rc == "TIMEOUT":
        return "hang (>20s)"
    if rc < 0:
        return f"killed by signal {-rc}"
    if rc == 101:
        return "rust panic (exit 101)"
    low = out.lower()
    for marker in (b"panicked at", b"internal error: entered unreachable",
                   b"attempt to subtract with overflow",
                   b"index out of bounds", b"slice index starts at",
                   b"range end index", b"capacity overflow",
                   b"memory allocation of"):
        if marker in low:
            return marker.decode()
    return None

def main():
    seeds = []
    for f in sorted(os.listdir(SEED_DIR)):
        p = os.path.join(SEED_DIR, f)
        if os.path.isfile(p):
            seeds.append((f, open(p, "rb").read()))
    if not seeds:
        print("no seeds"); return 1

    rng = random.Random(int(os.environ.get("FUZZ_SEED", "20260817")))
    findings = {}
    tmp = tempfile.mkdtemp(prefix="fuzzld")
    tried = 0

    for i in range(N):
        name, data = seeds[rng.randrange(len(seeds))]
        b = bytearray(data)
        if not b:
            continue
        # Mutation strategies biased toward ELF header/table fields, which is
        # where size/count/offset invariants live.
        strat = rng.randrange(4)
        if strat == 0:                      # single byte flip in first 1KB
            pos = rng.randrange(min(len(b), 1024))
            b[pos] ^= 1 << rng.randrange(8)
        elif strat == 1:                    # blast a u32/u64 field to huge value
            pos = rng.randrange(max(1, len(b) - 8))
            width = rng.choice([2, 4, 8])
            val = rng.choice([0xffffffffffffffff, 0x7fffffffffffffff,
                              0xffffffff, 0xfffe, 0, 1])
            b[pos:pos+width] = val.to_bytes(8, "little")[:width]
        elif strat == 2:                    # truncate
            b = b[:rng.randrange(1, len(b))]
        else:                               # random multi-byte smash
            for _ in range(rng.randrange(1, 6)):
                pos = rng.randrange(len(b))
                b[pos] = rng.randrange(256)

        ext = os.path.splitext(name)[1] or ".o"
        inp = os.path.join(tmp, f"m{i}{ext}")
        with open(inp, "wb") as fh:
            fh.write(bytes(b))
        outp = os.path.join(tmp, f"o{i}")

        if ext == ".lds":
            cmd = [LD, "-T", inp, "-o", outp,
                   os.path.join(SEED_DIR, "good.o")]
        else:
            # Rotate through the modes that have materially different emit
            # paths, so a mutant can reach the script/relocatable/KASLR
            # emitters and not only the userspace one. Previously every
            # mutant took the same code path, which is why a 6000-mutant
            # campaign still left emit_script and emit_rel unexercised.
            mode = i % 4
            if mode == 1:
                cmd = [LD, "-r", "-o", outp, inp]
            elif mode == 2:
                cmd = [LD, "-T", os.path.join(SEED_DIR, "seed_script.lds"),
                       "--emit-relocs", "-o", outp, inp]
            elif mode == 3:
                cmd = [LD, "-shared", "-o", outp, inp]
            else:
                cmd = [LD, "-o", outp, inp]
        rc, out = run(cmd)
        tried += 1
        d = classify(rc, out)
        if d:
            key = (ext, d)
            if key not in findings:
                keep = os.path.join(
                os.environ.get("FUZZ_CRASH_DIR",
                               os.path.join(tempfile.gettempdir(), "lccc_fuzz_crashes")),
                f"{len(findings):02d}_{d.replace(' ','_').replace('/','_')[:40]}{ext}")
                os.makedirs(os.path.dirname(keep), exist_ok=True)
                shutil.copy(inp, keep)
                findings[key] = (keep, out[-600:].decode("utf-8", "replace"))

    print(f"fuzzed {tried} mutants; {len(findings)} distinct defect classes")
    for (ext, d), (path, msg) in sorted(findings.items()):
        print(f"\n=== [{ext}] {d}\n    repro: {path}\n    {msg.strip()[:500]}")
    return 0 if not findings else 2

if __name__ == "__main__":
    sys.exit(main())

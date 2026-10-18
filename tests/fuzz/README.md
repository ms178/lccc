# Differential fuzzing

`differential_fuzz.py` emits deterministic, defined-behavior GNU C translation
units and compares LCCC's stdout/exit status against a reference compiler.  It
covers fixed-width arithmetic, CFG joins, calls, global state, arrays,
structs/bitfields, `volatile` locals, postfix increment/decrement, and limited
`__int128` paths.

Example:

```bash
python3 tests/fuzz/differential_fuzz.py \
  --ccc ./target/release/lccc --gcc gcc \
  --seeds 0:300 --levels O0,O1,O2,O3,Os,Oz --jobs 2 \
  --out /tmp/lccc-diff-fuzz
```

The generator avoids signed overflow, invalid shifts, division by zero,
out-of-bounds accesses, invalid pointers, and unspecified argument evaluation
order.  A failure retains its source under `--out` for reduction.

## CFG / phi-web stress

`phi_cfg_fuzz.py` specifically creates loop-carried values, branch diamonds,
switch joins, `continue` paths, and postfix increment/decrement operations to
exercise CFG-aware copy coalescing. For example:

```bash
python3 tests/fuzz/phi_cfg_fuzz.py \
  --ccc ./target/release/lccc --gcc gcc \
  --seeds 0:1000 --levels O3,Os --jobs 2 --out /tmp/lccc-phi-fuzz
```

## Unified entry point: `scripts/fuzz_diff.py --engine <name>`

Every differential engine in this directory is also launchable through the
unified harness `scripts/fuzz_diff.py`, which keeps each script byte-for-byte
unchanged and absorbs it through one of two mechanisms:

| script | `--engine` name | mechanism |
|---|---|---|
| `differential_fuzz.py` | `differential` | generator-config engine: the harness imports `generate(seed)` and runs its cases through the standard differential oracle (all references at all levels, GEN-BUG tripwire, repro retention) with the historical `-std=gnu11 -w -march=raptorlake -mtune=raptorlake -fomit-frame-pointer` flags; `--opts=-Oz` keeps `-Oz` for lccc and maps references to `-Os`. |
| `phi_cfg_fuzz.py` | `phi_cfg` | generator-config engine, same flags/levels as above (`gen(seed)`). |
| `fuzz_intcmp_thread.py` | `intcmp_thread` | generator-config engine (`gen_program`); the one historical rng is pre-generated sequentially so the worker pool stays deterministic; default seed 20260829 preserved. |
| `m32_differential_fuzz.py` | `m32` | forwarding engine: the script owns its nostdlib int80 exit-fold pipeline, so the harness launches it (`--lccc`→`--ccc`, first `--refs`→`--gcc`, `--seed`/`--count`→`--seeds LO:HI`, `--opts`→`--levels`) and adopts its exit code. |
| `regparm_differential.py` | `regparm` | forwarding engine (same shape, `-mregparm=3` variant). |
| `slot_rmw_differential.py` | `slot_rmw` | forwarding engine (slot RMW-collapse hinge). |
| `alias_fuzz_m32.py` | `alias_m32` | forwarding engine (redundant-load/GVN shapes). |
| `alu_torture_m32.py` | `alu_torture` | forwarding engine; the probe is fixed, so it runs once per level and `--count` has no effect. |
| `aarch64_fuzz.py` | `aarch64` | forwarding engine to `lccc-arm` vs `aarch64-linux-gnu-gcc` under `qemu-aarch64`; SKIPs cleanly when the cross toolchain is absent (the harness defaults `--lccc` to `target/fastbuild/lccc-arm` for this engine). |
| `punning_fuzz.py` | `punning` | forwarding engine (same `--ccc`/`--seeds`/`--levels` mapping, but no `--gcc`: the script's own byte model is the oracle, so no reference compiler is required); SKIPs off x86-64. |

Each engine keeps its historical defaults (seed span, levels, gcc oracle);
`--count`/`--seed`/`--opts`/`--refs` override them.

Host requirements and honest degradation: the m32 engines need a host whose
gcc can build ELF32 (`-m32 -nostdlib` sidesteps the multilib CRT) **and**
whose kernel permits the i386 `int $0x80` syscall gateway.  Sandboxed
kernels frequently link and exec ELF32 but block int80 (SIGSYS); there the
standalone testers would compare two identical deaths and "pass" vacuously,
so the harness probes this before forwarding and falls back to **compile-only
validation** instead (every generated case must still compile under both
`lccc -m32` and the reference with the historical flag matrix; lccc compile
errors are `LCCC_CRASH` with the source retained).  Compile-only passes are
labelled as such in the result message — they are not a codegen oracle.

Known semantic deltas of the generator-config engines (documented, accepted):
the unified oracle also compares stderr (the historical testers compared
stdout/exit only; the engines pass `-w` so stderr stays empty for both
sides), and `intcmp_thread`'s separate gcc-`-O0`-vs-gcc-`-O2` stability
check is not reproduced — the unified oracle compares gcc against lccc at
each level instead.

## Type-punning (byte-precise) fuzzing

`punning_fuzz.py` generates deterministic C translation units that store and load a
`union` (8 bytes) through defined punning forms only: union member access, `memcpy`
at arbitrary byte offsets, and `unsigned char` access. The expected output comes from
an independent Python byte model (little-endian, modular conversions) in the spirit of
Vellvm's `MemoryBytes` (concept only; no code copied).

```bash
# 1. Oracle self-check first: the model must agree with a trusted compiler.
python3 tests/fuzz/punning_fuzz.py --ccc gcc --seeds 0:300 --levels O0,O1,O2,O3,Os --out /tmp/p-gcc
# 2. Then the compiler under test.
python3 tests/fuzz/punning_fuzz.py --ccc ./target/fastbuild/lccc --seeds 0:1000 --out /tmp/p-lccc
# Inspect one case:
python3 tests/fuzz/punning_fuzz.py --emit-only 3
```

Failures keep their source under `--out`. `test_punning_fuzz.py` runs in the
`unittest discover` gate. Remaining scope gaps (overlapping `memcpy` between two
objects, bit-fields that straddle a storage unit, arrays of padded structs) are
tracked as T7 in `docs/VELLVM_VIR_TRANSPLANT_AUDIT.md`.

## C-level miscompile reduction (`c_reduce.py`)

`scripts/c_reduce.py` shrinks a C file while a predicate keeps accepting it
(ddmin, then a one-line fixpoint, so the result is 1-minimal).
`scripts/c_reduce_predicate.py` is the predicate: it accepts a candidate only
if GCC `-O0` with UBSan and ASan runs it cleanly and deterministically, and
then only if the compiler under test disagrees (`--kind miscompile`) or dies
(`--kind crash`).  Without the reference oracle a reduction can turn a real bug
into undefined behaviour and report a false miscompile.

```
scripts/c_reduce.py case.c --out min.c -- \
    python3 scripts/c_reduce_predicate.py --ccc target/fastbuild/lccc --level=-O2
```

The reducer and predicate are covered by `tests/fuzz/test_c_reduce.py`
(mutation tests with `fake_miscompiler.py`, a compiler with a known miscompile).

## Punning lanes (T7)

`punning_fuzz.py` covers `float`/`double` (bit patterns, exponent forced
finite), `__int128`, bit-fields (`struct bits`: signed and unsigned fields packed
into one 32-bit unit), padded structs (`struct pad`: padding bytes are never read,
whole-struct `memcpy` reads only the copy's members), and two lanes:
`--lanes base,v3` (`v3` adds `-march=x86-64-v3`).  Self-check the model against a
reference before trusting any verdict: `--ccc gcc`, `--ccc clang`, or `--ccc icx`
(the model agreed with all three, 240 of 240 each, for the bit-field and padding
corpus).  The layout of `struct pad` is also checked against GCC's
`sizeof`/`offsetof` in `test_punning_fuzz.py`.

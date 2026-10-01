# Byte-compare window phase — what it costs, what it buys, and how to re-derive both

**Tree:** `cdcf30de` (branch `work`), base `8db75621` (upstream main). Binary
under test: `target/fastbuild/lccc` (fastbuild profile; the arm is codegen-only,
so the profile does not affect the emitted loop).
**Box:** 2 vCPU / 1.9 GiB, Debian 14.2; **local reference** gcc 14.2.0.
**Remote oracles:** x86-64 gcc 16.2, clang 23.1.0, icc 2021.10.0, icx (latest)
via `scripts/codegen_oracle.py` (Godbolt).
**Kill switch:** `LCCC_NO_BYTECMP_VEC=1` (resolved once per TU in `run_passes`).

This document exists because the arm's original write-up priced it against a
benchmark target that could not be re-run from this tree ("`lz4_match_extend`
1271 vs 1787 ms"). That target *is* in the tree — it is
`tests/benchmark/programs/lz4_compress.c` built `-DMATCH_RICH=1 -DPASSES=2048`
by `tests/benchmark/run_benchmarks.py` — and every number below was produced by
that runner or by the interleaved harness, from this tree, on demand.

---

## 1. Runtime: the arm closes a real deficit to parity with GCC

Canonical corpus runner, one benchmark, `-march=x86-64-v3`, fresh timed runs:

```
python3 tests/benchmark/run_benchmarks.py --only lz4_match_extend \
    --compilers lccc,gcc --lccc target/fastbuild/lccc \
    --opt=-O2 --cflag=-march=x86-64-v3 --reps 15 --json … --markdown …
```

| level | arm | lccc/gcc (geomean) | reading |
|---|---|---|---|
| `-O2` | **off** (`LCCC_NO_BYTECMP_VEC=1`) | **1.265** | 26 % behind GCC |
| `-O2` | **on** | **0.997** | parity |
| `-O3` | **off** | **1.255** | 25 % behind GCC |
| `-O3` | **on** | **1.014** | parity |

Raw JSON/Markdown for all four runs: `canonical/`. The harness flags 5–10 %
coefficients of variation on this box and retains one MAD outlier — the *paired
adjacent-round* structure is what makes 15 rounds enough to separate 25 %; the
sub-2 % on/off differences should be read as "parity", not as a win.

Independent arm-vs-own-kill-switch measurement, three arms interleaved
sample-by-sample (25 samples each, same binary for both lccc arms so only the
transform differs):

| level | gcc | lccc arm OFF | lccc arm ON | ON vs OFF | ON vs gcc |
|---|---|---|---|---|---|
| `-O2` | 15.011 ms | 18.326 ms | 13.814 ms | **−24.6 %** (25/25) | −8.0 % (13/25) |
| `-O3` | 15.228 ms | 21.387 ms | 14.252 ms | **−33.4 %** (25/25) | −6.4 % (11/25) |
| `-O3` (second run) | 11.962 ms | 16.976 ms | 11.718 ms | **−31.0 %** (25/25) | −2.0 % (13/25) |

`tests/benchmark/programs/lz4_compress.c` is the tree's own extraction of
`LZ4_compress_fast`; `MATCH_RICH` makes the match extension long-match
dominated (the diagnostics in
`engineering/evidence/2026-09-26-followups/diagnostics/` record the length
distribution: 49 299 of 49 479 calls ≥ 8 bytes, and the default input reaching
*no* four-byte matches at all). Checksum `007c8f9a0a8617e8` is identical in
every arm of every run above, and matches the checksum recorded in that
diagnostics file.

## 2. Codegen: only LCCC vectorizes this shape at `-O2`; at `-O3` GCC does it and pays

`scripts/codegen_oracle.py` on `bytecmp.c` (three loops: the canonical loop, its
signed-bounds twin, and the LZ4 word-loop-plus-tail shape), all functions:

| compiler | `-O2` insns / loads / stores / spills / branch / **vector** | `-O3` (same) |
|---|---|---|
| **lccc** | 130 / 24 / 0 / 0 / 34 / **10** | 130 / 24 / 0 / 0 / 34 / **10** |
| gcc 16.2 | 56 / 6 / 4 / 0 / 22 / 0 | 483 / 101 / 26 / **13** / 119 / **71** |
| clang 23.1.0 | 58 / 10 / 0 / 0 / 19 / 0 | 58 / 10 / 0 / 0 / 19 / 0 |
| icc 2021.10.0 | 47 / 9 / 0 / 0 / 15 / 0 | 59 / 16 / 0 / 0 / 17 / 0 |
| icx | 140 / 25 / 0 / 0 / 41 / 0 | 148 / 25 / 0 / 0 / 44 / 0 |

`grep -c pcmpeqb` over the saved asm: **0** in every `-O2` artifact, **3** (one
per loop) in `gcc16.2.s` at `-O3`, 0 in clang/icc/icx at either level. So:

* at `-O2` no upstream compiler vectorizes this shape — lccc is the only one
  emitting a window phase (2 phases across the 3 loops), at +83 instructions
  over icc and 10 SIMD insns;
* at `-O3` GCC vectorizes it too and pays **3.7× lccc's size, 26 stores and 13
  spills** for the privilege, while clang/icc/icx still stay scalar. lccc's
  code is spill-free at both levels — the "only vectorizer at `-O2`, and the
  cheapest vectorizer at `-O3`" position.

Full asm + rank logs: `oracle-o2/`, `oracle-o3/`.

**What the hardening cost in code size** (`oracle-o2-prehardening/` is the same
source and command run on the epic-landing binary, kept for the comparison):

| | insns | `vpcmpeqb` | `pmovmskb` | `orq $4095` | `tzcnt` |
|---|---|---|---|---|---|
| epic landing (pre-hardening) | 92 | 2 | 2 | **0** | **0** |
| shipped tree | 130 | 2 | 2 | **2** | **2** |

+38 instructions over three functions — the page test, the exact-exit block, and
the phase's third phi edge — i.e. roughly 19 per vectorized loop. It replaces
code that was *not* equivalent: the pre-hardening artifact has no q-side guard
and no ctz exit, which is the code that SIGSEGVs in §4. The runtime tables in §1
are all from the shipped binary, so 26 % is what the *safe* arm delivers.

## 3. ISA contract (what the gate pins)

`tests/regression/check_bytecmp_vec_codegen.sh`, kernel
`tests/regression/bytecmp_vec/kernel.c` (one unsigned + one signed loop):

| configuration | data regs | packed cmp / mask | exit | page guard |
|---|---|---|---|---|
| default | `ymm` ×4 | `vpcmpeqb` + (v)`pmovmskb` | `tzcnt` | 1 per phase |
| `-march=x86-64` | `xmm` ×8 | `pcmpeqb` + `pmovmskb` | `bsf` or `tzcnt` | 1 per phase |
| `-mno-avx` | `xmm` ×8 | `pcmpeqb` + `pmovmskb` | `tzcnt` | 1 per phase |
| `-mno-sse -mno-mmx -mno-sse2` | none | none | scalar | none |
| `LCCC_NO_BYTECMP_VEC=1` | none | none | scalar | none |

The gate asserts the exact guard *count* (2 for the two-function kernel), so a
guard that is hoisted out of the loop or shared between the two phases fails
loudly instead of passing a "≥ 1" check.

## 4. Safety

* **The q-side guard is load-bearing, not defensive.** Neutering the page test
  (`q + WIDTH-1 ≤ q | 4095` → `q_span`, vacuously true), rebuilding, and running
  `tests/regression/bytecmp_vec_guard_page.c` gives **SIGSEGV, exit 139**;
  restoring it gives `checksum=884 failures=0`, stdout identical to GCC. The
  fixture puts q's last valid byte at the end of a mapped page with `PROT_NONE`
  after it, and every case in it is executable by the *scalar* loop without
  reading a guard byte.
* **Exact exit.** `vpcmpeqb` + `vpmovmskb` + `ctz(~mask)` hands the header
  `(p+t, q+t)`, t = first differing byte; a rescan is not needed and the
  exhaustive driver `tests/regression/bytecmp_vec_bounds.c` (lengths 0…399 ×
  every mismatch position × unsigned/signed, `checksum=21333200 bad=0`) checks
  every value against a closed-form expectation and GCC's stdout.
* **Residual, disclosed:** within the *same* mapped page the phase may read
  bytes past the end of the `q` object (never past the page). Object-granularity
  tools (ASan's redzones) can flag that read-ahead; the transform is
  conservative for pages larger than 4 KiB and is disabled wholesale by the kill
  switch.

## 5. What is not claimed

* **The index-form loop is not matched and is not a gap.** `k_matchlen.c`
  (`while (n < max && x[n] == y[n]) n++`) has one induction variable, not the
  two pointer phis this grammar requires. Measured on that kernel: lccc arm-off
  vs arm-on is a coin flip (+1.1 % min / −0.2 % median, 12/9), and lccc vs gcc
  is likewise neutral (−3.3 % min / +2.1 % median). Nothing is left on the table
  there by declining it: **gcc and clang do not vectorize it either** (0 packed
  compares at `-O2` and `-O3`).
* **Word-loop match extension** (current LZ4 `LZ4_count`, ZSTD's `ZSTD_count`
  body) is handled by the compilers' own word loops; the arm only ever sees the
  byte-wise tail, where it cannot fire unless ≥ 32 bytes remain.
* **The `zstd_count.c` code-size cost is real and accepted.** Its line-56 tail
  is the genuine shape, so the arm fires there and adds 13 instructions (123 →
  136) for a loop that can never pay off with a ≤ 7-iteration trip count. Trip
  count is data-dependent, so no static guard separates it from the 26 %
  winner; the codegen-quality baseline was refreshed with the trade documented
  in `engineering/FOLLOWUP-2026-09-30-s05-bytecmp-epic.md` §6.

## 6. Re-derive everything

```bash
# runtime, canonical runner (arm on / arm off)
python3 tests/benchmark/run_benchmarks.py --only lz4_match_extend \
  --compilers lccc,gcc --lccc target/fastbuild/lccc --opt=-O2 \
  --cflag=-march=x86-64-v3 --reps 15 --json out.json --markdown out.md
LCCC_NO_BYTECMP_VEC=1 python3 tests/benchmark/run_benchmarks.py --only lz4_match_extend \
  --compilers lccc,gcc --lccc target/fastbuild/lccc --opt=-O2 \
  --cflag=-march=x86-64-v3 --reps 15 --json out-off.json --markdown out-off.md

# runtime, one binary, arm vs its own kill switch, interleaved
python3 scripts/ab_interleaved.py --a target/fastbuild/lccc --b target/fastbuild/lccc \
  --a-env LCCC_NO_BYTECMP_VEC=1 --program tests/benchmark/programs/lz4_compress.c \
  --defines MATCH_RICH --samples 25 --flags "-O3 -march=x86-64-v3"

# multi-vendor codegen (remote); repeat with -O3 for the second column
python3 scripts/codegen_oracle.py --local target/fastbuild/lccc \
  engineering/evidence/2026-10-01-bytecmp/bytecmp.c --all-functions \
  --flags="-O2 -march=x86-64-v3" --local-flags="-O2 -march=x86-64-v3" \
  --artifact-dir oracle-o2 --json oracle-o2/manifest.json --markdown oracle-o2/rank.md

# emission contract + semantics, tri-config, kill switch
bash tests/regression/check_bytecmp_vec_codegen.sh
```

# zstd runtime performance: the prefetch gap, and what is left

Date: 2026-10-20. Base: `d8668ee` (merge of #882). Compilers under test:
LCCC fastbuild, GCC 16.2.0 (`gcc-16-bookworm`), Clang 23.1.3, all from the
`ms178/lccc` tag cache. zstd 1.5.7, `-O3 -march=x86-64-v3 -fno-lto`,
`nbWorkers = 0`, single-threaded library defaults.

Two harnesses, both checked into `/tmp` during the session and reproducible
from the commands below:

* **library A/B** — each arm builds upstream `lib/libzstd.a` with its own
  compiler, and the *identical* GCC-built driver object is linked to every
  arm. Timing is `CLOCK_PROCESS_CPUTIME_ID` inside the driver (this host has
  2 cores and an involuntary-descheduling-heavy scheduler), median of
  per-round ratios against a chosen base, shuffled arm order within a round,
  ≥5 rounds. Every arm must reproduce the reference compressed frame
  **byte-for-byte** and decode it back to the input before any timing runs.
* **CLI** — each arm builds upstream `programs/zstd`; child CPU time
  (rusage delta) of `zstd -<level> -c` and `zstd -d -c` on the same frame,
  best of 5. Frames must be identical across arms at every level.

## What was wrong: `__builtin_prefetch` was a no-op

`__builtin_prefetch` reached the lowerer as `BuiltinIntrinsic::Nop`, the
address was evaluated for its side effects, and the hint was dropped. GCC,
Clang and ICX all emit the instruction.

zstd's level-9 match finder is built on it: `ZSTD_RowFindBestMatch_noDict_5_4`
issues `PREFETCH_L1` for the hash row and the tag row, and 81 % of level-9
compression time is in that one function. Dropping the hints turned those
rows into serialised load-use chains.

With the hints restored (`IntrinsicOp::Prefetch`, see the commit message for
the encoding table and the `prefetchw` rationale):

| case (library A/B, 5 rounds, paired CPU time) | ratio to GCC before | after |
|---|---|---|
| text.tar L9 compress | 1.973 | **1.358** |
| text1m L9 compress | 1.704 | **1.303** |
| text.tar L9 decompress | 1.510 | **1.469** |
| 12-case geometric mean (L1/3/9 × c/d × text1m/text.tar) | 1.455 | **1.378** |

Compressed output is byte-identical in both arms (asserted by the harness),
so this is pure speed.

## Where the remaining gap is

The remaining deficit is not concentrated in one bad construct. Instruction
counts in the two hot functions are close to GCC's, but their *composition*
is not:

| function | LCCC | GCC |
|---|---|---|
| `ZSTD_RowFindBestMatch_noDict_5_4` (81 % of L9 c) | 572 insns, 279 `mov`, 104 frame refs, 9 prefetch | 559 insns, 195 `mov`, 83 frame refs, 9 prefetch |
| `ZSTD_decompressSequences_default` (92 % of decode) | 709 insns, 128 frame loads | 726 insns, 50 frame loads |

LCCC emits ~40 % more register-to-register moves and ~25-150 % more stack
traffic for the same source. That is the linear-scan allocator's assignment
quality, not a missed idiom: `ZSTD_compressBlock_fast_noDict_7_0` has 1503
live intervals, 133 of them call-spanning, and **34 % of all value uses are
stack-slot accesses** (`CCC_DEBUG_RA=1 CCC_DEBUG_RA_INTERVALS=1` dump of the
TU, 26 580 lines; the three hottest spilled values have 1320, 670 and 563
uses).

Attempts that did **not** pay off, measured, and therefore not shipped:

* **Call-free specialization** — inline a caller's last remaining callee so
  the whole caller-saved bank (r10/r11/r8/r9/rdi/rsi/rdx) becomes available
  to its hot loop. It works mechanically (`ZSTD_compressBlock_fast_noDict_*`
  goes from 2 calls to 0), but the clone costs more than the freed registers
  buy: 1.3865 vs 1.3782 geometric mean, text.tar L1 compress 1.438 vs 1.379.
  The reason is visible in the dump: the values that were spilled are mostly
  *not* call-spanning, so removing the call points does not free them.
* **Eviction policy** (`CCC_EVICT_MODE` 1/2/6 vs default 3): mode 1 is
  0.6 % better on the text suite (1.3694 vs 1.3780) but exactly neutral on
  unseen binary/random inputs (1.2215 vs 1.2203) with a worse tail
  (1.486 vs 1.412). Not a win.
* **`CCC_RA_LOOP_SPAN_RESERVE`** 1-5: helps text1m L1/L3 by up to 4 %, hurts
  text.tar L9 by up to 5 %. Not adopted.
* **Peephole budget** widening: no effect at all (the budget was not
  binding for any zstd TU).
* **Dispatch-trampoline specialization** (the closest match to what GCC
  actually does here): 21 zstd wrappers such as
  `ZSTD_compressBlock_lazy2_row` are 58-byte bodies that forward constant
  `(depth, mlBase, rowLog, dictMode)` tuples to one 18 KB
  `ZSTD_compressBlock_lazy_generic`. Admitting an oversized callee when the
  caller is a pure trampoline — one block, one call, the call's result is the
  function's result — produces exactly the shape GCC has (wrapper bodies grow
  to 8.2 KB / 6.3 KB / 5.0 KB against GCC's 2.1 k / 1.8 k / 1.8 k
  instructions) and the constants do fold. Measured: **1.4064 vs 1.3941**
  geometric mean over 16 cases. Level 9 and level 5 compression gain 4-5 %
  (1.271 vs 1.297, 1.290 vs 1.341 on text1m) but level 12 loses 12 %
  (1.516 vs 1.351) and the library grows 22 % (1 104 532 → 1 344 172 bytes),
  so the trade is a net loss and the change is not in the tree. Two lessons:
  the inliner is not the binding constraint on this workload, and a rule that
  clones a body into every wrapper needs a growth budget, not just a shape
  predicate.
* **`CCC_EVICT_SHORT_K`** 0 and 64 against the default 16 (the mode-3
  next-use gate's cost-ratio escape): 1.3886 / 1.3862 / 1.3842. Keep 16.

## CLI standing (the number that matters for the acceptance bar)

15 MB text slice, child CPU time, best of 5, identical frames, every level:

| level | compress vs GCC | decompress vs GCC |
|---|---|---|
| 1 | 1.302 | 1.208 |
| 2 | 1.230 | 1.286 |
| 3 | 1.194 | 1.367 |
| 5 | 1.271 | 1.355 |
| 7 | 1.241 | 1.412 |
| 9 | 1.238 | 1.182 |
| 12 | 1.337 | 1.622 |
| 15 | 1.095 | 1.353 |
| 19 | 1.344 | 1.389 |
| geo | **1.248** | **1.348** |

Clang 23.1.3 on the same harness: 0.972 compress, 0.956 decompress, i.e. no
level is slower than GCC for a non-GCC compiler — the harness is neutral.

1 MiB slice: compress geo 1.192 (worst 1.477 at L19), decompress geo 1.109
(worst 1.144).

So: the prefetch fix removes roughly a third of the original level-9
regression, and the CLI is still ~25 % behind on compression and ~35 % on
decompression. Closing that requires allocator work (fewer
register-to-register copies, fewer spill round-trips), not another
idiom-level fix.

One concrete gap is now identified with a mechanism: x86-64 has no
equivalent of the "steal a callee-saved register for a hot loop web" pass
that this backend already implements for i686 (`CCC_HOT_WEB_STEAL`, 3 by
default) and AArch64 (`CCC_LOOP_PIN`, 2 by default). The decode regression
is precisely a hot loop web (the output pointer, a loop-carried phi) that
loses its register to colder values; both other backends already decided
that losing such a web is worse than displacing what holds the register.
Porting the pass to x86-64 is the single best-attested next step, and it is
deliberately not attempted here: a register-stealing pass needs its own
A/B across the whole benchmark corpus, not a patch at the end of a
different change.

## Reproducing

```bash
# library A/B (per-arm static library + one shared driver object)
/tmp/prof/zbuild.sh <tag> <compiler>            # -> /tmp/zarms/<tag>/lib/libzstd.a + pdrv
/tmp/prof/zmeasure.py <tag>... --base gcc --levels 1,3,9 \
    --inputs /tmp/corpus/text1m.bin,/tmp/corpus/text.tar --rounds 5

# CLI
/tmp/prof/bench_cli.sh /tmp/zbench/cli /tmp/corpus/text15m.bin 1,3,9,12,19
```

`CCC_NO_PREFETCH=1` reproduces the pre-fix codegen from the same compiler
binary, which is how the "before" column above was produced.

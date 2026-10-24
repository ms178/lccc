# Exact label references and pinned branch commits

## Baseline and scope

Fresh checkout of upstream `9fa15cb4cf90301f0d809cd9dc0777f5d993dfb8`.
PR946 is closed; PR948 is merged at head
`b4d48d69d8d7d712a28865e1ff0710c49cfb5049`. Earlier uncommitted synthesis
results are not results for this checkout. This change implements the indexed
lookup portion of MB-20 and fixes two pinned-instruction rewrite paths. It does
not complete MB-20, the instruction-manual audit, the ABI follow-up list, or the
requested generated-code performance work.

## Decisions and safety review

* A fresh whole-file `LabelRefs` scan at each candidate label is quadratic.
  A single conservative snapshot would avoid scans, but could retain a deleted
  last reference and suppress a transformation that a fresh scan permits. Exact
  incremental counts preserve the original decisions within the invocation.
* Build lazily: a pass with no label query allocates no index. Each query reuses
  the same index. Every text rewrite/deletion in `combined_local_pass` now uses
  the cache's editing methods. Its separate high-bit XOR cancellation helper
  removes only fixed numeric-immediate/register instructions, which name no
  symbols. No transformation adds, renames or relocates label definitions.
* Retargeting updates both old and new token streams. Repeated mentions in one
  line count once, matching the full scan. Updates share the full scanner's
  tokenizer, including label-address and jump-table references; `.L1` does not
  match `.L10`. Full builds reuse their token scratch capacity across lines.
* Counts use `usize`: each count is bounded by the number of live lines. A
  deletion subtracts only for a live, editable line; replacing a NOP does not
  subtract its stale stored text. A refused pinned deletion changes no count.
* Branch inversion and one-move phi preinitialization now check `all_editable`
  for every edited/deleted participant before the first edit. Previously,
  `replace_line` could unpin an instruction or `mark_nop` could refuse deletion
  after the other half of a rewrite had already committed.
* Production state is per invocation, with no shared mutation or new locks.
  The scan counter exists only in tests and is thread-local. Other target
  backends and ISA-selection policy are unchanged.

## Executed evidence

Fastbuild (`-j2`), Rust1.97.0, active 8 GiB swap:

* 6,277 unit tests passed, 10 existing ignored; zero failures.
* All-target Clippy with `-D warnings` and rustfmt checks passed.
* Both mutation experiments failed as intended: removing the pinned commit
  guards trips the safety test; restoring one-shot queries produces 256 index
  builds instead of 1 in the scan-budget test. Restored branch tests pass.
* 94 benchmark-program/kernel sources produce byte-identical assembly, with
  zero compilation failures. This includes the entire `gzip_longest_match`
  assembly, so its stack-memory references cannot increase.
* Canonical benchmark runner, correctness-only: 52 shapes × four compiler
  arms = 208 successful results, checked against GCC16.2, including exit codes.
  Arms are upstream LCCC, candidate LCCC, GCC16.2 and Clang23.1.3. All 52 LCCC
  ELF files are byte-identical before/after. No program-runtime speedup is
  claimed from these runs.

CPU-pinned, randomized paired compilation, 15 measured rounds per input after
two warmups, `-O1 -S`, Intel Xeon 2.60GHz VM:

| Functions / 12 conditionals each | Before median | After median | Before/after |
| --- | ---: | ---: | ---: |
| 40 | 1.467182 s | 0.293780 s | 4.994× |
| 120 | 15.451337 s | 1.010693 s | 15.288× |

Every timed output was assembly-hash checked. A 300-function trial was stopped
before completion to bound the experiment; it is not a measured result. Raw
samples, hashes and the budget note are retained in
`../evidence/label-ref-index-2026-10-24/compile-cost.json`.

Callgrind on the 40-function input reports 19,845,265,946 → 3,232,109,164
simulated instructions (6.140× reduction). These are **not cycles** or
hardware-counter measurements. Candidate `LabelRefs::build` inclusive cost is
57,104,225 (1.77%); the peephole still accounts for 73.46% of compiler
instructions. MB-20's overall ≤35% peephole acceptance criterion is **not met**.
Remaining allocation, decoding and fixpoint scheduling work requires another
profile-guided change; this entry does not mark the whole item closed.

## Reproduction and limitations

```
python3 scripts/measure_label_ref_cost.py \
  --before /path/to/upstream/lccc --after target/fastbuild/lccc \
  --out /tmp/label-reference-cost --functions 40 120 --reps 15 --callgrind
python3 scripts/test_measure_label_ref_cost.py
python3 scripts/check_codegen_refactor_identity.py \
  --before /path/to/upstream/lccc --after target/fastbuild/lccc \
  --corpus tests/benchmark/kernel_corpus --corpus tests/benchmark/programs \
  --out /tmp/label-reference-identity --jobs 2
```

The compiler-free experiment tests reject changed assembly and verify warmup
exclusion. They run in local and hosted CI. Compact corpus and identity records
are beside `compile-cost.json`; large binaries/profiles remain workspace
artifacts, not repository payloads.

GCC16.2 and Clang23.1.3 are restored from verified transport caches; GNU as/ld
and inspection tools are built from verified binutils2.47 sources. The GCC
prefix is packaged for Bookworm, not verified as a Debian Sid package. The
available cache contains no ICX payload; fresh ICX2026.2 comparison is missing.
`perf` is not installed, and no PMU claim is made. The earlier SSE2 runtime gap
is not solved by a compiler-latency optimization. No universal optimality,
all-architecture certification or superiority over other C compilers follows
from this evidence.

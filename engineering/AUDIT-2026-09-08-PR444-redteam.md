# Red-team audit of PR #444 (hot-loop alignment, GAS `.p2align` parity) + CI-green rebase

Base: upstream main `b6d6e7bd` (Merge PR #444). Auditor: the PR #445 session
(`ms178-1.patch` S01+S02 lineage; rebased deliverable =
`b6d6e7bd` + 258473bd (S02) + 1bba858a (policy hardening / race fix / CI)).
Every verdict below was verified programmatically: A/B builds of the exact
commits (`20261722`, `64534d1`, `9d2119c`, `2c9b6e16`), the 122-case
GCC-differential hash battery, `cargo test` hammer runs, and `RUSTFLAGS=-D
warnings` reproductions of both CI failures.

## Context

PR #444 (`2c9b6e16`) and my PR #445 (`1aba0bd8` ≡ `9d2119c`) solved the SAME
problem domain independently and landed within hours of each other:

| Area | PR #444 (merged upstream) | PR #445 (mine) |
|---|---|---|
| `.p2align` grammar | full GAS `N[,fill][,max-skip]`, byte-count `.align`/`.balign`, clamped exponent 63, 776/776 asmdiff vs a locally built GNU as 2.47 | 3-operand form + parser-level `AlignChain` run-merging |
| Relaxation safety | markers recorded even for zero padding; max-skip re-evaluated against the relaxed offset; overflow-safe `div_ceil` alignment math | chain re-evaluated jointly post-relaxation |
| Directive policy | GCC-16.2-derived: vector loop = plain `.p2align 5`; scalar = `.p2align 4,,10`+`.p2align 3`; -Os none; PGO hot-only | bounded chains: vector 32B-cap-15 → 16B-cap-15; scalar 16B; outer 16/8 caps 10/7; tiny-trip ≤ 4 unaligned; PGO defers |
| Flags | `-falign-loops/jumps/functions[=N[:M]]`, GCC grammar, power-of-2 validation | `-falign-loops[=N[:max]]` only |
| PGO | hotness gates loop + join alignment | deferred to `pgo::layout` |
| `identical_blocks` | 4-piece soundness fix: directive transparency, entry-region pseudo-predecessors, fall-through-entry deletion guard, empty-preds-never-merge, merged duplicates keep trailing directives | directive transparency only; the entry-region hole documented as a known limitation |
| Extra fixes | `loop_unroll` side-effecting-latch/escaping-def unroll rejection (a real -O3 miscompile), `fold_staged_imm_into_alu` peephole, `is_vector_op` classification, dead `machinst_emit` path removal | — (mine had none of these) |
| Tooling | `scripts/check_loop_alignment.py` (CFG-reconstructing audit), 21-case `.p2align` casefile, `outer_loop_shapes.c` + `vectorize_int_map_lanes.c` corpora | GCC-differential hash battery (scratch), correctness-suite additions |

## Verdict: PR #444 is the better S01. Adopted wholesale; two real gaps found and fixed on top.

**Why #444 wins on the overlapping ground (all verified, not conceded from
reading):**

1. **Oracle discipline.** They verified every assembler semantic against a
   real GNU as 2.47 binary (776/776 differential) before writing code —
   including the corner I would never have probed: `.p2align 4,,10` that
   declines to pad STILL raises `sh_addralign` to 16; `.p2align 63` at a
   nonzero offset is an error, at offset 0 it silently does nothing; the fill
   truncates to a byte; an empty middle field selects default fill. My
   implementation was self-consistent but oracle-untested at these corners.
2. **The identical_blocks fix is the fix my own code said was needed.** My
   S01 shipped transparency-only and left a comment: "a general fix needs
   entry-region/island tracking in the block scan." #444 implemented exactly
   that (per-function NUL-prefixed pseudo labels for entry-region jumps AND
   the positional entry fall-through), then added two more soundness rails:
   a block with a fall-through entry can never be the deleted duplicate
   (identical pred SETS cannot prove entries rewritable — P can jump to the
   canonical and fall through into the duplicate), and empty predecessor
   sets never merge (defense in depth covering `loop`-instruction targets).
   My session's earlier attempt at the empty-pred guard broke the positive
   control (`non_jump_table_blocks_still_merge`) because I had no
   entry-region model to make legit blocks non-empty — the missing piece
   #444 supplied.
3. **The unroll miscompile fix is a genuine find** (outer-loop row sums
   stored in the latch dropped by ×2 partial unroll — 20/20 deterministic),
   validated across 12 opt/march combos, with unit tests and a positive
   control. Nothing in my PR covers it.
4. **Flag surface.** `-falign-jumps`/`-falign-functions` with GCC grammar
   (including the `N:M` max-skip and the `max_skip >= align` normalization)
   is a strict superset of mine.
5. **Architecture.** A `src/passes/` pass with a `LoopAlignConfig` input
   struct, driven from `driver::pipeline` post-layout, beats my backend-side
   hook: one policy location, PGO-input not PGO-parallel, A/B-testable.

**Where #444 is weaker than mine — both fixed in 1bba858a:**

1. **Unbounded 32-byte padding for vector loops.** GCC 16.2 does this, but it
   costs up to 31 bytes of one-shot NOP per vectorized loop, and lccc's own
   PGO ledger measured *unbounded* 32-byte alignment as a net loss on gzip
   `longest_match` (that history is why #444's scalar cascade is bounded).
   ICX and Clang 23.1 both stop at 16 bytes. The merged policy now emits
   `.p2align 5,,15` + `.p2align 4`: the 32-byte DSB win whenever the natural
   offset is within one 16-byte quantum of it (the common case), the
   ICX/Clang 16-byte form otherwise, worst-case padding halved from 31 to 15
   bytes. `check_loop_alignment.py` asserts the new contract in both
   directions (a missing tier fails; so does an unexpected one).
2. **Constant tiny-trip loops were padded.** GCC and Clang leave
   `for (i = 0; i < 3; i++)` unaligned — the one-shot padding cannot pay
   back over ≤ 4 iterations. The pass now proves a constant trip bound from
   the header exit comparison (`iv < Const(n)` and the mirrored
   `Const(n) > iv`; only compares feeding the header terminator count) and
   skips alignment for n ≤ 4. A `-falign-loops=N` override is an explicit
   user contract and wins over the exclusion. The audit corpus gained
   `tiny_trip_call` (constant trip 3, kept from unrolling by an opaque call)
   asserting the loop survives to codegen AND stays unpadded.

**Integration seams verified on the rebased tree:**

- My S02 intrinsics (`VecCmpI32x8/x4`, `VecBlendvI32x8/x4`, `VecMinI32x8`)
  are all in `produces_vector_value`/`vector_result_width`, so #444's
  `is_vector_op` classifies int-cond-map loops as VECTOR loops → they receive
  the (now bounded) vector cascade, not the scalar one.
- My `.p2align 4` lane-const bias labels flow through #444's writer, whose
  record-marker-even-when-padding-is-zero behavior is exactly what keeps
  them 16-aligned after jump relaxation (my S01 needed the same property;
  #444's is GAS-verified).
- Sequential marker evaluation in `fixup_alignment_markers` +
  `shift_offsets_after` gives chained directives GAS's moving-offset
  semantics, which is what my parser-level `AlignChain` item existed for —
  the explicit merge structure was unnecessary and is dropped with S01.

## The PR #445 CI failures — root causes (both reproduced, both fixed)

1. **Clippy job: `cargo fmt --all -- --check` failed.** My hand-written S01
   (elf_writer_common/generation/backend loop_align) was never rustfmt'd.
   Reproduced locally (`Diff in ...` ×N). Fixed by running `cargo fmt` and
   folding the result into the rebased commits; `--check` is clean.
2. **Test Suite job: `cargo test ... -j 2` exit 101.** NOT a code defect in
   the PR: a pre-existing **test-infrastructure race** in
   `machinst_tests::run_emitted` that CI's 4-core runner (4 test threads)
   hits and a 2-thread dev box usually does not. Probe scratch directories
   were named `/tmp/lccc-machinst-exec-{pid}-{body.len()}`: two tests whose
   generated probe bodies have the same byte length (e.g. the 3-line imul3
   probe and its neighbors) shared a directory, and whichever finished first
   `remove_dir_all`'ed it while the other's `as` ran → `can't create
   .../f.o: No such file or directory` → panic → exit 101. Reproduced
   locally at `--test-threads>=4` (~40% of runs over 15 iterations, always a
   different sibling test); 0/20 after the fix at 16 threads. The fix gives
   every probe site a unique directory (`probe_dir(tag)`: pid + atomic
   counter), matching the remedy `linker_common/args.rs` already adopted for
   this documented flake class. This also de-flakes upstream main, which
   carries the same latent race.
   (Verification that it is NOT compiler code: the exact CI command
   `cargo test --profile fastbuild --all-targets --locked -j 2` passes on the
   PR tree in this environment — 2070/0 — at 1, 2, 4, 8, and 16 test
   threads, with i686 multilib installed and with `RUSTFLAGS=-D warnings`;
   the failure only ever appeared at ≥ 4 threads via the temp-dir collision.)

## CI hardening delivered alongside

- `bench.yml` now builds with `scripts/build_lccc_fast.sh` and points every
  gate at `target/fastbuild/lccc` (was: release-profile opt-level override +
  `target/release/lccc`) — one profile definition shared with ci.yml; the
  profile shapes the compiler binary, never the generated code.

# Never-read DSE across in-body RSP shift windows (journal item, 2026-10-28)

## What changed

`dead_code::eliminate_never_read_stores` historically bailed out of an entire
function the moment it saw an in-body RSP shift (`push`/`pop`/`subq`/`addq
$N,%rsp`). Codegen emits such shifts for balanced variadic call-argument
staging (`pushq …; call; addq $N,%rsp`), so functions containing a variadic
`printf` kept never-read frame stores forever — the `loop_patterns` widening
reductions emitted nine dead `movdqu %xmm0, off(%rsp)` stores whose slots have
no plain reload. Two stacked conservative bails were responsible (see the
2026-10-28 journal):

1. the no-FP prologue backscan from `subq` stopped at the first interleaved
   code-less `.cfi_offset` directive;
2. any in-body RSP shift disabled the whole function.

The fix, all in `peephole/passes/dead_code.rs`:

- **Prologue backscan** steps over code-less `.cfi_*` directives (stopping on
  `cfi_startproc`/`cfi_def_cfa_offset` and on any non-CFI directive), so a
  `.cfi_offset` after each callee save no longer disqualifies the function.
- **`rsp_delta` tracking.** A constant signed depth (`push`/`subq` +,
  `pop`/`addq` −) is accumulated through the body read loop. Every frame read
  is translated back to its canonical slot via
  `canonical_frame_off(base, off, delta) = off - delta` for `%rsp` operands
  (`%rbp` never moves inside a function and is left untouched). A store at
  non-canonical depth, an SIB frame operand at non-zero delta, an
  unparseable non-epilogue shift, or a conditional/indirect branch reached at
  non-zero delta fails the function closed.
- **Path-end reset.** A fall-through or loop-header label is *not* a boundary
  (loop back-edge labels legitimately sit inside an open push window, e.g.
  cmpxchg retry loops). Only a label reached after the linear path ended in a
  `ret`/unconditional `jmp` resets the delta, because such a block is entered
  only through a branch at canonical depth. A shifted unconditional `jmp`
  whose target block names any frame operand fails closed.
- **Memory-form push/pop** (`pushq 0(%rsp)`) both shifts RSP and reads a frame
  slot at the pre-shift depth; it fails closed instead of being counted as a
  bare +8 step.

## Blast radius (assembly A/B against the pre-change compiler, `-O2`)

Of all **78 benchmark TUs, exactly one changes** — `loop_patterns`, a pure
nine-line deletion (zero additions, every other line byte-identical;
`loop_patterns.diff`). Of **1,182 regression TUs, 13 change**
(`regression-tu-changes.txt`); every diff was verified deletion-only with no
added or rewritten lines (includes `x86_fpo_i128_div_push_depth`,
`i128_divrem_pow2`, `vectorized_counter_escapes`). `loop_patterns` `.text`
shrinks 2367 → 2239 bytes (−128).

## Correctness

- Full GCC 16.2 differential corpus: **1195 passed, 0 failed**, 2
  skipped-compare, 63 host m32 skips identical on the host GCC
  (`corpus-differential.log.gz`).
- `cargo test --profile fastbuild --all-targets --locked -j2`: **6772 passed,
  0 failed**, 10 ignored; `cargo clippy` and `rustfmt` clean.
- Seven new `dead_code` unit tests cover each new rule with a paired mutation
  that inverts the expectation (push-window canonical translation,
  unbalanced-conditional bail, shifted-SIB bail, loop-label carry, post-`ret`
  label reset, memory-form push bail, shifted-`jmp` frame-block bail).
- Six differential programs that first regressed during development drove the
  soundness rules and now pass: the cmpxchg loop-label carry
  (`param_prestore_abi_read`), sign-extended negative index through an escaped
  frame pointer (`sse2_i32_product`, `bb_slp_v7`, `cond_reduction`),
  memory-form variadic push (`global_init_pointer_scalar_base`), and the
  cpuid inline-asm post-`ret` block (`asm_static_inline_pointer_outputs`).

## Wall clock (paired, CPU-pinned to core 0, interleaved AB/BA)

Static instruction counts are never accepted alone. On the only affected
benchmark, eight independent 21-repetition paired A/B runs give min-based
ratios (new/old) sorted
`0.9740 0.9761 0.9840 0.9896 0.9899 0.9943 1.0049 1.0073`:

- geometric mean **0.9899 (−1.01 %)**, median **−1.02 %**, 6 of 8 runs faster;
- the noise-resistant low3 mean over the four frozen sample runs is **0.9920
  (−0.80 %)** (`loop_patterns-ab-sample{1..4}.json`);
- a single 41-rep min outlier reached +3.25 % while its own low3 was 1.007,
  i.e. a one-round scheduling artefact, not a systematic regression.

The full 56-kernel paired suite (the other 55 kernels are byte-identical)
lands at geometric mean **1.0005 — no measurable difference and no
workload beyond ±2.1 % noise**, the same swing shown by unchanged TUs
(`full78.json`, `full78.md`). The win is therefore a small, repeatable
improvement on the affected kernel (dead stores removed from its hot
vectorized loop), neutral everywhere else, with a deterministic code-size and
soundness benefit; it is not claimed as a broad runtime uplift.

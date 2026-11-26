---
layout: doc
title: Optimization Passes
description: The CCC/LCCC optimizer pass pipeline — what runs, in what order, and why.
prev_page:
  title: Register Allocator
  url: /docs/register-allocator
next_page:
  title: Benchmarks
  url: /docs/benchmarks
---

# Optimization Passes
{:.doc-subtitle}
LCCC inherits CCC's optimizer. **Tiers are real** (authoritative: `src/passes/README.md`):

- `-O0`: skip the pipeline except mandatory inline-asm symbol resolution
- `-O1`: mem2reg, constant folding, copy-prop, DCE
- `-O2`: full scalar/IPO pipeline
- `-O3`: `-O2` plus size-increasing unroll
- `-Os` / `-Oz`: size; `-Oz` also disables inlining

Older text that claimed “all `-O` levels run the same pipeline” is **obsolete**.

The `-O` flags still set `__OPTIMIZE__` / `__OPTIMIZE_SIZE__` for kernel-style `#ifdef`s.

## Recently added passes

`src/passes/README.md` and `src/passes/mod.rs` are authoritative for the IR
passes; the backend peepholes are documented per backend
(`src/backend/<arch>/codegen/`). Landed since this page was written:

- **Path-duplicating jump threading** (`jump_thread.rs`, IR pass, `>= -O2`
  except `-Os`/`-Oz`, after the main loop) -- duplicates a path of at most 4
  blocks / 8 instructions for a predecessor that decides a later branch
  (constant phi arm, dominating test, implied integer compare); at most 16
  threads per function. Loop-header tail duplication is limited to
  load-fed header branches that choose within the iteration, 1-2 latches,
  private arms; measured rejections are listed in
  `engineering/subsystems/optimizer.md`. Disable: `CCC_DISABLE_PASSES=jump_thread`
  (not the x86 peephole of the same name, `CCC_PEEPHOLE_SKIP=jump_thread`).
- **Decided branches behind the rounds**
  (`jump_thread::thread_decided_and_fold`, same levels) -- the threads of
  the pass above without its latch copies, taken once more between the
  optimizer's rounds and the late passes, for the branch on a merge that one
  way in brings a constant to: `if (smaller(...)) j++;` with `smaller` an
  inlined `a < b || (a == b && c <= d)`. Four parts. If-conversion leaves
  such a branch (`if_convert::branch_is_decided_on_entry`: a phi of the
  block with a constant arm, read directly or through casts and compares
  against constants; a select would write the constant, test it and compute
  both arms on the way in that knew). The threader sends that way in to its
  arm; a function that took a thread is cleaned up and asked again, up to
  four rounds, a level of nested conditions a round. The constant is read
  through what the helper's return type puts between the merge and its
  branch (`jump_thread::constant_on_entry` folds the block for the one way
  in: nothing for `int`, `!= 0` for `_Bool`, a narrowing cast for `unsigned
  char`, and 256 as a byte is 0), and `simplify` reads the comparison
  through the same links (`truth_base`: a branch on an integer conversion of
  a truth value is a branch on the truth value). `range_fold` and
  `set_membership` then run once more on that function, because the hits of
  `while (i < n && separator(s[i])) i++` are at the loop's step only now and
  the set fold takes a chain of tests of a `char` only where its hits meet
  without a value: 32 instructions and nine branches a character before, 15
  and four with one bit test after (GCC 16.2: 14), whatever the helpers
  return. Measured (Cachegrind, instructions): `zlib_ng_huffman_heap`
  16,406,812 -> 14,027,589, `expat_xml_scan` 1,529,625 -> 1,428,038 (GCC
  16.2: 1,460,771). Gate: `tests/regression/check_boolean_merge_threads.py`.
  Disable: `CCC_DISABLE_PASSES=jump_thread_early` (this position),
  `jump_thread` (both).
- **Join store forwarding** (`join_store_forward.rs`) -- a reload after a
  join whose predecessors all store to the same pointer becomes a phi of the
  stored values (list_sort's `tail->next`). `join_store_forward`.
- **Phi relay folding** (`phi_relay_fold.rs`, `>= -O2`, right before phi
  elimination) -- phi-only latch relays fold into the header so each edge
  gets one parallel copy. `phi_relay_fold`.
- **Phi webs** (`ir/mem2reg/phi_webs.rs`) -- acyclic phi webs inside one
  innermost loop share one name (class cap 32, pass-through pairs first);
  cross-loop unions are refused because they spill. `CCC_NO_PHI_WEB_COALESCE`.
- **Accumulator recursion elimination** (`tail_call_elim.rs`,
  `accumulator_calls_to_loops`, after `rec2iter`) -- `return x OP f(...)`
  with an integer (<= 64-bit) Add/Mul/And/Or/Xor becomes a loop with an
  accumulator; one operator per function. It must run after `rec2iter`,
  which owns the O(n) Fibonacci rewrite. `tce_acc`.
- **Bounded recursive self-inlining** (`inline/self_recursion.rs`, off at
  `-Os`/`-Oz`) -- a frame-neutral, directly self-recursive body of at most
  32 instructions and 2 self-calls is inlined into itself (at most 8 copies,
  3 rounds, 160 instructions; no setjmp/`__builtin_setjmp`/non-local goto),
  so each level owns its base-case branch. binary_trees: Callgrind
  mispredicts 198,691 -> 89,642, Ir -4.7%. `self_inline`.
- **Compare narrowing rule** (`simplify.rs`) -- `cmp (ext x), C` narrows to
  the source width only when the extension's sole reader is the compare;
  otherwise the wide value stays live anyway, and the narrow compare plus
  the wide use cost an extra register (sqlite_vdbe_serial spill).
- **Aggregate-copy forwarding** (`sret_copy_elision.rs`) — forwards eligible
  same-block, dominated aggregate copies into their loads, retaining copies
  with escaping, ambiguous, volatile, or otherwise unowned uses. Set
  `CCC_NO_SRET_COPY_ELISION` (presence-based, including an empty value) to
  disable both pipeline invocations for diagnosis.
- **Two-block reduction unrolling** (`loop_unroll.rs`, Pass B) — guard-free
  k-body unroll of a two-block counted loop, k | trip. Two profit reasons
  admit a load-only latch whose k copies stay a single chain: a private
  **unsigned** accumulation (`loop_unroll_reduction.rs`, which then balances
  the additions) and, since the 2026-10-29 audit, a **serial carried
  accumulator of any type** — `acc = acc + x`, one accumulation per iteration,
  one use of the back-edge value. The second reason is what makes strict-FP
  reductions unrollable: the k bodies are threaded, so the association order is
  unchanged and no fast-math permission is needed. Measured with callgrind on
  `reduction_vecreg` (corpus worst row): 3,277,948,036 → 1,803,403,033
  instructions, ratio 1.905 → 1.048 against GCC 16.2. Gate:
  `tests/regression/check_fp_reduction_unroll.sh` (shape + bit-exact results
  across `-O0`/`-O1`/`-O2`/`-O3`/`-mno-avx`/`-march=x86-64-v3`, every trip
  residue modulo 8, association-sensitive payloads).
- **Constant-trip before the vectorizer** (`run_passes`, "Phase 2b-pre-vec") —
  IPCP also runs *before* iteration 0's vectorizer and post-vectorize unroller,
  not only at the end of each iteration. Without it a callee whose trip count
  only becomes constant when its `n` parameter is specialized (the ubiquitous
  `sum(const T *v, int n)` shape with one call site) is a dynamic-trip loop for
  both, and by iteration 1 `loop_rotate` has already rewritten it into the
  rotated self-loop form that no unroller accepts — so those callees never got
  unrolled at all. The late IPCP stays (call-site simplification still wants the
  end-of-iteration position); the early call is idempotent. It is placed after
  the Early unroller on purpose: earlier placement perturbed shapes that the
  store-motion and segment-SIB contracts depend on (measured: those gates'
  fixtures then unrolled completely, which is why the two gates now state their
  contract for what it is — see `check_loop_store_motion.sh`'s `UNROLLED` arm
  and `check_seg_gs_runtime.sh`'s `CCC_DISABLE_PASSES=ipcp` shape build).
- **DSE** (`dse.rs`) — same-block dead-store elimination with closed-alloca
  escape analysis and byte-range kills (`CCC_NO_DSE`).
- **Backedge PRE** — integer recurrences default-on (1.14x measured);
  FP variants gated (`CCC_BEPRE_FP=1` research).
- **GlobalAddr CSE** (`global_addr_cse.rs`) — oracle-derived placement:
  cold singletons branch-local, loop addresses to the innermost preheader,
  dominating defs reused, derived variable-index bases site-local.
- **Stencil vectorizer** — constant-tap affine loops, bit-exact vs scalar
  (`CCC_NO_STENCIL_VEC`).
- **Map expression trees** — elementwise FP/int map loops (`CCC_NO_MAP_VEC`).
- **Generator-map vectorizer** (`vectorize_generator_map.rs`) — counted
  loops that store a byte computed from the induction variable alone
  (`dst[i] = (u8)F(i)`, no source loads), AVX2 only, 32 elements per packed
  iteration with a scalar tail. Kill switch `CCC_NO_GENERATOR_MAP`; the
  `[VEC-GEN]` trace under `LCCC_DEBUG_VECTORIZE`. Gate:
  `tests/regression/check_generator_map.sh`.
- **Strided-2 i32 sum reductions** (`vectorize_strided_i32_reduce.rs`,
  pass `vectorize_strided_reduce_pre_unroll`) — counted loops whose byte IV
  steps by 8 and whose accumulator adds a pure 32-bit modular expression of
  the loaded dwords. Every vector covers four elements (32 bytes of the IV).
  **AVX2:** the elements sit in the even YMM lanes — four exact `vmovd` /
  `vpinsrd` reads, or one full `vmovdqu` when the window is proven — so the
  per-element DAG needs no in-loop shuffle; the epilogue masks the odd lanes;
  4x unrolled with two accumulator chains at `>= 8` safe vectors. **SSE2
  baseline:** the elements fill all four XMM lanes — a proven window is
  compacted from two `movdqu` with one `shufps $0x88`, an unproven pointer is
  gathered with four exact `movd` reads joined by `punpckldq`/`punpcklqdq` —
  so streams of either kind mix and no lane mask exists; unrolled twice with
  two accumulators and two in-place IV vectors. The loop bound is the scalar
  loop's own dereferenced footprint (`32v + 31 <= 8(T-1) + 3`), so the
  vector prefix never reads past the last scalar element. A stream qualifies
  for a gap-reading wide load only under a proof that its whole window lies
  in one object: a current-frame alloca of sufficient extent, or — through
  `ModuleSymbolFacts` — a strong, locally binding global whose defined size
  covers the window (interposable, weak, common, alias- and
  asm-label-redirected names are excluded, so a link-time replacement can
  never shrink the proof). Unproven pointers read only their elements:
  intervening bytes may belong to another, independently shared or
  device-mapped object. The untouched scalar loop runs the tail. Kill switch
  `CCC_NO_STRIDED_REDUCE`; i686 and AArch64 fail closed. The trip count, the
  wrap-free admission and the resume constant are pure functions with
  exhaustive unit tests (`cargo test -p lccc vectorize_strided_i32_reduce`);
  `tests/regression/check_vector_remainder_codegen.sh` pins both widths,
  checks exact access with hardware watchpoints on the gaps
  (`fixtures/strided_exact/`) and runs a boundary stress
  (`tests/regression/lib/strided_reduce_stress.py`) against GCC.
- **Vector fill** (`vector_fill.rs`, late, x86-64 SSE2+, `-O2`/`-O3`) —
  `for (i = 0; i < n; i++) p[i] = v` whose element is 16/32/64-bit (or a
  float constant) and whose value is not one repeated byte (those are
  `loop_memset`'s `memset`): a 32-byte-per-iteration pre-loop of two
  `movdqu` stores of the splatted value runs up to `n & -L`, then the
  untouched scalar loop finishes the tail and still defines every exit
  value. A constant trip count that is a multiple of the step exits straight
  past the (then dead) scalar loop. Exactness argument and admission rules
  (64-bit affine address chain, `sext` only under a signed exit test,
  negative signed bounds clamp to zero) in the module doc; obeys
  `-fno-tree-vectorize`; pass name `vector_fill`. Gate:
  `tests/regression/check_vector_remainder_codegen.sh`
  (`tests/regression/lib/vector_fill_stress.py`).
- **Widening + masked conditional-sum reductions** — I32->I64 `paddq`
  pipelines, VEX-only bodies (9x AVX-SSE transition penalty avoided).
- **Loop rotation** (`loop_rotate.rs`, IR) — `while (c) body` becomes
  `if (c) do body while (c)`. On by default at `-O2` and `-O3` **for the
  loops where it pays** (`RotatePolicy::Profit`); not at `-O1`, `-Os`, `-Oz`.
  The pipeline bottom-tests a loop whose test is register arithmetic without
  it (`loop_invert`, after phi elimination), so what rotation adds is one
  block: a preheader *behind the guard*, which runs exactly when the body
  does. Three things can only be
  moved there and nowhere else, because they must not run when the loop does
  not:
  - a load, a trapping division or remainder, or a read-only call with
    loop-invariant operands in the body (LICM; a top-tested loop keeps them
    in the loop, every trip);
  - a memory recurrence through a pointer, `*p += x`, which the promoter
    then keeps in a register with one load in front of the loop and one
    store on its exit edge;
  - a recurrence between elements, `a[i] = f(a[i-1])`, whose first load
    loop-carried forwarding puts in front of the loop.

  The decision is made by doing it: the candidate loops of a function are
  rotated in a copy, the three passes run on the copy, and a loop pays when
  what left it (a) had to wait for the guard, i.e. does not leave in a second
  copy that is only hoisted, (b) is worth at least 1/32 of one trip, counting
  a hoisted load as 1 instruction, a load replaced by a register recurrence
  as 8 (it was a store-to-load forwarding per trip) and a division or call
  as 24, and (c) when it is hoisted loads alone, holds no more registers
  through the loop than before (each load takes one of its class; an address
  nothing reads from the loop on gives one back). Only the loop's own
  instructions count, each known by the value it defines: what an inner loop
  lost is the inner loop's gain, and a load the promoter put in front of an
  inner loop is not a load that left. Stores are not counted: one that
  leaves does so together with the load of its recurrence. Loops that do
  not pay are set aside and the rest retried; a function in which none pays
  keeps its IR untouched, value numbers included. `[PASS]` reports
  `loop_rotate (func F): Ns (N loops, M moved)`; `CCC_DEBUG_LOOP_ROTATE=1`
  names, per loop, what left it (`rotation pays: 1 carried load left the
  loop, worth 8 of a trip of 8`) or why it was undone (`nothing left the
  loop`, `nothing left the loop that had to wait for the guard`, `... worth
  1 of a trip of 41232`, `... and the loop holds more registers for it`).

  Which loops are tried at all is a filter in front of that
  (`has_guarded_hoist_candidate`): a block that runs on every trip holds a
  load, a trapping division or a read-only call with operands the loop does
  not compute, or a load through an address the loop computes next to a
  store of that type; and for the loads, the loop's memory is known (no
  call that may write, nothing that writes other than a plain store). Over
  the 1,498 files of the regression corpus that is 1,134 trials, of which
  47 loops pay; compile time rises by 0.3%.

  One kind of loop is rotated for its branch and not for what can leave it:
  a loop whose header computes a value that is read outside the header,
  `while (n--) use(n);` (`rotate_fed_loops`, late, behind every pass that
  matches the shape of a loop). `loop_invert` copies the header into the
  latch after phi elimination and cannot hand the body the latch's copy of
  such a value, so these loops stayed top-tested, a jump back on every trip.
  A loop is taken when its header is phis and at most 8 pure instructions
  without a load (`MAX_FED_HEADER_INSTS`, the bound of `loop_invert`), a
  latch jumps back to it unconditionally, and **no header phi that the
  header steps has a reader behind the step**, in the header, the body,
  behind the loop or in another phi. Such a reader keeps the value of
  before the step alive next to the new one: the rotated loop then goes
  round with two values where it had one, pays a copy a trip for it, and
  where the old value is read at the bottom or behind the exit the copies
  do not fit into the latch and the back edge gets a block of its own (the
  rule was first asked of the header only: `expat_siphash24` +5.0% `Ir`, a
  word-copy loop that steps its pointer at the top and loads through the old
  one; with it the program is unchanged). The test of `while (n--)` is such
  a reader, which is why loop-test retargeting (next bullet) runs first.
  `while (n-- > 0)`, whose ordered test cannot be moved, and a body that
  reads the old value stay as they were. Over the regression corpus 163
  loops in 36 files are rotated this way, for 0.07% more compile time
  together with the retargeting and the phi facts below.
  `CCC_DEBUG_LOOP_ROTATE=1` says `computes what the loop reads: rotated so
  that it is bottom-tested`.

  The transformation itself takes multi-block bodies, several latches, exits
  from the body and nests: the header's instructions are copied into a single
  latch, and every reader of a header value gets the definition that reaches
  it through merges built on demand (Braun et al.). What it refuses is listed
  in `plan_rotation`, the only place that refuses.

  Switches: `CCC_NO_LOOP_ROTATE` or `CCC_LOOP_ROTATE=0|false|no|off` turns it
  off; `CCC_LOOP_ROTATE=1|true|yes|on|always` rotates *every* loop that can
  be, early in iteration 0 (`RotatePolicy::Always`). That mode is a stress
  test of the transformation, not an optimization level: measured against
  the default over the 91 benchmark programs it costs 2.5% in instructions
  (49 programs worse, `zlib_ng_slide_hash` 13 times its count,
  `percpu_ptr_walk` +33.7%; 35 better, `zlib_ng_huffman_heap` -9.3%,
  `linux_find_bit` -7.9%), which is why the default asks for a reason per
  loop. Measured for the default against the pipeline without it (87
  programs, cachegrind `Ir`, identical outputs): no program slower,
  `linux_rbtree` 2.45% faster; `*p += x` over 1,000 trips 1.81x faster,
  `a[i] = a[i-1] + b[i]` through parameters 2.4x faster, level with GCC 16.2.
  With loop-test retargeting, the phi facts and the late rotation on top
  (91 programs): 0.032% fewer instructions in total, `zlib_ng_adler32`
  -9.2%, `gzip_longest_match` -0.67%, `ra01_global_match` -0.62%, none
  worse; `while (n--) *d++ = *s++;` 0.405 s -> 0.348 s
  over 819 million trips (GCC 16.2, which keeps one induction variable
  where this compiler keeps three: 0.279 s).
  Gates: `tests/regression/check_loop_rotate_profit.sh` (the decision, read
  off the assembly in both directions, for the guarded motion and for the
  fed headers), `tests/regression/loop_rotate_profit_guarded_motion.c`
  (nothing that moved runs when the loop does not: unmapped and read-only
  operands, zero divisors, aliasing) and
  `tests/regression/loop_rotate_fed_header.c` (every value a body sees and
  what is left in the counter, for no trip, one, two and many; every integer
  width, `continue`, `break`, nests).
  Independently, the **ARM backend peephole rotates simple loops by default**
  (`rotate_simple_loops`, off only under `CCC_NO_LOOP_ROTATE`).
- **Loop-test retargeting** (`ir/mem2reg/phi_eliminate.rs`,
  `retarget_loop_tests`; in front of phi elimination, and in front of the
  late rotation above) — `do { … } while (--n);` arrives as `n1 = n - 1;
  n != 1`, because the mid-level compares the operands of a difference that
  is tested for zero. Then `n` and `n1` are alive at once, the copy between
  them stays, and the back edge gets a block of its own: `lea; cmp; je; mov;
  jmp` for what is `sub; jne`. A test `n ==/!= K`, or a branch on `n`
  itself, is made to read `n1 ==/!= K + k` when `n1` is `n` plus and minus
  constants, through the casts C puts around a narrow counter (at most
  `MAX_STEP_LINKS` = 4 links, each an integer type of at least the
  counter's width and at most 64 bits). `n1` has to be what the block hands
  the phi of `n`, or what the block computed ahead of the test. Equality
  only, because adding a constant to both sides is an identity in a ring
  and for nothing else; a 64-bit bound that would stop fitting a 32-bit
  immediate is left alone; every value involved must have one definition
  (the outputs of inline assembly count as definitions).
  Over the regression corpus 357 tests are moved, 124 of them through a
  cast, and (the late rotation included) the back edges that keep a block
  of their own for a one-block loop's copies go from 220 to 11.
  `LCCC_DEBUG_PHI_HOIST=1` prints `[PHI]
  loop test of block N: … becomes vM against K (step in the block | a
  dominating block[, through a cast])`.
  Test: `tests/regression/latch_test_retarget_wrap.c` (every width, both
  signs, steps of 1, 3 and 7 both ways, bounds at both ends of the range).
- **Phi facts in CVP** (`cvp.rs`, `phi_facts`) — a phi lies in the union of
  what its operands can be, each taken with its own edge's fact and with the
  facts of the block (at most `MAX_PHI_OPERANDS` = 16 operands; one operand
  of which nothing is known makes no fact). This is what carries a loop's
  guard into a bottom-tested loop: the body of a rotated `while (m != 0)`
  is entered from the guard and from the latch, each testing a value of its
  own, and sees their phi.
- **General complete unrolling** — nested/multi-block constant-trip loops
  with FP-aware expansion budget.
- **Tri-state FP contraction** — `FpContract { Off, OnExpr, Fast }`,
  default Off (GCC `gnu*` parity); FMA emission requires the FMA3 feature.
- **Identical basic-block merging** (`merge_identical_blocks`, ARM + RISC-V
  backend peepholes) — collapses duplicate blocks that a switch's phi
  trampolines, repeated guard sequences and cloned epilogue stubs leave behind.
  Ported from the x86 backend's `identical_blocks` pass.  Measured on the
  43-program corpus: **-45 / -30 instructions (ARM `-Os` / `-O2`)** and
  **-51 / -45 (RISC-V)**.

### Backend peepholes worth knowing about

- **Register-copy coalescing** — whole-function: renames a copy's destination
  onto its source everywhere and deletes the copy. Installed in **x86**
  (`passes/copy_coalesce.rs`), **ARM** and **RISC-V**
  (`coalesce_entry_copies`); i686 has its own equivalents. It is the only way
  to remove an entry-shuffle copy, whose destination is live to the end of the
  function, so no local pass can touch it.

  The copy must sit in the straight-line entry run, and then the admission
  question is whether the rename is safe afterwards. **x86 and ARM** answer it
  with exact backward dataflow: the source must be dead immediately after the
  copy, a later write to the source must not clobber the value, and a later
  write to the *destination* (the epilogue restore of a callee-saved register,
  typically) must not destroy whatever is still live in the source. **RISC-V**
  still uses the older syntactic rule — the source must not be mentioned
  anywhere else in the function — which is why it fires far less often.
  `scripts/coalesce_census.py` measures both against the emitted assembly.
- **Alias folding** (ARM `propagate_address_aliases`) — deletes `mov xD, xS`
  when dst is only ever used as an address base. Gated on an exact CFG query
  (`Cfg::redef_covers_all_reads`): every read of dst reachable *without passing
  the redefinition* must be a rewritable address use. A `ret` invalidates it
  for x0-x7, which carry the return value.

- **Dual-layout metric pairs** (`-O1` and up, **all four backends**) — when
  `block_layout`'s static chain cannot predict whether its reordering helps
  (measured: -42 instructions on `linux_rbtree`, +38 on `strlen_bench`), the
  backend emits both candidate orders, runs its full text peephole over each
  and keeps the smaller instruction count. The candidates are independent
  fragments, so the pair is split across a scoped worker and the caller. This
  is a compile-*latency* change only: every backend's metric peephole is a
  pure `text -> text` function, and generated assembly is byte-identical by
  contract, per backend, in the gate.

  The policy lives in `backend/metric_pair.rs` and is shared by x86-64, i686,
  AArch64 and RISC-V; each backend's `ArchCodegen::peephole_pair_for_metric`
  override supplies only its own `peephole_optimize`. `LCCC_PEEPHOLE_PAIR_PARALLEL`
  controls it: `0`/`off` never spawns, `1`/`on` always spawns on a multi-thread
  host (the A/B lever the gate uses to reach the concurrent path anywhere), and
  unset/`auto`/a typo keeps the heuristic — serial below
  `MIN_PARALLEL_PAIR_BYTES` (1 KiB combined, an 8x floor over the measured
  spawn crossover) or on a single-thread host.
  `CCC_TIME_PEEPHOLE=1` prints one
  `[pair] arch=… chain=…B orig=…B mode=… parallel=… serial=… deduped=…` line per
  pair, which is the only way to observe the split (both modes emit the same
  text). Identical candidates skip the second pass.

  `tests/regression/check_peephole_pair_parallel.py` pins, per backend and per
  pair, assembly identity across the three modes, the kill switch, the
  single-thread fallback, pair liveness, counter accounting
  (`parallel + serial + deduped` = pairs reported), **and reproducibility**
  (three compiles, one hash — the RISC-V identical-block pass used to iterate a
  randomly seeded `HashMap` and emit different labels on every run). It uses a
  header-free fixture that forces ten explorations per compile, so no cross
  sysroot is needed and no backend's verdict rests on one lucky pair; x86-64
  additionally compiles real benchmark TUs and repeats at `-O2`. `scripts/measure_peephole_pair.py`
  regenerates the timings. `CCC_DUMP_ASM` / `CCC_PEEPHOLE_TRACE` use `-chain`
  and `-orig` for the last distinct pair and `-dedup` for a pair evaluated
  only once. This keeps a later deduplication from replacing just one leg of
  the previous pair.

- **Phase-1 dirty-tick skipping** (x86-64, MB-20 step 5) — the peephole's
  phase-1 fixpoint re-runs a sub-pass only when a change was reported since
  that pass last started; the final convergence iteration (a pure
  re-verification sweep, measured at 27% of peephole time on a 40-function
  mixed TU) skips every pass that already proved itself clean on
  bit-identical input. Soundness contract documented on
  `backend::peephole_common::PassTicks`; the equality oracle lives in
  `peephole/passes/dirty_ticks_tests.rs`. Two knobs:
  `CCC_PEEPHOLE_NO_DIRTY_SKIP=1` restores the pre-tracker behaviour (for
  bisection, and required under `CCC_LCR_LIMIT`, whose process-global budget
  makes the opt-in relay pass call-count-dependent);
  `CCC_PEEPHOLE_VERIFY_DIRTY=1` digests the function state around every
  executed pass and panics if one mutates it while reporting "no change" —
  the one lie that would make a skip unsound. `CCC_TIME_PEEPHOLE=1` reports
  the per-iteration skip count on its `phase1 iteration` lines.

  The same step's trigger-presence rule gates the audited FP/vector pass
  family (28 pass names, 44 call sites across all phases) on one entry
  probe: a function whose text never names `%xmm`/`%ymm`/`%zmm` or
  `vzeroupper` cannot be mutated by passes whose rewrites are anchored on a
  parsed vector operand, so they are not invoked at all. The probe is
  monotone (no pass synthesizes a vector register into a function that has
  none), and it removed the 13.35% of compile instructions those passes
  spent scanning the all-integer 40-function TU. `CCC_PEEPHOLE_NO_VEC_GATE=1`
  disables the gate for bisection; the equality oracle covers both
  directions in `dirty_ticks_tests.rs`.

- **Identical basic-block merging** (`merge_identical_blocks`) — a block is
  deleted and its predecessors retargeted when its instruction text matches
  another block's and either (a) both have the **same non-empty predecessor
  set**, so the live-in state is identical, or (b) both are **live-in
  independent** — every register they read is defined inside the block — so
  the live-in state is irrelevant.  Additionally the deleted block must be
  entered only by explicit branches (a fall-through edge cannot be retargeted
  by renaming a label), must not be a function's entry block, and blocks whose
  predecessors are unknown (an indirect `br xN` jump table, or a `jalr`) are
  excluded rather than poisoning the whole function.

## Pass Order

The optimizer runs up to three full iterations. Each iteration runs this pipeline:

```
1.  CFG simplification      — remove dead blocks, thread jump chains, simplify branches
2.  Copy propagation        — replace uses of copies with original values
2a. Division-by-constant    — replace idiv/div with multiply-shift (iter 0 only)
2b. Integer narrowing       — shrink operation widths (e.g. i64 add → i32 add)
3.  Algebraic simplification — strength reduction, identity removal
4.  Constant folding        — evaluate constant expressions at compile time
5.  GVN + LICM + IVSR       — shared CFG analysis (computed once per function):
    ├── GVN                 — global value numbering / CSE across dominated blocks
    ├── LICM                — loop-invariant code motion
    └── IVSR                — induction variable strength reduction
7.  If-conversion           — convert branch+phi diamonds to cmov/Select
8.  Copy propagation        — clean up copies created by GVN, simplify, LICM
9.  Dead code elimination   — remove dead instructions (excluded from convergence check)
10. CFG simplification      — clean up after DCE empties blocks
10.5 IPCP                   — interprocedural constant propagation (all iterations)
```

**Phase 0** (before the loop):
- Tail-call elimination (TCE) — converts self-recursive tail calls to loops
- Function inlining (including the **post-structural** inlining round after
  TCE/rec2iter)
- `recursion_to_iter` (rec2iter) — non-tail binary recursion to iterative
  accumulators
- mem2reg
- Initial constant-fold/copy-prop round
- **Vectorization** (iteration 0 only) — transforms reduction loops to AVX2/SSE2 SIMD

**Phase 11** (after the loop):
- dead static function elimination — removes `static inline` functions that
  became unreferenced after inlining
- **Outline switch** — large cold switches lowered out of line when beneficial

Additional passes in the pipeline (see `src/passes/`): `resolve_asm` (inline-asm
symbol resolution), `vector_temp_promotion` (SIMD temp slot promotion), and the
standalone `univsr` (universal induction-variable strength reduction) pass that
runs under explicit control.

**PGO** (`src/pgo/`) wraps the optimizer: with `-fprofile-generate` it
instruments the post-optimization IR; with `-fprofile-use` it drives inlining,
loop unrolling, block/function layout, switch lowering, and indirect-call
devirtualization from a runtime profile. See the PGO section below.

## Convergence

Iterations stop when:
- No pass made any change (fixpoint reached), **or**
- This iteration made fewer than 1/20th the changes of the first iteration (diminishing returns).

DCE changes are excluded from the convergence check because DCE's large removal counts inflate the first-iteration baseline and cause premature exit.

## Pass Interactions

The passes form a dependency graph. LCCC uses a `should_run!` macro to skip passes when their upstream passes made no changes in the previous iteration:

| Pass | Only re-runs if... |
|------|-------------------|
| CFG simplify | constfold changed (constant branches) or DCE changed (empty blocks) |
| Copy prop | CFG, GVN, LICM, or if-convert changed |
| Simplify | Copy prop or narrowing changed |
| Constfold | Copy prop, narrowing, simplify, or if-convert changed |
| GVN | CFG, copy prop, or simplify changed |
| LICM | CFG, copy prop, or GVN changed |
| DCE | GVN, LICM, if-convert, or copy prop2 changed |

## Key Passes

### GVN (Global Value Numbering)

Eliminates redundant computations across basic blocks. If two blocks both compute `a + b` where `a` and `b` have the same value, GVN replaces the second with a copy of the first. Operates on `BinOp`, `UnaryOp`, `Cmp`, `Cast`, `GetElementPtr`, and `Load` instructions within the dominator tree.

GVN, LICM, and IVSR share a single `CfgAnalysis` (dominator tree + loop nesting) per function per iteration, saving significant compile time on large translation units.

### LICM (Loop-Invariant Code Motion)

Hoists computations out of loops when their operands don't change within the loop. Identifies natural loops via the dominator tree, then moves invariant instructions to loop preheaders. Critical for inner-loop performance on code like matrix multiply.

What LICM, store forwarding and register promotion never touch is an access
the program is observed through: a load or store that is `volatile`, and
since PR #1208 one that reaches an `_Atomic` object, by any spelling (a
typedef such as `atomic_bool`, the qualifier, `_Atomic(T)`, a member, a
pointee, a cast). Another thread may write between two reads of an atomic
object, so `while (!atomic_flag) ;` has to read on every trip; before, the
flag was memory like any other and the read was hoisted. The frontend marks
such an access volatile and records that nobody wrote `volatile`, so the
qualifier-conversion diagnostics stay those of GCC (`void *p = &atomic_obj`
is silent). A cast through a typedef of a volatile type, `*(vu32 *)REG`,
keeps the typedef's qualifier as well. Gate:
`tests/regression/check_volatile_access_semantics.sh`, section 6b.

The *operators* on an atomic object are atomic operations too (C11
6.5.2.4p2, 6.5.16.2p3, 6.2.6.1p9). They used to be the three steps of a
volatile object's update, and `refs++` from two threads lost counts
(`tests/regression/atomic_operators.c`: 229,349 of 400,000 increments
survived). `_Atomic` is now part of what a type records (`PtrQuals::ATOMIC`),
sema tells the lowering which lvalues designate an atomic object
(`SemaResult::atomic_lvalues`), and `src/ir/lowering/atomic_access.rs`
chooses the operation:

| Source | IR | x86-64 at `-O2` |
|---|---|---|
| a read of `x` | `AtomicLoad` | `mov` (AArch64 `ldar`) |
| `x = v` | `AtomicStore` | `xchg` (AArch64 `stlr`) |
| `x++;` `x -= v;` `x \|= v;`, result unused | `AtomicRmw` | `lock add` / `lock sub` / `lock or` |
| `++x`, `x += v`, result used | `AtomicRmw` | `lock xadd` |
| `x *= v`, `x <<= v`, `d += v` on a `double`, a `_Bool`, `-ftrapv` | load + `AtomicCmpxchg` loop | `lock cmpxchg; jne` |

The retry branch of the loop reads ZF from `lock cmpxchg` itself, and an
`AtomicRmw` whose result nobody reads -- `atomic_fetch_or` with its value
dropped included -- is one locked ALU instruction instead of `lock xadd` or a
five-instruction loop. Covered: integers and pointers on every target
(sixteen bytes through libatomic's `_16` entry points, as GCC does, so
`-latomic` at the link), `float` and `double` on x86. Not covered: `long
double`, complex and structure objects, which keep a volatile object's
accesses where GCC calls libatomic's generic `__atomic_load`,
`__atomic_store` and `__atomic_compare_exchange`; and a floating update does
not save and restore the floating-point environment around its loop (Clang
does not either; GCC does). Gate: `tests/regression/check_atomic_operators.sh`.

Because the type says atomic, the frontend also follows GCC and Clang where
it used to be silent: a pointer to an atomic object does not convert to a
pointer to the plain type or back (an error), `_Generic` tells `_Atomic int
*` from `int *` and `volatile int *` (it took the first association of the
right shape), and an atomic bit-field is refused. One spelling is still read
short: `volatile` written beside `_Atomic` on the same object is not
recorded as written, so `_Generic(&v, volatile _Atomic int *: ...)` selects
the `_Atomic int *` association and `void *p = &v;` does not report the
discarded `volatile`.

### IPCP (Interprocedural Constant Propagation)

When a function is always called with the same constant argument, IPCP specializes the function for that constant and folds the resulting dead branches. Important for Linux kernel code (`IS_ENABLED()`, `cpucap_is_possible()` chains) where `static inline` wrappers gate large blocks of dead code.

### Division-by-Constant

Replaces `idiv`/`div` instructions (20–90 cycle latency on modern CPUs) with multiply-and-shift sequences. Runs only on the first iteration, before narrowing and constant folding can further simplify the expanded sequence. Disabled on i686 where 64-bit multiply overflow semantics differ.

## Disabling Passes

For debugging, individual passes can be disabled:

```bash
CCC_DISABLE_PASSES="gvn,licm" ./target/fastbuild/lccc input.c -o output
CCC_DISABLE_PASSES="all"      ./target/fastbuild/lccc input.c -o output
```

Pass names: `cfg`, `copyprop`, `narrow`, `simplify`, `constfold`, `gvn`, `licm`, `ifconv`, `dce`, `ipcp`, `inline`, `ivsr`, `divconst`, `sccp`, `jump_thread`, `jump_thread_early`, `join_store_forward`, `phi_relay_fold`, `ret_dup`, `loop_memset`, `vector_fill`, `tce`, `tce_acc`, `rec2iter`, `self_inline`, `boolthread`.

Timing data is available via:

```bash
CCC_TIME_PASSES=1 ./target/fastbuild/lccc input.c -o output 2>&1 | grep PASS
```

## Performance-Relevant Knobs (measured defaults)

The tree carries 300+ environment knobs; most are debug switches with
neutral defaults. Two change CODE GENERATION and have measured,
data-backed defaults — a build that flips either silently changes the
benchmark standings:

| Knob | Default | What it controls | Why this default |
|---|---|---|---|
| `CCC_IVSR_SCALAR_DERIVED=1` | **off** (opt-in) | The scalar derived-IV flavor of `ivsr`: rewrites loop-recurrence-derived scalars (rbtree subtree pointers) into IV increments. | Measured net LOSS at default-on: linux_rbtree +1.2%, expat +4.0%, glibc_strstr +4.3%, five golden workloads past the codegen-quality bands (data in `src/passes/iv_strength_reduce.rs`; re-verified 2026-09-24 on x86-64: expat opt-in ≈+20%, glibc_strstr ≈+6.5%, linux_rbtree tie). Pinned opt-in by `tests/regression/check_ivsr_scalar_derived_default.sh`, which also proves the knob stays LIVE (knob-set and knob-unset asm must differ) so it cannot silently rot into a no-op. |
| `CCC_NO_STRIDED_REDUCE` | **unset** (vectorizer on) | Kill switch for the strided-2 i32 sum-reduction vectorizer (`vectorize_strided_i32_reduce.rs`). Set to any value (including empty) to disable the transform and keep the counted scalar loop. | The transform is gated on `x86_simd_available()` and the same `force_two_wide` / late-minmax exclusions as the rest of the vectorizer; there is deliberately no `force_sse2` exclusion because the reducer selects its own width (`x86_avx2_available() && !sse2_forced()` yields the 32-byte X8 schedule, otherwise the SSE2-safe X4 / X4Compact schedules), and its bound proof is the scalar loop's own footprint; the switch exists for bisection parity with the other vectorizers, not as a supported configuration. |
| `CCC_NO_ANDN_FUSION` | **unset** (fusion on) | Kill switch for the Not+And → 3-operand `andn` fusion (BMI1) and, with it, the middle-end CH-mux defer that leaves the Not for the backend to fuse. | The fused shape is the GCC/Clang/ICX four-oracle consensus for SHA-256's CH (`tests/regression/check_ch_maj_codegen.sh` pins exactly one `andn` under `-mbmi`, zero without). The switch exists for bisection parity with the other branch fusions, not as a supported configuration. |
| `CCC_NO_SELF_ZEXT_FALLTHROUGH` | **unset** (fallthrough tracking on) | Presence, including an empty value, restores the conservative conditional-branch barrier in the x86-64 self-zero-extension pass. Labels, unconditional transfers, unknown directives and clobbers remain barriers in either mode. | This is a diagnostic A/B control for the fallthrough policy, not a supported target or optimization level; the rest of the zero-extension pass remains active. |

The andn knob is resolved ONCE per `run_passes` (the `scalar_andn_available`
spelling shared by both `bit_idioms` tiers) and re-read at emission
(`supports_and_not`), so fold-time prediction and emission-time decision
cannot drift within one compilation.

## Debug / explain knobs added with the S07-S08 work

These three are pure diagnostics: each gates a trace the compiler otherwise
computes nothing for, none changes emitted code, and each is named here because
`scripts/audit_census.py` requires every `CCC_*`/`LCCC_*` literal in `src/` to be
documented in a tracked markdown file.

| Knob | What it shows |
|---|---|
| `CCC_DEBUG_VOLRED=1` | `volatile_reduce`'s interleave budget: how many lanes the pass admitted, which bound (lane count vs. budget) bound the group, and why a candidate was dropped. Budget exhaustion is a normal outcome, not a defect, so it is silent by default. |
| `CCC_DEBUG_STATIC_INIT=1` | The static-initializer builder's decision for an element it cannot fold: which evaluator ran (scalar constant, sema's folded value, address constant, byte fill), and the offset each one wrote. |
| `CCC_DEBUG_TYPES_COMPATIBLE=1` | `__builtin_types_compatible_p`'s verdict for each query with the two resolved types in canonical form, i.e. exactly the pair the predicate compared after qualification stripping and parameter-name erasure. |
| `CCC_DEBUG_INTCONST=1` | `int_const_hoist`'s per-loop decisions: the function and loop under consideration, whether a preheader exists, the wide-constant candidates found in every-iteration blocks, and which survived the leaf-loop pressure rule. Written for the silent failure mode where the hoist fires but a later cleanup re-folds it — compare this trace against the final assembly. |

Debug traces are opt-in and rate-limited by construction: each is a
`std::env::var_os` probe per decision site, so a knob left exported in a
benchmark or kernel environment changes nothing.  That is a checked invariant,
not a promise: `tests/regression/check_debug_knobs_neutral.py` compiles a
fixture that reaches all three decision sites at `-O0/-O1/-O2/-O3` with the
knob unset and set and requires the emitted assembly to be identical, and
requires the knob name to still be present in the compiler binary so a knob
cannot rot into dead documentation.

## Verifying IR Between Passes

`CCC_VERIFY_IR` runs a structural check on the IR after **every** pass, so the
stage it names is the pass that *produced* the malformed IR rather than a later
pass that tripped over it:

```bash
CCC_VERIFY_IR=1     ./target/fastbuild/lccc input.c -o output   # report to stderr
CCC_VERIFY_IR=abort ./target/fastbuild/lccc input.c -o output   # panic on the first violation
```

It checks that block labels are unique, that every branch and `asm goto` target
names a real block, that phis form a contiguous prefix of their block, and that a
phi's incoming labels are exactly its CFG predecessors — no stale entries, no
duplicates, none missing.

That last check matters more than it sounds. Phi elimination resolves an operand's
label to a block *index* and emits the copy there, so a phi naming a block that is
no longer a predecessor still "works" whenever that block happens to dominate. A
`loop_rotate` defect hid behind exactly that accident until SCCP — which trusts the
predecessor list and prunes operands on edges it proves dead — deleted an induction
variable's initialisation. See
`../engineering/journal/2026-08.md`.

When a violation names a stage, the *producer* is not always that stage: the
pipeline tail after the last checkpoint (live-range splitting, entry-parameter
prefix restoration, global location allocation) is not followed by another one,
so the next compile stage inherits already-drifted IR and the report names an
innocent consumer. `CCC_VERIFY_DUMP_BLOCK=1` prints the offending block on the
violation path — one row per instruction with its span and inline-instance id —
which identifies the producer directly:

```bash
CCC_VERIFY_IR=abort CCC_VERIFY_DUMP_BLOCK=1 ./target/fastbuild/lccc -O2 -g input.c -c
```

| Knob | What it shows |
|---|---|
| `CCC_VERIFY_DUMP_BLOCK=1` | The violating block's instruction/span/inline-id rows. Read only when a violation is already being reported, so it changes no decision and no emitted code; with `CCC_VERIFY_IR` unset it is never read at all. |

Verification is off by default and costs one atomic load per pass when disabled.

## Debugging SCCP

SCCP performs four independent rewrites. Each can be suppressed on its own, which
makes bisecting a miscompile a single rebuild rather than a bisect over the pass:

| Variable | Suppresses |
|---|---|
| `CCC_SCCP_NO_PRUNE` | pruning phi operands on provably-dead edges |
| `CCC_SCCP_NO_DEFS` | rewriting definitions to `Copy { dest, Const }` |
| `CCC_SCCP_NO_SUBST` | substituting constants into operands |
| `CCC_SCCP_NO_FOLD` | folding `CondBranch`/`Switch` into `Branch` |
| `CCC_SCCP_TRACE_PRUNE` | prints every pruned phi operand and its edge verdict |

## LCCC-Specific Passes

LCCC adds two optimization passes that run before CCC's main optimizer loop.

### Tail-Call Elimination (`tce`)

Converts self-recursive tail calls to back-edge branches. A tail call is a recursive call whose result is returned immediately — `return f(args)` with no further computation.

```c
// Before: 10M stack frames
long sum(int n, long acc) {
    if (n <= 0) return acc;
    return sum(n - 1, acc + n);
}

// After TCE: tight counted loop (identical to GCC output)
long sum(int n, long acc) {
loop:
    if (n <= 0) return acc;
    acc += n; n -= 1; goto loop;
}
```

TCE runs once after inlining, before the main optimization loop, so that LICM, IVSR, and GVN can subsequently optimize the resulting loop.

**Pass name:** `tce` (disable with `CCC_DISABLE_PASSES=tce`)

**Implementation:** [`src/passes/tail_call_elim.rs`](https://github.com/ms178/lccc/blob/main/src/passes/tail_call_elim.rs)

### Phi-Copy Stack Slot Coalescing (backend)

This is a backend optimization in `src/backend/stack_layout/copy_coalescing.rs`, not a pass in the traditional sense. It runs during stack layout, before code generation.

When CCC's phi elimination lowers SSA phi nodes to Copy instructions, it creates separate stack slots for the phi destination and its backedge update value. For a 32-variable loop, this generates ~20 redundant stack-to-stack `movq` pairs per iteration.

LCCC detects the phi-copy pattern — where the source is defined and killed in the backedge block — and aliases the source to use the phi destination's wider-live slot. The Copy becomes a same-slot no-op and is dropped by `generate_copy`.

**Result:** `arith_loop` (32 variables): 550 → 507 assembly lines; 0.124s → 0.104s.

## LCCC-Specific: Reduction Vectorization

**Added in Phase 8** — LCCC detects and transforms reduction loops into AVX2/SSE2 SIMD operations.

### What Gets Vectorized

Simple reduction patterns:
```c
// Sum reduction
double sum = 0.0;
for (int i = 0; i < n; i++)
    sum += arr[i];

// Dot product
double dot = 0.0;
for (int i = 0; i < n; i++)
    dot += a[i] * b[i];
```

### What deliberately stays scalar

- **64-bit multiplies** (I64/U64 `Mul`, in a map or as a dot-product lane op).
  No SSE2/AVX2 instruction produces a 64x64 low-half product (`vpmullq` is
  AVX-512DQ), so `VecMulI64x2` is a 13-instruction emulation per **two** lanes —
  6.5 instructions per lane against one scalar `imulq` — and the emulated
  value's live range is long enough that its operand also lands in a stack
  slot. callgrind on `unsigned_reduction`'s `dot_u64`: 291,009,947 instructions
  against 202,784,913 for GCC 16.2's **scalar** loop (+50.3M `movq` lane
  extractions, +77.6M `movdqu` spills, +33.6M `pshufd`). Refusing the shape
  drops that program to 175,665,816 instructions (ratio 0.866 vs GCC) with
  identical results. 64-bit **sums** keep their packed form: `paddq` is a real
  instruction and that shape wins. `tests/regression/check_unsigned_reduction_codegen.sh`
  pins both halves of the rule.
- **Strict-FP reassociation.** A horizontal FP add tree changes the summation
  order, so FP reductions either unroll as a *threaded* chain (see the
  two-block reduction unrolling note) or wait for `-ffast-math`
  (`fp_reassoc`) — never a reassociated packed tree.

### Transformation Strategy

1. **Pattern detection**: Identifies loops with a scalar accumulator PHI and a single reduction operation
2. **Loop splitting**: Divides loop bound by vector width (4 for AVX2, 2 for SSE2)
3. **Vector body**: Replaces scalar ops with vector intrinsics (VecLoad, VecAdd, VecMul)
4. **Horizontal reduction**: Extracts scalar from final vector (`vextractf128` + `vunpckhpd` + `vaddsd`)
5. **Remainder loop**: Handles `N % vec_width != 0` with scalar operations
6. **Correct return**: Exit block returns scalar from remainder loop, not vector from main loop

### Backend Implementation

Vector values are treated as first-class SSA values that:
- Get unique, never-reused stack slots (protected from slot recycler)
- Are excluded from GPR allocation (forced to stack)
- Use direct slot access in intrinsics (no pointer indirection)
- Support vector-to-vector Copy via ymm/xmm registers

### Generated Code (AVX2 Example)

```asm
vxorpd %ymm0, %ymm0, %ymm0          # Zero vector accumulator
.loop:
    vmovupd (%rax,%rcx), %ymm0      # Load 4 doubles
    vaddpd %ymm1, %ymm0, %ymm0      # Add 4 doubles
    ; loop back...

; Horizontal reduction
vextractf128 $1, %ymm0, %xmm1       # Extract high 128 bits
vaddpd %xmm1, %xmm0, %xmm0          # Add high + low (2 doubles each)
vunpckhpd %xmm0, %xmm0, %xmm1       # Unpack high double
vaddsd %xmm1, %xmm0, %xmm0          # Final scalar

; Remainder loop (scalar)
.remainder:
    movsd (%rbx,%r13,8), %xmm0      # Load single element
    addsd -24(%rbp), %xmm0          # Add to scalar accumulator
    ; loop back...

; Return scalar result
movsd -24(%rbp), %xmm0              # Return scalar (not vector!)
```

### Why It Beats GCC

GCC's auto-vectorizer is conservative on simple reductions:
- Worries about aliasing even with clear array indexing
- Considers the pattern "too simple" to benefit
- Falls back to 2× scalar loop unrolling

LCCC's pattern-based approach:
- Explicitly targets common reduction idioms
- Aggressively transforms when pattern matches
- Generates complete vectorization (4× for AVX2 vs GCC's 2× unroll)

**Result**: LCCC vectorizes patterns GCC -O3 leaves scalar, achieving ~2.7× speedup.

### Debug Flags

```bash
LCCC_DEBUG_VECTORIZE=1    # Show vectorization transformations
LCCC_DEBUG_PROTECT=1      # Show stack slot protection decisions
```

---

## PGO (Profile-Guided Optimization)

PGO is implemented in `src/pgo/` and wraps the pass pipeline. It uses the
GCC-compatible flag surface: `-fprofile-generate[=dir]`,
`-fprofile-use[=dir]`, `-fbranch-probabilities`, `-fauto-profile`, and
`-fprofile-update=atomic|single`.

### Profile generation (instrumentation)

`instrument.rs` instruments the *post-optimization* IR so no optimization pass
ever sees a counter. For each function it:

1. builds the CFG (with a virtual EXIT node for `return`-terminated blocks);
2. chooses the edges to count with a **maximum-weight directed arborescence**
   (backedges and cycle edges stay off the counter path; every non-entry node,
   including loop latches, keeps exactly one incoming tree edge);
3. emits a single `incq sym+off(%rip)` per counted edge (or a `lock`-prefixed
   increment under `-fprofile-update=atomic`), splitting critical edges so a
   counter never sits between a fused `cmp` and its branch;
4. instruments **indirect-call value profiling** (top-4 callee slots per site).

At runtime the counters are dumped to `lccc-<unit>-<pid>.profraw` files (or the
`LCCC_PROFILE_FILE` / `LLVM_PROFILE_FILE` override) and merged deterministically.

### Profile use

`profile.rs` loads the `.profraw` text format (multi-file merge, version-tolerant
parsing, per-file corruption tolerance) and `derive_block_counts` recovers every
block count and tree-edge count from the instrumented edges by **flow
conservation**.

**Identity & drift:** profiles are keyed by a PRE-pass CFG fingerprint (stable
across generate/use). A POST-pass fingerprint detects CFG drift from PGO-guided
transforms; drifted functions keep their stable entry count for hot/cold
*sections* but get **no** stale edge-derived data (fail-closed).

**Profile summary** (`summary.rs`): an LLVM `ProfileSummaryInfo` analogue. The
hot/cold thresholds come from the count distribution, and `has_spread()` tests
whether one function uniquely dominates the runner-up — the gate that keeps a
**flat** profile from perturbing the hot path.

### Consumers

- **Inlining** (`inline_pgo.rs`): percentile hot/cold classification,
  label-independent entry-count-ratio force-inline for hot loop sites, bounded
  force-inline budget, and a working hotness threshold multiplier.
- **Loop unrolling** (`unroll_pgo.rs`): derived trip count (`backedge / entry`)
  drives whether hot high-trip loops unroll and cold low-trip loops do not.
- **Layout** (`layout.rs`): hot/cold function sections, profile-driven switch
  lowering (dominant case hoisted out of a jump table; cold switches forced to a
  compare chain), switch-case ordering, and conservative
  (preserve-source-order) block layout that never perturbs register allocation.
- **Devirtualization** (`promote.rs`): cost-aware indirect-call promotion — a
  site whose top target is ≥95% of calls is already BTB-predicted and left
  indirect; genuinely multi-valued sites are promoted to a guarded direct call.
- **Backend branch inversion**: the hot successor of a `CondBranch` is made the
  physical fall-through so the hot path takes no branch, without reordering
  blocks.

### Debug / tuning knobs

```bash
CCC_INLINE_DEBUG=1          # explain inliner decisions
CCC_INLINE_PARAMREF_DEBUG=1  # trace parameter-reference remapping during inlining
LCCC_DEBUG_LAYOUT=1          # layout decisions
LCCC_DEBUG_PROMOTE=1        # devirtualization decisions
LCCC_PGO_NO_LAYOUT=1        # disable PGO layout
LCCC_PGO_PROMOTE_STABLE=95  # top-share % treated as "already predicted"
LCCC_PGO_HOT_FRAC / LCCC_PGO_COLD_FRAC  # summary percentiles
```

The PGO A/B harness — `tests/benchmark/run_pgo_ab.py` — builds plain and
profile-guided binaries for lccc and a reference compiler, verifies differential
output equivalence, and reports paired bootstrap-CI speedups.

## Exact FP recurrence reuse and late branch threading

- **`loop_square`**: first O2+ optimizer iteration on x86-64/SIMD, outside size
  mode; reuse one existing next-square per innermost loop. Signed-zero seed,
  dominance, SSA, single-latch and stable FP-environment proofs prevent
  speculation/reassociation. Six FP header phis reject the pressure heuristic.
  Disable: `CCC_DISABLE_PASSES=loop_square`.
- **`fp_double`**: x86-64/SIMD O2+ (including size modes), after scalar algebra and
  before FP constant hoisting. Exact `2*x` to `x+x`; protect copy-connected
  add/sub consumers only when permitted target FMA contraction can occur.
  Disable: `CCC_DISABLE_PASSES=fp_double`; timings: `CCC_TIME_PASSES=1`.
- **`jump_thread`**: late x86 peephole after `identical_blocks`, before
  `epilogue_merge`; retarget local branch-only relays, preserving labels/code.
  Only inert debug attribution crosses directive boundaries. Disabled for
  size-oriented codegen until layout-aware growth control is available.
  Disable: `CCC_PEEPHOLE_SKIP=jump_thread`.
- **`late_branch`**: final x86-64 text sweep; remove a direct local branch
  when both outcomes reach the same instruction without crossing a section,
  inline asm, an encoding directive, or a compiler marker. Disable just this
  sweep with `CCC_PEEPHOLE_SKIP=late_branch`; the other phase-9 cleanups remain
  active.

The floating transforms are not fast-math identities. Architecture expansion
requires equivalent rounding/environment proofs and target-specific measurement.

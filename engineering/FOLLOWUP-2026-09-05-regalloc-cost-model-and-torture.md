# Follow-up work — 2026-09-05 — register-allocator architecture & torture-suite defects

Base: `ms178/lccc` main @ `4927a77403ea080ac0a44daa718a9c7ffede53aa`
(re-fetched at the end of the session; this is still the tip of `origin/main`,
so the patch is rebased against the latest main by construction)
Deliverable: `ms178-1.patch` (cumulative, applies clean on the base)
Snapshot ledger: `artifacts/SNAPSHOT_LEDGER.md`

---

## 1. Accomplished this session

### 1.1 Correctness — every GCC C torture failure on x86-64 and i686 was root-caused and fixed

Baseline (pristine base, `gcc.c-torture/execute`, 1676 sources × `-O0,-O1,-O2,-O3,-Os`):

| target | cases | baseline pass | baseline run-fail | after |
|--------|-------|---------------|-------------------|-------|
| x86-64 | 8380 | 8361 | **7** | **8368 pass, 0 run-fail** (5 ref-compile-skip, 7 ref-run-skip) |
| i686   | 8380 | not runnable (no multilib in the sandbox) | **8** once runnable | **8355 pass, 0 run-fail** (20 ref-compile-skip, 5 ref-run-skip) |

The i686 corpus is now runnable at all (the sandbox needed `gcc-multilib` +
`libc6-dev-i386`; `scripts/x86_gcc_torture_i686.py` itself needed no change).
Without them the runner reports 1676 `reference-compile-skip` and passes
**vacuously** — check the pass count, not the exit status.

The fifteen failures reduced to **five independent root causes**, each a genuine
miscompile with a wider blast radius than the test that exposed it.

---

#### RC-1 — `iv_widen` admitted an int→float **conversion** into the integer widening closure

*Exposed by* `20060420-1.c` (`-O2`, `-O3`). *Blast radius:* every
`K * (float) i` / `(double) i` in a counted loop with an addressing use of `i`.

`iv_widen.rs` classified a cast as a widening extension with

```rust
to_ty.size() >= 4 && to_ty != Ptr && is_unsigned_ty(to) == is_unsigned_ty(from)
```

`F32` satisfies all three (size 4, not `Ptr`, and `is_unsigned_ty` answers `false`
for both `F32` and `I32`, so the "same sign" test succeeds). The cast was therefore
**dropped** and its consumers re-classified as *integer* closure members:

```
BinOp { v11 = v10 * F32(11.0) : F32 }      ->  BinOp { v11 = i:I64 * I64(0) : I64 }
```

`widen_const(F32(11.0))` goes through `to_i64()`, which yields `0`. Every such
product silently evaluated to **zero**.

Fix — the closure is an *integer* recurrence, enforced at every admission point:

* `Cast`: require `to_ty.is_integer()` before `WidenCast`; int→float now falls into
  the `NarrowCast` arm, which correctly retains it as `Cast { from_ty: wide_ty }`
  (i.e. `sitofp i64`), so the conversion is both correct *and* one instruction shorter.
* `BinOp` / `Cmp` / `Select`: an `!ty.is_integer()` gate placed **before** the
  `ty.size() >= 8` "wide chain op" shortcut — `F64`/`D64` also satisfy `size >= 8`
  and slipped through it.
* `widen_const` now returns `Option` and refuses non-integer constants instead of
  fabricating `0`.
* New `plan_is_integral()` re-checks the whole plan against the live IR inside
  `verify_plan`, before any mutation. `NarrowCast` is exempt by design (its
  `orig_ty` is the conversion *target*).
* `escape_read_needs_narrow` was unconditionally letting `Cast`/`Cmp`/`Phi` read the
  widened value; it now checks the reader's own declared type
  (`wide_read_is_type_correct`). This was a second, independent instance of the same
  width lie, caught by the new regression test's `escape_f32` case.

Test: `tests/regression/iv_widen_float_conversion_closure.c` (10 shapes: f32/f64/long
double, signed and unsigned IVs, both operand orders, mul/add/sub/div, a float select,
a float compare, the IV escaping into a float, and the exact `20060420-1.c` shape with
a cold `noreturn` call to keep the vectorizer away). **Verified to fail on the unfixed
compiler** and pass at `-O0..-Os` and against GCC.

---

#### RC-2 — call arguments recorded a **storage-level** type instead of the value's real type

*Exposed by* `pr59229.c`, `pr109938.c`, `pr109986.c` (all `-O0`).
*Blast radius:* every `size_t` parameter fed by an `int` expression.

`get_expr_type` answers a storage-level question and reports `I64` on LP64 for an
`IntLiteral` operand, so `i + 1` was typed `I64` even though the emitted `BinOp`
correctly carries `ty: I32`. That disagreement went straight into
`CallInfo::arg_types`:

```
__builtin_memcmp(p, q, i + 1)
  ->  args: [.., v:I32],  arg_types: [.., I64]     (no conversion at all)
```

At `-O0` the value lives in a 4-byte slot and the 8-byte argument load pulled the
**adjacent slot** in as its high half:

```asm
movl %eax, -36(%rbp)     # length (4 bytes)
movl $1,   -32(%rbp)     # neighbour
movq -36(%rbp), %rdx     # %rdx = 0x1_00000007   <-- memcmp runs off both buffers
```

With a prototype in scope the same lie produced `Cast { from_ty: I64, to_ty: U64 }`
— a same-width **no-op** where C requires `int -> size_t` to sign-extend first
(C11 6.3.1.3), so a negative length was zero-extended.

Fix:

* `expr_calls.rs` and the `BuiltinKind::LibcAlias` path now derive the argument's
  source type from `value_ir_type` — the query whose own doc comment says it exists
  for exactly this ("conversion boundaries must use `value_ir_type`"). It differs
  from `get_expr_type` only for `BinaryOp` and `!x`, i.e. precisely the expressions
  whose materialised width can be narrower than their storage width.
* New `builtins::libc_alias_param_types()` gives the `mem*`/`str*`/`*_chk` families
  their real signatures, so a `__builtin_*` call performs the parameter conversions
  even in a TU that never declared the libc symbol.

Test: `tests/regression/builtin_size_t_arg_width.c`.

---

#### RC-3 — the ABI **static-chain register** was allocatable across a nested call

*Exposed by* `920501-7.c` (`-O1`, `-O2`, `-O3`, `-Os`). *This is an RA defect.*

`SetStaticChain` is lowered as a direct write of `%r10` (x86-64) / `%ecx` (i686)
immediately before the call. It carries no IR dest, so nothing tells the linear scan
that the register is redefined there:

```asm
movl 48(%rsp), %r10d
subl $1, %r10d          # a-1 lives in %r10
movq %r11, %r10         # SetStaticChain -- clobbers a-1
movq %r11, %rdi         # arg0 relayed through the now-equal r11
call x.y                # y(chain) instead of y(a-1): infinite recursion -> SIGSEGV
```

It failed at **any** recursion depth, so the `dg-add-options stack_size` framing was
misleading; this was never a stack-depth problem.

Fix: `reserve_static_chain_reg()` removes the chain register from
`available_regs` / `caller_saved_regs` / `call_arg_regs` / `indirect_target_regs`
for functions that actually contain a `SetStaticChain`. Functions with only
`GetStaticChain` (a nested callee that never calls a sibling) are deliberately
unaffected — the chain is read once at entry into an ordinary home, and reserving
there would cost a register for nothing.

Per-point modelling of one hard register was rejected: it would add interference
surface to every ordinary function for a construct that is vanishingly rare.

Tests: `tests/regression/nested_fn_static_chain_regalloc.c` (recursion + non-local
goto, five live values across a nested call, loop-carried state across a nested call,
a nested call in a conditional) plus three unit tests in `regalloc_helpers.rs`.

---

#### RC-5 — a fused div/rem pair was allowed to have the **same** flavour

*Exposed by* eight i686 tests at `-O0`/`-O1`: `20040811-1.c`, `vla-dealloc-1.c`,
`pr43220.c`, `981001-1.c`, `20000511-1.c`, `ssad-run.c`, `usad-run.c`.
*Blast radius:* any basic block containing two div-like operations on identical
operands that CSE did not fold.

One `idivl`/`divl` yields the quotient in `%eax` and the remainder in `%edx`, so a
fused pair is only meaningful when the two consumers want **different** halves.
Every emitter derives the register split from the *head's* flavour alone —

```rust
let div_dest = if self_is_div { *dest } else { Value(partner_dest) };
let rem_dest = if self_is_div { Value(partner_dest) } else { *dest };
let _ = partner_from_eax;   // "flavour already encoded in the dest split above"
```

— i.e. they all *assume* the flavours differ (the AArch64 partner map does not even
carry the tail's flavour). `compute_i686_divrem_pairs` never enforced it: it paired
any two div-likes in a block with identical operands and matching signedness. Two
same-flavour operations compute the *same value* — a CSE miss, not a fusion
opportunity — and the tail was then stored from the wrong register:

```asm
movl %edi, %eax      # %edi held n / 1000, but this is x[n % 1000]
shll $2, %eax
addl -24(%ebp), %eax
movl $2, (%eax)      # index up to 999 into an (n % 1000 + 1)-element VLA
```

The three VLA tests looked like stack-exhaustion failures (they carry
`dg-add-options stack_size`), but they failed at **any** recursion/iteration count:
the `%esp` save/restore around the VLA was correct all along, and the SIGSEGV was
this out-of-bounds store.

Fix: require `div_flavor(op_i) != div_flavor(op_j)` when pairing, in the shared
`compute_i686_divrem_pairs` (so x86-64 and AArch64 are covered too), and turn the
emitter's silent assumption into a `debug_assert_eq!(partner_from_eax, !self_is_div)`.

Test: `tests/regression/divrem_pair_opposite_flavour.c` — two `SRem`s, two `SDiv`s,
two `URem`s, two `UDiv`s, three-of-a-kind followed by a legitimate opposite-flavour
pair, both orders of the pair that *should* fuse, the constant-divisor
(magic-number) head path, and the exact repeated-`n % 1000`-indexes-a-VLA shape,
all over negative/zero/large operands against `volatile` references.

---

#### RC-4 — a 256-bit reduction accumulator declared itself 128-bit

*Exposed by* `tests/regression/vector_guard_sum.c`, which was **already failing on
the pristine base** (`nat=279` vs GCC's `651`). Found by running the repo's own
regression suite; verified pre-existing by rebuilding at `4927a77`.

`IntrinsicOp::vector_result_width()` is the single authority for both the protected
slot size and the width of every copy the backend emits for a vector value.
`VecMaskedAddI32x8` folds eight I32 lanes into an I32x8 accumulator (`vpaddd %ymm`),
but it had been grouped with its 128-bit **widening** sibling
`VecWidenMaskedAddI32x4ToI64x2` (whose I64x2 result genuinely is 128-bit).

Consequence: the accumulator's loop-carried copy was emitted as legacy
`movdqa %xmm3, %xmm2`, which **preserves** bits 255:128 of the destination. The upper
four lanes never advanced:

```
low half   = A[0..3] and A[8..11] masked  = 1 + 36+43+50+57 = 187
high half  = never accumulated            = 0
scalar tail A[16]                         = 92
                                            187 + 92 = 279   <-- observed
```

The generalized unit test written for the fix then found **three more instances of
the same copy-paste defect** in the same list: `VecLoadI64x4`, `VecAddI64x4` and
`VecZeroI64x4` (4 × i64 = 32 bytes) were also declared 16, and
`VecHorizontalAddI64x4` was listed at all — violating the table's own documented
invariant that `VecHorizontalAdd*` reduces to a **scalar** and must never appear.
Those four are latent today (x86 has no lowering for the `I64x4` family and
`vec_interleave` deliberately excludes it), but they are loaded guns; all are fixed.

Tests: two unit tests in `src/ir/intrinsics.rs`, one pinning the two masked
reduction steps, one asserting generally that the declared width agrees with the lane
count in the operation's name and that no `VecHorizontalAdd*` claims a vector width.

---

### 1.2 Register allocator — the cost model became position-relative and per-use

This is the architectural half of the session. It changes *what the allocator
believes a spill costs*, which is the heart of any allocator, without touching the
backend's value→location model (so it carries no plumbing risk).

**Three structural defects in the old model.**

1. **One loop-depth scalar for a whole range.**
   `priority = (Σ_uses pgo_weight) × 10^min(max_loop_depth, 4)`.
   The depth is the *maximum* over the def block and all use sites, applied uniformly
   to every use. A value with one use at depth 3 and twenty uses outside is priced
   exactly like a value with twenty-one uses at depth 3. Both GCC
   (`REG_FREQ_FROM_BB`) and LLVM (`MachineBlockFrequencyInfo`) weight **per use site**.

2. **A global number used at a position-dependent decision.**
   `select_evict_victim` compared `incoming.priority > victim.priority`. At scan
   position `p`, the victim's uses *before* `p` are sunk cost — they were already
   served from a register. Comparing lifetime totals systematically over-protects a
   victim that is nearly dead.

   This is the defect the repo previously groped at twice with a bolted-on
   "zero-future-use dead victim" override (once as a ranking override, once as a
   last resort). Both were measured and reverted because they perturbed the eviction
   cascade in unrolled kernels (zlib-ng Adler DO8: 59.9 → 70.4 ms). They failed
   because they were special cases layered on an *unchanged* global order. Making the
   whole order position-relative subsumes the case: a dead victim is simply the
   cheapest one, and the profitability guard is expressed in the same currency as the
   ranking.

3. **The spill-weight denominator was the fat envelope**, not the occupied length,
   even though hole-aware segment coverage is already computed (RA-05).

**What landed** (`src/backend/live_range.rs`):

| addition | meaning |
|----------|---------|
| `use_weights: Vec<u64>` | per-use frequency: `10^min(depth(block(use)),4) × pgo(use)` |
| `suffix_cost: Vec<u64>` | suffix sums, so `remaining_cost` is a binary search + one index |
| `cost_boost: u64` | policy multiplier for reads invisible in the IR use chain |
| `occupancy_len: u32` | Σ segment lengths, maintained by `set_segments` |
| `remaining_cost(pos)` | weighted cost of uses **strictly after** `pos` |
| `total_cost()`, `spill_cost_at(pos)`, `occupied_points()` | derived queries |
| `set_uses_weighted()` | joint sort, duplicate use points **sum** their weights |
| `point_loop_depths()` | the program-point → block-depth table (LLVM's `MBFI` analogue) |

`select_evict_victim` gained **mode 6**, which keeps every soundness guard of mode 3
(steal-safety, never evict an ABI-hinted value, victim's next use must be past
`incoming.end`) and swaps only the *currency*: `spill_cost_at(incoming.start)`
instead of the global `priority`.

The three policy boosts in `regalloc.rs` (`bump_coalesce_group_priority`,
`bump_folded_index_priority`, `bump_gep_base_priority`) now also scale `cost_boost`,
so folded SIB indices, folded GEP bases and coalesce-group leaders keep their
structural weight in the new currency.

**Backwards compatibility is exact.** When `use_weights` is empty — hand-built unit
test ranges, synthetic vector intervals, the Phase-2b span ranges — every accessor
degrades to the unit-weight count, so `remaining_cost(pos) == future_uses(pos)`
pointwise. `set_uses` clears any stale cost table. A dedicated unit test asserts the
equivalence at every position, and another asserts that modes 1–3 reach an identical
decision whether or not weights happen to be attached.

**Default is unchanged (`CCC_EVICT_MODE=3`).** Mode 6 is opt-in until it has a
measured win on the benchmark corpus. Given this repo's documented history of
eviction-knob regressions, flipping the default without evidence would be exactly the
mistake the negative-results ledger warns about. See §2.1.

12 new unit tests in `live_range.rs` cover the cost table, its degradation path, the
`cost_boost` saturation behaviour, `occupied_points`, and four mode-6 behaviours
(evicts the value dead at the scan point; refuses a hotter *remaining* value; ranks by
per-use frequency rather than max depth; keeps the mode-3 soundness guards).

---

### 1.3 Validation

| gate | result |
|------|--------|
| `scripts/run_regression_suite.sh` | **624 PASS / 0 FAIL / 7 SKIP**, AB-diff failures 0 (baseline was 622 PASS / 1 FAIL) |
| `gcc.c-torture/execute` x86-64, `-O0..-Os` | see §1.1 |
| `gcc.c-torture/execute` i686, `-O0..-Os` | see §1.1 |
| `cargo test --lib` (new tests) | 12 cost-model + 3 static-chain + 2 vector-width + 3 libc-signature |
| new regression tests | 5 files, each verified to FAIL on the unfixed compiler and to match GCC at `-O0..-Os` on both targets |

Every fix was snapshotted immediately after validation (`artifacts/SNAPSHOT_LEDGER.md`),
and `/home/user/ms178-1.patch` is refreshed on every snapshot.

---

## 2. To-do — ordered by expected value

### 2.1 Measure mode 6 and decide the default  *(highest value, lowest risk)*

The machinery is in and tested; only the evidence is missing. Required before flipping
`evict_mode()`'s default from 3 to 6:

1. `scripts/ra_quality_census.py --json` with `CCC_EVICT_MODE` unset vs `=6`, over
   `tests/benchmark/programs` + `tests/benchmark/kernel_corpus`. The census buckets
   (`stkref`, `rrmov`, `push`, `insns`) are deterministic and are the right primary
   signal on a VM with no PMU.
2. `tests/benchmark/run_benchmarks.py --reps 21` paired, for the workloads the
   BACKLOG names as RA-sensitive: `gzip_crc32`, `zlib_ng_adler32`, `nbody`,
   `spectral_norm`, `mandelbrot`, `expat_xml_scan`, `sqlite_varint`.
   Point a wrapper script at `--ccc` that sets `CCC_EVICT_MODE=6` to get paired
   randomized rounds from the existing harness.
3. Guard-rails: gzip must not regress (mode 5 died here), and the Adler DO8 loop must
   not regress (the dead-victim experiments died there).

Expect the largest movement in functions with long-lived values whose uses cluster
early — `longest_match`'s preheader values are the canonical case.

### 2.2 RA-06 — reload-at-next-use / in-place splitting

Still the #1 generated-code gap (BACKLOG: 3–5× spill traffic). The cost model
delivered here is the missing *input* to it: a Belady/next-use-distance spiller needs
exactly `remaining_cost` and a next-use distance, both of which now exist per range.

Recommended architecture (unchanged from the session's analysis, now better
supported):

* The volatile-alloca IR dance in `split_ranges.rs` is an explicit anti-goal and the
  current heuristic is very narrow (simple GPR type, non-phi, all call sites in one
  block, `uses >= 4*calls+5`).
* Prefer decoupled **spill-then-color** (Braun–Hack, CGO 2009) over LLVM's
  greedy-with-last-chance-recoloring: compute register pressure per program point,
  and where pressure exceeds `k` apply Belady MIN using *global next-use distances*
  to pick the value to spill, materialising the store at the spill point and the
  reload immediately before the next use as a fresh SSA name. Phase 2 (the existing
  scan / Tier-2 colorer) then runs on IR that is approximately `k`-colorable.
* The one real blocker remains unchanged: the assignment result is a single
  value→location map, so a value cannot be in a register at one point and a slot at
  another. Splitting in the IR (new SSA names) sidesteps that, which is why the
  IR-level formulation is the right one for this codebase.

### 2.3 Extend the cost model with rematerialization

`spill_cost_at` currently prices every value's spill identically per weighted use. A
constant or a `GlobalAddr` costs almost nothing to spill because it can be
rematerialized, and the backend already has `CCC_NO_GLOBAL_ADDR_REMAT`. Add a
`RematKind` field populated from the defining instruction and scale `spill_cost_at`
by it. This is LLVM's `LiveRangeEdit::canRematerializeAt` in miniature and directly
serves BACKLOG RA-01/RA-01b.

### 2.4 Use `occupied_points()` in `calculate_spill_weight`

`occupancy_len` is now maintained but `calculate_spill_weight` still divides by the
fat envelope. Switching the denominator is a one-line change that was deliberately
*not* made here because it alters mode-3 behaviour and therefore needs its own
measurement round. Do it as an isolated A/B.

### 2.5 Per-use cost classes

A use that can fold into a memory operand (`addl slot, %reg`) is much cheaper to
spill than one that needs a reload into a register (a SIB base, a shift count). LLVM
models this via `mayFoldMemoryOperand`. `RegAllocConfig::folded_index_uses` and
`never_materialized` already carry most of the needed information; feeding it into
`use_weights` would sharpen the model further.

### 2.6 Remaining unfinished items from the session brief

Not reached; the session was spent on the four miscompiles and the cost-model
architecture, which were prerequisites (a defect-free baseline is required before any
performance measurement is trustworthy).

* Godbolt oracle comparison runs (`scripts/godbolt.py compare`,
  `scripts/codegen_oracle.py`, `scripts/tune_oracle.py`) against GCC 16.2 /
  Clang 23.1 / ICX / ICC. **The oracle is live and verified reachable from the
  sandbox** (`scripts/godbolt.py list --filter 'gcc 16.2'` resolves `cg162`), so this
  is ready to run.
* The linker oracle's build/version preferences.
* mold (X86+i686-only CMake preset) — not installed in this sandbox; the fastbuild
  script correctly falls back to `gcc` + `bfd`. No `clang` either.
* The BACKLOG's "10 worst results" table (gzip `longest_match` 118 stack refs,
  zlib-ng Adler ~1.50×, nbody/spectral/mandelbrot 1.23/1.31/1.23, …) — these are
  RA-01/RA-02/RA-03/RA-06 items and are gated on §2.1/§2.2.

---

## 3. Environment notes for the next agent

* Rust **1.98.0** via rustup in `/home/user/.cargo`; **every new shell must
  `export PATH=/home/user/.cargo/bin:$PATH`** (the `bash` tool sources no shell rc).
* 8 GiB `/swapfile` is active (`scripts/ensure_swap.sh`, which `build_lccc_fast.sh`
  calls itself).
* 2 cores, 1.9 GiB RAM. `scripts/build_lccc_fast.sh` does a full build in ~2 min and
  an **incremental rebuild in ~10–45 s** — the REACT cycle is fast; use it.
* No `clang`, no `mold`, no `icx`, no `qemu`. `gcc` 14.2.0 is the local oracle;
  Compiler Explorer covers the rest and **is reachable**.
* `gcc-multilib` + `libc6-dev-i386` were installed via `sudo apt-get`, which works.
  Without them the i686 torture runner reports 1676 `reference-compile-skip` and
  passes **vacuously** — check the pass count, not just the exit status.
* The GCC torture corpus lives at
  `/home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute` (1676 sources), provisioned
  by `scripts/ensure_gcc_torture.sh`. A full 5-level run is ~11 min per target at `-j2`.
* Useful diagnosis recipe used repeatedly this session:
  `CCC_DISABLE_PASSES=<pass>` and the `CCC_NO_*` kill switches bisect a miscompile to
  a single pass in seconds; `CCC_DUMP_EACH_PASS=1 CCC_DUMP_FUNC=<fn>` then shows the
  exact IR delta that pass produced.
* Always confirm whether a regression-suite failure is **pre-existing** by rebuilding
  at the base commit (`git stash push -u -- src/ && git checkout <base> -- src/`)
  before attributing it. `vector_guard_sum` was pre-existing; assuming otherwise
  would have sent the session chasing its own changes.

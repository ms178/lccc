# Follow-up: review-response round — merged PR #772 hardened, and one review finding measured and reverted

**Date:** 2026-10-07
**Base:** upstream `main` `6a9690c356efeda5b8bbfd496baee5d04f13e4c6` (merge of PR #772)
**Reviewed work:** `c325949267ad635dbe79e894ff403f1765332bcd`, tree `abef682c0fc8aaf5c7dfd8813fc528cae85cd5ac`
**Evidence:** [`evidence/2026-10-07-ivsr-review-hardening/`](evidence/2026-10-07-ivsr-review-hardening/)

An external review of PR #772 returned **CONDITIONAL MERGE**, score 8/10, with 18
findings (1 high, 5 medium, 10 low, 2 trivia) and a "10/10 reachable" note. The
work is already merged, so this round is a post-merge hardening pass. Every finding
was re-derived from the source rather than accepted, and the review's central
high-severity claim plus one of its medium claims did not survive contact with a
compiler and a measuring tape.

The headline result is not the hardening. It is that **one of the review's
recommendations is a measured performance regression**, and that following it cost
three regression-test files their vector-loop pointer induction. It is reverted,
with the assembly, and the reason it is safe to keep the code the review called
unsafe is now written down where the decision is made.

---

## 1. Adjudication of all 18 findings

"Verified" below means re-derived from the merged source and, where the finding is
about behaviour, from a build and a measurement — not read and agreed with.

| # | Sev | Claim | Verdict | Evidence |
|---|---|---|---|---|
| A1 | high | `unsigned_iv_bound` sets `hi = limit − 1` unconditionally; for a descending IV the body's values are `[limit+1, init]`, so `hi` is below the real range and `offset_product_cannot_overflow` can admit a wrapping narrow-U32 product | **AGREE — real defect, fixed and then improved past the fix** | §2.1. Reachability is nil today (§4.1) but the proof was wrong and is now right |
| A2 | medium | the guard is `modular_iv && size() >= 4 && is_unsigned()`, so sub-32-bit unsigned IVs skip the proof (fail-open) | **AGREE — confirmed at HEAD line 288, fixed** | §2.2. This round's *first* adjudication of it was wrong and is corrected below |
| A3 | medium | `is_used_as_address` delegates to `reads_pointer_arg()`, which over-approximates (true for pure vector ops); over-approximation is the unsafe direction for an address test | **DISAGREE — implemented it, measured it, reverted it** | §3: +7 insns/+4 stack on `simd_crc_adler`, lost pointer induction in 3 files |
| A4 | medium | `init_offset` reinterprets a constant via `v as u32` while the bound uses raw `to_i64()`: two notions of a constant's value | **AGREE — fixed** | §2.3: one helper `const_in_iv_domain`, both call sites |
| A5 | low | the product certificate's boundary behaviour at the ring top deserves an explicit test | **AGREE — pinned** | `offset_product_certificate_decision_table` |
| B1 | medium | the `CCC_NO_IVSR_PTR_ADD` kill switch has **zero** automated coverage; both test wrappers hardcode `ptr_add = true`; the test named `..._gated_by_the_kill_switch_and_ilp32` only exercises ILP32 | **AGREE — all three parts confirmed, all three fixed** | §2.4 |
| B2 | medium | the oracle's `affine_view` indexes `p[i + 0x80000000]` where `i` is a parameter and `(void)i` — loop-invariant, reads one element n times, exercises nothing | **AGREE — confirmed and fixed; checksum re-published** | §2.5 |
| C1 | medium | `ivsr_signed_wrap_impldef.c` is described as a reproducer | **AGREE — it is a pin**: it printed `OK` on the base at `-O1/-O2/-O3` | docs §1 of the 2026-10-06 report, now split into reproducers vs pins |
| C2 | medium | the merged commit message says `result_type()` "now returns F32/F64 unconditionally" but the hunk is comment-only (34+/0−) | **AGREE — confirmed by diffstat and by provenance**: the arm shipped in `dd012799` | `git log -S` in §2.6; the message is upstream and immutable, so the correction lives in the source comment |
| C3 | low | "66 sources × 6 configs = 402" | **AGREE — it is 67** (55 benchmark + 12 oracle) | fixed in the perf follow-up §4 |
| C4 | low | the ablation histogram lists +5/+12/+1/+2/+1 = 21 but says "(+15 total)" | **AGREE — the `sqlite_varint` −6 term was omitted**; 21 − 6 = 15 | `src/common/types.rs`, now spelled out |
| C5 | low | `callgrind-kernel-ab.json` is filed under the 2026-10-06 directory but measures the Oct-7 change; its `why_not_wall_clock` claims runtime is unmeasurable | **AGREE — both fixed**; wall clock *was* measured and is the confirmation metric | moved to `evidence/2026-10-07-perf-ivsr-ptradd/` |
| C6 | low | the `result_type()` consumer audit's census is stale | **AGREE — 18 sites in 10 files**, the 10th being test-only | census now in the comment, re-verified |
| C7 | low | arithmetic/phrasing in the perf report | **AGREE where checkable** | §4 table re-measured this round: 402 comparisons, 98002 → 98002 insns |
| C8 | trivia | the repro block does not say the clone must be full | **AGREE — and it was missing two more prerequisites** | 32-bit glibc headers (§5.2), and the sandbox-local patch path |
| C9 | trivia | the PR body is not in the repo | **AGREE — no action**: it is upstream in the merge commit; noted only |
| C10 | low | "Two shipped benchmark shapes segfault" | **AGREE — the claim is false and was corrected in two files**: the two shapes were *written for* the audit; no pre-existing benchmark has an index that crosses `UINT32_MAX` | 2026-10-06 report §2.2 and `backlog.md` |

Accepted and unchanged, per the review's own "do not" list: the two-predicate
split, the ILP32 gate, FP homes staying opt-in, the two reverted experiments
(SELECT-CHAIN-FOLD, RANGEFOLD-OR-2), and no "simplification" of any proof
predicate.

---

## 2. What changed

### 2.1 A1 — the descending bound now comes from the init, not from the limit

`unsigned_iv_bound` proved two separate things and returned them in one struct:

1. the IV **cannot wrap** (the header test guards every taken backedge, is in the
   IV's own unsigned type, has the polarity matching the step sign, and steps by
   one); and
2. the IV's values lie in a **numeric interval** `lo..=hi`, which is what lets
   `offset_product_cannot_overflow` certify that `iv * stride` cannot wrap a
   narrower ring.

For an ascending IV (`i < limit`, step +1) the body sees `[init, limit − 1]`, so
`hi = limit − 1` is the real maximum. For a descending IV (`i > limit`, step −1)
the body sees `[limit + 1, init]` — and `limit − 1` is not merely loose, it is
*below* the range. A certificate built on it checked `100 * 4` for a loop whose
executed maximum was `2^30 * 4`.

The review's recommendation was a conservative 4-line fail-closed: mark such a
bound inexact and withdraw the certificate. That was implemented first, and it
works — but it throws away the descending case entirely. So the bound is now
**derived properly**:

```rust
let floor = limit + 1;                       // from the header test
match iv.init {
    Operand::Const(c) => (floor, max(init, floor), exact),  // certified
    Operand::Value(_) => (floor, max, false),               // withdrawn
}
```

Three consequences, all pinned by tests:

* `for (unsigned i = 64; i-- > 0;) p[i]` — every value in `[1, 64]`, `64 * 4` fits
  U32 — is **certified and now fires**, where the fail-closed version declined it.
* `for (unsigned i = n; i-- > 0;) p[i]` has no provable numeric maximum, so the
  certificate is withdrawn and the loop keeps the modular index recurrence it had
  before this work. **No regression, because there was nothing to regress.**
* The IV no-wrap half of the proof is untouched: entry still requires
  `init > limit` and the floor is still `limit + 1 >= 1`.

An init at or below the limit never enters the body, so the interval is empty;
`hi = max(init, floor)` is a sound over-approximation of an empty set and is
pinned rather than left implicit.

`descending_unsigned_iv_bounds_come_from_the_init_not_the_limit` pins the
predicate; `descending_unsigned_address_loop_end_to_end` pins the whole 2×2 table
(direction × where the scaling happens) through `ivsr_function`.

### 2.2 A2 — confirmed, fixed, and this round mis-adjudicated it first

The merged source at HEAD line 288:

```rust
let unsigned_bound = if modular_iv && (iv.ty.size() as usize) >= 4 && iv.ty.is_unsigned() {
```

so a `U8`/`U16` IV skipped `unsigned_iv_bound` **entirely** and continued with
`unsigned_bound = None`. Whether that is fail-open then depends on
`offset_product_cannot_overflow`, whose own exemption is a *different* guard:

```rust
let ptr = crate::common::types::target_ptr_size();
if !mul_ty.is_integer() || (mul_ty.size() as usize) >= ptr || !mul_ty.is_unsigned() {
    return true;   // exempt
}
```

Note `mul_ty`, not `iv.ty`: it is the type of the **derived offset**, which for a
narrow counter is usually the pointer width after the widening cast. So a `U8` IV
whose offset arithmetic happens in `I64` reached the transform with *no* no-wrap
proof and an *exempt* product check. That is precisely the fail-open the review
described, and the reason it has not been observed is an invariant of a different
function: `find_basic_ivs` rejects the truncating backedge cast that C's integral
promotion produces for a sub-`int` counter (the IVSR-WRAP-1 fix), so such an IV is
never formed. Relying on that here is fail-open by construction.

Now:

```rust
let unsigned_bound = if modular_iv && iv.ty.is_unsigned() {
```

Every modular unsigned IV below the pointer width — `U8`, `U16`, `U32` — must
present a proof, and `unsigned_iv_bound`'s own rule (the header test must be in the
IV's own type) declines a promoted narrow counter, because C17 6.3.1.1p2 puts its
test in `int`. Failing closed costs nothing here, and the change is inert on the
corpus: 0 of 1480 comparisons changed (§5.1).

**This round's first adjudication of A2 was wrong, and the error is worth keeping
visible.** Reading the review's `>= 4` against `offset_product_cannot_overflow`
instead of against its caller makes the finding look like a misquote — the
predicate's exemption really is `>= target_ptr_size()`, and a pre-existing test
(`offset_product_proof_needs_an_exact_bound_only_below_the_pointer_ring`) really
does pin that. Both statements are true and they are about **two different
guards**. The finding was checked against the wrong one, was declared a misquote,
and was only caught when the actual diff was read line by line and the removed
`>= 4` showed up in the caller. The correction is recorded in the source comment at
the call site, in `offset_product_certificate_decision_table` (which now opens by
naming both guards), and here.

The pointer-width exemption in the predicate is *sound at any width*, including
ILP32 where `U32` is the pointer width: if `iv * stride` wrapped the pointer ring
then the source's own `p[iv]` would not point into or one past any object, which
C17 6.5.6p8 already makes undefined — so no defined program can distinguish the two
recurrences. One further cell looks like a hole and is not: a narrow **signed** IV
is admitted with no bound, because a signed product that wraps is UB in the source
(C17 6.5p5) and an executable that reaches this code cannot have one.

### 2.3 A4 — one meaning for "the value of a constant"

`IrConst` carries every integer in a signed `i64`, so a `U32` limit of
`0xFFFFFFFF` can arrive as `I32(-1)` and `to_i64()` reads `-1`. The initial-offset
computation reinterpreted through the IV's type; the bound did not, and would have
certified `hi = -2` for a limit of `0xFFFFFFFF` — then *wrongly admitted* an
overflowing product, because `-2 < 2^32` passes.

`const_in_iv_domain(raw, ty)` is now the single definition, used by both sites.
The frontend's constant evaluator normalises unsigned constants to a non-negative
`I64`, so this is unreachable from C source today; it is fixed because a proof
whose two halves disagree about what a constant *means* is not a proof. An exact
offset too large for an `IrConst::I64` operand now skips the transform instead of
truncating (needs `init * stride >= 2^63` bytes).

### 2.4 B1 — a kill switch with no tests is not a kill switch

All three parts of the finding were confirmed:

* both test wrappers (`ivsr_function`, and the fixture-driven one) hardcoded
  `ptr_add = true`;
* the test named `address_add_arm_is_gated_by_the_kill_switch_and_ilp32` exercised
  **only ILP32** — its own comment admitted it;
* the switch's only prior exercise was a manual gzip A/B.

Fixed at both levels:

* `ptr_add_parameter_is_a_real_kill_switch` drives the parameter directly
  (`ivsr_with_analysis(.., false)` vs `.., true)`), so it needs no environment
  mutation and therefore no serialisation against the rest of the suite;
* the misleadingly named test is renamed `address_add_arm_is_gated_by_ilp32` and
  now says what it covers;
* `check_ivsr_domains.sh` compiles **and runs** all five IVSR regression programs
  at all four optimisation levels with `CCC_NO_IVSR_PTR_ADD=1` — the environment
  spelling a user would actually type. The gate is green on the merged base and on
  this tree (20 configurations, exit 0 both).
* the two dead wrappers were removed rather than left as unused indirection.

### 2.5 B2 — the oracle's affine case did not test anything

```c
for (uint32_t k = 0; k < n; ++k) s += (uint64_t)p[i + UINT32_C(0x80000000)] * (k + 1u);
(void)i;
```

`i` is a parameter, the subscript does not involve `k`, and `(void)i` was the tell.
The loop read **one element n times**: the header described an affine IV view and
the body exercised a loop-invariant load. Now:

```c
for (uint32_t k = 0; k < n; ++k)
    s += (uint64_t)p[(i + k) + UINT32_C(0x80000000)] * (k + 1u);
```

With the caller's `i = 0x80000000` the subscript is `(0x80000000 + k) + 0x80000000
== k` modulo 2^32, so `n <= 6` keeps every access inside the 8-element array while
the compiler still sees the full affine unsigned form.

This changes the program's checksum, so the four-vendor oracle was **re-run** and
both JSONs re-published: 9 agree / 0 diverge / 0 error, LCCC 1512 instructions vs
GCC 1852, Clang 2274, ICC 3714, ICX 2441. `ivsr_index_domains` itself moved
163 → 165 (LCCC) while Clang went 343 → 348, ICC 338 → 376 and ICX 370 → 458 —
the repaired case is real work for everyone, and it is still the program where
LCCC's ratio against GCC is worst (1.398). Shipping the old checksums against the
new source would have been a stale-evidence defect of exactly the kind this round
exists to remove.

### 2.6 C2 — provenance, checked rather than asserted

```
$ git log --oneline -S "VecExtractLaneF32x4" -- src/ir/instruction.rs
dd012799 Fix IV strength-reduce wraparound, inlined va_arg_pack_len remap, and gate scalar FP lane-extract homes
$ git log --oneline -S "audited 2026-10-06" -- src/ir/instruction.rs
c3259492 Teach IVSR LCCC's own addressing form: ...
$ git show --stat c3259492 | grep -E 'instruction|loop_carried'
 src/ir/instruction.rs            | 34 ++++++
 src/passes/loop_carried_forward.rs |  9 ++
```

34 insertions, 0 deletions: comment-only. The F32/F64 returns shipped in
`dd012799`. The squashed message for `c3259492` was written by the merge, is
upstream and is immutable, so the correction is recorded in the source comment
itself, where anyone reading the arm will see it.

---

## 3. A3 — implemented, measured, reverted

The review's reasoning is tidy: `reads_pointer_arg()` is the allowlist for a
callee-**read-only** proof, where over-approximation is the safe direction, so it
is deliberately true for pure vector arithmetic (`VecAddF64x4` and friends, via
`produces_vector_value()`). Used as an *address* test, the same
over-approximation calls a never-dereferenced value an address. Narrowing it to
`may_read_memory()` is a one-word change.

It was made. Then the tree was A/B'd against the merged base over
`tests/regression` with each file's own flags:

| file | flags | insns | stack refs |
|---|---|---|---|
| `simd_crc_adler.c` | `-msse4.2 -mpclmul` | 711 → **718** | 68 → **72** |
| `simd_vecreg.c` | `-msse2` | 153 → **155** | 40 → **42** |
| `temp_promotion_window.c` | `-O2` | 90 → 89 | structurally worse, see below |

`simd_vecreg`, before and after:

```asm
; merged base                                    ; with the narrowing applied
    leaq -272(%rbp), %r9      ; hoisted              movslq %r8d, %r9
.L:                                              .L:
    movdqu (%r9), %xmm0                            leaq -272(%rbp), %rdi
    movdqa %xmm0, %xmm4         ; register           leaq (%rdi,%r9,4), %rdi  ; re-derived
    ...                                              movdqu (%rdi), %xmm0
    leaq 16(%r9), %r9           ; p += 16            movdqu %xmm0, -304(%rbp) ; SPILLED
                                                     ...
                                                     movdqu (%rdi), %xmm1     ; RELOADED
                                                     movdqu -304(%rbp), %xmm0 ; RELOADED
```

The narrowing did not merely fail to fire an optimization; it removed a **hoisted
pointer recurrence** and replaced it with a per-iteration index re-derivation, then
lost the register residency of the loaded vector and turned it into a spill/reload
pair. `temp_promotion_window` lost one instruction but also lost its
`pushq %r13` / `leaq 248(%rsp), %r13` / `leaq 16(%r13), %r13` recurrence and a
17-instruction unrolled byte-sum — smaller and worse, which is why instruction
count alone was not accepted as the verdict.

Reverted, and the soundness argument for keeping the wide predicate is positive
rather than merely pragmatic:

* `is_used_as_address` can only arm a recurrence on a value that
  `try_lower_pointer_arithmetic` already placed in **pointer-width** arithmetic;
* at pointer width the integer and pointer rings agree for every program without
  UB — a wrapping `base + i*elem` is UB in the source per C17 6.5.6p8, so no
  defined program can distinguish the two recurrences;
* the narrow-ring product, where the rings genuinely differ, is guarded separately
  and explicitly by `offset_product_cannot_overflow` (§2.2).

So over-approximation here costs nothing and buys the vector induction. Both the
call site and `address_classification_deliberately_over_approximates_intrinsics`
now carry that argument plus the measured numbers, and the test pins the divergence
between the two predicates so the choice stays visible to the next reader instead
of looking like an oversight.

After the revert: **1480 assembly comparisons, 0 changed** (§5.1).

---

## 4. New findings this round that the review did not make

The review could not compile or run anything. Four of the five findings below need
a build, an IR dump or a self-A/B, and the fifth is the largest open performance
item in this subsystem.

### 4.1 Descending loops get no induction-variable strength reduction at all

```c
unsigned g(const unsigned *p) { unsigned s = 0;
    for (unsigned i = 64; i-- > 0;) s += p[i]; return s; }
```

```
$ CCC_IVSR_DEBUG=1 lccc -O2 -S -o /dev/null desc.c
[IVSR] header=1 no basic ivs
[IVSR] header=1 no basic ivs
```

The frontend emits the recurrence as `Sub`, not as `Add` with a negative constant:

```
block 5 (header): v7 = Sub(v22, 1) : U32 ; v11 = Cmp Ne (v22, 0) : U32
block 6 (body):   ... ; v22 = Copy(v7)   ; backedge
```

and `find_basic_ivs` matches **only** `IrBinOp::Add` with a constant step
(`src/passes/iv_strength_reduce.rs`, the `if *op == IrBinOp::Add` arm), so the `i--`
and `i -= 1` spellings above form no phi at all.

> **CORRECTION 2026-10-07, audit-response round (finding F1).** This section went
> on to claim that *no descending loop in any C program gets an IV recurrence*,
> generalising from the single `i-- > 0` spelling shown above. **That reason is
> false.** A 16-program probe against the merged compiler shows the `i += -1`
> family *does* form a `BasicIV`; the full table, the mechanism and the two
> fail-closed gates that keep A1's descending arm unreachable anyway are in
> [`FOLLOWUP-2026-10-07-ivsr-audit-response.md`](FOLLOWUP-2026-10-07-ivsr-audit-response.md)
> §2, and the implementation consequences are filed under PERF-4 in
> [`../backlog.md`](../backlog.md).
>
> What survives is a narrower claim: no descending **recurrence** fired in any of
> the 16 probes — but because the derived-expression collector found no offset to
> collect (a backwards byte walk is already SIB-indexed; where an offset does
> exist it is scaled after a widening `Cast`, which is PERF-6), not because no phi
> was recognised. The severity conclusion for A1 is unchanged and now rests on
> measured ground instead of an assumed invariant.

The generated code shows where the real cost is. LCCC vs GCC 14.2, `-O2`,
`revvarint` (descending varint decode), per iteration:

```asm
; LCCC — 11 instructions, 2 branches                ; GCC — 7 instructions, 1 branch
.LBB6:                                              .L10:
    movq %rdx, %r9        ; copy                        movzbl (%rdx), %ecx
    shlq $7, %r9          ; shifted in a temp           salq $7, %rax
    movl %r8d, %r8d       ; REDUNDANT zero-extend       andl $127, %ecx
    movzbl (%rdi,%r8), %r11d                            orq %rcx, %rax
    andq $127, %r11                                     movq %rdx, %rcx
    movslq %r11d, %r10    ; REDUNDANT sign-extend       subq $1, %rdx
    movq %r9, %rdx        ; copy back                   cmpq %rcx, %rdi
    orq %r10, %rdx                                      jne .L10
    movq %r8, %rsi        ; copy
    jmp .LBB5             ; EXTRA BRANCH
```

Ranked by value, with the cause named:

1. **No loop rotation** — the test sits at the top with an unconditional `jmp` back
   to it, so two branches per iteration instead of one. GCC rotated it.
2. **Redundant extension elimination** — `movl %r8d, %r8d` zero-extends a value
   that `leal -1(%rsi), %r8d` already zero-extended (x86-64 32-bit ops do this by
   definition), and `movslq %r11d, %r10` sign-extends a value that `andq $127`
   already made non-negative. The IR literally carries a `Cast U32 -> U32` for the
   first one. GCC tracks `nonzero_bits`; LCCC does not.
3. **Copy/2-addressing around the shift** — three instructions
   (`movq`/`shlq`/`movq`) where GCC does one in-place `salq $7, %rax`.
4. **IVSR `Sub` support** — would give the descending pointer recurrence, but note
   the load is *already* SIB-indexed (`movzbl (%rdi,%r8)`), so this alone buys
   little here.
5. **Vectorization** — GCC's `g` is a 4-wide SSE reduction at ~1.75 insns/element
   against LCCC's 7. This dominates everything above and is a much larger project.

Item 2 is the same residue diagnosed earlier for `classify` (2.057× `Ir`) and
`namechars` (1.653×) as "backend boolean materialisation, RA/copy-coalescing
class". This round produced a minimal, crisp reproducer for it, which is the
missing ingredient for a fix. It is filed in `backlog.md` with the assembly above.

Nothing here was attempted: each item is a distinct pipeline or backend change
whose only validator is a gate this VM is instructed not to run, and starting one
mid-review-response would have left both unfinished.

### 4.2 The narrow-scaled offset shape is not collected in either direction

`find_derived_exprs` does not collect an offset that is scaled **inside** the
narrow ring and widened afterwards — `(I64)(i << 2)` — while it does collect
widen-then-scale, `(I64)i * 4`. Probed both directions rather than assumed:

```
PROBE descending=false narrow_shl fired=0     PROBE descending=false wide_cast fired=1
PROBE descending=true  narrow_shl fired=0     PROBE descending=true  wide_cast fired=1
```

Direction is irrelevant, so this says nothing about A1 — but it is a second
collection gap next to §4.1, and it is why the e2e test asserts the narrow cell as
declined in both directions with the reason stated instead of leaving a reader to
guess.

### 4.3 `-fprofile-generate` output is not reproducible, and it lies to A/B harnesses

Four files reported as "same size, different code":

```
17c17
<     .byte 57, 53, 53, 49, 46, 112, 114, 111, 102, 114, 97, 119, 0
>     .byte 57, 53, 53, 55, 46, 112, 114, 111, 102, 114, 97, 119, 0
910c910
< .hidden __lccc_pgo_dump_3028b24e91f41319_99551
> .hidden __lccc_pgo_dump_3028b24e91f41319_99557
```

Those bytes spell `99551.profraw` and `99557.profraw`: the PID. Proof that it is
noise and not a codegen change — the **same binary** run twice on the same source
differs from itself (`pgo_branchy`: DIFFERS, `switch_table`: DIFFERS). Any assembly
A/B over `tests/regression` therefore reports four phantom regressions unless it
normalises them. `ab_regression.py` now does, and the result is 0 changed out of
830. Believing the first run would have sent this round chasing a defect that did
not exist.

### 4.4 A wiped workspace silently disables every `-m32` gate

`check_ivsr_domains.sh` failed with `bits/libc-header-start.h: No such file or
directory`. It failed **identically on the merged base and on the new tree**, which
is how it was identified as environmental: the 32-bit glibc headers were missing.

The same class then appeared a second time, in a different package and a different
gate: `ci_local.sh --fast` reported `166 passed, 1 failed`, the failure being
`linker-suite` → `i386_dso_emit_semantics` → `bits/c++config.h: No such file or
directory`. That is the 32-bit **C++** header (`g++-multilib` /
`libstdc++-14-dev:i386`), not the C one; `libstdc++-14-dev-i386-cross` is *not* a
substitute, because it installs under `/usr/i686-linux-gnu/include/c++/14`, which
`g++ -m32` does not search. After installing it: **302 pass, 0 fail** (from 301/1).

Three consequences worth recording:

* an A/B or gate that fails the same way on both arms is not evidence about the
  change. Compare the arms against each other before believing either;
* 252 of the benchmark/oracle A/B's configurations were silently skipping while
  `-m32` was broken. The suite reported "compared=284 skipped=252" and it would
  have been easy to read 284 as coverage. After installing `gcc-multilib` and
  `libc6-dev-i386` the same harness reports **compared=402 skipped=0**. Skip
  counts are part of a result, not metadata;
* a red CI gate is not automatically a red change. `166 passed, 1 failed` here was
  one missing package, and the way to tell is that the failure is a *preprocessor*
  error inside a system header — not a diagnostic from this compiler. Fix the
  environment, re-run the gate alone to confirm (302/0), then re-run the whole
  suite so the certification stamp describes a green tree.

### 4.5 The review's own severity model

One high finding whose code path turns out to be unreachable from C source (§4.1),
and one medium *recommendation* that is a measured performance regression (§3).
Everything else the review raised was real: A2, A4, A5, B1, B2 and C1–C10 are all
confirmed and all now fixed, and B1/B2 in particular are exactly the kind of defect
only a careful reader catches — a kill switch with no test and an oracle case whose
own `(void)i` admitted it tested nothing.

The lesson is not "the review was unreliable". It is that a review without a
compiler produces *hypotheses* of uneven severity, and the only correct response is
to build each one and measure it — in both directions, including measuring a
recommended fix and finding that it costs 7 instructions and a spill/reload pair.

This round also mis-adjudicated A2 on the first pass by checking the review's claim
against the wrong of two adjacent guards (§2.2). That was caught by reading the
resulting diff rather than by reasoning about the code, which is a reminder that the
diff is evidence and the argument is not.

---

## 5. Validation

### 5.1 Static neutrality — 1480 comparisons, 0 changed

| corpus | configs | comparisons | skipped | changed | insns | stack refs |
|---|---|---|---|---|---|---|
| benchmark (55) + oracle (12) | `-O1,-O2,-O3` × (x86-64, `-m32 -msse2`) | **402** | 0 | **0** | 98002 → 98002 | 25745 → 25745 |
| `tests/regression` (845) | each file's own `.flags` | **844** | 1 | **0** | 232712 → 232712 | 39696 → 39696 |
| `tests/bench` (12 kernels) + `kernel_corpus` (24) + `patterns` (3) | `-O1,-O2,-O3` × (x86-64, `-m32 -msse2`) | **234** | 0 | **0** | 35844 → 35844 | 6978 → 6978 |

Arms: `/tmp/lccc-merged-main` (unmodified `6a9690c3`) vs `/tmp/lccc-hardened`
(this tree). Byte comparison of the assembly, not of the binaries — separately
built binaries differ in build metadata, so `cmp` on them proves nothing.

The third row matters most and was added last: it contains the twelve `k_*.c`
performance kernels — including `k_varint.c`, the file the merged win was measured
on — plus the 24 extracted real-workload files (gzip, zlib-ng, expat, SQLite,
kernel, glibc). An A/B that omitted the kernels would have proved neutrality
everywhere except where it counted.

**No Callgrind re-run was needed, and that is an argument rather than an omission**:
the assembly is byte-identical on all twelve kernels, so the instruction stream is
identical, so `Ir` is identical. Dynamic measurement would only have re-derived a
fact the static comparison already establishes. The merged round's numbers stand
unchanged (`k_varint` −9.59% `Ir`, −14.6% wall; suite −0.49%).

The single skip is a file whose `.flags` this host cannot satisfy. An earlier run of
the same harness reported `compared=830 skipped=15` — the 14 extra skips were the
`-m32` legs failing for want of 32-bit glibc headers (§4.4), and installing them
raised coverage to 844 with the verdict unchanged. The four PID-noise files are
normalised per §4.3 and were verified to be noise by a self-A/B of one binary
against itself.

### 5.2 Executed correctness

| gate | result |
|---|---|
| `scripts/run_regression_suite.sh` (GCC-differential, executed, incl. `-m32`) | **845 PASS / 0 FAIL / 8 SKIP, 0 AB-diff failures** |
| `tests/regression/check_ivsr_domains.sh`, this tree | exit 0, 20 configurations |
| same gate, merged base binary | exit 0 — so the added kill-switch leg is not the difference |
| four-vendor oracle, `--oracle-set all-vendors`, `-O2` | **9 agree / 0 diverge / 0 error** |
| lib unit tests | **4155 passed / 0 failed / 7 ignored** (base 4149) |
| env-read ratchet (`check_env_test_hygiene.sh`) | **153 / 155** — 2 reads *removed*, not budget raised |

The ratchet went down because three `CCC_IVSR_DEBUG` reads inside
`find_derived_exprs` were hoisted out of the loop body. That is both a hygiene win
and a small real one: the flag was being read from the environment per derived
expression.

Two of the five IVSR programs self-skip under `-m32` ("needs a 64-bit address
space"), which is why the gate reports 52 `OK` lines over 20 configurations rather
than 60. That is the loud self-skip added in the previous round working as
designed, not missing coverage.

### 5.3 Toolchain

`cargo fmt --all -- --check` clean; `cargo clippy --profile fastbuild
--all-targets --locked -j2` clean under `-D warnings`; `scripts/ci_local.sh --fast`
green; `check_doc_links` green. Build is the fastbuild preset, Rust `-O1`, `-j2`,
on a 1.9 GiB VM with an 8 GiB swap file.

---

## 6. What remains open

1. **Descending-loop codegen** (§4.1), ranked: loop rotation, then
   redundant-extension elimination via `nonzero_bits`-style tracking, then
   copy/2-addressing around shifts, then `Sub` support in `find_basic_ivs`, then
   vectorization. The first three are backend work with a crisp new reproducer; the
   fourth is small but buys little on its own because the load is already
   SIB-indexed; the fifth dominates and is a project.
2. **Narrow-scaled offsets** (§4.2) — teach `find_derived_exprs` the
   `(I64)(i << 2)` spelling. Low risk, unmeasured benefit until a corpus shape
   needs it.
3. **`classify` and `namechars`** — still the two worst kernels against GCC
   (2.057× and 1.653× `Ir`). §4.1 item 2 is the same root cause with a better
   reproducer than either kernel provides.
4. **PGO output determinism** (§4.3) — the PID in the `.profraw` name is arguably
   correct behaviour for a profile file, but it makes every PGO object
   non-reproducible. Worth a `-fprofile-deterministic` style option, or at least a
   note where the name is formed.

No PMU counters, no cycles, no uops, no Raptor Lake measurement and no speedup
claim appears anywhere in this round. The host is a Xeon VM; the target machine is
an i7-14700KF and nothing here is a statement about it.

---

## 7. Self-assessment

What went well: every one of the 18 findings was re-derived instead of accepted,
and that is what caught the one that mattered most — a medium *recommendation* that
measurably destroys vector-loop pointer induction (§3), decided by assembly diffs
and instruction/stack counts rather than by argument. The high-severity finding was
real, was fixed, and was then fixed **better** than recommended: deriving the
descending bound from the init recovers an optimization the conservative version
threw away. Two harness defects that would have produced false conclusions (PGO PID
noise, silently skipping `-m32`) were found by distrusting the first result. And a
wrong adjudication of A2 — declared a misquote after being checked against the
wrong of two adjacent guards — was caught by reading the resulting diff line by
line, then corrected in the source comment, the test and this document rather than
left standing (§2.2).

What did not go well: the A2 mis-adjudication above is the round's most serious
process failure — a finding was publicly declared wrong on the basis of reading the
neighbouring function, and it took a diff review to catch it. The first pass at the
new tests was also careless: three of them were written against helper functions
that do not exist in this file, and one line-range edit deleted eight tests that had
to be rebuilt. That cost several
build-and-test cycles (~3 min each) which a read of the fixture inventory first
would have avoided. Two test expectations were also written from reasoning about
what the code *should* do rather than checking what it does (that I64/Ptr are
exempt at pointer width, and that a `Copy(Const)` init stays non-literal), and both
had to be corrected by the test run. The descending-loop discovery (§4.1) is the
most valuable thing this round produced and it arrived late, from a failing test
fixture, rather than from looking at descending loops deliberately.

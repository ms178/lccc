# Follow-up: red-team audit of Agent B's iv_widen rewrite, and the synthesis

Session date: 2026-09-02
Base: `ms178/lccc` main @ `7cb11e0f` (Agent B's merge, PR #348, `f197d35d`)
Result: Agent B's revision audited line-by-line AND empirically; the best of
both implementations merged; two additional defects fixed; everything
re-validated and rebased on main.

---

## 1. Verdict: is Agent B's version better than mine?

**Substantially better in architecture, slightly worse in one case coverage, and
it shipped one latent soundness hole.** The right outcome is neither version
verbatim — it is a synthesis, which this session produced.

### What Agent B did better (adopted)

1. **`CmpAction::Trunc` — keep a cmp narrow instead of bailing the whole
   loop.** My S03 bailed the entire IV when a cmp's predicate or other operand
   could not be safely widened (e.g. a signed predicate on an unsigned IV, or
   a loop-variant cmp operand). Agent B inserts a single truncation before the
   cmp and keeps widening the IV and its addressing. Strictly more widening.
   (On x86 the trunc is free: `cmpl` reads the low 32 bits of the wide reg.)
2. **`verify_plan` — re-validate the whole plan against the live IR before any
   mutation.** Belt-and-suspenders, but it makes the provenance contract
   machine-checked and turns any analyze/apply drift into an atomic bail.
3. **Escape handling per member with `escape_read_needs_narrow`** — only reads
   that actually consume the narrow value get the truncation; wide-typed slots
   (GEP offsets, intrinsic indices, cast sources) keep reading the wide value.
   Broader than my seed-only, dominated-blocks repair.
4. **Same-width cast handling** (C's `int < unsigned` promotion) as retained
   truncations — keeps mixed-signedness cmps exact (`i < (unsigned)n`,
   `(unsigned)i < (int)n`).
5. **`BinOpMember`** (member op member, signed only), **`Select` data arms**,
   and **rotated self-loops**.
6. **25 in-module unit tests** on hand-built IR — pinning the provenance and
   the plan/verify/apply contract.

### What my S03 did better (re-added on top)

1. **Narrow constant-count shifts.** Agent B bails on narrow `Shl`/`AShr`/
   `LShr` ("not extension-transparent"), so `a[i<<1]` spelled as
   `cast(Shl(i,1))` keeps a per-iteration `movslq`. My closure widens it
   (`MemberKind::ShiftConst`, count left narrow). Measured: Agent B
   `t_shift` = 6 `movslq`/`movslq %ebx,%rbx` re-extension per iteration; the
   synthesis = 0 (load `movslq` only).

### Defects found in Agent B's version (fixed here)

1. **Latent unsound i8/i16 widening.** Their candidacy admits `I8/I16/U8/U16`
   IVs and their unit test widens a synthetic `Add(i8,1):I8` latch. That shape
   is unreachable from the C frontend (which emits `trunc(Add(phi,1):I32)` for
   promoted narrow IVs), but if it ever occurred the transform would change
   the **defined** 8/16-bit wrap of the truncation — the
   signed-overflow-is-UB theorem does NOT cover promoted-narrow wrap. Fixed:
   candidacy is restricted to `I32`/`U32`; `test_i8_iv` now asserts the
   decline. Real i8/i16 widening needs a narrow-width no-wrap bound proof
   (§5, the remaining follow-up).
2. **Escape truncation missed `Copy` reads** (`escape_read_needs_narrow(Copy)
   = false`): a Copy of a narrow member in an exit block would read the wide
   value into a narrow dest. Latent (copy-prop normally folds it) but a type
   hazard; fixed to `true`.
3. Minor: `std::collections::HashMap` → `FxHashMap` for the use map (repo
   convention + speed).

### What both versions agreed on (no regression either way)

The two miscompiles my audit found in S01/S02 — `Add(step, phi)` latch
overwriting `rhs`, and loop-variant-step hoisting — are correctly fixed in
Agent B's rewrite (verified by re-running the reproducers). Signedness of
`widen_const`, trip-bound provenance, `Sub(step,phi)` rejection, and
source-span maintenance are all handled correctly.

---

## 2. Validation (all gates green on the synthesized version)

| Gate | Result |
|---|---|
| `cargo build --profile fastbuild` + `-D warnings` | clean |
| `cargo test --lib` | 1505 passed / 0 failed / 6 ignored (28 iv_widen tests) |
| `run_regression_suite.sh` | **PASS=565 FAIL=0 SKIP=15** |
| `check_benchmark_outputs.sh` (INF-BENCHGATE-1) | **PASS=152 FAIL=0** |
| `ir_verify_sweep.py` O0–O3 | 2260 configs, 0 violations |
| corpus differential vs gcc @ `-O3 -march=x86-64-v3` | 37/37 match |
| red-team batteries (bug_battery / shapes / edge / exit_copy) | O0–O3 all match GCC |
| A/B harness (N=21 for the noisy ones) | no real regressions |

A/B median ratios (widen ON / OFF): sieve 0.745, loop_patterns 0.967,
zlib_ng_adler32 0.980, tls_seg_access 1.000, histogram 1.000,
sqlite_varint 1.000 (N=21), nbody 0.988 (N=21), arith_loop 0.993 (N=21).
The kernels where widening fires 0× (adler32) or is noise-bound stay flat;
no kernel regressed beyond measurement noise.

Godbolt oracle, `-O3 -march=x86-64-v3`, aggregate of the 8 shape functions
(lccc / gcc16.2 / clang23.1 / icc / icx): **191 / 242 / 641 / 218 / 372**
instructions — lccc's scalar loops are the smallest on this battery.

---

## 3. What changed vs. main (the delta on `7cb11e0f`)

* `src/passes/iv_widen.rs`:
  - `MemberKind::ShiftConst` (narrow `Shl`/`AShr`/`LShr` with constant count;
    count never widened), with admission + `verify_plan` + apply arms.
  - `escape_read_needs_narrow(Copy) = true`.
  - Candidacy restricted to `I32`/`U32` (i8/i16 decline documented + pinned).
  - `FxHashMap` use map.
  - Module header: corrected the i8/i16 claim, documented shift support and
    the Trunc policy.
  - New unit tests `test_narrow_shift_closure`, `test_shift_bad_count_bails`;
    `test_i8_iv` rewritten to assert the decline.
* `tests/regression/iv_widen_latch_and_bound.c` (new) — pins the Add(step,phi)
  latch, loop-variant step, header-computed bounds, decrementing latch,
  runtime-bound signed closure, and cmp signedness.
* `tests/regression/iv_widen_derived_closure.c` (new) — the closure shapes.
* `scripts/bench_iv_widen_ab.sh` (new) + `.gitignore` negation.
* `engineering/FOLLOWUP-2026-09-02-iv-widen-audit.md` (the previous session's
  audit) is preserved; this doc supersedes it.

---

## 4. Performance note

The synthesized pass widens strictly more loops than either parent (my
narrow-shift coverage + Agent B's Trunc cmp / escape / same-width-cast
coverage). `sqlite_varint` now widens 4 IVs (S03: 0) with 5 fewer
sign-extensions and 5 fewer instructions — the kernel's runtime is
branch-bound so it stays at ratio 1.000.

---

## 5. Remaining follow-up (prioritized)

1. **Real i8/i16 widening** needs a no-wrap proof at the narrow width: the
   promoted latch `trunc(Add(phi,1):I32)` wraps *defined* at 8/16 bits, so
   widening requires proving the IV provably stays in `[MIN, MAX]` (a counted
   bound with `bound ≤ 2^(w-1)` for signed, `≤ 2^w - 1` for unsigned). With
   that proof (const trip bounds would cover the common cases), the trunc can
   be dropped like a `WidenCast`. This is now precisely specified.
2. **Unsigned IVs with runtime bounds** still decline wrap-sensitive members
   (no interval proof without a counted bound). A trip-count/range pass
   handling runtime bounds would unlock `unsigned i; a[i+1]`.
3. `loop_memory_promote` still cannot prove pointer-param pointees disjoint
   without TBAA — the biggest remaining `stencil`-vs-GCC gap, orthogonal to
   IV widening.
4. Backend: decrement latches emit `leaq -1(%rbx),%r10; movq %r10,%rbx`
   (a Copy of a BinOp that copy-prop/regalloc do not fold to `decq`).

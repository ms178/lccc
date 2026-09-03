# Natural-width conditional selects (constant arms) — x86/IR fix

Date: 2026-09-03. Branch `wip/audit358-359`, on top of upstream main `0a12c392` (= PR #365
merge). Commit `73c3e077` "x86/IR: natural-width conditional selects with constant arms".
Working tree clean at the time of writing.

## 0. Problem

C `int` expressions are 32-bit values on LP64, and GCC 16.2 / Clang 23.1.0 / ICX keep
`int` conditional selects 32-bit (`cmovl`/`movl`). LCCC had a two-part gap:

1. **The ternary merge slot forced the target-int width.** Every conditional-expression
   merge was staged through an entry-block alloca sized/typed as the target int (I64 on
   LP64); the mem2reg Phi, and therefore the Select, became I64 even for `int` results —
   64-bit cmovs/moves plus redundant widen/narrow `Cast` pairs. (Stage-1 fix, committed in
   `73c3e077` together with the constant-arm work below.)
2. **Constant arms were born I64 and re-widened the merge.** Integer literals lower to
   `IrConst::I64` regardless of C type (`src/ir/lowering/expr.rs`, literal emission). Arm
   merge types came from `get_expr_type()`, which reports that *storage* type, so
   `x>255 ? 255 : x` merged `I64(255)` against an `I32` operand, promoted the common type
   to I64, and the select stayed 64-bit even after stage-1 made the *slot* natural width.

Observed on `-O3` before this round's fix (evidence `/tmp/t7.ir`, `/tmp/t6.ir`):
`hi`/`both`/`k` lowered to `Cast I32→I64` + `Select{ty: I64, I64-const}` +
`Cast I64→I32`; `clampi` to nested I64 selects with I64 `Const(0/255)`.

## 1. Root cause

- Ternary arm *types* used storage types instead of C types for the C common-type /
  merge computation: `lower_conditional`, `lower_gnu_conditional`, and
  `get_expr_type(Expr::Conditional|GnuConditional)`.
- The surviving I64 `Const` operands were then stored into (post-stage-1) natural-width
  slots without narrowing, and the declared `from_ty: I64` of the expression caused a
  phantom `Cast{src: <I32 select>, from_ty: I64, to_ty: I32}` right before the consumer.

## 2. Fix (all in `src/ir/lowering/` + one backend width rule)

- `expr_ops.rs` — `ternary_arm_type(&self, e)`: reports the **C-semantic** arm type.
  When storage is I64/U64 but the C type is `int`/`unsigned` (i.e., a literal), reports
  I32/U32; pointer arms report `Ptr`; everything else is unchanged. Used for the merge
  `common_ty` in `lower_conditional` and `lower_gnu_conditional`.
- `expr_types.rs` — `get_expr_type` for `Expr::Conditional`/`GnuConditional` now computes
  the common type from `ternary_arm_type` arms, so every consumer of a conditional's type
  (return lowering, enclosing binary ops, nested conditionals) sees the natural width and
  emits no phantom `from_ty: I64` cast.
- `expr_ops.rs` — `emit_ternary_merge_store()`: the two ternary merge helpers narrow
  integer constants to the slot type at the store, so mem2reg's Phi — hence the
  Select/cmov — stays at the natural width instead of carrying an I64 constant into an
  I32 slot.
- `expr.rs` — `emit_implicit_cast()`: normalizes an integer constant to its declared
  source type when the storage representation is wider. This is what makes the remapped
  arms above safe in every context (no no-op wide constant escaping, no `Const` operand
  whose representation disagrees with a `Cast`'s `from_ty`).
- `backend/x86/codegen/comparison.rs` — `emit_cmov_typed()`: any `ty.size() <= 4` now
  emits `cmov{l}` (removed the `is_unsigned()` restriction). A 32-bit cmov selects and
  writes only the low 32 bits, which is exactly I32/U32 select semantics regardless of
  signedness; the previous 64-bit cmovs on 32-bit data additionally required every arm to
  be sign-extended to 64 bits.

Sub-int types (I8/I16), floats, pointers, wide aggregates, and >8-byte scalars keep their
historical target-int staging — unchanged behavior, verified by the full gates.

## 3. Evidence

### 3.1 IR before/after (`-O3`, x86-64 LP64)

`int hi(int x){ return x>255?255:x; }`:

```
before: Cast(I32->I64) of x; Select{ty:I64, true:I64(255), false:x64}; Cast(I64->I32)
after:  Cmp Sgt I32; Select{ty: I32, true: Const(I32(255)), false: x}     (no Casts)
```

`clampi` (nested): two natural-width I32 selects with I32 `Const(0/255)`, no casts.
`both`/`k`/`p2`: single `Select{ty: I32}`.

### 3.2 Codegen counts (`-O3`, function bodies incl. `ret`)

| fn | GCC 16.2 | Clang 23.1.0 | ICX | LCCC after | LCCC gap vs best |
|---|---|---|---|---|---|
| min2 | 4 | 4 | 4 | **4** | — (parity) |
| sel  | 4 | 4 | 4 | 5 | register-allocator select-dest/return-eax copy |
| absi | 4 | 4 | 4 | 6 | neg-flags idiom + RA copy |
| clampi | 7 | 7 | 7 | 10 | two cmovs not chained in one reg; RA copies |
| rotl | 4 | 4 | 4 | 11 | no IR rotate node (shl/shr/or split; documented elsewhere) |
| lz | 6 | 4 | 7 | 8 | clz branch-shape + redundant reg moves |
| lz64 | 6 | 4 | 7 | 8 | clz branch-shape |
| tz | 6 | 3 | 6 | 7 | clz/ctz branch-shape |
| wl | 3 | 3 | 3 | 9 | clz shape + unnecessary movslq/guard |
| pc | 5* | 16 | 16 | 19 | software popcount instruction-selection |

*GCC calls `__popcountdi2` at plain `-O3` (no `-mpopcnt`).

Selects are now emitted at the correct width everywhere (all cmovs in the battery are
`cmov{l}`); every remaining gap is a **register-allocation / idiom / ISel** artifact, not
a width artifact. Residual categories, with loci, are itemized in §6.

### 3.3 Semantic differential stress

Custom tests driving int/uint/double/pointer/enum-ish ternary results and negative-I32
select results into 64-bit signed consumers (multiply, divide, shift, casts, array-index
derivations) match GCC output exactly at `-O0`, `-O1`, `-O2`, `-O3`
(`/tmp/tern_ctx.c`, `/tmp/sel_signed_ctx.c`).

## 4. Gates

| gate | result |
|---|---|
| x86-64 regression suite (`run_regression_suite.sh`, `-O2`, GCC-differential + A/B slot-width diff) | PASS=580 FAIL=0 SKIP=15, A/B diff 0 |
| same suite at `-O3` | PASS=580 FAIL=0 SKIP=15, A/B diff 0 |
| baseline A/B (upstream `0a12c392` binary, same suites) | identical PASS=580 FAIL=0 SKIP=15, A/B diff 0 |
| multi-opt subset sweep (conditional/select/phi/clz files, `-O0..-O3` vs GCC exit code) | 68 runs, 0 fails |
| correctness of changed-type paths (`tern_ctx`, `sel_signed_ctx`) vs GCC at 4 opt levels | all match |

## 5. Residual items (each needs its own project; do not regress width work to chase them)

1. **Return-register coloring for select results** — `sel` (5→4), and 1 instr on most
   scalar selects: the value computed in a non-`%rax` reg is copied to `%eax` for the
   return. GCC/Clang compute the select directly into `%eax` when it feeds `Return`.
   Fix locus: register allocation preference/coalescing of the function-return value
   (currently the accumulator `%rax` is generally excluded from allocation).
2. **Neg-flag abs/clamp idiom** — `absi` (6→4), `clampi` shape: recognize
   `Select(cond: x<0, -x, x)` and `min/max` clamp chains and reuse the `neg`/`sub` flag
   result (`cmovs`), as GCC does.
3. **IR rotate** — `rotl` (11→4): no rotate IR node; recognized shl/srl/or triple.
   Deliberately not attempted as a backend hack.
4. **Clz/Ctz branch shape + move hygiene** — `lz`/`tz`/`wl`: the zero-guard expansion
   builds extra blocks/moves (`movq %rdi,%rax`, jump-to-shared-done) and `wl` carries a
   spurious `movl %eax,%eax`. Also a latent `testq` (64-bit) on 32-bit select conditions
   in `test_select_cond_in_place` — correct under the de-facto zero-extended calling
   convention but worth switching to `testl` using the condition's IR width.
5. **Software-popcount instruction selection** — `pc` (19 vs clang/icx 16).

## 6. Notes for upstream handoff

- The constant-arm work depends on the stage-1 slot helper `ternary_merge_slot`
  (same commit). If rebasing, keep both.
- `emit_implicit_cast` narrowing is a generic safety net; it only fires when a caller
  declares an integer source type narrower than the constant's storage repr — which,
  after `ternary_arm_type`, is exactly the C-typed-constant case.
- i686 is unaffected by design: on ILP32 the target int *is* I32, so the new exact-type
  rule coincides with the old behavior for every category.

# Session 65 — Semantic audit of levkropp/lccc commits since 2026-08-19

Date: 2026-08-23

Upstream base: `85ad7d73cfe58f5ba4e317ac9fbf70490633e1eb` (`ms178/lccc` main at session start).
Audited upstream: `levkropp/lccc@2ed29fd84c4d0d3963d42bdab2163bb694b413ea`.
Command used for the inventory:

```bash
git fetch levkropp main --shallow-since=2026-08-19T00:00:00Z
scripts/audit_levkropp_since.py --since 2026-08-19 --out docs/history/2026-08-23-levkropp-commit-inventory.md
```

The raw per-commit inventory is in `docs/history/2026-08-23-levkropp-commit-inventory.md`.

## High-level audit result

`git cherry` is not useful for this fork comparison: many levkropp commits have
already been independently rewritten, hardened, or partially superseded in
`ms178/lccc`, so patch IDs do not match. I therefore audited semantically by
feature cluster and by concrete code markers.

### Already present / superseded in ms178 main

These levkropp ideas are already present in stronger or heavily hardened form:

- `passes::alias` + affine memory forms.
- `redundant_loads` late same-block redundant load pass.
- `quadratic_sr` triangular second-order strength reduction.
- `global_addr_cse` substitution-based global address CSE.
- Hardened `aggregate_copy_forward` with default-closed escape handling.
- Store-to-load forwarding with default-closed transfer function, volatile load
  guard, and root escape checks.
- Several AArch64 and register-allocation ideas are present in larger ms178
  rewrites; exact levkropp patches should not be blindly replayed.

### Not adopted but high-value follow-up

- `a08f676` backedge PRE: potentially valuable (reported mandelbrot win) but
  not present in ms178 main. It is large enough to require a dedicated
  correctness+performance spike: import the pass behind `CCC_DISABLE_PASSES=bepre`,
  build a mandelbrot/spectral micro-corpus, and run the GCC torture focused
  slice before integration. Do **not** merge blindly: it rewrites uses across
  loops and must be checked against LICM/vectorization/pass-order invariants.
- Computed-goto label identity remains a correctness cluster. It interacts with
  levkropp's label-address/static-initializer changes and current remaining
  torture failures (`20071220-1`, `920302-1`, etc.). Fix as IR label identity,
  not as assembly peephole.

## This session's adopted semantic fix

### Pointer compound assignment index widening (`2ed29fd` subset)

levkropp's latest commit identified a real scalar lowering bug: `p += i` / `p -= i`
scaled `i` without first widening it to pointer width. A negative `int` or
signed-char-derived index could be multiplied as a bogus 64-bit positive value.

I adopted the C-semantics-correct piece in `src/ir/lowering/expr_assign.rs`:

```text
rhs_val -> emit_implicit_cast(rhs_ir_ty, target_int_ir_type()) -> scale_index
```

This matches plain `ptr + int` lowering and preserves signedness before the
scale multiply. The volatile store-load-forwarding guard from the same levkropp
commit was already present in stronger form in ms178 main.

Added regression:

- `tests/regression/pointer_compound_assign_signed_index.c`

Validation:

```text
scripts/build_lccc_fast.sh
target/fastbuild/lccc -O2 tests/regression/pointer_compound_assign_signed_index.c -o /tmp/pointer_compound_assign_signed_index
/tmp/pointer_compound_assign_signed_index
# PASS
```

Focused regression validation also passed:

```text
PASS tests/regression/path_range_var_minus_one_uintmax.c
PASS tests/regression/vla_struct_assignment_runtime_memcpy.c
PASS tests/regression/pointer_compound_assign_signed_index.c
```

I also ran `cargo test --profile fastbuild --lib range_check --locked -j 2`.
That exposed an upstream unit-test build break from the earlier LICM
must-execute-load API change: private LICM tests still called
`hoist_loop_invariants` without the new dominator argument. The patch fixes all
seven stale test call sites by passing the already-computed `idom`, and the
filtered unit suite now passes (`3 passed; 0 failed`).

## Backedge PRE spike result

I imported levkropp `a08f676` (`backedge_pre`) and tested it late in the scalar
pipeline. The naive FP behavior was rejected: with `CCC_BEPRE_FP=1`, Mandelbrot
reduced static `vmulsd` count from 5 to 4, but the extra loop-carried FP phi/copy
increased register pressure and regressed runtime. The production pass therefore
uses a cost model:

- integer recurrences are enabled by default;
- FP recurrences are disabled by default on current ms178 backends;
- `CCC_BEPRE_FP=1` keeps the FP path available for research;
- `CCC_DISABLE_PASSES=bepre` disables the pass.

Default Mandelbrot assembly is identical with and without the pass, proving the
FP guardrail:

```text
target/fastbuild/lccc -O2 mandelbrot.c -S -o default.s
CCC_DISABLE_PASSES=bepre target/fastbuild/lccc -O2 mandelbrot.c -S -o off.s
cmp -s default.s off.s  # identical
```

Integer recurrence evidence is positive:

```text
Backedge PRE on : 1 imul in run()
Backedge PRE off: 2 imul in run()
Runtime median: on 0.1619 s, off 0.1850 s, speedup 1.14x
```

Full evidence is in `engineering/evidence/levkropp/backedge_pre_x86_spike.md`.

## Why these code changes?

The audit found many performance-oriented levkropp commits, but the project rule
is correctness first under generated-code performance. The pointer compound
assignment bug is a low-risk, high-confidence semantic fix with a small proof and
a permanent regression. Backedge PRE is now integrated only where the current
backend shows a measured win (integer recurrences) and guarded away from the
known x86 FP regression. Label-address CFG identity and nested-function/static-chain
support remain correctness projects that require dedicated design+validation cycles.

## Next concrete follow-up queue

1. **Computed-goto label identity**: diagnose why static label initializers can
   reference a different `.LBB` than the kept `global_init_label_blocks` target.
   Use `20071220-1.c` and `920302-1.c` as minimal oracles.
2. **Backedge PRE FP follow-up**: only revisit FP PRE after regalloc can prove
   or coalesce the added loop-carried FP state; keep using `CCC_BEPRE_FP=1` for
   experiments and require a mandelbrot/nbody/spectral A/B before defaulting it.
3. **VLA aggregate `va_arg`**: extend dynamic VLA-struct handling to varargs
   (`20020412-1.c`).
4. **Bitfield cluster**: reduce `20040709-{1,2,3}.c`; compare extracted field
   masks and aggregate returns against GCC.
5. **AArch64 levkropp tail**: revisit range-safe ADRP+ADD/ADRP+LDR changes and
   LDUR/STUR assembler variants under a real AArch64 assembler/QEMU setup.

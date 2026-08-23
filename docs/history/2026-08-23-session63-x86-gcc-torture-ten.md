# Session 63 — Ten native x86 GCC torture execute fixes

Date: 2026-08-23

Upstream base: `a944b9a0eda8acef26e8b0a8f82bab6cbd39fb08` (latest `ms178/lccc` main at session start).
Build mode: `scripts/build_lccc_fast.sh` (`fastbuild`, effective Rust -O1, `-j2`).
Swap: `/home/user/lccc.swap` was active in the sandbox (`swapon --show`).

## Result

Fixed **10** previously-failing native x86 `gcc.c-torture/execute` tests at `-O2`:

| GCC torture test | Previous symptom | Root cause | Fix |
| --- | --- | --- | --- |
| `20010518-1.c` | abort | FPO leaf with callee-saved push-only prologue addressed stack args as if `%rsp` still equaled entry `%rsp` | Use actual pushed save depth as `rsp_frame_size` for RSP-relative references |
| `20030218-1.c` | abort | Peephole folded `movswq %ax,%rax; movq %rax,%rsi` into `movswl %ax,%esi`, zeroing high 32 bits | Signed byte/word relay folds now target 64-bit GPRs (`movsbq`/`movswq`) |
| `20070212-2.c` | abort | Aggregate field DSE deleted stores to an alloca later read via pointer phi across roots | Pointer phis merging different tracked roots mark the roots escaped |
| `20041019-1.c` | abort | GVN store-to-load forwarding through unknown-base pointer survived a later store to a known alloca | Unknown-base memory epoch is bumped on every store |
| `20051215-1.c` | segfault | LICM speculated a guarded `*z` load out of a conditional loop block | Load hoisting now requires the load block to dominate the loop body (must-execute) |
| `20080604-1.c` | abort | Aggregate DSE treated a phi of tracked alloca and untracked global as non-escaping | Mixed tracked/untracked pointer phis mark tracked roots escaped |
| `20050121-1.c` | abort | `_Complex double/float` imaginary return half was omitted when register-allocated | `SetReturnF{32,64}Second` always routes source through FP operand loader to `%xmm1` |
| `20070614-1.c` | abort | Same `_Complex` `%xmm1` return-half bug in pure complex function loop | Same fix |
| `20060929-1.c` | abort | Discard flag for outer post-inc leaked into dereference lvalue address: `(*p++)++` used new `p` | Dereference lvalue address computation disables top-level discarded-postinc shortcut |
| `20180112-1.c` | segfault | `find_param_alloca` treated the Nth remaining entry-block alloca as parameter N after mem2reg removed the real param home | Parameter homes are now resolved through `func.param_alloca_values` and verified against an existing `Alloca` |

Permanent regression coverage added under `tests/regression/`:

- `x86_fpo_stack_params_many_args.c`
- `x86_signed_extend_relay_i16_to_long.c`
- `aggregate_phi_alias_store_live.c`
- `licm_conditional_load_not_speculated.c`
- `x86_complex_second_return_register.c`
- `discarded_outer_postinc_nested_lvalue.c`
- `param_alloca_not_nth_alloca.c`

## Validation performed

```text
scripts/build_lccc_fast.sh
scripts/x86_gcc_torture_slice.sh \
  20010518-1.c 20030218-1.c 20070212-2.c 20041019-1.c \
  20051215-1.c 20080604-1.c 20050121-1.c 20070614-1.c \
  20060929-1.c 20180112-1.c
```

Each named test passed after its fix. The final cumulative validation command is recorded in the assistant transcript and should be rerun by the next agent together with the new regression files.

## Remaining high-priority x86 GCC torture gaps

1. **Nested functions / trampolines / non-local gotos** remain the largest compile-fail cluster in the first 500 sorted torture tests: `20000822-1`, `20010209-1`, `20010605-1`, `20030501-1`, `20040520-1`, `20061220-1`, `20090219-1`, `920415-1`, `920428-2`, `920501-7`, `920612-2`.
2. **Computed goto / label-difference cases** still fail or time out: `20041214-1`, `20071210-1`, `20071220-1`, `920302-1`. These should be attacked as a label-address/codegen cluster, not individually.
3. **VLA aggregate varargs**: `20020412-1.c` still aborts; investigate variable-size aggregate `va_arg` layout/copy.
4. **Scalar storage order bitfields**: `20230630-2.c` and `20230630-4.c` still abort; likely missing reverse scalar storage order support in bitfield layout/init/load/store.
5. **Boolean/range fold**: `20041114-1.c` still leaves `link_failure`; path-sensitive range reasoning after `var <= 0` false should prove `(unsigned)(var - 1) < UINT_MAX`.

## Engineering notes for future agents

- The aggregate DSE fixes are intentionally conservative. Do not re-open them with a purely local read-set argument; pointer phis with partial tracking are escape boundaries.
- The GVN unknown-pointer epoch preserves precise per-base epochs for known globals/allocas while invalidating unknown-base entries on every store. This is the right trade-off for correctness and still keeps the gzip-style distinct-global optimization.
- LICM load hoisting now has a must-execute requirement. If this loses performance on hot loops, add a real dereferenceability/proven-nonnull analysis instead of relaxing the dominance guard.

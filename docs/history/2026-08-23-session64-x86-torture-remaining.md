# Session 64 — Remaining native x86 GCC torture failures: three more fixed

Date: 2026-08-23

Upstream base: `9d2aba1c4f69e379807d1c507637b6230d2345c6` (latest `ms178/lccc` main at session start, containing session 63).
Build mode: `scripts/build_lccc_fast.sh` (`fastbuild`, effective Rust -O1, `-j2`).
Swap: `/home/user/lccc.swap` was active in the sandbox.

## Fixed this session

| GCC torture test | Previous symptom | Root cause | Fix |
| --- | --- | --- | --- |
| `20041114-1.c` | link failure: unresolved `link_failure` | No path-sensitive range fact from the false edge of `var <= 0`; LCCC could not prove `(unsigned)(var - 1) < UINT_MAX` for positive signed `int` | Added a local, typed path fold in `range_check`: on the `x > 0` edge, `(u32)(x - 1) < UINT_MAX` folds to true without assuming signed overflow |
| `20070919-1.c` | abort at all opt levels | GCC VLA-struct extension lowered zero-byte allocas and zero-byte fixed `Memcpy` for `struct S { char w[y]; }` locals and assignments | VLA-containing struct locals now use `DynAlloca`, carry `LocalInfo.vla_size`, and struct assignment emits libc `memcpy(dest, src, runtime_size)` |
| `20180112-1.c` | segfault | Already fixed on merged main by resolving param homes through `func.param_alloca_values`; retained as validation target because it remained in the pre-rebuild failure list | Permanent regression retained: `param_alloca_not_nth_alloca.c` |

## Permanent regressions added

- `tests/regression/path_range_var_minus_one_uintmax.c`
- `tests/regression/vla_struct_assignment_runtime_memcpy.c`
- `tests/regression/param_alloca_not_nth_alloca.c`

## Validation

```text
scripts/build_lccc_fast.sh
scripts/x86_gcc_torture_slice.sh 20041114-1.c 20070919-1.c 20180112-1.c
# 3 pass / 0 fail

target/fastbuild/lccc -O2 tests/regression/path_range_var_minus_one_uintmax.c -o /tmp/path_range_var_minus_one_uintmax && /tmp/path_range_var_minus_one_uintmax
target/fastbuild/lccc -O2 tests/regression/vla_struct_assignment_runtime_memcpy.c -o /tmp/vla_struct_assignment_runtime_memcpy && /tmp/vla_struct_assignment_runtime_memcpy
target/fastbuild/lccc -O2 tests/regression/param_alloca_not_nth_alloca.c -o /tmp/param_alloca_not_nth_alloca && /tmp/param_alloca_not_nth_alloca
# all PASS
```

First-500 native x86 GCC torture status after these fixes in this sandbox: `479 PASS / 21 FAIL` before the final three-test focused validation; the remaining failures are dominated by known larger feature clusters.

## Remaining clusters and recommended attack plan

1. **Nested functions / trampolines / non-local gotos**: still the largest compile-fail group (`20000822-1`, `20010209-1`, `20010605-1`, `20030501-1`, `20040520-1`, `20061220-1`, `20090219-1`, `920415-1`, `920428-2`, `920501-7`, `920612-2`). Proper solution: parser support for block-scope function definitions, IR representation for static chain, x86 trampoline emission for address-taken nested functions, and non-local goto label environment support.
2. **Computed goto / label address identity**: `20071220-1` shows static initializer data containing `.LBB3` while `global_init_label_blocks` identifies `BlockId(2)`; the label trampoline block is lost/retargeted inconsistently. Fix this as a label-address mapping bug, not as a peephole workaround. Related: `20041214-1`, `20071210-1`, `920302-1`.
3. **VLA aggregate varargs**: `20020412-1.c` still aborts; now that dynamic VLA-struct allocation/copy exists, extend `va_arg` aggregate handling to use the runtime size/alignment when the requested type contains a VLA member.
4. **Bitfield arithmetic/layout**: `20040709-{1,2,3}.c` still abort; use `gcc -E` on the torture files and bisect the failing generated `testX` function. Likely field extraction/storage or return-by-value bitfield aggregate copy.
5. **Reverse scalar storage order bitfields**: `20230630-{2,4}.c` still abort; implement `__attribute__((scalar_storage_order(...)))` in layout metadata and bitfield init/load/store byte order.

## Design notes

- The path fold is deliberately narrow and typed. It does not introduce a half-baked global range analysis; it encodes one proven CFG edge fact and only rewrites the exact unsigned-bound idiom.
- Runtime VLA-struct assignment uses libc `memcpy` instead of extending fixed-size IR `Memcpy`. This preserves the current backend optimization contract for constant-size copies while providing correct dynamic semantics.

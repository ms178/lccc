# Session 46 — MachInst partial-register narrowing segfault (2026-08-22)

Base: `ms178/lccc` main `8b7e3bc` (PR #183). Build: `fastbuild`, Rust
opt-level 1, no LTO, two jobs, 4 GiB swap. Host: constrained VM without PMU.

## The bug

zlib-ng's `zng_emit_dist` (`trees.c` / `trees_emit.h`) computes

```c
uint8_t code;
code = d_code(dist);            /* U32 -> U8 narrowing cast   */
extra = extra_dbits[code];      /* folded scale-4 SIB index   */
```

and lccc emitted

```asm
movl %esi, %r9d
movb %r9b, %r10b        # truncating move: upper 56 bits STALE
leaq extra_dbits(%rip), %rcx
movslq (%rcx, %r10, 4), %rax   # reads %r10 at 64-bit width -> garbage index
```

→ out-of-bounds read / segfault. Reproduced standalone with
`tests/regression/session46_dist_code_index.c` before fixing.

### Root cause chain

1. **MachInst `lower_cast`** (`src/backend/x86/codegen/isel.rs`) lowered every
   narrowing/same-size cast as a plain truncating move
   (`emit_mov_operand_r(src, dst, to_size)`) — `movb`/`movw`/`movl`.  x86
   `movb` preserves the destination's upper bits, so the register home was
   left **undefined above the narrow width**.
2. The folded-SIB index path (never-materialized cast result, possibly
   die-at-birth-shared with the source's home) consumes the home at **full
   64-bit width** without an explicit extension instruction — its safety
   contract is "the home holds a zero-extended value".
3. The mature (non-MachInst) path always extended
   (`try_emit_cast_reg_direct` emits `movzbl`/`movswl`/`movslq`), which is
   why `CCC_NO_MACHINST=1` was clean and why the bug appeared only when the
   loop-body MachInst gate fired.

## The fix

`lower_cast`'s narrowing/same-size arm now emits **extending** moves exactly
like the mature path and GCC/Clang:

| Cast | Before | After |
|------|--------|-------|
| U32→U8 / U64→U8 | `movb` | `movzbl %rXb, %rYd` |
| I32→I8 / I64→I8 | `movb` | `movsbl %rXb, %rYd` |
| →U16 | `movw` | `movzwl` |
| →I16 | `movw` | `movswl` |
| →U32 | `movl` (elided on self-move!) | `movl` via `Movzx` (never elided) |
| →I32 | `movl` | `movslq` (sign semantics, matches mature path) |
| const → U8/U16/U32 | `movb/movw/movl $imm` | `movl $imm` (zero-extends) |
| const → I8/I16/I32 | `movb/movw/movl $imm` | `movq $imm` (sign-extends) |

Same length, no partial-register false dependency, and every wider reader —
folded SIB indices included — now sees a defined value.  The `U32` case uses
`Movzx{S32,S32}` deliberately: a plain `Mov` is elided as a self-move on a
die-at-birth shared home and would skip the extension.

### Why the class is now closed

The IR guarantees an explicit Cast between any sub-32-bit value and every
wider consumer (GEP indices are widened to I64 in the IR, as the dump showed:
`Cast { from_ty: U8, to_ty: I64 }` before the `Shl`).  With every cast's home
extended at definition time, the folded-index contract holds again.  The
remaining `movb` writers (8-bit ALU results, `setcc`, U8 loads) are safe
because their wider consumers always flow through a now-extending cast.

## Validation (all on the final tree)

| Gate | Result |
|------|--------|
| `cargo test --lib` | **1046 pass**, 6 ignored (incl. 3 new `lower_cast` unit tests) |
| Regression corpus | **375 pass / 0 fail / 8 documented skips** (383 total, incl. 2 new session-46 regressions) |
| `tests/intrinsics` (t128/t256/t512) | **3/3** |
| zlib-ng 2.3.3 pinned gate | **ctest=8, roundtrip=0 failures** |
| gzip 1.14 (pinned SHA) | **30/30** + doc roundtrip |
| expat 2.7.1 (git HEAD f9a3eeb, lccc-built) | **2/2 ctest (runtests + runtests_cxx)** |
| phi CFG fuzz 0:600 × 3 levels | 1800/1800 |
| differential fuzz 0:500 × 3 levels | 1500/1500 |
| alias fuzz m32 0:540 × O2/Os | 1080/1080 |
| checkers (sema/bit-test/redundant-test/rmw-copyprop) | all pass |

## Oracle distillation snapshot (post-fix, `-O2 -march=x86-64-v3` whole-file CE)

| Kernel | lccc | gcc16.2 | clang | icc | icx | note |
|--------|-----:|--------:|------:|----:|----:|------|
| `zlib_ng_adler32` | 356 | 156 | 147 | 248 | 147 | DO8 stack traffic — **largest remaining integer gap** (RA-03) |
| `gzip_crc32` | 118 | 50 | 82 | 60 | 100 | SIB + remat (RA-01/IS-22) |

## Follow-up work

1. **RA-03** Adler DO8: `sum2`/`n` must stay in GPRs (the 356→~150 gap is
   almost entirely slot traffic in the unrolled DO8 loop).
2. **RA-01/IS-22** CRC: file-scope remat + indexed SIB loads.
3. The sched/ISel front still emits redundant `movl %eax, %eax`
   (zero-extend artifact after `movslq`) — a peephole opportunity.
4. Sema accepts writes through `const` arrays (`static const int t[]; t[i]=x;`)
   that GCC rejects; add the C11 6.5.16.2 assignment-to-const-object
   diagnostic (found while authoring the torture regression).
5. Re-measure gzip/zlib-ng runtime on the i7-14700KF with PMU top-down
   counters once the RA-03/RA-01 work lands.

# Follow-up — 2026-08-29: AVX2 matmul IV promotion to I64 (eliminate movslq)

**Base.** `ms178/lccc` main `4630de02` (#302 SIB+hoisted disp, 74 insns). Previous
session left 2× `movslq` per inner iteration:

```
.LBB6:
  movslq %r12d, %r10
  vmovupd (%rbx,%r10), %ymm0
  ...
  addl $128, %r12d
  movslq %r12d, %r12
  jmp .LBB5
```

- `movslq %r12d, %r10` – sign-extend i32 IV (byte offset j*8) to 64-bit for SIB
- `movslq %r12d, %r12` – sign-extend incremented IV for remainder calc (redundant,
  since `addl` zero-extends on x86-64, but needed for `movq %r12, %r13` after loop)

**Root cause:** IV Phi was I32 (original C `int j`), but SIB addressing needs 64-bit
index. Backend emitted `Cast I32->I64` as `movslq`.

## Fix: Promote IV to I64 in `transform_to_fma_f64x4`

Added Step 2c after increment change:

1. **Phi promotion:** Find Phi `dest==pattern.iv` in header, change `ty I32→I64`,
   promote incoming Const I32→I64.
2. **Increment promotion:** Change latch `Add` ty to I64, rhs to I64(128).
3. **Cast elimination:** For all `Cast I32->I64` where src is IV-derived, replace
   with `Copy` (now no conversion needed).
4. **Comparison promotion:** Change `Cmp` ty I32→I64 and consts to I64.

This is safe because byte offset is 0..2048, always non-negative and <2^31,
so I64 preserves semantics. The loop limit was already promoted to 2048 (byte limit).

## Result

```
.LBB5: cmpq $2048, %r12 / jge .LBB7   # was cmpl
.LBB6:
  vmovupd (%rbx,%r12), %ymm0
  vfmadd231pd (%r14,%r12), %ymm1, %ymm0
  vmovupd %ymm0, (%rbx,%r12)
  vmovupd 32(%rbx,%r12), %ymm0
  vfmadd231pd 32(%r14,%r12), %ymm1, %ymm0
  vmovupd %ymm0, 32(%rbx,%r12)
  vmovupd 64(%rbx,%r12), %ymm0
  vfmadd231pd 64(%r14,%r12), %ymm1, %ymm0
  vmovupd %ymm0, 64(%rbx,%r12)
  vmovupd 96(%rbx,%r12), %ymm0
  vfmadd231pd 96(%r14,%r12), %ymm1, %ymm0
  vmovupd %ymm0, 96(%rbx,%r12)
  addq $128, %r12
  jmp .LBB5
```

- **0 movslq in hot loop** (was 2)
- LBB6: **14 insns** (was 16) = 12 FMA + addq + jmp
- Function: **72 insns** (was 74), gcc 25, clang 438 → gap 47 (was 49)
- Total movslq in function: 1 (remainder `movslq %r13d, %rax`, not hot)

## Validation

- `cargo test --profile fastbuild --lib`: 1306 passed, 0 failed
- `lccc -O3 -march=x86-64-v3 -S`: verified no movslq in LBB6, cmpq, addq
- `codegen_scoreboard.py`: 72 insns, gap 47
- `godbolt.py compare`: lccc 72 vs gcc 25 vs clang 438

## Remaining gaps vs GCC (25 vs 72)

GCC:

```
vbroadcastsd (%rdi), %ymm2
vbroadcastsd 8(%rdi), %ymm1
.L3:
  vmovapd (%rcx,%rax), %ymm0
  vfmadd213pd (%rdx,%rax), %ymm2, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  vfmadd231pd (%rsi,%rax), %ymm1, %ymm0
  vmovapd %ymm0, (%rdx,%rax)
  addq $32, %rax
  cmpq $2048, %rax
  jne .L3
```

- k-unroll by 2: 2 broadcasts per j-loop, 2 FMAs per 32 bytes, halving k-loop
- Uses `vmovapd` (aligned) vs `vmovupd` – base is 32-byte aligned? Our BSS is 16-byte,
  but could be 32-byte for AVX.
- No outer i/k loop overhead in inner loop count (our outer loops add ~30 insns)

**Next:** Implement k-unroll by 2 with dual broadcast (ymm1, ymm2) and dual B bases,
plus `vzeroupper` to avoid AVX-SSE transition. This would match GCC's compute density
and potentially beat it with our 4-wide j-unroll (8 FMAs per 128 bytes vs GCC 2 per 32).

## Artifacts

- `/home/user/ms178-1.patch` 6.5K, 137 lines, APPLIES-CLEAN on 4630de02
- `/tmp/matmul.s` final with no movslq in hot loop

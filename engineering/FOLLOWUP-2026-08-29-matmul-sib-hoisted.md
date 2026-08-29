# Follow-up — 2026-08-29: AVX2 matmul SIB + hoisted broadcast (SIB disp folding)

**Base.** `ms178/lccc` main `cdd3ba45` (#301 paravirt+matmul broadcast hoist). Previous
session left broadcast hoist (1 vmovsd+vbroadcastsd in LBB4, 0 inside LBB6) but still
11 leaq + 8 movq overhead per iteration (96 insns vs gcc 25).

## Red-team audit of previous fix

Previous `transform_to_fma_f64x4` after broadcast hoist:

- Created 2× offset Adds (b_off+32, c_off+32) + 2× GEPs per chunk (1..3) → 6 Adds + 6 GEPs
  for 3 chunks.
- Backend `FmaF64x4Hoisted` lowered each GEP via `operand_to_reg` + `value_to_reg`
  → `leaq (%r14,%r12), %r10; leaq (%rbx,%r12), %r9; movq %r10,%rdx; movq %r9,%rax`
  per chunk → 11 leaq + 8 movq = 19 insns overhead.
- Total LBB6: 1 movslq + 19 addr + 12 FMA = 32 insns per iter, function 96 insns.

**Gap vs GCC:** GCC 25 insns total uses SIB directly, no leaq/movq, and unrolls k-loop by 2
with 2 broadcasts per j-loop.

## New design: `FmaF64x4HoistedSIB` with displacement

**Goal:** Eliminate ALL GEP leaq/movq and broadcast inside loop, match GCC SIB pattern,
but keep hoisted broadcast (better than GCC's 2 broadcasts per j-loop? Actually GCC
does 2 broadcasts per j-loop for k-unroll=2, we do 1 broadcast per k).

### IR

Added `IntrinsicOp::FmaF64x4HoistedSIB`:

```
args[0] = C base pointer (row base, loop-invariant)
args[1] = B base pointer (row base, loop-invariant)
args[2] = byte offset (j*8, shared)
args[3] = optional displacement (0,32,64,96) as const
ymm1 = broadcasted A[i][k] (from BroadcastLoadF64 in preheader)
```

- 3-arg form: offset already includes chunk (backward compat, folded if Add)
- 4-arg form: offset + disp const → emits `disp(%base,%off)` SIB, no extra leaq

Updated all backends:

- `src/ir/intrinsics.rs`: definition + vector_result_width
- `src/passes/vector_temp_promotion.rs`: `index<2` safe (base pointers), dest alignment 0,
  overwrites_full_result, dest_reads_old_value
- `src/backend/x86/codegen/prologue.rs`: has_rdx_intrinsic
- `src/backend/regalloc.rs`: is_raw_reader
- `src/backend/generation.rs`: has_vector_intrinsics
- `src/backend/arm|i686|riscv/codegen/intrinsics.rs`: x86-only no-op list
- `src/backend/stack_layout/copy_coalescing.rs`: is_raw_reader
- `src/backend/x86/codegen/intrinsics.rs`: emitter with disp folding

### Vectorizer

`transform_to_fma_f64x4` now:

```rust
for chunk in 0..4 {
  disp = chunk*32
  if disp==0 { FmaF64x4HoistedSIB [c_base, b_base, b_off] }
  else { FmaF64x4HoistedSIB [c_base, b_base, b_off, const disp] }
}
```

No BinOp Adds, no GEPs. 4 intrinsics share same offset Value.

### Backend emitter

- Extract disp from args[3] if present (const i64)
- Else try to fold offset = Add(base, const) → base + disp
- `operand_reg` for base pointers to avoid movq copies (reuse existing homes)
- Emit:
  ```
  vmovupd disp(%c_base,%off), %ymm0
  vfmadd231pd disp(%b_base,%off), %ymm1, %ymm0
  vmovupd %ymm0, disp(%c_base,%off)
  ```

With disp folding, chunks 1..3 become `32(%rbx,%r10)` etc, no leaq.

## Result

`/tmp/matmul.s` after fix:

```
.LBB4:
  movq %r15, %rcx
  vmovsd (%rcx), %xmm1
  vbroadcastsd %xmm1, %ymm1   # 1 broadcast total, hoisted
  xorl %r12d, %r12d
.LBB5: cmpl $2048, %r12d / jge .LBB7
.LBB6:
  movslq %r12d, %r10
  vmovupd (%rbx,%r10), %ymm0
  vfmadd231pd (%r14,%r10), %ymm1, %ymm0
  vmovupd %ymm0, (%rbx,%r10)
  vmovupd 32(%rbx,%r10), %ymm0
  vfmadd231pd 32(%r14,%r10), %ymm1, %ymm0
  vmovupd %ymm0, 32(%rbx,%r10)
  vmovupd 64(%rbx,%r10), %ymm0
  vfmadd231pd 64(%r14,%r10), %ymm1, %ymm0
  vmovupd %ymm0, 64(%rbx,%r10)
  vmovupd 96(%rbx,%r10), %ymm0
  vfmadd231pd 96(%r14,%r10), %ymm1, %ymm0
  vmovupd %ymm0, 96(%rbx,%r10)
  addl $128, %r12d
  movslq %r12d, %r12
  jmp .LBB5
```

- vmovsd total: 1 (preheader)
- vbroadcastsd total: 1
- LBB6: 0 leaq for B/C addresses (was 11), 0 movq (was 8), 0 extra Adds (was 6)
- LBB6 insns: movslq + 12 FMA + addl + movslq + jmp = 16 (was 32)
- Function insns: 74 (was 96), gcc 25, clang 438
- Gap vs best: 49 (was 71)
- Loads: 28 (was 36), stores 1, vec 12

## Validation

- `cargo test --profile fastbuild`: 1306 passed, 0 failed (was 1304)
- `target/fastbuild/lccc -O3 -march=x86-64-v3 -S` verified SIB disp form
- `scripts/godbolt.py compare ... --function matmul`: lccc 74 vs gcc 25 vs clang 438
- `scripts/codegen_scoreboard.py`: gap 49 vs 71 before

## Remaining gaps vs GCC (25 vs 74)

GCC inner loop (from godbolt):

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

- GCC unrolls k-loop by 2: 2 broadcasts per j-loop, 2 FMAs per j iteration
  accumulating 2 k values into same C without reload? Actually it loads B[k][j],
  FMA into C, stores, then FMA B[k+1][j] into same C (reusing ymm0).
- Our version does 1 k per j-loop, 4-wide j-unroll. GCC's approach reduces k-loop
  iterations by 2× and improves cache reuse (A row stays in register for 2 k's?).

**Next steps:**

- Implement k-unroll by 2 in vectorizer: hoist 2 A broadcasts (ymm1, ymm2) and emit
  2× quad SIB FMA per j iteration, or 8× FMA with 2 broadcast registers.
  This would match GCC's 2× k unroll and potentially beat GCC on j-unroll.

- Eliminate double movslq: IV is byte offset i32, but SIB needs 64-bit index.
  32-bit ops zero-extend on x86-64, so `addl` already zero-extends; `movslq`
  is only needed for sign-extension. Since offset is always positive <2GB,
  zero-extend = sign-extend, so second movslq is redundant. Make IV i64.

- Use `vmovapd` vs `vmovupd`: GCC uses aligned `vmovapd` (faster on some uarchs)
  when it can prove alignment. Our C/B arrays are 16-byte aligned but j offset
  may not be 32-byte aligned for all chunks? 32-byte aligned for chunk 0, but
  32-byte offset for chunk1 etc preserves alignment if base is 32-byte aligned.
  We have 16-byte aligned BSS, not 32. Could promote to 32-byte align for AVX.

- Consider `vzeroupper` after matmul to avoid AVX-SSE transition penalty (GCC does).

## Artifacts

- `/home/user/ms178-1.patch` 22K, 462 lines, APPLIES-CLEAN on cdd3ba45
- `/tmp/matmul.s` final SIB+hoisted asm
- `/tmp/matmul_godbolt/` manifest: lccc 74, gcc 25, clang 438

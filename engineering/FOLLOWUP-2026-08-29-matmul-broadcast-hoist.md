# Follow-up — 2026-08-29: AVX2 matmul broadcast hoist (A[i][k] invariant)

**Base.** `ms178/lccc` main `796058f0` (#300 sticky-SSE). Previous session left
paravirt-improved commit `04ed7f41` on top.

## Problem

Final asm `/tmp/matmul.s` for `tests/benchmark/programs/matmul.c` (`-O3 -march=x86-64-v3`)
still had 4× `vmovsd (%r15)` + `vbroadcastsd` **inside** vector loop LBB6:

```
.LBB6:
  vmovsd (%r15), %xmm0
  vbroadcastsd %xmm0, %ymm1
  vmovupd (%rbx,%r12), %ymm0
  vfmadd231pd (%r14,%r12), %ymm1, %ymm0
  ... repeated 4× for +0,+32,+64,+96
```

Root cause analysis via `CCC_DEBUG_LICM_ALIAS=1`:

- First LICM iteration for header=5 (j-loop) **did** hoist `Value23 GEP base21 offset22` (= A[i][k])
  plus 8 other GEPs/Casts (`count=9`). Symbol map `LICM-SYMBOL` confirmed
  `A=62, B=63, C=64` canonical.
- After `vectorize` pass (between `loop_unroll` iter0 and `run_gvn_licm_ivsr_shared`),
  j-loop body is rewritten to `FmaF64x4` intrinsics:
  `Intrinsic FmaF64x4 dest_ptr 44 args [Value103, Value33]` x4 with offsets +32,+64,+96.
  `FMA ARG Value103 def_block Some(3) Phi dest103 ty Ptr incoming [(106,Block2),(104,Block7)]`
  where 106=Phi(62 from0,107 from3) and 104=GEP base103+8. So Value103 is k-loop
  induction for A[i][k] pointer, **invariant for inner j-loop** (def_block 3 outside
  body 5,6). No explicit `Load` in j-body; load is inside FMA intrinsic semantics
  (`*args[0]` broadcast).
- LICM only looks at `Load`/`Store` in `pointer_values={72,44,68,76}` (FMA dests), zero loads,
  `count=0` for second iteration. So invariant A load hidden inside `FmaF64x4` args
  cannot be hoisted by LICM.
- `value_to_base_global` maps Phi 90 (C) to itself (conflict 109 vs 90), not to canonical 64,
  so `stored_global_bases` is {90} or empty, but A base 62 is distinct, so scalar A load
  **was** hoistable for header=11 (Load dest86 ptr23 base21 offset22 def_block4 out-loop).
  After vectorize, A pointer becomes Value103 mapping to 103 (self), not 62.

Thus vectorizer reintroduced invariant load after first LICM hoist.

## Fix

`src/passes/vectorize.rs` `transform_to_fma_f64x4`:

- **Previously:** emitted 4× `FmaF64x4` with args `[A_ptr, B_gep]` inside loop.
  Backend lowering for `FmaF64x4` does `vmovsd (A_ptr), xmm1; vbroadcastsd; vmovupd (C); vfmadd231pd (B), ymm1, ymm0; vmovupd`.
  So 4 broadcasts per iteration.

- **Now (like 2-wide path):** hoist `BroadcastLoadF64` into preheader:

```rust
func.blocks[preheader_idx].instructions.push(Intrinsic {
  dest: None,
  op: BroadcastLoadF64,
  dest_ptr: None,
  args: vec![Value(pattern.a_ptr)],
});
```

  and emit 4× `FmaF64x4Hoisted` with only B ptr:

```rust
Intrinsic { op: FmaF64x4Hoisted, dest_ptr: Some(c_gep), args: vec![Value(b_gep)] }
```

Backend x86 lowering:

- `BroadcastLoadF64`: `vmovsd (%rcx), %xmm1; vbroadcastsd %xmm1, %ymm1` (ymm1 holds A)
- `FmaF64x4Hoisted`: `vmovupd (%rax), %ymm0; vfmadd231pd (%rdx), %ymm1, %ymm0; vmovupd %ymm0, (%rax)`

## Result

`/tmp/matmul.s` after fix:

```
.LBB4:
  movq %r15, %rcx
  vmovsd (%rcx), %xmm1
  vbroadcastsd %xmm1, %ymm1   # hoisted
  xorl %ebp, %ebp
.LBB6:
  movslq %ebp, %r12
  leaq (%r14, %r12), %r10
  ...
  vmovupd (%rax), %ymm0
  vfmadd231pd (%rdx), %ymm1, %ymm0
  vmovupd %ymm0, (%rax)
  ... x4, 0 vmovsd inside LBB6
```

- `vmovsd` count total: 1 (preheader) vs 4 before inside loop.
- `vbroadcastsd` count total: 1 vs 4 before.
- LBB6 vmovsd count: 0 (was 4).

## Validation

- `cargo test --profile fastbuild --locked`: 1304 passed, 0 failed.
- `target/fastbuild/lccc -O3 -march=x86-64-v3 -S -o /tmp/matmul.s tests/benchmark/programs/matmul.c`
  verified hoist.
- `scripts/godbolt.py compare ... --function matmul`:
  - lccc 96 insns, gcc16.2 25 insns, clang 438 insns.
  - Still behind GCC (GCC uses tighter SIB and less LEA overhead), but broadcast hoist
    removes 8 insns per iteration (4 loads + 4 broadcasts) and reduces memory traffic.
- `scripts/codegen_scoreboard.py .../matmul.c`: gap 71 vs best (gcc=25), loads 36, stores 7, vec 12.

## Next steps

- Reduce LEA/movq overhead in quad FMA: current emits `leaq (%r14,%r12), %r10; movq %r10, %rdx; movq %r9, %rax`
  for each chunk. Backend could use SIB directly: `vmovupd (%rbx,%r12), %ymm0` and
  `vfmadd231pd (%r14,%r12), %ymm1, %ymm0` without extra moves. Requires `FmaF64x4HoistedSIB`
  or smarter regalloc that recognizes GEP base+offset as single SIB.
- Consider unrolling k-loop or tiling for cache, but P0 is correctness + broadcast hoist.
- Apply same hoist to `transform_to_fma_f64x4` remainder handling if needed.
- Update snapshot ledger, deliver patch.

## Artifacts

- `/home/user/ms178-1.patch` (12K, 245 lines) — applies clean on `796058f0`, includes:
  - `04ed7f41 paravirt-improved` (Cast/Sub, versioned symbols, GOT/TLS guards, Copy-chain const extraction)
  - matmul broadcast hoist fix.
- `/tmp/matmul.s` final asm with hoisted broadcast.
- `/tmp/matmul_godbolt/` compare artifacts.

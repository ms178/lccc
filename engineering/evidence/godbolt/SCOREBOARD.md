# Godbolt corpus scoreboard — all `tests/benchmark/programs/*.c`
**Tree intent:** guide LCCC vs **gcc 16.2 (`cg162`)**, **clang 22.1 (`cclang2210`)**, **icx latest (`cicxlatest`)**.  
**Flags:** `-O2 -march=x86-64-v3`. **Date:** 2026-08-20. Whole-file CE compile (includes `main` harness).  
**Not** LCCC asm (no local `lccc` in this run). Compare to LCCC `-S` separately.  
Insn counts include `main`/init — use **feature columns** more than raw insns.

| Kernel | gcc ins | clang | icx | gcc ymm | icx ymm | gcc fma | icx fma | notes |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| `ackermann` | 133 | 31 | 34 | 0 | 0 | 0 | 0 |  |
| `arith_loop` | 223 | 113 | 251 | 0 | 0 | 0 | 0 | Clang 83 ymm on integer loop (!); gcc/icx stack-heavy 107/120 |
| `binary_trees` | 1478 | 146 | 134 | 0 | 0 | 0 | 0 |  |
| `bitops` | 99 | 94 | 112 | 0 | 0 | 0 | 0 | GCC/Clang `popcntl`; ICX did not in this build |
| `constant_recursion` | 135 | 32 | 35 | 0 | 0 | 0 | 0 |  |
| `expat_xml_scan` | 172 | 218 | 327 | 4 | 26 | 0 | 0 | GCC `btq`; ICX ymm on corpus fill |
| `fannkuch` | 135 | 226 | 109 | 6 | 6 | 0 | 0 |  |
| `fib` | 217 | 31 | 34 | 0 | 0 | 0 | 0 | clang/icx rec2iter-like tiny; gcc 217 recursive |
| `fp_memfold_stencil5` | 121 | 154 | 116 | 36 | 45 | 0 | 0 | all ymm stencil |
| `glibc_memcmp` | 157 | 227 | 311 | 3 | 3 | 0 | 0 |  |
| `gzip_crc32` | 50 | 82 | 100 | 0 | 0 | 0 | 0 | GCC: `xorl gzip_crc32_table(,%rdx,4)` SIB; no crc32 insn at -O2 |
| `hash_table` | 116 | 140 | 142 | 0 | 0 | 0 | 0 |  |
| `linux_find_bit` | 156 | 116 | 121 | 0 | 34 | 0 | 0 | GCC `andn`+`cmov` on ffs tree (not tzcnt); ICX uses ymm |
| `loop_patterns` | 179 | 354 | 245 | 91 | 163 | 0 | 0 | all ymm; clang 314 ymm aggressive |
| `mandelbrot` | 47 | 59 | 64 | 0 | 0 | 3 | 7 | all FMA scalar; ICX 7 gcc 3 |
| `matmul` | 82 | 164 | 131 | 50 | 55 | 1 | 8 | all three ymm; ICX/Clang more FMA than gcc (1 vs 8/16) |
| `nbody` | 215 | 266 | 531 | 0 | 97 | 24 | 58 | ICX ymm+more FMA; GCC scalar FMA 24 |
| `qsort` | 26 | 43 | 52 | 0 | 7 | 0 | 0 |  |
| `reduction_vecreg` | 107 | 105 | 125 | 36 | 70 | 0 | 8 | ICX/Clang FMA 8; gcc ymm no FMA |
| `sieve` | 45 | 365 | 376 | 0 | 42 | 0 | 0 | gcc scalar 45 ins; clang/icx vectorize fill (365/376) — not always better |
| `spectral_norm` | 174 | 365 | 253 | 103 | 111 | 0 | 10 | ICX FMA 10 gcc 0; gcc already ymm 103 |
| `sqlite_varint` | 325 | 375 | 421 | 0 | 41 | 0 | 0 |  |
| `strlen_bench` | 129 | 224 | 408 | 0 | 26 | 0 | 0 | ICX 40 calls + ymm — libc/vector mix |
| `struct_copy` | 100 | 76 | 82 | 0 | 0 | 4 | 27 | Particle kernel: gcc/icx FMA on distance; field copies still xmm |
| `switch_dispatch` | 70 | 80 | 87 | 0 | 0 | 0 | 0 | no stack gcc/clang |
| `tce_sum` | 10 | 10 | 13 | 0 | 0 | 0 | 0 | all ~10 ins counted loop |
| `vector_remainder` | 139 | 204 | 281 | 31 | 15 | 0 | 2 | Clang FMA 9; gcc ymm no FMA |
| `zlib_ng_adler32` | 156 | 147 | 147 | 0 | 0 | 0 | 0 | all ~150 ins, almost no stack (gcc 2 icx 3 clang 0) |

## Feature hits (gcc / clang / icx)

| Kernel | popcnt | tzcnt/bsf/lzcnt | andn | btq/btl | cmov | stk gcc/icx |
|---|---|---|---|---|---|---|
| `ackermann` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 6/3 |
| `arith_loop` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 107/120 |
| `binary_trees` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 237/5 |
| `bitops` | 1/1/0 | 0/0/0 | 0/0/0 | 0/0/0 | 7/7/7 | 0/3 |
| `constant_recursion` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 8/5 |
| `expat_xml_scan` | 0/0/0 | 0/0/0 | 0/0/0 | 1/0/0 | 0/2/0 | 4/3 |
| `fannkuch` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 3/2/3 | 0/6 |
| `fib` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 66/5 |
| `fp_memfold_stencil5` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 3/5 |
| `glibc_memcmp` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 22/8 |
| `gzip_crc32` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/3 |
| `hash_table` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 8/3 |
| `linux_find_bit` | 0/0/0 | 0/0/0 | 3/2/2 | 0/0/0 | 11/10/9 | 15/3 |
| `loop_patterns` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/3/1 | 2/4 |
| `mandelbrot` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/3 |
| `matmul` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 3/5 |
| `nbody` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/139 |
| `qsort` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/3 |
| `reduction_vecreg` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 1/0/0 | 1/3 |
| `sieve` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/6 | 2/17 |
| `spectral_norm` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 8/3 |
| `sqlite_varint` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/1 | 24/23 |
| `strlen_bench` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 5/56 |
| `struct_copy` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 8/3 |
| `switch_dispatch` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/3 |
| `tce_sum` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 2/5 |
| `vector_remainder` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 3/73 |
| `zlib_ng_adler32` | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 2/3 |

## How to reproduce

```bash
python3 scripts/godbolt.py compile gcc16.2 tests/benchmark/programs/KERNEL.c \
  --flags '-O2 -march=x86-64-v3'
# aliases: gcc16.2=cg162 clang=cclang2210 icx=cicxlatest
```

Asm dumps: `engineering/evidence/godbolt/corpus/KERNEL_{gcc16.2,clang,icx}.s`

## Implications for LCCC (precise)

1. **CRC:** table SIB is the bar at `-O2`; nobody used `crc32` insn here. LCCC 1.47× is addressing, not HW CRC.
2. **Adler:** oracles ~0 stack in the *whole file*. LCCC spilling sum2 in DO8 is uniquely bad.
3. **find_bit:** GCC lowers `~a & b` to **`andn`**, ffs tree to **cmov** not tzcnt. Teach ISel `andn` + consider tzcnt for `__builtin_ctz` (already in alu.rs) vs C if-tree idiom.
4. **bitops:** `popcntl` — LCCC alu.rs already has popcnt for UnaryOp; ensure BitOps kernel uses IR Popcount not a C loop.
5. **expat:** `btq` is gcc-specific; ICX uses ymm on fill + cmp chain. LCCC should match gcc `btq` on classify.
6. **nbody/matmul/spectral/mandelbrot/reduction:** ICX FMA count is the FP bar. GCC ymm without FMA on spectral/reduction.
7. **sieve:** clang/icx vectorizing memset/fill is optional; gcc scalar is short. Don't chase clang sieve size.
8. **fib/ackermann:** clang/icx collapse recursion; LCCC TCE/rec2iter already wins vs gcc — not a codec priority.
9. **struct_copy:** still not 2× ymm memcpy of Particle; distance uses FMA. ABI/SROA remaining.
10. **arith_loop:** clang vectorized integer; gcc 107 stack refs. LCCC RA pressure test.

# Current compiler state

SHA at last doc refresh: **`0cbdc40`** (`ms178/lccc` main). Re-verify line numbers before editing.

## What is production

- **C frontend** → SSA IR → `-O0` skip / `-O1` light / `-O2` full / `-O3` +unroll / `-Os`/`-Oz` size (`src/passes/README.md`).
- **Linear-scan RA** in `src/backend/live_range.rs` + policy in `regalloc.rs` (waves, coalescing, XMM/NEON, i686). Not a 3-phase greedy allocator. No `linear_scan.rs`.
- **SROA** `aggregate_sroa.rs` load-forward + chain collapse **on**. Copy-out **off** (`CCC_SROA_COPYOUT` hangs tests).
- **Alias** `alias.rs` (loop SCEV-lite). LICM does **not** yet use it for GEP loads (`licm.rs` TODO).
- **FMA** scalar `vfmadd231sd` and vector `vfmadd231pd` **emitters exist**. Auto-vectorize of non-reduction loops and FMA-in-vector-body are the remaining gaps.
- **YMM memcpy** exists in `memory.rs`. By-value struct ABI still shuttles `double` through `%rax` (`struct_copy` 21× vs GCC).
- **MachInst** exists; **disabled** when loop insts > 32 (`CCC_MI_MAX_LOOP_INSTS`) because the local scheduler **regressed gzip ~3%**.
- **PGO** generate/use; layout must not reorder hot loops (expat 131→248 ms). `vectorize_gate` is `return true`.
- **`enable_splitting`** in the scan is unused (`false`). Splitting is the IR pre-pass `split_ranges.rs` only.
- **`outline_switch`** min cases = **40**.
- UnaryOp already emits `lzcnt`/`tzcnt`/`popcnt`. C if-trees (`__ffs`, hand-rolled popcount) do not become those insns until recognized.

## What is still losing vs GCC 14 / gcc16.2 / ICX

| Gap | LCCC | Oracle |
|-----|------|--------|
| gzip `longest_match` | 118 stack-mem, GOT, 248 B frame | gcc RIP `window(%r9,%rcx)`, 1 push |
| Adler-32 kernel | 1.49×; `sum2`/`n` on stack in DO8 | CE whole-file ~0–3 stack refs |
| CRC-32 kernel | 1.49×; 2 vs 0 spills | gcc `xorl table(,%reg,4)` — **no `crc32` insn** at `-O2` |
| Expat name scan | 1.95× | gcc `btq` |
| xmltok / inflate TUs | 12× / 15× stack-mem | segment RA |
| struct_copy | 21.06× | SysV SSE class + no xmm↔rax field copies |
| nbody / spectral / mandelbrot | 3–9× screening | ICX FMA+YMM (copy **ICX**, not gcc16 horiz-per-iter) |
| find_bit | 1.85× screening | gcc `andn`+`cmov` on ffs tree (**not** tzcnt) |
| bitops | — | gcc/clang `popcntl` if IR is Popcount |
| sieve | 1.3× | gcc 45-insn **scalar**; clang ymm explosion — **do not copy clang** |
| fib/TCE geomean | LCCC can beat gcc | **not** a codec metric |

## Dual pipelines (do not “just enable”)

Text ISel + string peephole **and** MachInst + a second linear scan. Homes are many HashSets, not a `ValueLocation` enum. PhysReg **(11) = `%r10`**, (10) = `%r11`.

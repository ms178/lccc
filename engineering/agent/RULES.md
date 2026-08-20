# Rules

1. Correctness first: `cargo test --lib`; RA/SROA/MachInst also need gzip/expat/sqlite or the targeted kernel.
2. **gzip `longest_match` stack-mem must not rise.** That is the RA veto.
3. After every codegen change:  
   `python3 scripts/godbolt.py compile gcc16.2 tests/benchmark/programs/KERNEL.c --flags '-O2 -march=x86-64-v3'`
4. Copy **ICX** `vfmadd231pd` YMM accumulators. Do not copy GCC 16.2’s per-iteration horizontal add on reductions.
5. Do not force MachInst on large loops (`CCC_MI_MAX_LOOP_INSTS`; gzip −3%).
6. Do not enable `CCC_SROA_COPYOUT` without a dominator proof (hangs `structs_bitfields`, `simd_sse_float`).
7. Do not set `CCC_EVICT_MODE=5` or `CCC_PGO_WEIGHT_MAX>1` (gzip regressions).
8. PhysReg(11)=`%r10`, PhysReg(10)=`%r11`.
9. Do not put `cmp` in `CCC_X64_NOHOME_CLASSES` (flag replay vs `%rax`).
10. Fib/TCE geomean is not a win on codecs.
11. Nobody used the `crc32` instruction on the gzip kernel at `-O2`. The bar is **SIB table xor**.
12. Do not vectorize sieve like Clang (365 ins vs gcc 45 scalar).
13. Binary: `lccc`. Build: `scripts/build_lccc_fast.sh`. Swap: `scripts/ensure_swap.sh` on 2 GiB VMs.

Kill-switches: `CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`, `CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`.

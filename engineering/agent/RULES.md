# Rules

1. Correctness first: `cargo test --lib`; RA/SROA/MachInst also need gzip/expat/sqlite or the targeted kernel.
2. **gzip `longest_match` stack-mem must not rise.** That is the RA veto.
3. After every codegen change:  
   `python3 scripts/godbolt.py compile gcc16.2 tests/benchmark/programs/KERNEL.c --flags '-O2 -march=x86-64-v3'`
4. Copy **ICX** `vfmadd231pd` YMM accumulators. Do not copy GCC 16.2's per-iteration horizontal add on reductions.
5. Do not force MachInst on large loops (`CCC_MI_MAX_LOOP_INSTS`; gzip −3%). The `machinst_regalloc.rs` module is dead code — delete it (P0-01), do not wire it up.
6. Do not enable `CCC_SROA_COPYOUT` without a dominator proof (hangs `structs_bitfields`, `simd_sse_float`).
7. Do not set `CCC_EVICT_MODE=5` or `CCC_PGO_WEIGHT_MAX>1` (gzip regressions).
8. PhysReg(11)=`%r10`, PhysReg(10)=`%r11`.
9. Do not put `cmp` in `CCC_X64_NOHOME_CLASSES` (flag replay vs `%rax`).
10. Fib/TCE geomean is not a win on codecs.
11. Nobody used the `crc32` instruction on the gzip kernel at `-O2`. The bar is **SIB table xor**.
12. Do not vectorize sieve like Clang (365 ins vs gcc 45 scalar).
13. Binary: `lccc`. Build: `scripts/build_lccc_fast.sh`. Swap: `scripts/ensure_swap.sh` on 2 GiB VMs.
14. **Do not refactor `immediately_consumed` (RA-23) or `SlotAddr::Indirect(StackSlot(0))` (RA-24) without full differential testing** — they are hard blockers that will miscompile silently if the accumulator load order or `reg_assignments` convention is violated.
15. **Do not re-enable `bytes[i] as char` in `peephole_common.rs`** — it corrupts UTF-8. The `find_whole_word` + `from_utf8_unchecked` path is the only safe one.
16. **Do not re-introduce `MAX_ITERATIONS` in liveness** — the worklist dataflow is provably terminating (monotonic). A cap is a silent miscompile.
17. **`__builtin_cpu_supports` must fold from an exact allowlist** — the old "return 1 for everything except avx512" produced SIGILL paths on non-v3 CPUs.
18. **`usual_arithmetic_conversion` else-arm must use `size` comparison** — `signed_ty.size() > unsigned_ty.size() ? signed : signed.to_unsigned_version()`. The old "return signed" was wrong for `1LL + 1UL`.

Kill-switches: `CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`, `CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`, `CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`, `CCC_NO_PHI_COALESCE`, `CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_FOLDED_INDEX_LIVENESS`, `CCC_NO_LOAD_HAZARD_REFINE`, `CCC_NO_EAX_ALLOC`, `CCC_NO_LOOP_PIN`, `CCC_NO_VECREG`, `CCC_PGO_WEIGHT_MAX`, `CCC_TRACE_ALLOC`, `CCC_X64_NOHOME_CLASSES`, `CCC_NO_MACHINST`, `CCC_RA_EXPLAIN`.

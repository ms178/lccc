# 2026-08-21 session 35 — O0 multi-def correctness and C2x `__VA_OPT__`

Base: `b650e4daf094b10d049972f203b08ddcb7898681` (PR #161). Round-7 remains included cumulatively because upstream had not incorporated it when fetched.

## RA-27: non-SSA `-O0` correctness

The phi CFG differential exposed 125/200 O0 mismatches while O2/Os were clean. A pristine upstream build reproduced the failure. Seed 1 was localized to a loop-carried `uint64_t c`: direct `return c` was correct, but `return rot(c,39)` passed a stale physical home and returned zero. `CCC_NO_REGALLOC=1` fixed the complete seed.

Phi elimination intentionally leaves multi-def Copy webs, while the production scan assumes one definition per value. At `-O0` the optimizer does not restore SSA before codegen. `CodegenOptions::disable_regalloc` now carries the optimization-level contract into every backend; x86-64, i686, AArch64, and RISC-V pass empty register pools at O0 and use canonical stack homes. O1/O2/O3/Os/Oz are unchanged and retain full allocation.

Evidence:

- phi CFG differential: 475/600 before → **600/600** after (`O0/O2/Os`, 200 seeds);
- i686 alias differential: **540/540**;
- dedicated two-invocation runtime regression with pinned 64-bit checksums;
- structural check: no RA stats at O0, normal allocation at O2;
- standalone O0 compile on x86-64, i686, AArch64, and RISC-V.

## FE-22: C2x `__VA_OPT__`

Macro expansion now processes balanced `__VA_OPT__(tokens)` groups before stringification and token paste. The selected tokens then follow the ordinary parameter-substitution and rescan algorithm. Variadic presence handles both standard `...` and GNU named `args...`; empty or whitespace-only tails omit the group.

Runtime regression coverage:

- empty and nonempty comma insertion;
- nested expression tokens;
- `##` token paste inside `__VA_OPT__`;
- standard `__VA_ARGS__`;
- GNU named variadics.

## GlobalAddr CSE/GVN regression repair

PR #161's entry-block GlobalAddr hoisting merged variable-index table bases into one long-lived web. That evicted gzip CRC's masked index and regressed `movl sym(,%idx,4)` into shift/add/reload code. Variable-index GEP bases are now a third, site-local GlobalAddr class: the CSE pass neither hoists nor merges them, and GVN keys each by its original value id. The scale-4 SIB regression is restored.

Cross-block GlobalAddr hoisting is also refused in multi-block functions containing intrinsics. Intrinsic emitters still carry hidden accumulator/XMM location state (RA-23); hoisting across that state miscompiled `rdtscp` ordering and deferred SIMD chains. Same-block GVN and single-block intrinsic rewriting remain enabled. Runtime regressions for CRC, rdtsc/rdtscp, and deep SIMD defer all pass, while the original GlobalAddr CSE merge-count regression remains green.

## Validation

- 978 unit tests passed, 6 ignored.
- 50/50 correctness.
- 377/377 lccc-only regressions.
- 600/600 phi CFG differential.
- 540/540 i686 alias differential.
- gzip 1.14: 30/30; `longest_match` is 331 instructions / 119 stack references for both pristine PR #161 and treatment (zero stack-memory delta).

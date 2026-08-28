# Rules

Non-negotiable constraints. Each rule exists because breaking it shipped a
bug or a measured regression — see [`../DECISIONS.md`](../DECISIONS.md) for
the evidence. Re-oracle and check gzip `longest_match` stack-mem after
**every** codegen change.

## Correctness gates

1. Correctness first: `cargo test --lib` under `-D warnings`; RA/SROA/
   MachInst/vectorizer changes also need gzip/expat/sqlite or the targeted
   kernel plus the differential-fuzz battery.
2. **gzip `longest_match` stack-mem must not rise.** That is the RA veto.
3. After every codegen change:
   `python3 scripts/godbolt.py compile gcc16.2 tests/benchmark/programs/KERNEL.c --flags '-O2 -march=x86-64-v3'`
4. Benchmark numbers are only valid with output checksums (or exit codes)
   byte-verified against the GCC baseline; run the canonical runner
   (`tests/benchmark/run_benchmarks.py`), never single wall-clock runs
   (±9 % spread on identical binaries was measured).
5. Differential tests must compare **exit codes**, not just stdout. A new
   test must be mutation-verified: it must fail when the bug it targets is
   reintroduced.
6. Cloning/duplicating IR must use the canonical visitors
   (`for_each_operand_mut` / `for_each_value_use_mut` / terminator
   variants) — handwritten substitution missed 25 use positions once.
7. Any per-function backend state must be reset in `reset_for_function`
   (stale folded-GEP state fired spurious rematerialisations across
   functions).

## RA contracts

8. Keep register allocation disabled at `-O0` until the scan models non-SSA
   multi-def values (stack homes are the correctness contract; O1+ runs the
   production scan).
9. PhysReg(11)=`%r10` (static chain, live into every call), PhysReg(10)=
   `%r11`. `%cl` is a fixed operand of variable shifts — never renamed.
   A value live across a call must not ride a caller-saved register
   (window-local rule; keep the source eligible).
10. Preserve `ExplicitLocation::{Reg,Accumulator}` as the single non-stack
    scalar contract and exact `SlotAddr::Reg(PhysReg)` consumption — no
    dummy frame offsets, no location inferred from missing slots.
11. Do not delete `graph_coloring.rs`, `reg_hint`, `enable_splitting`, or
    `handled`: Tier-2 colorer is production (`CCC_NO_TIER2_GRAPH`),
    `enable_splitting` is the RA-06 stub gate, the others are wired
    verifiers/hints.
12. Do not set `CCC_EVICT_MODE=5` or `CCC_PGO_WEIGHT_MAX>1` as defaults
    (measured gzip regressions). Do not resurrect zero-future-use eviction
    or the adler accumulator-leader promotion without the Adler/gzip
    oracles (both measured negative, see DECISIONS.md).
13. Do not put `cmp` in `CCC_X64_NOHOME_CLASSES` (flag replay vs `%rax`).
14. GlobalAddr CSE placement is dominance/loop constrained: cold singletons
    and mutually-exclusive siblings stay local; loop values move only to
    the innermost preheader; derived variable-index bases stay site-local;
    no new preheader homes across intrinsic-bearing loops.

## Codegen contracts

15. Copy **ICX** `vfmadd231pd` YMM accumulators. Do not copy GCC's
    per-iteration horizontal add on reductions.
16. Do not force MachInst on large loops (`CCC_MI_MAX_LOOP_INSTS`; gzip
    −3 %). Keep the MachInst ISel/emit path; the dead RA module stays
    deleted.
17. Do not enable `CCC_SROA_COPYOUT` without a dominator proof (hangs
    `structs_bitfields`, `simd_sse_float`).
18. Fib/TCE geomean is not a win on codecs. Nobody used the `crc32`
    instruction on the gzip kernel at `-O2` — the bar is **SIB table xor**.
    Do not vectorize sieve like Clang (365 ins vs gcc 45 scalar).
19. `FpContract::Off` is the default (GCC `gnu*` parity); contraction never
    crosses tagged statement roots under `OnExpr`; FMA emission requires
    the FMA3 ISA feature (SIGILL on pre-Haswell otherwise).
20. Loop rotation (`loop_rotate.rs`) stays **opt-in**
    (`CCC_LOOP_ROTATE=1`) until the 15 known miscompile shapes are
    root-caused (list in DECISIONS.md). Never run it before vectorize.
21. Volatile is load-bearing: every pass that touches loads/stores gates on
    it (mem2reg once silently rebuilt loads with `volatile:false`). Do not
    fold `0.0 + x`, `fabs(x) >= 0`, or NaN/Inf float-to-int. Volatile
    loads are never merged or CSEd.
22. Narrowing/same-size casts always emit extending moves (stale upper
    bits killed zlib-ng); scalar FP staging never uses `movaps`; mixed
    legacy-SSE/VEX inside YMM loops re-triggers the 9× AVX-SSE transition
    penalty.
23. Do not re-enable `bytes[i] as char` in `peephole_common.rs` — it
    corrupts UTF-8. Do not re-introduce `MAX_ITERATIONS` in liveness (the
    worklist dataflow is provably terminating; a cap is a silent
    miscompile). Text-peephole operands split at the **last** comma
    (AT&T destination last).
24. `__builtin_cpu_supports` folds from an exact Raptor Lake allowlist
    (`expr_builtins.rs`, `PRESENT` const) — the old "return 1 for
    everything" produced SIGILL paths on non-v3 CPUs.
25. `usual_arithmetic_conversion` else-arm uses `size` comparison
    (`types.rs`): `signed.size() > unsigned.size() ? signed :
    signed.to_unsigned_version()`. The old "return signed" was wrong for
    `1LL + 1UL`.

## Environment / process

26. Build: `scripts/build_lccc_fast.sh` → `target/fastbuild/lccc`. Swap:
    `scripts/ensure_swap.sh` on 2 GiB VMs (needs CAP_SYS_ADMIN; otherwise
    rely on `-j2`). Mold-less sandboxes: pass
    `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc` and `RUSTFLAGS` as
    environment variables — `--config` does not override
    `.cargo/config.toml`.
27. Linker oracles (mold, wild) must be **git-HEAD builds**; stale release
    binaries produced false failures. GAS/binutils pin: 2.47. Record
    resolved oracle revisions in `ORACLE_REVISIONS.txt`.
28. ARM regression tests stay freestanding (host headers mask backend
    failures). QEMU wall time and shared-VM percentages are screening
    evidence only — PMU claims wait for the 14700KF (MS-14).
29. Snapshot immediately after every validated change; verify patches apply
    to a pristine `origin/main` worktree; move `artifacts/.base_ref` on
    every rebase.

## Kill switches (bisection / soundness fallback)

`CCC_NO_LOAD_CAST_FOLD`, `CCC_NO_X64_IMMED_NOHOME`, `CCC_MI_FORCE_LOOPS`,
`CCC_MI_MAX_LOOP_INSTS`, `CCC_SROA_COPYOUT`, `CCC_EVICT_MODE`,
`CCC_NO_COALESCE`, `CCC_DEBUG_RA`, `CCC_DUMP_IR`, `CCC_NO_PHI_COALESCE`,
`CCC_NO_LEAF_PARAM_GPR`, `CCC_NO_FOLDED_INDEX_LIVENESS`,
`CCC_NO_LOAD_HAZARD_REFINE`, `CCC_NO_EAX_ALLOC`, `CCC_NO_SEGMENT_FILL`,
`CCC_NO_INDEX_HOME`, `CCC_NO_ABI_REG_HINTS`, `CCC_NO_LICM_ALIAS`,
`CCC_NO_ANDN_FUSION`, `CCC_NO_64B_YMM_COPY`, `CCC_ENABLE_TIER2_GRAPH`,
`CCC_NO_TIER2_GRAPH`, `CCC_NO_LOOP_PIN`, `CCC_NO_VECREG`,
`CCC_PGO_WEIGHT_MAX`, `CCC_TRACE_ALLOC`, `CCC_X64_NOHOME_CLASSES`,
`CCC_NO_MACHINST`, `CCC_RA_EXPLAIN`, `CCC_NO_SEGMENT_FILL`,
`CCC_NO_GLOBAL_ADDR_REMAT`, `CCC_NO_DSE`, `CCC_NO_STENCIL_VEC`,
`CCC_NO_MAP_VEC`, `CCC_NO_MAP_VECREG`, `CCC_NO_DEFER_OVERFLOW_VECREG`,
`CCC_NO_BT_BRANCH_FUSION`, `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`,
`CCC_NO_PF06_ADD_PEEL`, `CCC_LOOP_ROTATE` / `CCC_NO_LOOP_ROTATE` /
`CCC_DEBUG_LOOP_ROTATE`, `CCC_BEPRE_FP`, `CCC_VERIFY_REGALLOC`,
`CCC_TRACE_ALLOCSTATS[=filter]`, `CCC_RA_EXPLAIN`, `LCCC_DEBUG_VECTORIZE`,
`LCCC_WHY_NOT_VECTORIZE`, `LCCC_DUMP_IR`, `LCCC_NO_PEEPHOLE` (x86-64;
`CCC_NO_PEEPHOLE` is the i686 gate), `CCC_M16_KEEP`, `CCC_M16_FULL_PIPELINE`,
`CCC_PEEPHOLE_SKIP=pass,…` (i686), `LCCC_LD_PARALLEL`, `LCCC_LD_ICF`.

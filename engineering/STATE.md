# Current compiler state

State refreshed against **`2f5e9af3`** (`ms178/lccc` main, PR #281) with
16-byte-granular `va_arg` support on AArch64/RISC-V64 and target-aware
plain-char signedness. Re-verify line numbers before editing. The item catalog is
[`agent/BACKLOG.md`](agent/BACKLOG.md); the active queue is
[`tasks/`](tasks/README.md); the negative-results ledger is
[`DECISIONS.md`](DECISIONS.md).

## What is production

- **C frontend** → SSA IR → `-O0` skip / `-O1` light / `-O2` full /
  `-O3` +unroll / `-Os`/`-Oz` size (`src/passes/README.md` is the
  authoritative tier list).
- **Linear-scan RA** in `src/backend/live_range.rs` (scan) +
  `src/backend/regalloc.rs` (policy). **Segment-aware interference is the
  scan's primary model** (RA-05a landed): `LiveRange::segs` +
  `segments_conflict`, coalesce-leader piece/use union, Phase-2f residual
  fill generalized to all targets and default-ON (multi-piece values
  ranked first, pressure-gated). Kill switches: `CCC_NO_SEGMENT_FILL`.
  Tier-2 hole-aware graph coloring is **production default** for the
  eligible subset (`CCC_NO_TIER2_GRAPH` restores the scan-only path).
- **ABI physical hints** (RA-26) retain leading ParamRefs across safe
  call-free x86 CFG leaves; ordered caller homes; stack-arg/mixed/calling
  shapes fail closed (`CCC_NO_LEAF_PARAM_GPR`,
  `CCC_NO_EMPTY_LOCAL_FRAME_ELISION`).
- **RA verifier**: `CCC_VERIFY_REGALLOC=1` hard-verifies segment interference,
  final assignments, and the eviction occupancy history (half-open
  `[start, cut)` for evicted ranges).
- **-O0** deliberately uses canonical stack homes on all four backends
  because phi elimination leaves non-SSA multi-def webs (RA-27).
- **Liveness** `src/backend/liveness.rs` — worklist backward dataflow (no
  `MAX_ITERATIONS` cap), fat `intervals` plus hole-aware `segments`.
- **FMA / FP contract**: `FpContract { Off, OnExpr, Fast }` threaded
  cli→pipeline→passes→backend. Default **Off** (GCC `gnu*` parity);
  `-ffp-contract=fast`/`-ffast-math` enable Fast; `-ffp-contract=on`
  contracts only within a tagged statement root. FMA emission requires the
  FMA3 ISA feature (SIGILL guard). Scalar and packed `vfmadd231*` emitters
  are production.
- **Vectorizers**: reduction (single/multi/secondary accumulator),
  widening I32→I64 + masked conditional-sum, stencil (constant-tap affine),
  elementwise map expression trees, plain-copy; per-natural-loop PGO
  profitability gate (exact trips; trip <8 rejected, >80-inst bodies need
  ≥32 trips). Kill switches: `CCC_NO_STENCIL_VEC`, `CCC_NO_MAP_VEC`,
  `CCC_NO_VECREG`.
- **Loop rotation** `src/passes/loop_rotate.rs` is **opt-in**
  (`CCC_LOOP_ROTATE=1`): correctness-clean for the canonical counted-loop
  shape (v14 exit-merge-phi fix, v17 cross-phi latch-incoming rewrite), but
  15 shapes still miscompile under default-enable (DECISIONS.md). Runs
  after vectorize.
- **DSE** `src/passes/dse.rs` (same-block, closed-alloca escape analysis,
  byte-range kills; `CCC_NO_DSE`); backedge PRE (integer recurrences
  default-on; FP variants gated); GVN per-object epochs for disjoint
  non-escaping allocas + `restrict` params; GlobalAddr CSE with
  oracle-derived placement (cold branch-local, loop preheader, reuse of
  dominating defs; derived variable-index bases site-local).
- **Aggregates**: AVX2 64-byte assignment = 2 YMM pairs + `vzeroupper`;
  32/48-byte copies stay XMM (measured). SysV all-SSE 16-byte struct
  returns use xmm0/xmm1.
- **MachInst** ISel/emit path exists, gated off when loop body > 32 insts
  (`CCC_MI_MAX_LOOP_INSTS`; the local scheduler regressed gzip ~3 %). The
  dead `machinst_regalloc.rs` module was deleted (P0-01a).
- **PGO** generate/use; layout must not reorder hot loops (expat
  131→248 ms). Per-loop trip/body profitability only.
- **Sema** enforces assignment/prototype-arity/return constraints,
  `__seg_fs`/`__seg_gs` declarator/local threading, GNU char-pointee
  pointer-sign warning parity; C2x `__VA_OPT__`, `_Pragma` operator,
  address-space-qualified lvalue stores (LK-26 fix, `afa22485`).
- **Multi-arch**: x86-64, i686 (natural 4-byte slots, m16 boot pipeline,
  32 KiB boot gate PASS at `.text` 23,378 / cliff 23,384), AArch64
  (CASP, MOVW `:abs_g*:`, `.org`, PREL64, G1/G2/SABS reloc repair),
  RISC-V (va_arg struct{long double} end-to-end padding). Assembler +
  ELF linker in-tree for all four.
- **Kernel bring-up**: linux-cachymod 6.18.46 boot code compiles + links;
  `-pg`/`-mfentry` mcount family emitted; objtool interop is the next hard
  gate (see `tasks/`).

## What is still losing vs GCC (canonical 2026-08-28 screening)

Ratio = LCCC/GCC, `-O2`, paired medians, checksums verified. Geomean
0.738 (33 pairs), conventional-code geomean 1.096 (30 pairs, recursion
folds excluded). Full table: root `README.md`; evidence:
`evidence/benchmarks/2026-08-28-1b3994e7/`. Root-caused gaps, by magnitude:

| Gap | Kernel | Root cause | Tracked as |
|-----|--------|-----------|------------|
| 2.15× | tls_seg_access | thread-pointer materialization; direct `%fs:offset` covers only link-time-constant offsets | unassigned (candidate IS) |
| ~1.50× | adler32 | arithmetic-chain copy webs; no reload-at-use | RA-06/PF-05 (P0) |
| 1.23× / 1.31× | nbody / spectral | non-reduction FP loops; multi-store scatter; marching-pointer slot-homing (mandelbrot 1.23×) | OP-05b, RA-01b (P0) |
| suite-wide | every loop kernel | loop rotation default-off (double-jump preheader, ~1 branch/iter) | PF-17 hardening (P0) |
| ~1.80× | expat | hash-multiply `imul` chains | OP-25/PF-15 (P1) |
| ~1.51× | linux_find_bit | branchy ffs tree → andn+cmov chain | IS-11 (P1) |
| isort (CE) | instruction count | secondary-IV strength reduction (43 vs 23) | PF-06 (P1) |

## Dead code — nuanced assessment

Not all dead code should be deleted:

| Item | Verdict |
|---|---|
| `machinst_regalloc.rs` | DELETED (P0-01a) |
| `graph_coloring.rs` (slot colorer) | KEPT/PERFECTED — Tier-2 production default, `CCC_NO_TIER2_GRAPH` |
| `reg_hint: Option<PhysReg>` | WIRED (RA-26) |
| `enable_splitting: bool` | KEPT — the RA-06 stub gate (do not delete) |
| `handled: Vec<ActiveInterval>` | WIRED (RA-13 verifier) |

## Hard constraints (short form; full list in `agent/RULES.md`)

1. gzip `longest_match` stack-mem must not rise; adler/gzip/expat oracles
   gate RA work.
2. Do not enable MachInst on large loops; do not enable `CCC_SROA_COPYOUT`
   without a dominance proof; do not set `CCC_EVICT_MODE=5` or
   `CCC_PGO_WEIGHT_MAX>1` as defaults.
3. PhysReg(11)=`%r10` (static chain), PhysReg(10)=`%r11`; `cmp` never in
   `CCC_X64_NOHOME_CLASSES`.
4. `FpContract::Off` is the default; FMA needs the FMA3 feature gate.
5. Loop rotation stays opt-in until the 15 known miscompiles are
   root-caused.
6. Preserve `ExplicitLocation::{Reg,Accumulator}` and exact
   `SlotAddr::Reg(PhysReg)`; no dummy frame offsets.

## S08: wide va_arg + target char signedness (2026-08-28)

- **16-byte-granular va_arg, AArch64 + RISC-V64**: `va_arg(ap, __int128)`
  has dedicated 16-byte-aligned arms on both backends (AArch64 rounds
  `__gr_offs` UP like glibc; RISC-V rounds the overflow-area POINTER);
  `va_arg(ap, long double)` on RISC-V stores the full binary128 value
  (was silently `__trunctfdf2`-truncated). The riscv arms stage values
  through t3/t4 — t0/t1 clobbered the `&va_list` pointer needed for the
  write-back (SIGSEGV). Align-16 composites now take even-aligned GP
  pairs on BOTH arches, named and variadic (GCC oracles verified;
  the old "AAPCS64 needs no even pairs" call_abi note was wrong and is
  gone). `tests/regression/va_arg_wide_struct.c` green on aarch64 AND
  riscv64; lib 1273/0/6; ARM 378/30/75; x86 536/5 (same 5 pre-existing).
- **Target plain-char signedness**: plain `char` was compiled signed on
  every target (x86 default leaking into AArch64/RISC-V SysV where char
  is unsigned). Resolved at the parse boundary — the parser rewrites
  plain char to UnsignedChar on unsigned-char targets, so downstream
  CType/IR mappings stay unconditional; `-fsigned-char` /
  `-funsigned-char` override (both directions GCC-verified). Caught by
  cast_chain_fold.c GCC differential; new
  `tests/regression/char_signedness_target.c` pins the
  declaration-vs-constant agreement invariant on all targets.

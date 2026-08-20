# Engineering research report — beat GCC / Clang / ICX on register allocation

**Date:** 2026-08-20  
**Target CPU:** Intel Core i7-14700KF (Raptor Lake P-cores)  
**Constraint:** C semantics, SysV AMD64 / i686 / AArch64 / RISC-V  

## Competitive position (qualitative)

LCCC now has a **real** linear-scan RA with ABI-aware waves, not a
stack-only compiler. That is necessary and **not sufficient**.

GCC `ira`+`reload`/`lra`: graph coloring, regional allocation, remat,
call-save, pressure-aware.

LLVM: `RegAllocGreedy` (eviction + split + hints + evicting to make
hints), plus `RegAllocFast` at -O0. Live intervals are **hole-aware**.
Remat and fold memory operands.

ICX/ICC: aggressive remat, GPR↔XMM spill, loop IV pinning, often wins
integer codecs.

**Hypothesis:** LCCC loses on (1) spanning values without remat,
(2) whole-lifetime spills in `longest_match`-class loops, (3) missing
memory-operand folding after spill, (4) inliner/RA feedback.

## Literature → LCCC applicability

| Paper / system | Idea | In LCCC? | Action |
|----------------|------|----------|--------|
| Poletto & Sarkar 1999 | linear scan | yes | keep as fast path |
| Traub / Holloway / Smith 1998 | second-chance binpacking | no | prototype D |
| Wimmer & Mössenböck 2005 | use-point split | IR-only timid | P0/P1 |
| Braun & Hack 2009 CC | MIN next-use SSA | tie-break only | make primary |
| Chaitin 1982 / Briggs 1994 | optimistic coloring | stack slots only | residual graph |
| George & Appel 1996 | iterated coalescing | merge-disjoint only | affinity |
| Park & Moon 2004 | optimistic coalescing | no | after H |
| LLVM Greedy | cascade, split, hints | cascade only | import split, not code |
| Pereira & Palsberg 2005 | puzzle / SSA | no | spike if SSA denser |
| Colombrini et al. / PBQP | irregular arch | no | AArch64 later |
| Rong et al. / ML-inlining | ML | no | after heuristic ceiling |

**Do not copy GCC/LLVM.** Re-derive cost models on 14700KF counters
(cycles, loads retired, L1 D-miss, port 2/3 pressure).

## Milestones to beat the SOTA (RA slice)

### M0 — Measurement harness (2 weeks)

- Golden: gzip (`longest_match`, `deflate`, `fill_window`), zstd
  `ZSTD_compressBlock`, zlib-ng adler32/crc32, sqlite `vdbe`,
  libjpeg-turbo, kernel `copy_page`.
- For each: LCCC vs gcc-14 -O2 vs clang-20 -O2 vs icx -O2 if present.
- Artifacts: asm of hot function, `perf stat -e cycles,instructions,L1-dcache-load-misses,branches`, spill count, callee-saved push count.
- Database: `hotspots/ra/`.

**Blocker:** no RA-specific spill metric in CI. Add
`CCC_TRACE_ALLOCSTATS` aggregation: assignments, spills, evictions.

### M1 — Remat + call-site save (4 weeks)  [ideas B, C]

Win condition: nbody / kernel address-heavy functions ≤ GCC stack ops;
gzip not regressed.

Blockers: codegen remat of GlobalAddr; cost model for call-site
store/load vs callee-saved.

### M2 — Segment scan + stronger split_ranges (4 weeks) [A, I, D lite]

Win: sqlite diamond CFGs fewer spills than today; no miscompile on
csmith/yarpgen.

Blockers: one-home vs multi-home; phi insertion for cross-block splits.

### M3 — Second-chance / use-point reload (6 weeks) [D]

Win: gzip `longest_match` within 2% of GCC -O2 **or** beat it.
This is the RA boss fight.

Blockers: spill insertion vs SSA; interaction with peephole; must not
break i686 eax model.

### M4 — Residual Briggs + affinity coalescing (6 weeks) [E, H]

Win: sqlite geomean; kernel compile of `amdgpu` hot files smaller stack.

### M5 — Pressure feedback to inliner/LICM (ongoing) [G]

Win: no “inline then spill everything” on sqlite.

### M6 — GPR→XMM spill cache (optional) [J]

Win: integer kernels with 10+ live values; ICX-class.

## Blockers (engineering)

1. **One home per value** in codegen — splits need a project, not a
   function.
2. **No official spill/reload IR** — split_ranges uses volatile alloca;
   a first-class `Spill`/`Reload` would simplify D.
3. **Emitter clobber oracle** — i686/x86 scratch is duplicated in RA by
   hand. Generate clobber sets from the same tables as emit.
4. **PGO default off** — without block freq, 10^depth is a blunt
   instrument.
5. **Test time** — OnceLock env knobs make parameterized RA tests hard.
6. **Arch matrix** — x86-64, i686, AArch64, RISC-V; a greedy feature
   that helps gzip-x86 must be kill-switched elsewhere.

## Statistical protocol

Raptor Lake: pin P-core (`taskset`), disable turbo for variance or
document it, N≥15 runs, report median + MAD. Reject RA patches on
noisy 0.3% arith_loop. gzip and zstd are the gates.

## “Beat ICX” specifically

ICX’s typical edges: remat, XMM spill cache, IV pinning, instruction
selection folding `[mem]` so a spill is **free**. Parallel work:

- ISel memory operands for spilled values (add/sub/cmp with mem)
- Then RA **should spill** if it enables a mem-op and frees a reg
  (contra naive “spills are always bad”)

That interaction is how ICX often looks like “better RA” when it is
**better ISel+RA**.

## Screening already run (2026-08-20)

[VALIDATION_ZLIB_GZIP_EXPAT.md](VALIDATION_ZLIB_GZIP_EXPAT.md): gzip /
zlib-ng / Expat `-S -O2` vs GCC 14. `longest_match` 118 vs 0 stack
ops; Adler 1.49×; Expat scan 1.95×; inflate 15× stack-mem. Confirms
M1 remat and M3 second-chance as the gzip path; M2 segments for
xmltok/inflate.

## Next 30 days (concrete)

1. Remat + `%rip` for file-scope `window`/`prev`/`strstart` (gzip
   `longest_match` is the unit test).
2. Next-use eviction so Adler DO8 does not spill `sum2`/`n`.
3. RA explain dump for `longest_match`.
4. Cache `RangeMetadata` across waves; fix 2c `exchange_eviction` drift.
5. Clang `-O2 -S` on the same TUs (ICX if available).
6. Do **not** turn on mode 5 globally.
7. Do **not** raise `CCC_PGO_WEIGHT_MAX` without gzip numbers.

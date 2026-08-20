# Weaknesses, progress, and ideas (audit 2026-08-20)

Priority score ≈ expected runtime impact × #workloads × confidence / cost.

## Measured 2026-08-20 (zlib-ng / gzip / expat)

See [VALIDATION_ZLIB_GZIP_EXPAT.md](VALIDATION_ZLIB_GZIP_EXPAT.md).

Kernels (correct, median of 7): Adler **1.49×** GCC, gzip CRC **1.47×**,
Expat name-scan **1.95×**. gzip `longest_match`: **118 stack-mem vs 0**,
248 B frame, GOT reloads of `window`/`prev`. zlib-ng `inflate`: **15×**
stack-mem. Expat `xmltok.c`: **12×** stack-mem. CRC is **not** a spill
problem (2 vs 0 stack-mem).

## Progress since March 2026 docs

Shipped: linear scan kernel, copy groups, phi latch coalescing, hole-aware
call-span, multi-wave caller-saved, i686 ecx/edx/eax, XMM/NEON, vecreg
whitelist, AArch64 index liveness + loop-pin, split_ranges pre-pass, PGO
hook (neutral default), steal-safety, cascade, ILP rotation, group/GEP
priority bumps, many measured kill-switches from real miscompiles
(sqlite varint, gzip longest_match, nbody GlobalAddr, printf arg regs,
adler32/memcmp params, fa[i] aarch64).

**Not shipped:** in-scan splitting, remat, graph-coloring GPRs, PBQP,
second-chance, live-range splitting at every use, ML heuristics.

## Defects / correctness landmines

1. **Fat intervals in the scan.** Segments exist but Phase 1/2 still
   allocate on `[start,end]` envelopes. Mutually exclusive arms interfere
   in the allocator even when `place_edge_copy_blocks` tried to help.
2. **`verify_no_overlap` vs die-at-birth.** Debug checker uses strict
   overlap on fat intervals; false positives hide real bugs.
3. **Two coalescers.** RA copy groups vs `stack_layout/copy_coalescing.rs`
   can assign a register to a leader and a *different* slot story to a
   member if a later pass drops the assignment.
4. **`enable_splitting` dead.** Callers may think splitting is on.
5. **Vecreg fail-closed whitelist.** New SIMD intrinsic not listed →
   silent stack; listed wrong → miscompile. Process: tests per intrinsic.
6. **PGO OnceLock.** First parse wins for the process; tests cannot
   vary `CCC_EVICT_MODE` / `CCC_PGO_WEIGHT_MAX` in one binary.
7. **i686 `%eax` model** is instruction-shape based, not a real clobber
   set from the emitter. New inst class = latent clobber.
8. **Loop-pin x86 disabled** for gzip; AArch64-only means x86 IVs still
   spill in `longest_match`.

## Cost-model holes (performance)

| Hole | Why GCC/ICX win |
|------|-----------------|
| No remat | GlobalAddr/lea/const occupy callee-saved for the whole function |
| Whole-lifetime spill | One conflict demotes every remaining use to memory |
| Use-count ≠ next-use distance | Braun–Hack MIN only as tie-break, not primary |
| No block frequency except PGO=1 | Nested-loop 10^d saturates at depth 4 |
| No register pressure in inliner | Inliner can create uncolorable regions |
| No caller-saved *across* calls with save/restore around the call | LLVM/GCC insert call-site saves; LCCC forces callee-saved or spill |
| Rotation vs ABI | ILP rotation ignores “prefer rbx already saved” |
| XMM vs GPR coupling | Independent scans; no cross-class packing |
| Split pre-pass too timid | Cross-block post-call uses skipped (needs phis) |

## Brilliant / high-leverage ideas (post-audit)

### A. Segment-aware linear scan (P0)

Feed **segments**, not envelopes, into `LinearScanAllocator`. A value is
several ranges that may reuse the same physreg or different regs. This
is 80% of Wimmer splitting without SSA rename if holes are truly dead.

*Blocker:* codegen assumes one home. Need either (i) one physreg for all
segments of a value, still reducing interference, or (ii) insert copies
at hole boundaries.

### B. Call-site caller-saved with extra spill/reload (P0)

For a spanning value, compare:

- callee-saved (prologue save once)
- caller-saved + store/load around **each** call in its segments
- remat at each use

Cost = `n_calls * (store+load)` vs `2 * prologue` vs `n_uses * remat`.
On Raptor Lake, a leaf-ish function with one printf should **not** burn
`r12` for a format string if remat `lea` is one µop.

### C. Rematerialization class (P0)

Classify: `Imm`, `GlobalAddr`, `LabelAddr`, `LEA(base+off)` with base
always live, `zext` of already-live, `xor dest,dest`. Prefer remat over
spill. GCC’s reload and LLVM’s `isTriviallyReMaterializable` are
baselines — **do not copy**; measure lea vs mov on 14700KF.

### D. Use-point split / second chance (P1)

Wimmer: spill only until next use, then reload into a free reg. Requires
spill/reload IR or machine instrs. Highest expected gzip/zstd win.

### E. Interference graph on residual (P1)

After linear scan, build interference on **spilled** + **high-priority**
values only; Briggs optimistic color with 1–2 rounds. Hybrid beats pure
scan on diamond CFGs. Keep scan as the fast path for `|values| < 64`.

### F. Next-use distance as primary key (P1)

Replace `priority = uses * 10^d` with `spill_cost = Σ block_freq[u] *
reload_latency` and evict max next-use (Belady/MIN). Keep loop depth as
freq proxy when PGO absent. Re-tune gzip `longest_match` as the oracle.

### G. Pressure-aware inlining / LICM (P1)

Export `reg_pressure_estimate(loop)` to inliner and LICM. A LICM that
lengthens a live range across a call is often a **loss** with this
allocator. GCC’s `ira` pressure model is the competitor.

### H. Copy coalescing as pre-color / affinity (P1)

Instead of merging intervals (which lengthens live ranges and causes
spills), keep values separate and add **affinity edges** (prefer same
reg). PBQP or iterated coalescing (George–Appel). Current merge is
correct only when intervals are disjoint — it **cannot** coalesce
overlapping copies (the profitable case).

### I. Caller-saved around loops with peeling (P2)

If a loop is call-free but the header is after a call, split at the
header (already in `split_loop_transparent_ranges`) more aggressively:
multiple exits, inner-loop only.

### J. XMM/YMM as spill cache (P2)

On x86-64, GPR spill to XMM (`movq %r12, %xmm8`) avoids cache. ICX/ICC
do this. Raptor Lake has 16 XMM; integer kernels with 12 live ints can
beat stack. Need type-tag on PhysReg class.

### K. Explainability dump (P2)

`CCC_RA_EXPLAIN=longest_match` → “v42 not inlined into r12: spanning
call at pp 318; remat unavailable; eviction lost to v17 priority 1000”.
Required for the research loop.

### L. Superopt of copy sequences (P3)

After RA, e-graph on the copy/shuffle between calls. Small.

### M. ML eviction (P3)

Only after F is a strong heuristic baseline. Train on gzip/zstd/sqlite
**held-out** files. Reject if geomean ≤ mode-3.

## Backlog table

| ID | Item | Evidence | Expected | Workloads | Diff | Status |
|----|------|----------|----------|-----------|------|--------|
| A | Segment scan | diamonds interfere | 1–5% | sqlite, kernel | M | open |
| B | Call-site save | format strings in r12 | 1–8% | gzip, printf-heavy | M | open |
| C | Remat | nbody +274B story inverted | 2–10% | nbody, kernel | M | open |
| D | Second-chance | gzip mode-5 fail | 3–12% | gzip, zstd | H | open |
| E | Residual coloring | scan local optimum | 1–4% | sqlite | H | open |
| F | Next-use cost | longest_match | 1–6% | gzip | M | open |
| G | Pressure to inliner | spill after inline | 2–8% | sqlite | M | open |
| H | Affinity vs merge | overlapping copies | 0.5–3% | all | M | open |
| I | Stronger loop split | split_ranges timid | 1–4% | loops | M | open |
| J | GPR→XMM spill | ICX | 0–5% | integer | M | open |

## Regression policy (RA-specific)

Any RA change: run gzip, zstd, zlib-ng, sqlite, arith_loop, sieve.
Reject >2% gzip `longest_match` unless explained. Miscompile = revert.
Keep a kill-switch for 1 release.

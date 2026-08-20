# Register allocation roadmap

## Phase 1 (March 2026) — COMPLETE

The Week 1–3 checklist below is **historical**. Linear scan, live_range.rs,
and integration shipped. There is no feature flag `linear-scan-allocator`.
Paths are `src/backend/`, not `ccc/src/backend/`.

Historical targets (11 KB → 256 B, 3–4× on a 32-var micro) are **not**
the current KPI. See [README.md](README.md).

---

## Phase 2+ (August 2026 onward) — beat GCC/Clang/ICX

Copy of the engineering plan; details in
[RESEARCH_REPORT.md](RESEARCH_REPORT.md) and
[WEAKNESSES_AND_BACKLOG.md](WEAKNESSES_AND_BACKLOG.md).

| Phase | Theme | Done when |
|-------|-------|-----------|
| 2.0 | Docs + metrics | this directory matches code; spill counts in CI |
| 2.1 | Remat GlobalAddr/const + call-site save/restore | nbody/kernel memops ≤ GCC; gzip unregressed |
| 2.2 | Segment-aware scan | sqlite diamonds; no csmith RA miscompile |
| 2.3 | Stronger `split_ranges` (multi-exit, more splits) | loop-transparent IVs |
| 2.4 | Second-chance reload | gzip `longest_match` within 2% GCC -O2 |
| 2.5 | Affinity coalescing (not interval merge) | fewer moves |
| 2.6 | Residual Briggs coloring | sqlite/kernel stack |
| 2.7 | Pressure → inliner/LICM | no spill-after-inline disasters |
| 2.8 | GPR→XMM spill cache | ICX-class integer kernels |

### Definition of done for “beat SOTA” (RA contribution)

- Geomean runtime on {gzip, zstd, zlib-ng, sqlite, libjpeg-turbo} ≤
  best of gcc-14 -O2 / clang -O2 / icx -O2 on i7-14700KF, **and**
- no workload in that set >5% slower unless justified with counters,
- fuzz/regression green.

RA alone may not get there; ISel mem-ops and inlining are in the loop.

---

## Historical Week 1–3 checklist (do not execute)

### Week 1: Infrastructure & Analysis — done 2026-03

- [x] Study then-current 3-phase allocator
- [x] Design linear scan
- [x] Integration plan

### Week 2: Core implementation — done (evolved far past the checklist)

- [x] `src/backend/live_range.rs` (not `ccc/src/...`)
- [x] LinearScanAllocator (in live_range.rs, not linear_scan.rs)
- [ ] register_pool.rs — **never created; not needed**
- [x] Wire allocate_registers — **replaced**, no fallback 3-phase

### Week 3: Validation — ongoing forever

Correctness is continuous (phi tests, fuzz). 3–4× stress-test target
is obsolete.

## File structure (actual)

```
src/backend/
├── live_range.rs      # scan kernel
├── regalloc.rs        # policy
├── liveness.rs
├── split_ranges.rs
└── stack_layout/      # spilled values
```

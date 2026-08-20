# Phase 1 Week 1 (March 2026) — historical

**Status:** Complete as of 2026-03-19. **Not a description of the
current allocator.**

The findings below (574-line 3-phase, ~5% eligibility, 11 KB stack,
Week 2 not started) were accurate for that week. By 2026-08-20 the
production allocator is linear scan plus coalescing, hole-aware
call-span, i686/XMM/AArch64 policy — see [README.md](README.md) and
[CURRENT_ALLOCATOR_ANALYSIS.md](CURRENT_ALLOCATOR_ANALYSIS.md).

Do not schedule “Phase 2a: create live_range.rs”. It exists (1261
lines) and is the scan kernel.

---

## Original report (archived)

**Timeline:** Week 1 of 3 (March 19, 2026)

**Goal:** Understand current allocator, design replacement algorithm, plan integration.

### Completed tasks (9/10)

Analysis of the then-current three-phase allocator, register map,
32-var shuttle pattern, Poletto/Sarkar design, prologue integration.
Deliverables: CURRENT_ALLOCATOR_ANALYSIS, LINEAR_SCAN_DESIGN,
INTEGRATION_POINTS (all since rewritten).

Task 10 (32-var test binary) was deferred; the micro is no longer the
gate. gzip `longest_match` is.

### Then-stated weaknesses (still conceptually valid, different magnitude)

- Conservative eligibility
- No in-scan interval splitting (**still true**)
- No coalescing (**false** — copy groups + hints exist)
- Greedy without rich cost analysis (**partially true** — loop weights + mode 3)

### Then-stated 3–4× speedup

Plausible vs a **non-allocating** compiler. Not the remaining gap vs
GCC -O2.

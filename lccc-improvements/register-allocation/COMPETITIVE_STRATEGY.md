# Competitive strategy — register allocation

LCCC must beat GCC, Clang, and ICX. RA is necessary but not the only
axis. This note is how the RA team spends the next cycles.

## Do not optimize the March metric

“32-var function, 11 KB → 256 B, 3–4×” was a 2026-03 diagnosis of a
**non-allocating** compiler. That war is over. The next war is:

> Why does GCC’s `longest_match` run in fewer cycles than LCCC’s
> with similar instruction counts?

If IPC is lower: spills, false deps, extra moves. If instruction
count is higher: missing coalescing/remat/ISel fold. If branches:
block layout, not RA.

## Oracle workloads (RA)

| Workload | Why |
|----------|-----|
| gzip `longest_match` | eviction / spanning / IV |
| zstd match finder | same, more ILP |
| zlib-ng adler32 | param homes, GEP base (already burned us) |
| sqlite vdbe | diamonds, phis, calls |
| nbody | GlobalAddr remat |
| kernel copy/unaligned | i686/x86 addressing |

If a patch helps a microbench and hurts gzip: **reject**.

## Algorithm posture

Keep linear scan as the **workhorse**. Invent on top:

1. Better **inputs** (segments, remat class, true frequencies)
2. Better **outputs** (reload points, mem-op folds)
3. Better **feedback** (pressure to inliner)

A greenfield PBQP allocator is a research spike, not a rewrite, until
M0 numbers say scan is the ceiling.

## Invent vs adapt

| Problem | Adapt | Invent if |
|---------|-------|-----------|
| Remat | LLVM trivially-remat set | Raptor Lake lea vs mov cost differs |
| Split | Wimmer use-point | SSA+volatile alloca already in tree |
| Evict | Greedy cascade | mode 5 already lost; try MIN+freq |
| Coalesce | George–Appel | disjoint-merge is a local maximum |
| Spill to XMM | ICC | encoding/REX cost on 14700KF |

## Interaction wins (often larger than RA)

- ISel: `add mem, reg` for spilled
- LICM vs RA pressure
- Unroll vs live values
- Inlining vs call-span (inlining can **remove** spanning!)

A 10% “RA win” may be “inline the helper so the value is call-free
and takes r11”. Track that as IPO, not RA pride.

## Cadence

Every cycle: workload → perf → asm → root cause → paper or invention
→ prototype on 10 micros + 5 reals → correctness fuzz → integrate →
measure again. Abandon if gzip + sqlite disagree without a cost-model
fix.

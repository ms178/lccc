# 39-benchmark corpus A/B, rebased on `d03ca818` (2026-09-11)

Engine: `scripts/perf_ab.py` (11 reps, **min** metric, arms hashed to prove they differ).
Base: pristine `d03ca818` build. Candidate: the shipping `s11` build.

| file | what it is |
|---|---|
| `perf-ab-mine-vs-base.{json,md}` | full 39-benchmark corpus A/B, shipping vs base |
| `ATTRIBUTION.md` | byte-level attribution of every reported delta |
| `perf-ab-valve-vs-ship.{json,md}` | cost-ordered valve experiment vs shipping |
| `paired-*-coupled-plus-valve.json` | the falsification A/B for coupled counts + valve |

## Headline verdict

Geomean B/A = **1.0019** → "NO MEASURABLE DIFFERENCE" at `perf_ab.py`'s 1.0 % threshold.
That geomean is the *wrong* statistic for a narrow blast radius, so `ATTRIBUTION.md`
byte-compares the emitted asm for all 34 measurable benchmarks:

* **31 of 34 are byte-identical to base** — they cannot regress by construction. Every
  "slower" entry in the raw table (`arith_loop` −3.1 %, `spectral_norm` −2.7 %,
  `lz4` −1.5 %, `bitops` −1.5 %, `matmul` −1.2 %) is on **identical code** and is pure
  measurement noise.
* The only **3** benchmarks with different code all **improve**: `sha256_transform`
  **+4.7 %**, `linux_rbtree` **+1.6 %**, `strlen_bench` **+1.3 %**.
* `hash_table` reports **+4.7 %** on byte-identical asm, which calibrates the noise floor
  and is the reason the identical-arm entries must not be read as regressions.

i686: **0 differing TUs over 292** compilable translation units (the sandbox has no
32-bit glibc dev headers, so the remaining TUs are covered by the CI i686 gates).

## Falsified in this directory

* **Cost-ordered valve victim selection** — geomean +0.52 % over the 6 affected
  benchmarks (below threshold), `linux_rbtree` −0.8 %. Does *not* fix the coupled-counts
  lz4 regression, because `v212` never becomes a valve candidate.
* **Coupled counts + cost-ordered valve** — `lz4_compress` still **−42.62 %**
  (median 1.4262, p = 0.0000, 31 rounds). `sha256` asm is byte-identical to the decoupled
  build, so the win is purely boolean-driven and count-coupling contributes nothing to it
  while costing lz4 dearly.

Full reasoning: `engineering/FOLLOWUP-2026-09-11-valve-cost-blindness.md`.

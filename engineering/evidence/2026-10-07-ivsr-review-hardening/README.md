# 2026-10-07 — review-response hardening evidence

Backing data for
[`../../FOLLOWUP-2026-10-07-ivsr-review-hardening.md`](../../FOLLOWUP-2026-10-07-ivsr-review-hardening.md),
the round that answered an external review of merged PR #772 (`c3259492`).

| file | what it is |
|---|---|
| `static-neutrality.json` | the whole neutrality argument: 1480 assembly comparisons over three corpora (402 benchmark+oracle, 844 `tests/regression` with per-file `.flags`, 234 `tests/bench` + `kernel_corpus` + `patterns`), 0 changed, 1 skipped, with per-file numbers for the one proposal that was **measured and reverted**, and the `-fprofile-generate` PID nondeterminism that had to be normalised before "0 changed" meant anything |
| `four-vendor-oracle.json` | `godbolt_oracle.py --oracle-set all-vendors` at `-O2` re-run after the oracle program changed: 9 agree / 0 diverge / 0 error, LCCC 1512 insns vs GCC 16.2 1852, Clang 23.1.0 2274, ICC 2021.10 3714, ICX latest 2441 |
| `four-vendor-oracle-ivsr-domains.json` | the same sweep restricted to `ivsr_index_domains`, whose affine case was repaired this round (it indexed a loop-invariant parameter, so it exercised nothing) — 1 agree / 0 diverge, LCCC 165 vs GCC 118, Clang 348, ICC 376, ICX 458 |

The 2026-10-06 copies of the two oracle JSONs are superseded: the checksum of
`ivsr_index_domains` changed when its affine case was fixed, so the old records
would not reproduce. They are left in place as the record of that round.

Static counts, VM executions and Compiler Explorer only. **No PMU, no cycles, no
Raptor Lake, no speedup claim.**

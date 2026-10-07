# 2026-10-06 — IVSR index-domain evidence

Backing data for
[`../../FOLLOWUP-2026-10-06-ivsr-domain-audit.md`](../../FOLLOWUP-2026-10-06-ivsr-domain-audit.md).

| file | what it is |
|---|---|
| `summary.json` | machine-readable adjudication: defects fixed, proposal rejected with its measured zero delta, corpus A/B, consumer count, open gaps |
| `four-vendor-oracle.json` | `godbolt_oracle.py --oracle-set all-vendors` over the whole standalone corpus at `-O2`: 9 agree / 0 diverge / 0 error, plus per-program instruction counts for LCCC, GCC 16.2, Clang 23.1.0, ICC 2021.10 and ICX latest |
| `four-vendor-oracle-ivsr-domains.json` | the same oracle restricted to the new `ivsr_index_domains` program, whose base-tree build segfaults |
| `corpus-ab-value-only-predicate.json` | the 55-program static A/B that measured the cost of the *value-only* cast predicate (8154 vs 8139 instructions) |

All measurements are static instruction/stack-reference counts or VM executions.
No PMU counters, no Raptor Lake, no runtime speedup claim. The slow CI half was
not run this round by explicit instruction; see §7 of the follow-up document.

# 2026-10-06 — IVSR index-domain evidence

Backing data for
[`../../FOLLOWUP-2026-10-06-ivsr-domain-audit.md`](../../FOLLOWUP-2026-10-06-ivsr-domain-audit.md).

| file | what it is |
|---|---|
| `summary.json` | machine-readable adjudication: defects fixed, proposal rejected with its measured zero delta, corpus A/B, consumer count, open gaps |
| `four-vendor-oracle.json` | `godbolt_oracle.py --oracle-set all-vendors` over the whole standalone corpus at `-O2`: 9 agree / 0 diverge / 0 error, plus per-program instruction counts for LCCC, GCC 16.2, Clang 23.1.0, ICC 2021.10 and ICX latest |
| `four-vendor-oracle-ivsr-domains.json` | the same oracle restricted to the new `ivsr_index_domains` program, whose base-tree build segfaults |
| `corpus-ab-value-only-predicate.json` | the 55-program static A/B that measured the cost of the *value-only* cast predicate (8154 vs 8139 instructions) |
| `refactor-neutrality.json` | proof that sharing the two cast predicates onto `IrType` (previously spelled three ways in three files) changed no generated code: 5388 = 5388 assembly-identical over 911 sources |

**Superseded checksums.** `four-vendor-oracle.json` and
`four-vendor-oracle-ivsr-domains.json` record the `ivsr_index_domains` program as it
was on 2026-10-06. That program's affine case was repaired on 2026-10-07 (it indexed
a loop-invariant parameter, so it exercised nothing), which **changes its checksum**:
re-running the oracle against today's source will not reproduce these two files. The
current records are
[`../2026-10-07-ivsr-review-hardening/`](../2026-10-07-ivsr-review-hardening/). These
are kept as the record of this round, not as a live baseline.

`callgrind-kernel-ab.json` used to live here and has moved to
[`../2026-10-07-perf-ivsr-ptradd/`](../2026-10-07-perf-ivsr-ptradd/): it measures the
October-7 performance round, not this one.

All measurements are static instruction/stack-reference counts or VM executions.
No PMU counters, no Raptor Lake, no runtime speedup claim. The slow CI half was
not run this round by explicit instruction; see §7 of the follow-up document.

# Main integration, 2026-11-03 — bounded evidence, not final acceptance

Integrates main `03b3aea212ea8e47988a831020838fe81ab01a9d` into the
PR1065 task branch. The previously published `dca36a18e` has 26 successful
hosted checks, including the full kernel/objtool/QEMU job. That historical
result does **not** validate this merged source.

## Confirmed integration failures and repairs

1. Incoming SOURCE_DATE_EPOCH error construction omitted the diagnostic kind
   introduced on this branch. The fastbuild failed with E0063. Mark the
   diagnostic Cpp, consistently with existing hard preprocessor diagnostics.
   It remains in the hard-error collection, not a suppressible warning.
2. Incoming lazy DATE/TIME expansion bypassed the old MacroDef flag-based
   warning hook. Negative control on the merged binary, with only repair 1:
   `const char *x = __DATE__;` under `-E -Werror=date-time` exited **0**.
   Record uses in the ownership-checked dynamic DATE/TIME branches, like
   TIMESTAMP. Remove the now-unreachable flag mechanism and its constructor
   boilerplate instead of retaining two competing ownership mechanisms.
   Existing macro ownership/save/restore logic remains authoritative.

Expanded date/time regressions exercise TIME and TIMESTAMP aliases, unused
arguments, stringification, user redefinition and push/pop restoration, in
addition to the existing DATE cases. Oracle compilations are non-kernel
fixtures, never GCC-built kernel translation units.

## Local merged-source results

- Fastbuild all compiler/linker binaries: PASS.
- Library units: **7687 passed, 0 failed, 10 existing ignored**.
- All-target Clippy with `-D warnings`: PASS.
- Rust formatting and diff whitespace checks: PASS.
- Date/time diagnostics: **1108 checks**, mandatory GCC16.2/Clang23.1.4
  controls: PASS.
- Preprocessor builtin-scope/epoch validation: PASS.
- Composed warning precedence gate, including include paths, layout, enums,
  enum ABI and cross-target verbose assembly: PASS. ICX is explicitly
  unmeasured locally, not replaced by Clang.
- Expanded parity/popcount cross-backend and runtime contracts: PASS,
  runtime hash `09dc4aea9979cf33`.
- CI lane-plan/shard-partition checks and census ratchet: PASS.

The conflicting workflow and lane plan retain the executable-command
multiplicities of **both** parents. Regenerated frontend partition: 153 gates,
50/50/53 gates per lane; recorded-cost predictions about 318.5 seconds each,
0.1-second imbalance. These are predictions, not measured CI completion times.

The proposed 128-bit constant-mask issue in the incoming count-width optimizer
was rejected: the consumer scalar gate admits only 8/16/32/64-bit integers.
Shared-count and architecture profitability guards were also retained; no
speculative optimizer patch was made on those hypotheses.

## Still open

The all-13-warning/options requirement, zero kernel diagnostics, remaining ABI
and mitigation investigations, broader performance work, and current-source
full local kernel/QEMU acceptance are **not complete**. Local host prerequisites
are being recovered; Linux v6.18.58 has been fetched at
`2926d88bb7fc4608fd942453d929c39e4e5ec006`. No newly booted kernel is claimed.
The authenticated ICX provisioner failed with a TLS EOF in this network
sandbox; its historical hosted success does not become a local measurement.
Fresh hosted checks must validate the new merge commit before any green-CI
claim about this source. PR1065 remains a draft.

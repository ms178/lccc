# Empirical PR826 red-team evidence

See ../../PR826-REDTEAM.md for methods, limitations, and interpretation.

- `reference-corpus-o2.json`, `reference-corpus-o3.json`: raw existing-runner
  GCC 12.2 / Clang 21.1 correctness screenings. **No LCCC locally; no timing claims.**
- `lld-valid.json`: 38 well-formed executable/shared invocations, field bytes
  and sentinels checked, with commands/fixture hashes/generator identity.
- `lld-malformed.json`: intentional disagreement experiment. 86 FAIL records
  mean LLD accepted malformed inputs contrary to the strict rejection contract;
  do not summarize this file as a passing normative gate or weaken LCCC to match.
- `lld-incomplete-field.json`: byte-level accepted-image probe. The field is
  incompletely relocated; the adjacent sentinel remains intact.
- `local-results.json`: tool identities, provenance hashes, local/hosted validation
  distinctions, swap confirmation, and the hardware-cycle availability probe.

Hosted results in the snapshot are for the named tested head, not automatically
for a later commit. Final-head verification remains necessary. Compiler versions
and supported workloads are stated exactly, not relabeled as the newest possible
competitors or every file in the source directory.

Generated binaries/disassembly stay outside Git. Raw JSON is retained because
it contains source hashes, commands, outputs, and failure records, not just grades.

# Whole-tree audit evidence, 2026-10-14

Initial baseline: `3b66cd1d73743a7a79f912c01df48ac15cc646ef`.
Final base: `1e83776f41529e2d754380d1d934ff901e25a097`; files suffixed
`-rebased` and rebase-validation.json carry new executions, not relabelled data.
Read `engineering/AUDIT-2026-10-14-whole-tree.md` and the A14 queue in root
backlog.md before interpreting the files. No runtime improvement is asserted.

- `inventory.json`: immutable tracked-tree inventory, not semantic proof.
- `contracts.json`: 29 focused observations, including positive controls;
  14 contract violations on the audited baseline. Replay via
  `scripts/audit_contract_probes.py --compiler-probes --extended --check`.
  Nonzero status is intentional while those implementation tasks remain open.
- `environment.json`: actual CPU/kernel/local tools/swap/compiler identity.
- `script-syntax.json`: exact files checked by Python AST / Bash syntax parsing.
- `ctest-gcc.xml`, `ctest-lccc.xml`: original Expat 4,932-check reports.
  The CTests pass; the existing JUnit interpretation then fails (A14-29).
- `gzip-correctness.json`, `gzip-check-*.log`: 30/30 tests for each compiler
  and six exact compressed-stream matches; deliberately excludes invalid timing.
- `zlib-ctest.log`: 68/69 original upstream tests, not a claimed pass.
- `zlib-maketrees/`: standalone licensed miscompile extraction, expected/wrong
  table, sanitizer and TU-mixing receipts, replay instructions (A14-30).
- `nonutf8-symbols/`: source + mutation recipe (A14-31), no opaque ELF file.
- `ci-memory/`: order-dependent automatic resource selection and observed
  Clippy OOM (A14-34); replay uses actual functions with stub Cargo/free.
- `oracle/`: requested remote compiler identities, commands, assembly and
  static-count screen; ICC v3 rejection remains explicit.

Temporary absolute paths in captured command receipts identify the original
run; do not expect them to survive a wipe. Source text is embedded in the
contract records; source fixtures and replay instructions are committed here.
Compiler binaries, source clones and build trees must be regenerated. The
separate persisted source archive/cumulative patch restore this exact work.


## Final-main repetitions

`rebase-validation.json` is the summary/provenance for rebuilt 1e83776;
`contracts-rebased.json` has 30 observations and 13 violated contracts. The
new multiline-string control explains why the current count is not just the
old count minus two fixes. `ctest-*-rebased.xml`, `gzip-check-*-rebased.log`,
`zlib-ctest-rebased.log`, `expat-parser-rebased.log`, and the child-directory
`*-rebased.json` files retain new executions without overwriting old evidence.

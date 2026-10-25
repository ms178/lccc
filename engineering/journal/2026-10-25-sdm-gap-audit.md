# SDM gap-count red-team — 2026-10-25

## Scope

Follow-up to MB-45 and PR #968. The user requested an implementation campaign
for the 896 / 454 `--missing` list and replacement of Clang 23.1.3 with the
latest released LLVM toolchain.

## Findings implemented

* The row audit was originally modelled once per source row. That is not sound
  when one dispatch arm binds multiple spellings whose emitted opcodes differ
  (`lzcnt`/`tzcnt`/`popcnt`, width-suffixed rotate/shift forms). The gap builder
  now evaluates each `(row, spelling)` pair, matching the already spelling-aware
  verification path.
* Corrected Group-1 ALU and `test` model opcode width-bit handling: byte uses
  the base opcode; word/dword/qword forms use `opcode | 1` where the ISA defines
  the W pair. Regression tests pin `84/85`, `A8/A9`, `F6/F7` and `D0/D1`.
* Model `+rd` register-in-opcode families as a full 0..7 range for push/pop,
  bswap, movabs, xchg and the 16-bit push/pop variants. Added long-mode FS/GS
  push/pop opcode forms and i686 short inc/dec variants.
* Replaced the 23.1.3 Clang pin throughout active workflow, installer, test and
  tool references with LLVM 23.1.4. Refreshed the official release asset SHA-256
  from the GitHub release API: `efdd1dbd089b99bfcfcde7b8eb1236163f2e88df5a981cca4f783ec9cb5e0378`.

## Re-derived state

`python3 scripts/gas_encoding_crossref.py --missing --backend all` now reports
**806 encodings / 438 mnemonics**, down from 896 / 454. This is a report count,
not evidence that all 813 are missing executable functionality: the analysis
still mixes legacy, VEX/EVEX, APX, operand-class variants and aliases. A complete
implementation claim requires separating those classes with provenance and
byte tests; this session does not claim that the entire list is closed.

## Validation

* `python3 -m unittest tests.corpus.test_gas_encoding_crossref tests.corpus.test_gas_snapshot_decode` — 39 tests, one expected skip without the pinned oracle pair.
* `python3 scripts/gas_encoding_crossref.py --verify --strict --backend all` — 1638 rows, 224 code-model forms, zero findings.
* `python3 scripts/check_doc_links.py` — pass.
* `PYTHONPATH=/tmp/arena-pydeps python3 scripts/check_ci_workflow_shell.py` — pass.
* `python3 tests/regression/check_ci_link_oracle_provisioning.py` — 16 tests pass.
* `git diff --check`, shell syntax checks — pass.

No compiler build/test was run locally; GitHub CI is the build and runtime gate.

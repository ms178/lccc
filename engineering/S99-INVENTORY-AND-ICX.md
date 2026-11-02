# S99: inventory integration and actual current Intel oracle

## Evidence inventory RED/GREEN

The first fresh FAST run exposed PR836/PR837 integration incompatibility.
The cumulative manifest relocated the original PR831 audit to a retained
historical path and added `relocated_from`; PR837 strengthened row schema and
introduced a separate closed scope still pointing at the rewritten live doc.
The checker correctly rejected the unknown field; merely removing it would
still fail independent completeness.

Alternatives rejected: do not allow arbitrary extra fields, drop the file,
weaken closed scope, or refresh its digest from current review prose. Verify
against `git show 63e783b15586eca86f306bd75a52b82c57304bbb:engineering/PR831-AUDIT.md`:
exact equality with the retained file, SHA256
e3af0932bb02500031d3289d6aaff7979a3e389bdffa27e6baf0571811961956.

Repair DATA only: remove the unsupported provenance field, explicitly update
the independently reviewed scope to the retained location, document that
relocation in its description. No verifier relaxation, file deletion, content
hash change or capture-commit change. New pinned-original regression plus
existing actual-inventory test were RED (1failure/1error); all8 integrity tests
now pass, including malformed-schema, deleted-row, symlink, corrupt-gzip and
size-cap controls. Read-only checker verifies all69 original artifacts.
The ongoing mixed-tree FAST run is diagnostic only, never a final pass stamp.

## Actual newest Intel package oracle

The official Intel apt index's highest Debian-ordered compiler metapackage is
2026.1.1-325. Five compiler/CPU-runtime/header packages were downloaded and
SHA256-checked, then extracted in excluded tool cache (no toolkit-wide install).
The executable identifies **ICX2026.1.1 (2026.1.1.20260724)**. Package receipts,
index-version selection proof, provisioning/replay scripts and all raw case
records are in `engineering/evidence/redteam99/fresh-oracles.tar.xz`.

Default Intel FP policy is NOT equivalent to the requested strict semantics:
51/65 semantic cases matched,10 FENV compilations rejected precise-disabled
pragmas,4 packed-reciprocal output hashes differed. All4 original/bad directive
controls matched their expected exit statuses. Do not report those as proved
Intel compiler bugs.

With explicitly recorded Intel `-fp-model=strict`, **65/65 semantic cases and
4/4 directive controls pass; compiler stderr is empty throughout**. This is a
separate qualified arm, not a relabeling of the default-policy failures.
A preliminary `-fno-fast-math` FENV probe also passed. Compiler Explorer remains
an independent12-control execution set using its actual2024.0.0 ICX binary;
local2026.1.1 results must not be presented as remote CE2026 execution.

The same archive preserves the earlier candidate/GCC/Clang65-case matrix and
pinned GAS2.47/ArmXML provisioning receipts. GCC optimized5/65 fixture-contract
exceptions remain recorded, not adjudicated vendor defects. No timings or
external expert approvals follow from any of these semantic controls.

# PR835 follow-up evidence — 2026-10-15

This inventory authenticates bytes, not scientific interpretation. Reports
retain original commands, source/compiler hashes, comparison labels and raw
outputs. `paired-callgrind-raw.tar.xz` retains the original paired manifests,
profiles and logs for all 45 cases and focused controls. It contains no frozen
compiler executable: build the corresponding named source checkpoint. Absolute
workspace paths in original reports are historical producer paths, not claims
that those files exist in a fresh checkout.

The follow-up native94, FENV and packed tests are execution evidence; opcode
checks and source contracts are structural tripwires, not execution proofs.
Callgrind Ir is simulated work, not hardware timing, IPC or EPYC emulation.
See `../../PR835-REVIEW-AUDIT.md` for attribution and unclosed work. None of these
receipts substitutes for an exact-final-tree FAST receipt or hosted slow CI.

The uploaded PR contained 118 added evidence files plus one modified README,
not 119 newly added files. Seven byte-identical redundancies in five groups
have been removed. `ARTIFACTS.json` records former paths, authenticated canonical
references and bounded inventory scope; it deliberately does not fabricate
119 unique files after deletion. Original bytes remain in Git history.

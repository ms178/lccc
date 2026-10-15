# Retained-evidence verifier contract

The independent scope file enumerates exactly 69 historical retained paths;
it is deliberately separate from the byte/hash rows. Deleting both evidence
and its hash row is incomplete. This is a closed historical inventory, not a
claim that all present or future engineering evidence is covered.

Metadata schema version 1 rejects unknown/missing keys, duplicate JSON keys,
wrong scalar types (including boolean byte counts), and noncanonical digests.
Data hashes are streamed in at most 1 MiB chunks. Per-file limits: 256 MiB stored,
64 MiB decoded, 4 MiB JSON metadata. Gzip CRC/truncation errors fail verification.
All evidence path components must be nonsymlinks; raw-log members are leaf names.
The original raw-log digests remain independent cross-checks, not regenerated
historical claims. No retained evidence payload was changed.

Storage policy: the 1.31 MiB `pr834-runtime.tar.xz` remains in Git because CI
must independently verify all 14,639 exact historical payloads without a
network dependency. This is an explicit exception, not a new default for
arbitrary large binaries: future archives over 1 MiB require a documented
inventory, reproducibility/provenance justification, CI-verified hashes and
review of whether external content-addressed storage with pinned download
hashes can preserve offline/CI availability. Do not move this archive or
rewrite its historical hashes just to save space.

This checks integrity and closed-inventory completeness in a stable checkout.
It does not authenticate the author, protect against concurrent filesystem
replacement, prove execution, or make historical timings reproducible by itself.
Measured-tree/tool/argv and reproducible corpus linkage are a separate evidence
attachment, not an interpretation of these 69 historical hashes.

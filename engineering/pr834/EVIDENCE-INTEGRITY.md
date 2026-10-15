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

This checks integrity and closed-inventory completeness in a stable checkout.
It does not authenticate the author, protect against concurrent filesystem
replacement, prove execution, or make historical timings reproducible by itself.
Measured-tree/tool/argv and reproducible corpus linkage are a separate evidence
attachment, not an interpretation of these 69 historical hashes.

## Storage policy

New generated artifacts larger than **1 MiB stored** belong outside Git by
default: Actions artifacts for short-lived investigations, GitHub releases for
long-lived retained attachments, with SHA-256, byte count, producing tree/tool
identity and retention location recorded in a small manifest. An expiring
Actions URL alone is not permanent evidence. The existing hash-linked
`pr834-runtime.tar.xz` and its inventory intentionally remain in-tree: offline
CI verifies their complete linked receipts without network access or executing
the archived binaries. They are an explicit frozen exception, not permission
to add routine compiler binaries, parent profiles or unbounded traces. Any new
large in-tree exception needs explicit storage review, a bounded verifier and
an independently pinned inventory; never rewrite existing receipts to meet a
size target. The verifier's defensive reader caps are not storage allowances.

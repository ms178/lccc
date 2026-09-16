# Session journal

Point-in-time engineering history, distilled from per-session follow-up
documents into one dated digest per period. This directory is the **only**
place session narrative lives in the tree; everything older is in git
history (upstream merges pin the exact commits each entry describes).

## Retention rule (binding)

- **Journal** (here): what was measured, fixed macro-architecturally,
  reverted, or falsified in a dated session — in the terms the next agent
  needs to *not repeat it*. One entry per session document, compressed
  hard. Validation-gate tables and snapshot ledgers are summarized in one
  line; the full tables are in git history.
- **[`../DECISIONS.md`](../DECISIONS.md)**: the canonical ledger for
  decisions and negative results that must never be re-derived. Journal
  entries may link into it, never duplicate it.
- **[`../subsystems/`](../subsystems/)**: durable per-subsystem contracts
  and open items. Session narrative must not live there.
- **[`../evidence/`](../evidence/README.md)**: frozen data-bearing
  artifacts (oracle corpora, paired A/B records). The journal links to
  those; it does not hold numbers of record.
- New sessions **append a `## <date>` section to the current period
  file** — they do not create new per-session documents. If a session's
  content is not worth a digest entry, it is not worth committing.

## Period files

| File | Window | Source lineage |
|------|--------|----------------|
| [`2026-08.md`](2026-08.md) | 2026-08-18..08-31 | boot-quest start, matmul trilogy, SCCP adoption, IR-verifier birth, Regehr corpus |
| [`2026-09-W1.md`](2026-09-W1.md) | 2026-09-01..09-05 | IR-verify-zero → MachInst build-out → Tier-2 audit → RA cost-model wave |
| [`2026-09-W2.md`](2026-09-W2.md) | 2026-09-06..09-12 | Rust modernisation, PF-17/OP-05 epics, GLA Phase 1 + remat, i686 addressing/casts/encoder audits |
| [`2026-09-W3.md`](2026-09-W3.md) | 2026-09-13..today | linker arc (PIE, CET, .gnu.hash, ICF), flag-consumer walk, BB-SLP |

## Disposition of deleted sources

Every deleted per-session document has exactly one entry in the period
files above, tagged `<!-- src: <original-path> -->`. The disposition
classes:

- **digested** — entry below carries the durable content; full text lives
  in upstream git history at the merge commit the entry names.
- **moved-durable** — content was durable (contracts, gates) and moved
  into `../subsystems/`, `../agent/`, or `../DECISIONS.md`; the entry says
  where.
- **subsumed** — fully superseded by a later documented state; the entry
  names the superseding state instead of replaying the session.

The complete deleted-source list with per-file disposition is at the end
of each period file (machine-checkable; `scripts/check_doc_links.py`
keeps every cross-reference from live docs to these files valid).

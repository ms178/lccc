# Next agent

1. Read [`../STATE.md`](../STATE.md).
2. Read [`RULES.md`](RULES.md).
3. Check [`../DECISIONS.md`](../DECISIONS.md) for anything touching your area
   (measured negatives, reverted experiments, root causes).
4. Pick work from [`../tasks/`](../tasks/README.md) — the active queue. The
   item catalog behind it is [`BACKLOG.md`](BACKLOG.md).
5. Execute in the order given by [`SEQUENCE.md`](SEQUENCE.md).
6. Re-oracle with [`../evidence/godbolt/`](../evidence/godbolt/) as the baseline.

Do not invent a new allocator from scratch. Do not cite obsolete paths
(`ccc/src`, `linear_scan.rs`, binary name `ccc`). Do not resurrect anything
marked REVERTED in `../DECISIONS.md` without new evidence and the named
oracles.

**Session notes policy (2026-09-16+):** do NOT write new `FOLLOWUP-*.md` /
`AUDIT-*.md` documents for a session. Append one entry — tagged with
`<!-- src:` comments only if digesting an older document — to the current
period file under [`../journal/`](../journal/README.md), and keep durable
contracts in [`../subsystems/`](../subsystems/README.md). The doc-link gate
(`scripts/ci_local.sh` gate `doc-link-integrity`, `scripts/check_doc_links.py`)
rejects broken markdown references.

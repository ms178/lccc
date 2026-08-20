# Historical development records

Point-in-time status documents, phase summaries, and benchmark snapshots.
**Nothing in this directory describes the current state of the compiler** —
numbers, gap analyses, and TODO lists here are superseded. Do not implement
from a May/June “next steps” file.

**Live engineering (RA, backlog, Godbolt):** [`../../engineering/README.md`](../../engineering/README.md)

Authoritative, maintained documentation:

| Topic | Location |
|---|---|
| Getting started / binaries | `../getting-started.md` |
| Architecture | `../architecture.md` |
| Register allocator (live) | `../register-allocator.md`, `lccc-improvements/register-allocation/` |
| RA vs gzip/zlib-ng/expat | `lccc-improvements/register-allocation/VALIDATION_ZLIB_GZIP_EXPAT.md` |
| Optimization passes | `../optimization-passes.md`, **`src/passes/README.md`** (tiers) |
| Benchmarks | `../benchmarks.md`, `tests/benchmark/` |
| SIMD | `../simd-audit.md` |
| Roadmap / backlog | `../roadmap.md`, `ideas/`, `hotspots/RESEARCH_BACKLOG.md` |
| Active bugs | `current_tasks/` |

`updates/phase*.md` is the same class of archive.

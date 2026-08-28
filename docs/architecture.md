---
layout: doc
title: Architecture
description: Compilation pipeline and what LCCC actually is.
prev_page:
  title: Getting Started
  url: /docs/getting-started
next_page:
  title: Register Allocator
  url: /docs/register-allocator
---

# Architecture
{:.doc-subtitle}
LCCC is a performance fork of CCC. Source lives in `src/` (no `ccc/` submodule).

## Layout

```
lccc/
├── src/{frontend,ir,passes,pgo,backend,driver,common}
├── tests/benchmark/     ← canonical suite
├── tests/workloads/     ← e.g. gzip-1.14 runner
├── engineering/         ← live engineering docs (STATE, DECISIONS, agent/, tasks/, evidence/, subsystems/)
├── docs/                ← user-facing site docs
├── ideas/               ← ranked raw improvement notes
└── Cargo.toml
```

Binaries: `lccc`, `lccc-x86`, `lccc-arm`, `lccc-riscv`, `lccc-i686`, `lccc-ld`.

## Pipeline

```
C source
  frontend/  preprocessor → lexer → parser → sema → AST
  ir/        lowering (alloca IR) → mem2reg (SSA)
  passes/    -O0 skip; -O1 light; -O2 full; -O3 +unroll; -Os/-Oz size
             TCE, rec2iter, inline, vectorize, GVN/LICM/IVSR, DSE,
             backedge-PRE, loop_rotate (opt-in), …
  pgo/       -fprofile-generate / -fprofile-use
  backend/   split_ranges → liveness → linear-scan RA (+ Tier-2 graph
             coloring) → stack_layout → ISel (ArchCodegen) → peephole
             → assembler → linker
ELF
```

**Authoritative pass tiers:** `src/passes/README.md` (not "all -O levels are identical").

## Register allocation

See [Register Allocator](/docs/register-allocator). Policy in `regalloc.rs`,
scan in `live_range.rs`; segment-aware interference and a Tier-2 graph
colorer are production; `RegAllocConfig` includes XMM, call-arg regs, folded
AArch64 indices.

## Licensing

Dual license: LCCC additions MIT/Apache/BSD; CCC-derived CC0. Paths are
`src/…`, not `ccc/src/…`. Details: [Licensing](/docs/licensing).

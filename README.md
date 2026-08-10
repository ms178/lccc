# LCCC — Lightning Fast Claude's C Compiler

> An optimized fork of [CCC](https://github.com/anthropics/claudes-c-compiler) based on Lev Kropp's additional [work] (see: https://www.levkropp.com/lccc/).

---

## What is LCCC?

CCC (Claude's C Compiler) is a zero-dependency C compiler written entirely in Rust by Claude Opus 4.6 and Arena.ai Agents,
capable of compiling real projects — gzip, zlib-ng, expat, SQLite, glibc, the Linux kernel — for x86-64, AArch64,
RISC-V 64, and i686, with its own assembler and linker.

LCCC is a performance fork. It is still WIP and a personal AI agent research project with performance in mind.

---

## Licensing

LCCC uses a dual-license model to separate original contributions from CCC-derived code.

**LCCC contributions** (new files, regalloc changes, benchmarks, docs) —
MIT OR Apache-2.0 OR BSD-2-Clause (your choice). See `LICENSE-MIT`, `LICENSE-APACHE`, `LICENSE-BSD`.

**CCC-derived code** (frontend, SSA IR, optimizer, backends, assembler, linker) —
CC0 1.0 Universal (public domain). CCC was released as CC0 by Anthropic.

See [`LICENSING.md`](LICENSING.md) for the full breakdown and per-file guidance.

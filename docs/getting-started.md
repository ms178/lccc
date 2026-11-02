---
layout: doc
title: Getting Started
description: Build LCCC from source and compile a C program.
prev_page:
next_page:
  title: Architecture
  url: /docs/architecture
---

# Getting Started
{:.doc-subtitle}
Build LCCC and compile C. Canonical repo: [ms178/lccc](https://github.com/ms178/lccc).

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| **Rust stable** | [rustup](https://rustup.rs/) |
| **Linux x86-64** | ELF targets; macOS/Windows untested |
| **GCC headers** | `stddef.h` / `stdarg.h` via GCC include dir |
| **Python 3.9+** | Canonical benchmark runner |
| **Swap on small VMs** | `scripts/ensure_swap.sh` (Rust build OOMs at ~2 GiB) |

Assembler and linker are built-in. Cargo features `gcc_assembler` / `gcc_linker` are optional fallbacks.

## Clone and build

```bash
git clone https://github.com/ms178/lccc.git
cd lccc
./scripts/ensure_swap.sh          # if RAM is tight
./scripts/build_lccc_fast.sh      # fastbuild: -O1, no LTO  → target/fastbuild/
# ship-quality:
./scripts/build_lccc_o1_j2.sh     # release, thin LTO, 2 jobs
```

| Binary | Arch |
|--------|------|
| `lccc` | x86-64 (default) |
| `lccc-x86` / `lccc-arm` / `lccc-riscv` / `lccc-i686` | explicit |
| `lccc-ld` | linker |

The driver still answers GCC-style version probes for autotools.

## Hello

```bash
GCC_INC="-I$(gcc -print-file-name=include)"
./target/fastbuild/lccc $GCC_INC -O2 -o hello hello.c
```

Flags: `-O0`–`-O3`, `-Os`, `-Oz`, `-S`, `-c`, `-E`, `-g`, `-D`, `-I`, `-fprofile-generate`/`-use`.

## Benchmarks (canonical)

```bash
python3 tests/benchmark/run_benchmarks.py
python3 tests/benchmark/run_benchmarks.py --only gzip_crc32,zlib_ng_adler32,expat_xml_scan
```

RA validation notes: `engineering/evidence/workloads/gzip-zlib-expat.md`.

## Tests

```bash
cargo test --lib
```

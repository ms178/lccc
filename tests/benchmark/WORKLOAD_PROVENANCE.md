# Workload-kernel provenance and scope

This directory contains **small, standalone, workload-derived kernels**, not
claims that these programs represent the end-to-end performance of their parent
projects.  Their purpose is to provide reproducible, inspectable compiler
reproducers between synthetic microbenchmarks and full package builds.

Every kernel has all of the following:

1. a pinned source release/commit selected from `ms178/archpkgbuilds`;
2. a source-file SHA-256 captured during extraction;
3. an explicit upstream license in the source header;
4. a deterministic harness and a local algorithmic self-check; and
5. compiler-output equivalence checks in `run_benchmarks.py`.

The sources below were retrieved on 2026-08-10.  The source-file digests are
more useful than an archive digest for these narrow extracts: they identify the
exact file from which the benchmark kernel was derived, even when an upstream
release archive is regenerated.  Corresponding upstream license texts are
shipped in [`third_party_licenses/`](../../third_party_licenses/README.md);
these test-source licenses do not relicense the compiler/runtime.

| Benchmark | `archpkgbuilds` selector | Extracted upstream source | License | SHA-256 of selected source file(s) | Standalone boundary |
|---|---|---|---|---|---|
| `gzip_crc32` | `packages/gzip/PKGBUILD`, `gzip` 1.14 | `lib/crc.c`, scalar table path | LGPL-3.0-or-later | `crc.c`: `9d9c9bfe6001049602d1101dd4dcc329bbca9bbbb7f556a5749acb23dfa1ecb2` | Keeps the scalar lookup table and CRC recurrence; removes configure-time slice/PCLMUL dispatch and gzip interfaces. |
| `zlib_ng_adler32` | `packages/zlib-ng/PKGBUILD`, tag `2.3.3` | `arch/generic/adler32_c.c`, `adler32_p.h` | Zlib | `adler32_c.c`: `274b9bacf8fa2a1ead2b202b8c9b878b68b100d2df51dee1790b27e47d3bf1de`; `adler32_p.h`: `514cb0157c2e0f15c19c90c3408ea8af1c643cb73991e27073c7aca3659e7dc7` | Keeps NMAX blocking and 8-byte accumulation.  Omits zlib-ng dispatcher, ISA variants, and copy API. |
| `expat_xml_scan` | `packages/expat/PKGBUILD`, Expat 2.8.2 | `lib/xmltok_impl.c`, especially `nameLength`/UTF-8 token progression | MIT | `xmltok_impl.c`: `94eeaef9ed46d10f80f748924fbf41f950b4859d6f234f61fdb2c8f1a8e77bd1` | A UTF-8-only specialization of the name-scanning path.  It is deliberately **not** a complete XML parser. |
| `sqlite_varint` | `packages/sqlite/PKGBUILD`, SQLite 3.53.4 (`sqlite-src-3530400`) | `src/util.c`, `sqlite3PutVarint` and `sqlite3GetVarint` | Public domain | `util.c`: `f3ad5a891625459352c1b57c053d3b89dcc973206644126a857c0ff82b7ce37f` | Keeps the fast 1–9 byte decoder control flow; removes SQLite types, assertions, and database interfaces. |
| `linux_find_bit` | `packages/linux-cachymod-6.18/PKGBUILD`, base `linux-6.18.42` | `lib/find_bit.c`; `include/asm-generic/bitops/__ffs.h` | GPL-2.0-or-later | `find_bit.c`: `5d9a03505728d024e52462662252c127545b0629ff2f7006e4d494372d30459d`; `__ffs.h`: `1e7c8db53caf177033f9832966fd2eac8c9a834cc46ac717646359d6140b7939` | Expands the portable `FIND_NEXT_BIT(addr1[idx] & ~addr2[idx], ...)` form only.  No kernel headers, locking, exports, or random source are benchmarked. |
| `glibc_memcmp` | `toolchain-stable/glibc/PKGBUILD`, commit `7cba77790f3279bec3ac20e9c7632b021cd53f95` | `string/memcmp.c`, common-alignment strategy | LGPL-2.1-or-later | `memcmp.c`: `5502c6d8ed5a4f9744ac232512575643a5f682bfcc208952a3a04e207157a5bb` | Keeps a defined-behavior, aligned-array adaptation of the four-word fast path.  It intentionally does not call host libc or cover glibc's arbitrary-alignment machinery. |

## Archive retrieval record

The extraction artifacts (not committed to LCCC) had these SHA-256 values:

```text
GNU gzip 1.14 tar.xz              01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6
zlib-ng 2.3.3 GitHub tag archive  f9c65aa9c852eb8255b636fd9f07ce1c406f061ec19a2e7d508b318ca0c907d1
Expat 2.8.2 tar.bz2               69e7f52417d85b1c2b7fe855e176eec55d0b2d7d92d691372d833a1c7df7923b
SQLite 3.53.4 source zip           d18fa15aec74d8c17e1463f861095adc01b5ad190256acb4f91d22f0368d232b
Linux 6.18.42 tar.xz               35a8ef5e8fb44467e56a42b40c23c4c10a61d1e95ff8d738f285230e8bed63e7
```

### Integrity audit note — recipe checksum mismatches

The following current recipe checksums did **not** match a repeated fetch of
the exact recipe URL on 2026-08-10.  This is recorded rather than silently
ignored; it may reflect stale package metadata or an upstream archive
regeneration, but requires resolution/signature verification before a package
artifact is treated as independently validated.

| Recipe | Recipe digest | Re-fetched artifact digest | Re-fetch result |
|---|---|---|---|
| `packages/gzip/PKGBUILD` (SHA-256) | `7454eb6935db17c6655576c2e1b0fabefd38b4d0936e0f87f48cd062ce91a057` | `01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6` | Two GNU FTP downloads were byte-identical. |
| `packages/expat/PKGBUILD` (SHA-512) | `46cc9d725f359b77681a2875bfefa15ceee50eb9513f6577607c0c5833dfa4241565c74f26b84b38d802c3cd8c32f00204fd74272bcecbd21229425764eef86c` | `9dc9d7c064c5ffe43af704af5006465f805d13602e458118cb9c32e0ae0b8f88b72a7824c85d313c529774af65c5e4d6a0ef1636362ac57514c176d3119dea22` | GitHub release artifact was fetched once; verify the release signature before package use. |
| `packages/sqlite/PKGBUILD` (SHA-256) | `2d7b032b6fdfe8c442aa809f850687a81d06381deecd7be3312601d28612e640` | `d18fa15aec74d8c17e1463f861095adc01b5ad190256acb4f91d22f0368d232b` | Two SQLite URL downloads were byte-identical. |

The benchmark sources are additionally pinned by the selected source-file
digests in the table above.  That makes the small test kernels reproducible,
but does **not** resolve the package-recipe supply-chain discrepancy.

## Validation performed on 2026-08-10

Before the screening comparison, all six files passed:

1. GCC `-O1 -fsanitize=address,undefined -fno-omit-frame-pointer` execution
   with `ASAN_OPTIONS=detect_leaks=0` and `UBSAN_OPTIONS=halt_on_error=1`;
2. a differential optimization-level matrix in which GCC and LCCC at `-O0`,
   `-O1`, and `-O2` all produced the same deterministic output per kernel; and
3. a 15-round, two-warm-up, paired/randomized, CPU-pinned LCCC/GCC/Clang VM
   screening run.  The raw JSON, assembly, compile commands, and section
   reports were retained outside the source tree by the runner.

The exact cross-compiler outputs were:

```text
gzip_crc32       372e56ab
zlib_ng_adler32  8c331ae0
expat_xml_scan   626766774715194881
sqlite_varint    deedcdd4edc1c0f1
linux_find_bit   0
glibc_memcmp     2158787064
```

This validates the benchmark harnesses and catches common optimization-level
miscompilations; it is not a proof of full parent-project behavior.

## Common algorithm microbenchmarks

The following files are self-contained algorithms written for this repository
(no copied upstream code, MIT/Apache-2.0 dual-license as with the rest of LCCC)
to broaden coverage beyond the original synthetic set while retaining exact,
fast cross-compiler output oracles:

| Benchmark | Shape | Primary stresses | Deterministic oracle |
|---|---|---|---|
| `ascii_case_fold` | byte parser loop | byte loads, compare/branch/select, dependent accumulator | fixed ASCII table and FNV-style checksum |
| `binary_search` | sorted table lookup | midpoint splitting, branch-heavy search, array loads | sum of successful search indices |
| `ring_fifo` | SPSC bounded queue | mask-based wrap, producer/consumer state, dependent load/store | final FNV/LCG stream checksum |
| `histogram` | 256-bin reduction | scattered indexed increments, loop reduction, 64-bit sum | total count and final checksum |

These are not substitutes for gzip/zlib-ng/expat/SQLite/glibc/kernel extracts;
they are stable screening kernels for regressions in addressing, branches,
register allocation, and memory-op folding before graduating to the pinned
end-to-end workloads.

## What these kernels can and cannot establish

They can isolate code-generation questions such as:

- whether a load/index/shift recurrence spills or uses a poor addressing mode;
- whether a branch-heavy varint or parser path has an unnecessary dependency;
- whether scalar unrolling, modulo lowering, and bit-scan selection are sound;
- whether LCCC and reference compilers agree on an exact deterministic output.

They cannot establish full-project wins.  Any promising result must graduate to
an upstream build/run workload with pinned inputs, assembly capture, controlled
runtime conditions, and—on bare metal—hardware-counter evidence.

## Graduated end-to-end workload: GNU gzip 1.14

`tests/workloads/gzip-1.14/run.py` now builds and executes the complete pinned
GNU gzip 1.14 project rather than extrapolating from `gzip_crc32`. The full
runner verifies archive SHA-256
`01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6`,
requires gzip's upstream test suite to pass 30/30 under both LCCC and GCC,
generates deterministic 8 MiB source-tree and structured/binary corpora,
requires bit-identical level-1/6/9 streams and exact decompression, captures
full binary disassembly/size/build logs, and retains randomized CPU-pinned raw
samples with individual and aggregate statistics.

The first seven-round VM screen on 2026-08-18 did **not** show an LCCC win. LCCC
was slower in every case: source compression ratios to GCC were 1.94882 (level
1), 1.81785 (level 6), and 1.88842 (level 9); mixed level 6 was 1.99878; source
decompression was 1.74329. Arithmetic and geometric ratios were 1.87943 and
1.87721. LCCC's binary text was 114,738 bytes versus GCC's 106,982. These
regressions are intentionally explicit rather than hidden by a kernel-only
score.

A live CE triage of the scalar gzip CRC recurrence found LCCC at 36 static
instructions, GCC 16.2 at 15, Clang 22.1 at 49, ICC 2021.10 at 35, and latest
ICX at 64. GCC's key advantage there is a compact pointer/end loop with a
memory table operand; LCCC repeatedly materializes byte and table addresses.
That is one plausible generated-code cause, not proof that CRC alone explains
the end-to-end gap. Raw evidence is retained under
`artifacts/simd-fp-four/gzip-e2e/`.

This remains VM wall-clock screening without PMU data. The recipe/archive
checksum discrepancy described above is still unresolved, and the archive
signature was not verified in this VM; the end-to-end runner reports both
limitations on every run.

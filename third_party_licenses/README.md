# Third-party license texts

These texts accompany third-party-derived material in LCCC: the benchmark
kernels under `tests/benchmark/programs/`, and the curated test corpus under
`tests/corpus/clang-c/`.  They do **not** relicense LCCC itself.  See
`../tests/benchmark/WORKLOAD_PROVENANCE.md` for source-file hashes,
extraction boundaries, and the authoritative per-file mapping; see
`../tests/corpus/clang-c/LICENSE-NOTICE.txt` for the corpus mapping.

## Benchmark kernels

| Kernel | License text |
|---|---|
| `gzip_crc32.c` | `GNU-LGPL-3.0-or-later.txt` |
| `zlib_ng_adler32.c` | `Zlib.txt` |
| `expat_xml_scan.c` | `Expat-MIT.txt` |
| `sqlite_varint.c` | `SQLite-public-domain.txt` |
| `linux_find_bit.c` | `Linux-GPL-2.0-or-later.txt` |
| `glibc_memcmp.c` | `GNU-LGPL-2.1-or-later.txt` |

The code headers and provenance manifest control if any wording here differs
from a package-level license file.  The kernel itself is test/measurement
material and must not be linked into the compiler/runtime without separate
license review.

## Test corpus

| Material | License text | Notes |
|---|---|---|
| `tests/corpus/clang-c/**` (EDG `tests/tests/imported/clang/c`) | `EDG-Clang-Apache-2.0-LLVM.txt` | Apache-2.0 WITH LLVM exception; SPDX attribution in every file |
| `tests/corpus/gnu-torture-manifest.jsonl` (EDG `tests/tests/imported/gnu/c`) | `GNU-LGPL-3.0-or-later.txt` (upstream) | GPL-3.0 upstream: **names/directives only** are recorded; no source text is redistributed |

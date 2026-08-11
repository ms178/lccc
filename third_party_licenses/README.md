# Third-party benchmark-source license texts

These texts accompany the narrow, third-party-derived benchmark kernels under
`tests/benchmark/programs/`.  They do **not** relicense LCCC itself.  See
`../tests/benchmark/WORKLOAD_PROVENANCE.md` for source-file hashes, extraction
boundaries, and the authoritative per-file mapping.

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

# Loop-density survey (-O2 -march=x86-64-v3)

WARNING: loop/work equivalence is not verified; this is a heuristic screen, not performance evidence.

Ratio = LCCC candidate-loop instructions per byte / best oracle's. **< 1.0 means LCCC does less work per byte.**

| ratio | benchmark | function | LCCC insn/B | best insn/B | best oracle | LCCC loop |
|---|---|---|---|---|---|---|
| 160.00 | expat_xml_scan | `main` | 6.0000 | 0.0375 | icx | 6/1 B |
| 24.00 | sqlite_varint | `sqlite_put_varint` | 6.0000 | 0.2500 | clang | 6/1 B |
| 7.20 | linux_find_bit | `main` | 2.2500 | 0.3125 | clang | 18/8 B |
| 2.71 | sqlite_varint | `main` | 2.3750 | 0.8750 | icx | 19/8 B |
| 2.13 | gzip_crc32 | `main` | 8.0000 | 3.7500 | icx | 8/1 B |
| 1.42 | glibc_memcmp | `main` | 2.0000 | 1.4062 | icx | 16/8 B |
| 1.05 | glibc_memcmp | `glibc_memcmp_common_alignment` | 0.6562 | 0.6250 | gcc16.2 | 21/32 B |
| 0.13 | zlib_ng_adler32 | `main` | 0.4375 | 3.2500 | icx | 14/32 B |
| 0.05 | expat_xml_scan | `expat_utf8_name_length` | 3.8125 | 70.0000 | gcc16.2 | 61/16 B |

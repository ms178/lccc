# First ten tickets

Re-oracle and check gzip `longest_match` stack-mem after **each** item.

| Step | ID | Work |
|------|-----|------|
| 1 | MS-07 | LCCC `-S` of gzip `deflate.c` `longest_match` on **this SHA** (prior −S was an older revision). |
| 2 | RA-04 | `CCC_RA_EXPLAIN=fn` spill dump. |
| 3 | RA-01 | RIP-relative `window`/`prev`/`strstart` (not GOT+stack). CE: gcc `window(%r9,%rcx)`. |
| 4 | RA-02, RA-03 | Next-use: match IVs in GPR; Adler DO8 keep `sum2`/`n`. CE Adler ~0 stack. |
| 5 | IS-01 | CRC `xor table(,%rcx,4)`. |
| 6 | IS-ANDN, IS-POPCNT | `andn` for find_bit; IR Popcount → `popcntl` on bitops. |
| 7 | IS-02, AB-01 | `double` stays in XMM; SysV SSE-class aggregates. |
| 8 | IS-03 | 64-byte assignment → 2× `vmovdqu %ymm`. |
| 9 | IS-04, OP-05 | ICX-style YMM FMA on nbody/matmul/spectral/reduction. |
| 10 | IS-06, OP-01 | `btq` classify; LICM uses `alias.rs` for GEP loads. |

Veto: gzip match stack-mem. Stop if sqlite/expat miscompile.

# Bit-test fusion: browser-derived kernels, four targets

Evidence for the bit-test/bit-modify fusion work (x86-64, i686, AArch64,
RISC-V).  Every number here is reproducible from this directory and the tree
it ships in.

## Corpus

`tests/regression/fixtures/bit_width/browser_bits.c` holds 47 kernels, each a
verbatim or minimally reduced bit-index idiom from pinned browser sources
(every kernel carries its file:line):

* Chromium `66fd4204b000bb138183d18ae80cb479e48ffc63`: libxml2 `valid.c`,
  LZMA SDK `7zArcIn.c`/`7z.h`, ICU `icu_utf.h`;
* Firefox `141096d7cd9e6f6118d94d79ea0c1777839efe9e`: SQLite `sqlite3.c`,
  libsrtp `datatypes.h`/`rdb.c`, usrsctp, ICU `utf8.h`, dav1d
  `decode.c`/`thread_task.c`, brotli `dec/decode.c`, FreeType
  `ttcmap.c`/`pshrec.c`/`sfwoff2.c`, NSS `mplogic.c`.

## Instruction counts (`measure.py`)

`python3 measure.py --base <main build dir> [--icx $LCCC_ICX_BIN] [--json OUT]`
compiles the corpus at `-O2` with GCC 16.2, Clang 23.1.4 (and ICX 2026.2 when
given), the merge-base build of lccc and this tree, and counts instructions
per kernel (labels and directives excluded, `ret` included).  Full tables:
[`counts-O2.md`](counts-O2.md), machine-readable [`counts-O2.json`](counts-O2.json).

| target | GCC 16.2 | Clang 23.1.4 | lccc main (`d202421`) | lccc (this tree) |
|---|---:|---:|---:|---:|
| x86-64 | 480 | 468 | 507 | **414** |
| i686 | 642 | 664 | 1232 | 1133 |
| AArch64 | — | 404 | 1048 | 966 |
| RISC-V 64 | — | 457 | 1898 | 1751 |

No kernel got longer than on main on any target.  On x86-64 lccc is at or
below the better of GCC and Clang on 39 of the 47 kernels; the eight gaps:

### Remaining x86-64 gaps and their root causes

| kernel | lccc | best | cause (not bit-test specific) |
|---|---:|---:|---|
| `libxml_occurs` | 17 | 13 | GVN load CSE is block-local (`src/passes/gvn.rs`, "Cross-block Load CSE is currently disabled"): the guarded block reloads `v->occurs`/`v->depth` and recomputes the mask. |
| `srtp_rdb_check` | 22 | 17 | three generic misses: `return bit ? 9 : 0` stays a branch (Clang: `setb; lea (%rcx,%rcx,8)`), `movl %r8d,%r9d; addq $128,%r9` is not a `leaq` (the zero-extension proof is cross-instruction), and the `cmp`/`sub` pair is not merged. |
| `sqlite_maskbit` | 8 | 7 | `iCol >= 64 \|\| !(mask & bit)` is not if-converted (both compilers select with `cmov`). |
| `sqlite_dbmask` | 8 | 7 | the select materializes its true value in a second register (`movzbl %al,%r8d`) instead of `cmove` of a zero into `%eax`. |
| `sqlite_uses_btree` | 12 | 11 | an unreferenced fall-through label splits a `movq %rcx,%r8; testl %r8d,%r8d` window (Clang: 13). |
| `icu_lead3`/`icu_lead4` | 9 | 8 | GCC computes the shift count straight into `%cl`; the x86 allocator never homes `%rcx`, so the shift form costs the `bt` form's count. |
| `ft_mask_any3` | 29 | 28 | allocator copies across the three tests. |

## Runtime correctness

* `tests/regression/check_bit_width_runtime.py --run`: the corpus driver
  (`browser_driver.c`, freestanding `start-*.s`) on x86-64, i686, AArch64 and
  RISC-V (QEMU user) at `-O0 -O1 -O2 -O3 -Os`, each with the fusion on, with
  `CCC_NO_BIT_SELECT=1` and with every `CCC_NO_BT_*` knob: 150 executions,
  every one returning the pinned checksum `2932816357278149644`.  GCC 16.2 and
  Clang 23.1.4 compute the same value at every level; a
  `-fsanitize=undefined,bounds` trap build of both reproduces it without
  trapping.
* `tests/regression/check_bit_select_fold.sh`: instruction pins for the folded
  shapes, the controls and the browser rows, cross-target never-longer
  checks, and a checksum that -O0..-Os, the kill switch, GCC and Clang agree on.

## ICX 2026.2

ICX is not installable on the development host (the Intel APT endpoint fails
TLS).  The `icx-oracle` CI lane runs `measure.py --icx "$LCCC_ICX_BIN"
--target x86-64 --notice` and the browser checksum under ICX; the counts are
published as `::notice` annotations on that check run and in its
`intel-evidence/` artifact.

## Decisions measured and rejected

* MSB-first (`0x80 >> k`) rewrites on AArch64/RISC-V: one instruction longer
  per test there (`tst`/`and` of a shifted constant is already two); kept to
  x86, and the modify form to x86-64 (i686's accumulator staging made
  `ft_mask_set` 21 vs 20).
* `tbz`/`tbnz` on AArch64: ±32 KiB range and no branch relaxation in the
  integrated assembler.
* Memory-destination `bts`/`btr`: microcoded with a register index and a
  data race on the containing word.
* i686 use-count gating of the register-base `bt`: net worse (1150 vs 1133).

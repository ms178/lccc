# What the assemblers replace

The x86-64 and the i686 assembler write a cheaper instruction for the same
effect -- `movl $1, %eax` for `movq $1, %rax`, `movq %rsp, %rsi` for `leaq
(%rsp), %rsi`, `movlps` for a `movsd` store, `vpxor %xmm1, %xmm1, %xmm2` for
`vpxord %zmm1, %zmm1, %zmm2` -- in everything the compiler emits, and under
`-Wa,-O2` or `-Wa,-Os` in a text a person wrote. The rules, and why each
replacement is the same instruction, are in
`src/backend/x86/assembler/encoder/optimize.rs`; the text of a compilation is
rewritten by `src/backend/x86/assembler/generated.rs`.

`scripts/asm_optimize_sweep.py` holds that to two references: GNU as 2.47,
which has `-O2` and `-Os` too, and the processor.

## The statements

The rows of GNU as's own instruction table (`opcodes/i386-opc.tbl`), as
`scripts/gas_table_sweep.py` turns them into statements, and on top what an
optimizer looks at:

* immediates at the edges of every width (`$0x7f`, `$0x80`, `$0x7fffffff`,
  `$0x80000000`, `$-0x80`, ...), into two registers of each width;
* one register in every position that takes it (`xor %ecx, %ecx`, `sub %r9,
  %r9, %r9`, `vpxor %ymm2, %ymm2, %ymm2`);
* every form of address that can be written another way, and some that must
  be left alone (`(,%rcx,2)`, `(%r13,%rcx)`, `(%rbp,%rcx)`, `%fs:(%rdx)`,
  `(%ecx)`, `sym(,%rcx,2)`, a bare `sym`);
* `{nf}` in front of every statement of a row that takes it.

A statement whose number does not fit the width it operates on is left out:
both assemblers cut it down, each its own way.

## Is the encoding as short as GNU as's?

`--check` assembles every statement with both assemblers as written, under
`-O2` (`-Wa,-O2`) and under `-Os`, and holds lccc to GNU as's `-O2` encoding
and, for size, to the shorter of GNU as's two. It fails on

* a statement lccc assembles as written and refuses or crashes on under
  `-Wa,-O2` or `-Wa,-Os`;
* a statement lccc encodes as written to other bytes or relocations than GNU
  as, unless lccc's are the shorter;
* any difference from `expected-<mode>.txt`, in either direction: a longer
  encoding the file does not name, one it names that is gone, another count
  of rewritten statements or another digest of what they are rewritten to.

| | 64-bit | 32-bit |
|---|---|---|
| statements | 164,621 | 83,258 |
| assembled by both as written | 67,727 | 29,498 |
| rewritten under `-O2` / `-Os` | 7,630 / 8,173 | 2,309 / 2,394 |
| as GNU as, `-O2` / `-Os` | 60,415 / 61,601 | 26,739 / 26,693 |
| shorter than GNU as, `-O2` / `-Os` | 6,879 / 5,710 | 2,678 / 2,727 |
| the same length, other bytes, `-O2` / `-Os` | 403 / 382 | 81 / 78 |
| longer than GNU as, `-O2` / `-Os` | 30 / 34 | 0 / 0 |

`shorter`: lccc folds an index without a scale into the base as written,
takes the sign-extended byte for an immediate that fits one at every width,
and replaces what GNU as leaves (a `movsd`, `movq` or `movdqa` store, a
packed-double move, `and %r, %r`, `shl $1`, a bare symbol's `lea`).
`other`: mostly a clearing idiom GNU as keeps (`andnps %xmm1, %xmm1` as
`xorps`, which every processor clears by without running anything), and
`cfcmovcc` of three equal registers, which GNU as turns into a `nop` that
does not clear the upper half of a 32-bit destination.

## The encodings that are longer, and why

Every `longer` line is a replacement GNU as makes that is not the same
instruction, or needs more of the processor.

| statement | GNU as `-O2` / `-Os` | why not |
|---|---|---|
| `vcmpfalse_osss %xmm9, %xmm1, %xmm1` and the other scalar `vcmp` | sources exchanged for the 2-byte VEX prefix | bits 127:32 of the result are the first source's: with `%xmm9` first they are `%xmm9`'s. (Run both: `--execute --insn 'vcmpeqss %xmm9, %xmm1, %xmm1'` passes because lccc leaves it; GNU as's bytes differ in the vector register.) |
| `vextractf64x2 $0, %zmm20, (%rsp)`, `vextracti32x8 $0, %zmm20, (%rdx)` | the 128- or 256-bit EVEX move | that move needs AVX512VL; the written instruction does not. |
| `{nf} sub %ecx, %ecx, %ecx`, `{nf} xor %r9, %r9, %r9` and the like, every width | `sub %ecx, %ecx` | it writes the flags `{nf}` is there to keep. lccc loads a zero where that is shorter (`movl $0, %ecx`). |
| `{nf} sub $-0x80000000, %rcx` (`-Os`) | `lea -0x80000000(%rcx), %rcx` | less -2^31 is plus 2^31: the `lea` subtracts it. (For a 32-bit register the two are the same, and lccc makes that one.) |

The same reasons keep three more of GNU as's replacements out that the
statements above do not reach at these registers: `vinsertps` with every
element zeroed as `vxorps` on `%xmm16` and up (AVX512DQ), `psllq $1, %mm0`
as `paddq` (SSE2), and a relocation that changes its type.

## Is the replacement the same instruction?

`--execute` runs every rewritten statement on this processor both ways, as
written and as replaced, from the same machine state -- every general
register, the flags, MXCSR, every vector and mask register, the `%mm`
registers, an 8 KiB block of memory the statement's address is made to point
into -- 24 times with other values, edge values among them. Afterwards all
of it has to be the same, but for the flags the written instruction leaves
undefined. No table and no GNU as is needed for this when the statements
come from `--insn` or `--file`.

A statement this processor does not run is counted and skipped; one whose
replacement it does not run fails, since a replacement may need nothing the
written instruction does not. On a processor with AVX-512:

| | 64-bit | 32-bit |
|---|---|---|
| replacements (each once) | 5,094 | 1,994 |
| run both ways, the same | 3,383 | 1,862 |
| not run by this processor | 1,711 | 132 |

What is not run there is APX, which no processor has yet, and AVX512-FP16
and AVX10.2 on one without them. An APX replacement is argued in
`optimize.rs` and held by the unit tests there; the rules that make it are
the rules that make the replacements that are run. (`--minimum` fails a run
that ran fewer than it names: a processor without AVX still runs the 1,934
and 1,055 replacements that are neither VEX- nor EVEX-encoded.)

Beside `--check` the same run takes the statements lccc encodes *as
written* to other bytes than GNU as -- an index folded into the base, the
short immediate, the sources of a commutative VEX instruction exchanged for
the 2-byte prefix -- and runs GNU as's bytes against lccc's:

| | 64-bit | 32-bit |
|---|---|---|
| encodings that are not GNU as's (each pair once) | 882 | 672 |
| run both ways, the same | 648 | 584 |
| not run by this processor | 234 | 88 |

`--self-test` plants replacements that are wrong in each way the run can see
-- another value, the upper half of a register, a flag, a register that is
no longer written, the rest of a vector register, another byte of memory, a
store into another place, an instruction the processor does not have -- and
requires every one to be found; CI runs it before it trusts a run.

## Running it

    scripts/asm_optimize_sweep.py --check [--execute] [--32]
    scripts/asm_optimize_sweep.py --update [--32]        # after a change
    scripts/asm_optimize_sweep.py --execute --insn 'movq $1, %rax'
    scripts/asm_optimize_sweep.py --self-test

`--gas`, `--objdump`, `--lccc` and `--cc` name the tools (`LCCC_GAS`,
`LCCC_OBJDUMP`, `LCCC`, `LCCC_EXEC_CC`); 32-bit code is built with
`$LCCC_M32_CC` if that is set and `cc -m32` otherwise. `--verbose` prints
the encodings behind the `longer` and `other` lines.

`tests/corpus/test_asm_optimize_sweep.py` holds the tool's own logic without
an assembler.

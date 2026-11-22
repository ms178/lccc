# GNU as instruction-table sweep

`scripts/gas_table_sweep.py` turns every row of GNU as 2.47's own instruction
table (`opcodes/i386-opc.tbl`, template blocks expanded) into concrete
statements -- a few registers, memory operands and immediates per operand
class, with and without a size suffix -- keeps the ones GNU as assembles, and
assembles the same statements with lccc. The case files in `tests/asm-diff/`
hold what somebody thought of writing down; this holds the integrated
assembler to the reference's own list.

It runs in four modes, each with its own expectation file:

| mode | code | file |
|---|---|---|
| (default) | 64-bit | `expected-64.txt` |
| `--32` | 32-bit | `expected-32.txt` |
| `--16` | 16-bit, under `.code16` | `expected-16.txt` |
| `--16gcc` | 16-bit, under `.code16gcc` | `expected-16gcc.txt` |

In 16-bit code the default operand and address sizes are the other way
round, and so is every size prefix; `.code16gcc` keeps the stack instructions
32 bits wide on top of that. The 16-bit modes sweep 16-bit addresses
(`(%bx,%si)`) beside the 32-bit ones.

| verdict | meaning | gate |
|---|---|---|
| `SAME` | identical bytes and relocations | |
| `DIFF` | same length, other bytes | fails |
| `RELOC` | same bytes, other relocations | fails |
| `LONGER` | lccc's encoding is longer | fails |
| `CRASH` | lccc neither assembled nor refused the statement | fails |
| `SHORTER` | lccc's encoding is shorter | fails unless recorded |
| `REJECT` | GNU as assembles it, lccc refuses it | fails unless recorded |
| `ACCEPT` | GNU as refuses it, lccc assembles it | fails unless recorded |

## The other direction

The table also says what a mnemonic does *not* take. The 64-bit, 32-bit and
`.code16` sweeps put in

* for every mnemonic and operand position, a register of each kind that no
  row of the mnemonic has there -- `rdrand %xmm0`, `adc %cl, %mm3`, `addps
  %ecx, %xmm1`;
* in the vector positions of every row, vector registers of every width in
  every combination -- `vaddps %ymm1, %xmm2, %xmm3`, `vpermd %xmm1, %xmm2,
  %xmm3`, `vpmovsxbw %ymm1, %ymm2`;
* every statement made from the rows themselves that GNU as refuses -- a
  size suffix on a register of another width (`cmovaq (%rsp), %r9w`,
  `cvttss2siq %xmm1, %ecx`, `sarl $1, %r9b`, and in 32-bit code `addw $-3,
  %ecx`, `movb $0x90, %cx`, `sall %cl`), general registers of two widths
  where a row takes one of several (`shrd %cl, %ebp, %r13`, `lar %ebp,
  %r9w`), a vector register only EVEX reaches on an opcode that has none
  (`vfrczpd %xmm17, %xmm17`), an address an instruction implies written
  without a register (`xlat 0x1000`, `lodsb sym`), a far branch whose offset
  does not fit its field (`ljmpw $0x1234, $0x12345678`), a mnemonic of
  another mode (`rmpquery` in 32-bit code);

and of the statements GNU as refuses, lccc has to refuse every one. An
encoder that goes by a register's number and never asks its kind, takes an
instruction's vector length from one operand and never looks at the others,
or takes the operand size from a suffix and never looks at the register,
assembles such a statement into an instruction on other registers: before
these sweeps existed 17,000 of the first kind and 1,200 of the second were
assembled, and of the third 670 in 64-bit code and 270 in 32-bit code.

Each of these statements is a file of its own, because lccc stops a file at
the first statement it refuses and says nothing about the ones behind it.
lccc is handed four hundred of the files in one run: like GCC it goes on with
the inputs behind one it refuses, so an object is there for a statement it
assembles and not for one it refuses. With a run for each statement the
64-bit sweep took 115 seconds for one statement in three; it takes 36 for
every one of them -- 135,000 statements in 64-bit code, 126,000 in each of
the other two. A run that ends in anything but a refusal says nothing about
the files it left without an object, and those get a run each (the self-test
plants a driver that stops at the first file it refuses beside one that goes
on, and the findings have to be the same).

What an encoding takes is checked in one place for every instruction
(`src/backend/x86/assembler/encoder/operand_class.rs`): the kinds of register
an opcode addresses, and for the integer instructions, the conditional moves,
the scalar conversions and BMI the width its general registers have -- read
off the encoding, in which `66` means 16 bits in 32- and 64-bit code and 32
bits in 16-bit code. The widths of vector register each VEX, EVEX and XOP
opcode takes together are a table there, written from GNU as's own answers by
`scripts/x86_vector_widths.py` (`--check` holds the table in the tree to GNU
as, `--write` regenerates it).

One kind of statement is left, in 16-bit code, and recorded: a gather whose
address names no general register (`vgatherdps %xmm6, 0x1000(,%xmm7,8),
%xmm1`). Such an address is 16 bits wide for GNU as, which has no 16-bit form
of it and wants `addr32` written in front; lccc writes the prefix, and the
bytes are the ones GNU as makes of the statement with `addr32`.

## The instructions learned from GNU as

An instruction is encoded by the arm that was written for it, and one nobody
wrote an arm for was refused however regular its encoding: before the table
below 2,968 of the 18,492 swept statements in 64-bit code, 3,237 in 32-bit
code and 3,308 in 16-bit code -- 3DNow!, FMA4, XOP, TBM, LWP, Key Locker, AMX,
AVX10.2, SM3, SHA512, the MMX and SSE conversions, and odd forms of
instructions the arms know.

`scripts/x86_learned_rows.py` closes that list with a table instead of an arm
per instruction. Of every spelling of a mnemonic on its list it asks GNU as
what it assembles -- each operand a general, MMX, vector, mask or tile
register of one width, memory, or a number -- and from the bytes of two
statements that differ in their registers it works out where each operand
goes: the prefix, the map and the opcode, the `ModRM` fields, `vvvv`, the
upper half of a trailing byte. Further statements say what else the form
takes: a mask and zeroing, a broadcast and its width, a rounding mode, a
register of APX, how a one-byte displacement is scaled. Nothing is copied
from the table's columns; its rows only say which statements are worth
asking. What comes out is `src/backend/x86/assembler/encoder/learned_rows.rs`
(2,453 rows of 734 mnemonics), which `encoder/learned.rs` reads for both
assemblers. A row is tried only where the arms and the plain table refuse a
statement, or wrote it in an encoding that cannot say it (`{evex}`, `{vex}`,
a register beyond `%r15`), so what the compiler emits is encoded as before.

The same script asks GNU as how large the one-byte number of every
instruction may be (`$256` is refused in `pshufd`, cut with a warning in
`add`, and `$-128` is taken by some forms and not by others): 342 ranges of
263 mnemonics, which every statement is held to before it is encoded.

```
scripts/x86_learned_rows.py --as AS --verify --lccc LCCC --lccc32 LCCC32   # the gate
scripts/x86_learned_rows.py --as AS --check                 # is the file GNU as's answers?
scripts/x86_learned_rows.py --as AS --write [--add M]       # solve again
scripts/x86_learned_rows.py --as AS --show --add M          # the rows of one mnemonic
```

`--verify` holds the file in the tree to GNU as (it has to be what solving
gives today), and then lccc to GNU as on what the solving held still: every
register of a kind, the addresses, the masks, the broadcasts, the rounding
modes, the numbers and the encoding hints of every row -- 174,000 statements
in 64-bit code and 100,000 in each of 32-bit and 16-bit code. The 169,000
that GNU as assembles have to come out as the same bytes and relocations, and
of the 204,000 it refuses lccc has to refuse every one. Twenty-seven
statements are left out, and lccc refuses them: GNU as assembles them into
another instruction than the one written (`data16` in front of an EVEX
instruction of map 4 becomes a bit of the prefix; `data16 vmread` is
`extrq`).

With the rows, 793 statements are refused in 64-bit code, 479 in 32-bit code
and 527 in 16-bit code. What is left is no instruction a table of register
positions can say: `ccmp` and `ctest` with their flag values, the string
instructions written with operands, `imulzu`, far and sized branches and
returns, the x87 environment forms with a size suffix, and prefixes written
as statements of their own.

## The expectation files

An expectation file is an inventory, not a ceiling: `--check` fails on any
difference from it, in either direction.

* `rows`, `statements`, `assembled`, `same`: how much was swept -- table
  rows, the statements made from them, the ones GNU as assembles, and of
  those the ones lccc assembles to the same bytes and relocations. A sweep
  that assembles nothing finds nothing; these numbers are what makes "nothing
  found" a statement about the table.
* `foreign`, `refused`: the same for the other direction -- statements with a
  register of another kind, and the ones GNU as refuses.
* `shorter <statement>`: an audited shorter encoding. All of them are one
  kind: a commutative VEX instruction whose sources lccc swaps to reach the
  2-byte VEX prefix, which GNU as does under `-O2` only.
* `accept <statement>`: a swept statement that GNU as refuses and lccc
  assembles.
* `reject <mnemonic> <count> <digest>`: the number of swept statements of
  that mnemonic that GNU as assembles and lccc refuses, and a digest of which
  they are -- as many refusals of other statements are a difference too.

A fix makes the file stale (`STALE`, `COUNT`), and `--update` rewrites it; the
diff of the file is the review of what the fix changed. `--update` refuses to
record a `DIFF`, `RELOC`, `LONGER` or `CRASH`: those are defects, not
expectations.

A statement GNU as itself assembles into something no processor reads as
written is left out of the sweep (`ORACLE_INVALID` in the script, with the
reason beside each).

```
scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE]                # report
scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE] --only MNEMONIC…
scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE] --check        # the gate
scripts/gas_table_sweep.py --as AS --lccc LCCC [MODE] --update       # after a fix
```

`--check` and `--update` need GNU as 2.47 and the whole table: the files
record what that release assembles and refuses.

The table is GPL-3.0-or-later and is read, never copied into this
repository: it is fetched at a pinned commit, verified against a pinned
SHA-256 and cached under `~/.cache/lccc-gas-table/` (`--tbl FILE` takes a
local copy, which has to hash the same). What is sampled is chosen by a
SHA-256 of the row and the statement, so the files mean the same statements
on every machine and every Python.

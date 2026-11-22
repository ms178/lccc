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
assembled, and of the third 670 in 64-bit code and 270 in 32-bit code. Each
of these statements takes a run of lccc of its own (lccc stops at the first
statement it refuses), so of the first two kinds one in three is swept; of
the third, every one.

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

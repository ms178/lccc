# GNU as instruction-table sweep

`scripts/gas_table_sweep.py` turns every row of GNU as 2.47's own instruction
table (`opcodes/i386-opc.tbl`, template blocks expanded) into concrete
statements -- a few registers, memory operands and immediates per operand
class, with and without a size suffix -- keeps the ones GNU as assembles, and
assembles the same statements with lccc. The case files in `tests/asm-diff/`
hold what somebody thought of writing down; this holds the integrated
assembler to the reference's own list.

| verdict | meaning | gate |
|---|---|---|
| `SAME` | identical bytes and relocations | |
| `DIFF` | same length, other bytes | fails |
| `RELOC` | same bytes, other relocations | fails |
| `LONGER` | lccc's encoding is longer | fails |
| `SHORTER` | lccc's encoding is shorter | fails unless listed below |
| `REJECT` | GNU as assembles it, lccc refuses it | fails above the recorded count |

`expected-64.txt` and `expected-32.txt` are what `--check` allows beside
`SAME`:

* `shorter <statement>`: an audited shorter encoding. All of them are one
  kind: a commutative VEX instruction whose sources lccc swaps to reach the
  2-byte VEX prefix, which GNU as does under `-O2` only.
* `reject <mnemonic> <count>`: the number of swept statements of that
  mnemonic lccc refuses. A ratchet: the count may only go down, a mnemonic
  that is not listed may not be refused at all, and `--update` rewrites the
  file after a fix. (`--update` itself refuses to record a `DIFF`, `RELOC` or
  `LONGER`: those are defects, not expectations.)

```
scripts/gas_table_sweep.py --as AS --lccc LCCC [--32]                # report
scripts/gas_table_sweep.py --as AS --lccc LCCC [--32] --only MNEMONIC…
scripts/gas_table_sweep.py --as AS --lccc LCCC [--32] --check        # the gate
scripts/gas_table_sweep.py --as AS --lccc LCCC [--32] --update       # after a fix
```

The table is GPL-3.0-or-later and is read, never copied into this
repository: it is fetched at a pinned commit, verified against a pinned
SHA-256 and cached under `~/.cache/lccc-gas-table/` (`--tbl FILE` takes a
local copy, which has to hash the same). What is sampled is chosen by a
SHA-256 of the row and the statement, so the files mean the same statements
on every machine and every Python.

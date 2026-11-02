# 2026-10-25 — the GAS snapshot's direction bit, red-teamed

Base: `6fe42ba` (= `main`, merge of PR #965).  Evidence:
[`engineering/evidence/gas-direction-redteam-2026-10-25/`](../evidence/gas-direction-redteam-2026-10-25/README.md).

PR #965 made the reference oracle reject *invented encoding contracts* — exact
VEX/EVEX token sets, numeric `VexW`, no blanket `v`-stripping, `--strict`
failing on `uncovered`.  The first run of the new MB-45 code-model gate
(`verify_code_models()`, 224 encodings over 160 mnemonics) reported **16
findings, all of them for the integer ALU `reg` form** — `and 0x22`,
`add 0x02`, `cmp 0x3a`, and their five siblings.  The encoder was right; the
snapshot was wrong, in both directions at once.

## The direction bit has four schemes, not one

`_directions()` expanded `D` as `opcode ^ 1` for every row.  `tc-i386.c`
(`install_template`, `match_template`) actually picks:

| case | bit |
|---|---|
| `SPACE_BASE` (and the APX `EVexMap4` rows) | `Opcode_D` = 0x2 |
| extended maps (`0F`, `0F38`, `0F3A`, VEX/EVEX) | `Opcode_ExtD` = 0x1 |
| `(opcode & 0xee) == 0x6e` (compact SIMD integer moves) | `Opcode_SIMD_IntD` = 0x10 |
| `FloatReg Class=RegFP|Tbyte` (x87 register forms) | `Opcode_FloatD` = 0x4 |

and three cases move **no** opcode: `Ugh` rows (`faddp %st(1),%st` is rewritten
to `faddp %st,%st(1)`, both `de c1`), FMA4/XOP rows (the order lives in VEX.W;
the scalar block is `6a/6b/6e/6f/7a/7b/7e/7f` for four different
instructions), and commutative rows — `C` in the table is
`#define C StaticRounding`, which GAS reads as `opcode_modifier.commutative`,
so `test %eax,(%rax)` and `test (%rax),%eax` are both `85 00`.

Two more transcription gaps came out with it: `D` and `W` compose (`mov` has
all four of `88/89/8a/8b`, not three), and a row with no ModR/M keeps its
register in the opcode (`push %rcx` is `51`), so one row covers eight
encodings.

## What that bought

Snapshot: 3,975 → **4,022** encodings; 105 wrong entries removed and 151 right
ones added.  Removed, among others: `movdqa 66 0f 6e` (that is `movd`),
`movd 66 0f 6f` (`movdqa`), `test 86`/`xchg 84` (`xchg`/`test`), `faddp da`
(`fiadd`), `mov 8d` (`lea`), `mov 0f 25` (nothing).  Added: the eight ALU
`D` pairs, `mov 8a/8b/a2/a3`, `mov 0f 22/23/26`, `push 51..57`, `pop 59..5f`,
`xchg 91..97`, `bswap c9..cf`, `mov b9..bf`.

## Verification

Every change was cross-checked against a second implementation rather than
against a second reading of the table: `objdump -D` on synthesized bytes, all
192 direction-sensitive encodings in one pass
(`tests/corpus/test_gas_snapshot_decode.py`, skipping when the pinned 2.47 pair
is absent).  Six encodings the decoder itself spells differently are exempt with
their reasons recorded in the JSON evidence (TR registers, `No64` rows, `nop`,
the memory-only `dc /2`, and `movq`'s REX.W form of `movd`).

The new gate is green and now covers the code paths:

```
$ python3 scripts/gas_encoding_crossref.py --verify --strict --backend all
rows checked against as 2.47: 1638
code-model encodings checked: 224 in 160 mnemonics
findings: 0   code-model findings: 0   unmodelled rows: 0   not in the as 2.47 snapshot: 0
```

Counts re-derived (the ones in MB-45 were measured against the broken snapshot
and are superseded): `--missing` **896** encodings / 454 mnemonics (`i686`
756/357, `x86` 859/437), `--absent` **1354** (`i686` 1524, `x86` 1380), triage
**740** UNMENTIONED / **614** MENTIONED.

Lesson recorded in the tool's own docstring: an oracle that is only checked
against the encoder cannot be trusted in either direction — the checking has to
go through a third implementation, and the model has to be verified before its
absence from the gap list means anything.

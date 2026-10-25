# Red-team review: the direction bit in the GNU as reference snapshot

This report covers the oracle only. It changes no Rust compiler source and
makes no runtime claim. It is the follow-up audit of
[PR #965](../gas-reference-redteam-2026-10-25/README.md), which made the GAS
reference oracle *reject invented encoding contracts*: the audit found that the
snapshot it ships still violated that guarantee in both directions.

## What was wrong

`scripts/isa/gas-2.47.json` is built by expanding `opcodes/i386-opc.tbl`, and
the expansion has to turn each table modifier into the opcodes GNU as really
emits. Three of them were transcribed as if they were one:

1. **`D` is not "XOR 1".** `tc-i386.c` picks one of four schemes —

   | case | bit |
   |---|---|
   | base map (`SPACE_BASE`), including the APX `EVexMap4` rows | `Opcode_D` = 0x2 |
   | extended maps (`0F`, `0F38`, `0F3A`, VEX/EVEX) | `Opcode_ExtD` = 0x1 |
   | compact SIMD integer moves, `(opcode & 0xee) == 0x6e` | `Opcode_SIMD_IntD` = 0x10 |
   | x87 register forms (`FloatReg Class=RegFP|Tbyte`) | `Opcode_FloatD` = 0x4 |

   and three cases change **no** opcode at all: `Ugh` rows are *rewritten* by
   the front end (`faddp %st(1),%st` assembles as `faddp %st,%st(1)`, both
   `de c1`), FMA4/XOP put the operand order in VEX.W, and commutative rows
   (`C`, spelled `#define C StaticRounding`) keep their opcode.
2. **`D` and `W` compose.** `mov` carries `D|W` and `as` accepts all four of
   `88`, `89`, `8a`, `8b`; enumerating the modifiers one-dimensionally produced
   three and declared the fourth illegal.
3. **Rows without a ModR/M keep the register in the opcode.** `push %rcx` is
   `51`, `bswap %ecx` is `0f c9`, `mov $1,%ecx` is `b9`: one row covers eight
   encodings, and the snapshot knew only the first byte of each range.

Net effect on the snapshot (`--summary`): 3,975 → **4,022** encodings, with 105
wrong entries removed and 151 right ones added.

### Invented encodings the old snapshot accepted

| snapshot entry | what those bytes really are |
|---|---|
| `movdqa 66 0f 6e` | `movd` |
| `movd 66 0f 6f` | `movdqa` |
| `movq 66 0f 6f` | `movdqa` |
| `test 86`, `test 87` | `xchg` (commutative row, no opcode moves) |
| `xchg 84`, `xchg 85` | `test` (same reason) |
| `faddp da /0`, `fmulp da /1` | `fiadd`, `fimul` (`Ugh` row) |
| `vfmsubss 0f3a 66 7e` | `vfnmsubss` (FMA4 row: no opcode moves) |
| `mov 0f 25` | not an instruction |
| `mov 8d` | `lea` |

### Real encodings the old snapshot was missing

`add 02/03`, `or 0a/0b`, `adc 12/13`, `sbb 1a/1b`, `and 22/23`, `sub 2a/2b`,
`xor 32/33`, `cmp 3a/3b` (the `Opcode_D` pairs), `mov 8a/8b/a2/a3`,
`mov 0f 22/23/26` (CR/DR/TR direction), `push 51..57`, `pop 59..5f`,
`xchg 91..97`, `bswap c9..cf`, `mov b9..bf`. The eight ALU pairs are exactly
what `verify_code_models()` reported as 16 false-positive findings on its first
run — the model was right and the oracle was wrong.

## Evidence

* `decode-proof.json` — all 192 direction-sensitive encodings of the snapshot,
  as bytes, next to the mnemonic `objdump -D` prints for them, produced by
  `decode_proof.py` against the pinned GNU binutils 2.47 pair
  (`as`/`objdump` 2.47.20260726). Reproduce with:

  ```
  ASMDIFF_TEST_AS=<gas-2.47>/as ASMDIFF_TEST_OBJDUMP=<gas-2.47>/objdump \
      python3 engineering/evidence/gas-direction-redteam-2026-10-25/decode_proof.py
  ```

  Six encodings are exempt, each recorded with its reason in the JSON: `0f
  24`/`0f 26` (obsolete TR registers, which objdump does not decode), `06`/`07`
  (`No64` rows, invalid in 64-bit mode), `xchg 90` (spelled `nop`), `fcom dc
  /2` (a memory-only form whose reverse shape GAS still produces) and the
  `movq 0f 6e` pair (the REX.W form of `movd`; the table does not record W).
* `oracle-counts.txt` — the re-derived gate output for both backends. The
  numbers published in `backlog.md` MB-45 before this pass (349/286 missing,
  1767 absent, 1061/706 triage) were measured against the broken snapshot and
  are superseded: `--missing` **896** encodings / 454 mnemonics, `--absent`
  **1354**, triage **740**/**614** (`--backend all`).

## Gate

`--verify --strict` stays green and now covers more:

```
$ python3 scripts/gas_encoding_crossref.py --verify --strict --backend all
rows checked against as 2.47: 1638
code-model encodings checked: 224 in 160 mnemonics
findings: 0   code-model findings: 0   unmodelled rows: 0   not in the as 2.47 snapshot: 0
```

The 224 code-model encodings are new in this pass: `verify_code_models()` now
requires every claim of `code_models()` — the ALU (`encode_alu`), the
condition-code spellings (`cc_from_mnemonic`), the i686 one-byte `inc`/`dec`
and the FMA3 grid (`fma3_opcode`) — to be a form `as` 2.47 accepts. It is
wired into `--verify` and into `--strict`, so a model that drifts from the
encoder fails CI instead of shrinking the gap list.

## Regression tests

* `tests/corpus/test_gas_encoding_crossref.py` —
  `GasDirectionModelTests` (the nine direction cases above, the ALU pairs, the
  `D|W` cross product, the SIMD-int pair, the commutative rows, `Ugh`/FMA4,
  x87, the special-register moves, `+rd`) and `GasCodeModelGateTests` (every
  claim verified on both backends; a mutated claim must produce `CODE-EXTRA`
  and `CODE-UNKNOWN`).
* `tests/corpus/test_gas_snapshot_decode.py` — the objdump anchor: the whole
  direction-sensitive subset is laid out in 16-byte slots and decoded in one
  pass; it runs when the pinned pair is present (`ASMDIFF_TEST_AS` /
  `ASMDIFF_TEST_OBJDUMP`) and skips otherwise, so no distro `objdump` can act
  as a second-class authority.

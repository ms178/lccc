# Assembler differential test suite

LCCC has its own integrated assembler and its own ELF writer. A divergence
from GNU as in the bytes, relocations or symbols it emits is either a
miscompile that only surfaces at run time, or dead bytes in the I-cache.
Neither is visible from the compiler's own output — only a direct comparison
against a reference implementation finds them.

GNU as is the oracle. LCCC must either agree exactly, or reject exactly what
GNU as rejects.

## Layout

```
tests/asm-diff/
  *.casefile        generated corpus, one group per encoding decision area
  instructions.s    hand-written legacy cases (raw .text comparison)
  jmp_relax_boundary.s
  run.sh            legacy raw-.text runner
```

The corpora are produced by `scripts/gen_asmdiff_corpus.py` and consumed by
`scripts/asmdiff.py`. Generation validates every case against GNU as first, so
the corpus can never contain an input that would make the differential report
a false failure.

## Status

All 721 cases pass against GNU as 2.47, and the per-instruction differential
agrees byte-for-byte on every one of the 9,805 distinct instructions in the
corpus (the remaining 63 are the deliberate reject list, which both assemblers
refuse).

`insndiff.py` synthesises any numeric local label an instruction refers to, so
`jmp 1f` is compared as a real jump rather than being rejected by the oracle
for want of a `1:` in the file.

## Running

```bash
# Build the oracle once (any recent binutils works; 2.47 is what CI uses)
export LCCC_GAS=/path/to/as LCCC_OBJCOPY=/path/to/objcopy

# Whole-object differential: sections + relocations + symbols
scripts/asmdiff.py --as "$LCCC_GAS"

# One instruction at a time, for reducing a failure to a single mnemonic
echo 'addw $65535, %ax' | scripts/insndiff.py --as "$LCCC_GAS"

# Sweep a template over a register/immediate matrix
scripts/insndiff.py --sweep 'imul{S} ${IMM}, %{R{S}}, %{R{S}}' --only-diff

# Regenerate the corpus after adding a generator
scripts/gen_asmdiff_corpus.py --as "$LCCC_GAS" --out-dir tests/asm-diff
```

## What each group covers

| Group | Encoding decision under test |
|---|---|
| `modrm` | ModRM/SIB structure: rsp/r12 base (SIB required), rbp/r13 base (disp required), index-only, every scale, every displacement boundary, RIP-relative, absolute/moffs |
| `rex` | REX.B/X/R/W; spl/bpl/sil/dil vs ah/ch/dh/bh conflict; every zero/sign-extension width pair |
| `imm` | Immediate width selection at the exact boundaries; sign-extended imm8 forms; AL/AX/EAX/RAX short forms; `imul` imm16 vs imm32 |
| `shift` | by-1 short form vs imm8 vs `%cl`, all widths |
| `prefix` | All six segment overrides; LOCK on every RMW form; REP/REPE/REPNE on every string op |
| `branch` | Jcc/jmp relaxation at every disp8 boundary, forward and backward; chains where relaxing one jump moves another; relaxation interacting with `.p2align` |
| `padding` | Alignment padding shape for every gap size at every alignment, after both instructions and raw data |
| `x87`, `sse`, `avx`, `bmi` | Mandatory-prefix selection, VEX2 vs VEX3, VEX.W/L/vvvv, immediate-carrying forms |
| `lea` | The full addressing grammar with no memory access |
| `misc` | Multi-byte NOPs, string ops, push/pop, in/out, mul/div across all widths |
| `directive` | Data directives, sections, symbol attributes, symbol arithmetic, `.rept`/`.irp`/`.macro`, CFI, relocation flavours |
| `interact` | Relaxation + alignment + symbol arithmetic together — the configuration in which layout bugs actually manifest |
| `reject` | Input GNU as refuses. Silently accepting any of these encodes a *different valid instruction*, turning a typo into a miscompile |

## Failure classes

`asmdiff.py` and `insndiff.py` classify divergences by severity:

| Class | Meaning |
|---|---|
| `FALSE-ACCEPT` | LCCC emitted bytes for input the oracle rejects. Worst case: a typo becomes a miscompile with no diagnostic. |
| `WRONG-BYTES` | Same length, different meaning. A silent miscompile. |
| `REJECTS-VALID` | Cannot assemble valid input. Blocks real code from building. |
| `LONGER` | Correct but wastes I-cache. A missed size optimization. |
| `SHORTER` | Correct and smaller than the oracle. Verify the semantics, then keep it. |

A `SHORTER` result is not automatically a pass: confirm the shorter encoding is
semantically identical before accepting it. For example LCCC elides a `%ss`
segment override because it is a no-op in 64-bit mode (GNU as already elides
`%ds` for the same reason), which is correct and strictly better.

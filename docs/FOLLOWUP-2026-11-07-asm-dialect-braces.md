# RESOLVED 2026-11-07 — inline-asm dialect braces: one rule, one implementation

**Status:** fixed, gated, and measured against **both** GCC 16.2 and Clang
23.1.5. This file started as an open follow-up; the item is now closed. It is
kept because the investigation corrected two plausible-but-wrong conclusions,
and the reasoning is worth more than the diff.

## The defect was bigger than the symptom

The visible symptom was that `mov{l}` behaved differently from GCC. The actual
root cause was **two independent implementations of the same GCC rule**, in
different layers, with different semantics:

| layer | file | behaviour |
| --- | --- | --- |
| shared emission | `backend/inline_asm.rs::expand_dialect_alternatives` | strip braces only when a depth-1 `\|` exists; otherwise preserve |
| x86 operand substitution | `backend/x86_common.rs::resolve_dialect_alternatives` | strip braces **always**, ignore `%{`/`%}` |

The second ran *after* the first, on every line, for x86-64 and i686. So the
shared pass's decision was irrelevant on those targets, and the net behaviour
was "always strip, no escapes". Measured consequences:

* `__asm__("mov{l} %eax, %ebx")` — **basic** asm — was expanded to `movl`. GCC
  emits basic asm verbatim.
* `__asm__("nop # %{lit%}" : : "r"(a))` lost its escaped braces: the shared pass
  turned `%{`/`%}` into `{`/`}`, then the second pass stripped them. GCC emits
  `nop # {lit}`.
* On AArch64/RISC-V the shared pass *did* strip `{a|b}`, destroying constructs
  where braces are literal.

**Fix:** delete `resolve_dialect_alternatives` entirely and make the shared
pass authoritative. One rule, one implementation, applied once to the whole
template before it is split into lines. `substitution_layering_tests` in
`x86_common.rs` now fails if a brace-touching pass is ever reintroduced at that
layer. Deleting the duplicate also removes a `Vec<char>` (4 bytes per input
byte) plus a `String` allocation per brace-carrying asm line.

## The rule, from GCC's source, not from inference

`gcc/final.cc::output_asm_insn` (fetched from the GCC 16 release branch):

```c
#ifdef ASSEMBLER_DIALECT
      case '{':
      case '}':
      case '|':
        p = do_assembler_dialects (p, &dialect);
        break;
#endif
```

Two independent gates follow from this and from how asm is lowered:

1. **Target.** `ASSEMBLER_DIALECT` is defined only by i386
   (`#define ASSEMBLER_DIALECT (ix86_asm_dialect)` in `gcc/config/i386/i386.h`).
   `gcc/config/aarch64/aarch64.h` and `gcc/config/riscv/riscv.h` contain **zero**
   occurrences. On those targets `{`, `}` and `|` are ordinary characters —
   which is exactly what makes `ld1 {v0.4s}, [x0]` work, and why GCC's own
   AArch64 backend can emit register lists without escaping them.
2. **Statement shape.** Operands, clobbers **or** goto labels lower to
   `ASM_OPERANDS` and run through `output_asm_insn`. None of those lowers to
   `ASM_INPUT`, which is emitted verbatim. `volatile` is irrelevant.

`do_assembler_dialects` is a **flat** state machine, not a recursive one: a `{`
inside a group is a hard error ("nested assembly dialect alternatives") and a
group without `}` is another ("unterminated assembly dialect alternative").
With `dialect_number == 0` (AT&T) the first alternative is kept.

LCCC now implements that machine exactly. For the two malformed shapes it
returns the template untouched, because the backend owns no diagnostic channel
and fabricating half-expanded text would be worse than letting the assembler
produce the error.

## Two wrong conclusions this investigation corrected

**(1) "Clang disagrees with GCC."** It does not. The first measurement showed
Clang rejecting `mov{l} %1, %0`, which looked like Clang preserving braces.
It was actually Clang's integrated assembler rejecting `movl %rax, %rsi` — a
32-bit mnemonic with 64-bit registers. Clang validates asm text even for `-S`;
GCC does not. Re-measured with templates that are real instructions after
expansion:

| target | shape | GCC 16.2 | Clang 23.1.5 |
| --- | --- | --- | --- |
| x86-64 | extended `mov{l} %%eax, %%ebx` | `movl %eax, %ebx` | `movl %eax, %ebx` |
| x86-64 | extended `no{p\|q}` | `nop` | `nop` |
| x86-64 | extended `nop{\|}`, `nop{}` | `nop` | `nop` |
| x86-64 | basic `mov{l} %eax, %ebx` | verbatim | verbatim (then as rejects it) |
| aarch64 | extended `ld1 {v0.4s}, [x0]` | literal (no dialect) | `ld1 { v0.4s }, [x0]` |
| aarch64 | extended `no{p\|q}` | literal → invalid | literal → **error** |

They agree on every row. LCCC now matches both, which is a stronger result
than picking a winner.

**(2) "Pipeless braces must be preserved for NEON."** Only on targets without
assembler dialects. On x86, GCC strips `{v0.4s}` too, and documents `%{`/`%}`
as the way to write a literal brace. Preserving them everywhere was not
"leniency", it was the wrong rule on x86 and the right rule on AArch64, applied
uniformly.

## Gates

* `src/backend/inline_asm.rs::dialect_alternative_tests` — 8 tests: the
  GCC-measured shape table, basic-asm passthrough, all three single-dialect
  targets, malformed passthrough, `%%` layering, multi-byte safety, and a
  **40 000-case** deterministic corpus comparing the byte-slice production
  implementation against an independent char-at-a-time transcription of GCC's
  algorithm.
* `src/backend/x86_common.rs::substitution_layering_tests` — braces must reach
  the substitutor's output untouched; `%%` and multi-byte text must still be
  handled there.
* `tests/regression/check_asm_dialect_alternatives.py` — 15 x86 shapes on
  x86-64 **and** i686 against GCC 16.2 and Clang 23.1.5, 3 AArch64 shapes
  against Clang cross, 3 RISC-V text-fidelity rows. RISC-V has no instruction
  whose syntax contains braces, so no oracle can accept such a template there;
  those rows are reported as rule-only rather than silently skipped.
  `--self-test` covers fence extraction for GCC's `#APP` and Clang's `//APP`.
  Wired into `ci.yml` (shard b) and `scripts/ci_local.sh`.

## Pointers

* `src/common/types.rs::target_has_assembler_dialects` — the target gate, with
  the GCC source citation.
* `src/backend/inline_asm.rs::expand_dialect_alternatives` — the one rule.
* `docs/DWARF-CAMPAIGN.md` §D7a — the neighbouring `-S -g` acceptance work that
  surfaced this.

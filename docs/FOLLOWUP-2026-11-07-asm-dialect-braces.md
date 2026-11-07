# FOLLOWUP 2026-11-07 — inline-asm dialect braces: a measured GCC divergence

**Status:** open, pre-existing, out of scope for the D6 `-S -g` acceptance work.
**Found while:** making `expand_dialect_alternatives` allocation-free (one buffer
and one `&str` memcpy per run instead of a `Vec<char>` plus a `String` per brace
group). That rewrite is *equivalence-proved* against the previous implementation
by a 20 000-case deterministic corpus (`dialect_equivalence_tests`), so it
changed no behaviour — including the behaviour described here.

## The rule GCC 16.2 actually implements

Measured with `gcc-16 (GCC) 16.2.0`, x86-64, `-O2 -S`, default `-masm=att`.
Whether braces are processed at all depends on how the statement is lowered:

| statement shape | GCC lowering | `{...}` processing |
| --- | --- | --- |
| no operands, no clobbers | `ASM_INPUT` | **none** — the template is emitted verbatim |
| has operands **or** clobbers | `ASM_OPERANDS` → `output_asm_insn` | braces are **always** removed and the **first** alternative is kept |

Repro (`dialect4.c`), which isolates the trigger:

```c
void g(void) {
  __asm__ ("A nop {solo}");                    /* -> A nop {solo}   */
  __asm__ volatile ("B nop {solo}");           /* -> B nop {solo}   */
  __asm__ ("C nop {solo}" ::: "memory");       /* -> C nop solo     */
  __asm__ volatile ("D nop {solo}" ::: "memory"); /* -> D nop solo  */
  __asm__ ("E nop {a|b}");                     /* -> E nop {a|b}    */
  __asm__ ("F nop {a|b}" ::: "memory");        /* -> F nop a        */
}
```

`volatile` is irrelevant; a clobber list alone is enough to switch paths. With
operands present the removal is unconditional, including in places where the
braces look structural:

```c
long g(long a) {
  long r = a;
  __asm__ ("mov{l} %1, %0"                 : "=r"(r) : "r"(a)); /* movl  %rdi, %rax */
  __asm__ ("xchg{l|} %1, %0"               : "=r"(r) : "r"(a)); /* xchgl %rdi, %rax */
  __asm__ ("nop {|} %0"                    : : "r"(a));         /* nop  %rdi        */
  __asm__ ("# c {x|y} %0"                  : : "r"(a));         /* # c x %rdi       */
  __asm__ ("vaddps {rn-sae}, %%zmm1, %%zmm2, %0" : : "r"(a));   /* vaddps rn-sae, … */
  __asm__ ("ld1 {v0.4s}, [%0]"             : : "r"(a));         /* ld1 v0.4s, [%rdi]*/
  __asm__ ("nop {solo}"                    : : "r"(a));         /* nop solo         */
  return r;
}
```

Two consequences worth stating plainly, because they are easy to disbelieve:

* `#` does **not** protect braces. `# c {x|y} %0` becomes `# c x %rdi`. (A `#`
  comment in a *no-operand* template is verbatim only because the whole template
  is verbatim on that path.)
* AVX-512 decorators and NEON register lists are **not** special-cased. GCC
  strips `{rn-sae}` and `{v0.4s}` too; a user who wants literal braces must write
  `%{` and `%}`, which is the documented escape.

## What LCCC does today

`src/backend/inline_asm.rs::expand_dialect_alternatives` is target-independent
and operand-count-independent:

* a group with a depth-1 `|` → first alternative, braces dropped (**agrees**);
* a group with no `|` → braces and content preserved (**differs**);
* the expansion runs even for a no-operand, no-clobber statement (**differs**).

So, with operands:

| template | GCC 16.2 | LCCC today | verdict |
| --- | --- | --- | --- |
| `mov{l} %1, %0` | `movl …` | `mov{l} …` | LCCC output does not assemble |
| `ld1 {v0.4s}, [%0]` | `ld1 v0.4s, …` | `ld1 {v0.4s}, …` | LCCC is lenient, GCC is not |
| `nop {a\|b}` (no operands/clobbers) | `nop {a\|b}` | `nop a` | LCCC expands where GCC does not |

`mov{l}` / `pushf{l|d}` / `pop{l}` are common idioms in kernel and libc sources,
so the first row is the one with practical cost. The second row means LCCC
currently accepts templates GCC rejects; tightening it is a deliberate
compatibility decision, not a free win.

## Why this was not fixed in the D6 PR

The D6 work is about `-S -g` acceptance parity and inline-instance provenance.
Changing brace semantics touches every inline-asm template on all four targets,
needs to become operand/clobber-aware at three call sites
(`inline_asm.rs:792`, `:811`, `:913`), and needs its own oracle matrix
(4 targets × {no operands, operands, clobbers only} × {pipe, no pipe, escaped}).
Folding that into a debug-info PR would make both harder to review and to
bisect. Nothing in `tests/` relies on the lenient behaviour today (checked), so
the fix is not blocked by corpus debt.

## Suggested plan

1. Thread `has_operands_or_clobbers` into the expansion; return
   `Cow::Borrowed(template)` when false, matching `ASM_INPUT`.
2. When true, always drop the braces and keep the first alternative — including
   the pipeless case — and keep honouring `%{` / `%}`.
3. Gate it with a GCC-oracle table over the shapes above on all four targets,
   plus the escaped-brace forms, and update
   `dialect_equivalence_tests::a_pipeless_brace_group_is_preserved_today`
   (deliberately labelled a characterization test) in the same commit.
4. Re-run `check_debug_acceptance_parity.py` and `check_inline_asm_utf8.py`:
   both compile templates with operands, so both see the new path.

## Pointers

* `src/backend/inline_asm.rs` — `expand_dialect_alternatives`,
  `dialect_equivalence_tests` (equivalence corpus + characterization).
* `tests/regression/check_debug_acceptance_parity.py` — the four-target
  inline-asm acceptance/UTF-8/`%=` matrix this was found next to.
* `docs/DWARF-CAMPAIGN.md` §D7 — why `-S -g` assembles the text at all.

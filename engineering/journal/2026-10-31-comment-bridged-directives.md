# A directive is a phase-4 line, and a comment can join two physical lines into one

**2026-10-31 — comment-bridged directives, and the two miscompiles the first fix
left behind**

`fs/nfs/callback_xdr.c` ends `CB_OP_GETATTR_RES_MAXSZ` with a comment that
closes on a line whose remaining tokens are a backslash and then the rest of
the arithmetic. C11 5.1.1.2 runs comment replacement (phase 3) *before*
directive recognition (phase 4), so the comment's own unspliced newline — the
one phase 2 never touches — is deleted with the comment and the next physical
line becomes part of the same `#define`. The joiner in
`frontend/preprocessor/text_processing.rs` decided whether it could keep a
physical break by looking at one physical line at a time, and it decided the
*lexical* state the same way. Both answers were wrong at that newline:

| question | what the joiner answered | what the standard says |
|---|---|---|
| may this break be kept? | yes, the line is ordinary | no: the line is inside a `#define` |
| what state does the next line start in? | `Code` | inside the block comment |

The first answer truncated the macro body at the kept newline and leaked
`(2 + 2 + 3 + 3 + 3 + 3 + 3) * 4)` as top-level tokens; the second read quotes
and `//` inside comment text as literals and line comments.

## The invariant

Two facts cross a real newline, and only two: whether a block comment is open
(a literal cannot legally span a newline, a `//` comment ends at it), and
whether the line was in directive context. The joiner now carries both
(`carry_block`, `carry_dir`), initialises the next joined line's scan from the
first and its splittability from the second, and asks the phase-4 question with
that state:

> **A line in directive context is never splittable, and a line that begins
> inside a comment is scanned, not rescanned.**

`first_token_is_directive(line, starts_in_block_comment)` is the predicate that
makes the second half possible: `*/ #define A 1` is a directive when the
comment it closes began on the previous line, and `/* x */ #define A 1` is one
when the comment closes on the same line. `starts_directive` still answers the
phase-2 question for raw text; the two predicates are deliberately separate and
their doc comments say which phase each belongs to.

## What the first pass missed

The first version of the fix asked the predicate about raw text, so it could
not see a directive whose comment opened on an **ordinary** line:

```c
/* header
   comment */ #define A 1 + \
  2
int arr[A];
```

GCC 16.2 emits `int arr[1 + 2];`. The first version emitted `int arr[1];` and a
stray `+ 2`, on all five frontends — the same truncation, one shape narrower.
Found by a 16-shape differential sweep against `gcc -E -P`, fixed by passing
the entry state into the predicate and by making directive context the single
authority on splittability (it subsumes the old `starts_directive` test).

## The fast path, and what it is actually worth

The new state tracking scans every physical line of a file that contains a
backslash anywhere. That is a full extra lexer pass, so the scan is skipped
when it cannot change the answer: an end state of `BlockComment` needs either an
open comment coming in or a `/*` in the line, i.e. a `/` byte, and `contains`
lowers to `memchr`. The equivalence is not argued — it is exhaustively checked
over every string of length ≤ 4 from `/*"'\\1a\r` in every entry state
(4 681 strings × 5 states) against the full scan.

Measured on this host (2 cores, `fastbuild`), `-E -P`, median of 9–13
interleaved runs:

| corpus | parent `8e7277c` | first fix | this branch |
|---|---:|---:|---:|
| 1.5 MB kernel-shaped TUs | 68.9 ms | 70.1 ms | 70.0 ms |
| 16 MB | 805 ms | 804 ms | 809 ms |
| 34 MB, one backslash, 2 M comment-free lines (worst case for the scan) | 1 438 ms | 1 449 ms | 1 434 ms |

The honest reading: the extra pass costs **≤ 1 %**, and the fast path is
**within noise** on every corpus — it does not buy wall clock. It is kept
because it removes provable work (no scan on the majority of lines), costs one
`memchr` per line, and keeps the double-lexing from becoming a real cost on a
slower host or a pathological input. An earlier audit claim that this was a
meaningful regression did not survive measurement.

## Why not phase-order the pipeline instead?

Doing comment replacement before the join decision is the architecturally clean
answer: one lexical pass, no carry. It also rewrites the `LineSegments`,
`CommentSegments` and `LineResolver` coordinate systems, every `__LINE__`
consumer, and the `--keep-break` elision that exists to preserve physical-line
granularity for those consumers. That is a separate change with its own
regression surface; this one keeps the existing coordinate system and adds two
carried facts with an explicit invariant.

## Evidence

* `tests/regression/check_pp_directive_comment_bridge.sh` — 10 fixtures × 5
  frontends (x86-64, i686, AArch64, RISC-V) against `gcc -E -P`, a 24-case
  seeded differential sweep, and a value pin (the kernel macro evaluates to 84).
* `physical_line_tests.rs` — 5 new tests: the kernel shape through macro
  expansion, the ordinary-line bridge, `*/#define` without space, the
  destructive half of phase 3 (a bridged statement stays macro body), and
  marker/column resync after the merge.
* `text_processing.rs` tests — the phase-4 predicate as a table, and the
  exhaustive fast-path equivalence.
* Negative control: the upgraded gate run against the pre-fix binary fails 14
  times (2 fixtures × 5 frontends, plus 4 sweep cases). A gate that cannot fail
  is decoration.

The sibling change — `-fno-delete-null-pointer-checks` as a silent off-request
with a ledger row, and the kernel harness compiling the package's own config —
is recorded in `engineering/session-logs/2026-10-30-defconfig-continuity.md`.

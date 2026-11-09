# Pragma-pack stack underflow follow-up

## Finding and design

The parser previously reset the current packing alignment when `pack(pop)`
found an empty stack. GCC 16.2 instead diagnoses the unmatched pop and leaves
the current alignment unchanged; silently resetting it can change record
layout and cross-translation-unit ABI. The smallest safe state transition is
therefore to leave `pragma_pack_align` untouched on underflow and report a
`Pragmas` warning at the directive token. A matched pop still restores the
saved state. Parser-level diagnosis is preferable to a second preprocessor
stack: the parser already owns this state, including invalid-push handling,
and the diagnostic engine resolves scoped warning settings at the token span.
Preprocessing alone does not execute this parser transition.

The regression exercises two nested valid pushes and pops, repeated
underflows at file and block scope, suppression by a diagnostic pragma, layout
after each transition, `-Werror=pragmas`, `-Wno-pragmas`, `-E`, and object
compilation. It runs LCCC's four target executables; a hosted vendor lane also
compiles the layout assertions with pinned GCC 16.2, Clang 23.1.5, and
authenticated ICX 2026.2, plus GCC and Clang in i686 syntax-only mode.
Reference warning categories are not assumed identical. The test additionally
replaces the first unmatched pop with a reset in the fixture and requires
LCCC to reject it; this models the old state transition but is not a mutated
compiler binary. The fixture is a narrow layout witness, not exhaustive ABI
proof.

A subsequent audit found that `_Pragma("pack(...)")` queued the directive's
preprocessor effects but discarded the returned layout marker. Common macro
wrappers could therefore silently produce natural rather than packed layout.
For the supported complete literal forms (reset, push, pop, and canonical
decimal alignment), macro expansion now emits a blue-painted marker at the
operator's source position; the existing queued effect still owns diagnostics.
The regression places multiple operators on one line, tests nested macro
expansion and block scope, defines macros named like internal markers, and
checks that `-E` output can be compiled again. This catches the previous
missing-marker behavior. It does not establish full pragma grammar or every
possible placement; octal, hexadecimal, suffixed and named forms are not
implemented by this operator path.

## Evidence and limits

Locally, GCC 16.2 (verified against the repository's pinned archive SHA-256)
accepted the layout assertions in syntax-only mode for both `-m64` and `-m32`.
It emitted warnings for the unsuppressed underflows, promoted them under
`-Werror=pragmas`, suppressed them under `-Wno-pragmas`, and emitted none in
`-E` mode. No local compiler build or local CI was run. Hosted results must be
read from this revision's CI, not inferred from these probes.

This change does not implement `-fpack-struct`, named pack stacks, or the full
pragma grammar. It does not establish cross-architecture vendor equivalence,
all possible pragma placement semantics, performance parity, or universal
compiler correctness. The earlier audit's nesting and `-fgimple` failures and
other follow-ups remain open. No external expert approval is asserted.

## In-progress ABI and switch-index work

GCC 16.2 x86-64 and i686 probes distinguish an explicit member alignment in
an attribute-packed record (it may exceed one) from the same member under
`#pragma pack(2)` (it is capped at two), including packed records and unions.
The new hosted fixture also covers `_Alignas`, typedef alignment, an ordinary
member of a packed record under a pragma, and object emission. This is not
`-fpack-struct` support; current-head hosted results are still required.

The switch-lowering duplicate check now indexes covered intervals instead of
scanning every previous label. Empty ranges remain empty and overlapping
erroneous ranges are unioned for subsequent diagnostics. A deterministic
unit test compares the index with a linear reference and a 30,000-case test
exercises the scaling path. This only removes one quadratic component of the
case-label problem; it does not remove the recursive AST/parser/lowering
limits or establish a performance measurement for a complete translation
unit. The 50,000-operation local Python model check is not an LCCC test.

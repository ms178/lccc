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
authenticated ICX 2026.2. Reference warning categories are not assumed
identical. The fixture is a narrow layout witness, not exhaustive ABI proof.

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

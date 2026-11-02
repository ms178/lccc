# Audit note: i686 copy-register guard

The whole-tree census adds one panic site for `base_name` in
`src/backend/i686/codegen/emit.rs`. This is an intentional fail-closed guard at
the i686 small-copy emitter boundary: only physical 32-bit general-purpose
register names are valid bases. The values reaching it come from the backend's
physical-register allocator, not C source or ELF input. If the invariant is ever
violated, rejecting code generation is safer than emitting an operand with an
invalid register spelling. Keep this guard and its diagnostic rather than
silencing the census with an unchecked fallback.

The census also adds the existing `CCC_INLINE_PARAMREF_DEBUG` diagnostic
switch to its documented inventory. It is read once at pass-pipeline setup and
controls only parameter-reference remapping diagnostics; it does not affect
optimization decisions or generated code.

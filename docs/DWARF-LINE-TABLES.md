# DWARF line-table support

`-g` source locations currently flow through GAS-compatible `.file` and `.loc`
assembly directives. The x86-family relocatable-object writer serializes those
records as DWARF v4 `.debug_line` units and retains a relocation from each unit
to its code section, so code layout and branch relaxation are reflected in the
final address rows.

The x86-64 linker preserves non-allocated `.debug_*`, `.zdebug_*`, `.gdb_index`,
and `.gnu_debug*` input sections. It merges same-named sections, resolves their
static relocations, and emits file-backed section headers outside `PT_LOAD` for
both executables and shared libraries. This includes GCC 16.2 DWARF 5 DIE,
abbreviation, string, range, and line sections; the regression gate checks
multi-object executable and shared links.

## Scope and limits

- LCCC currently emits source line tables, not a complete compiler-generated
  `.debug_info` graph with variable locations, type DIEs, or inline-call DIEs.
- Existing DWARF sections from GCC inputs are preserved and relocated on the
  x86-64 link path; this is not a claim of complete GNU ld compatibility.
- The i686 assembler emits relocatable DWARF v4 line tables, but the current
  i686 executable linker emits no section-header table, so linked i686 debug
  sections are not yet preserved.
- `--strip-all` continues to omit debug sections on the x86-64 executable
  path.

The focused GCC 16.2 validation is
`tests/regression/check_dwarf_line_tables.py`.

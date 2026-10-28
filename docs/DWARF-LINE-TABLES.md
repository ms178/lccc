# DWARF line-table support

`-g` source locations currently flow through GAS-compatible `.file` and `.loc`
assembly directives. The x86-family relocatable-object writer serializes those
records as DWARF v4 `.debug_line` units and retains a relocation from each unit
to its code section, so code layout and branch relaxation are reflected in the
final address rows.

A line table alone is invisible to debuggers: gdb, `addr2line` and `objdump -l`
reach line programs only through a compile unit's `DW_AT_stmt_list` (gdb on a
line-table-only image reports "No symbol table is loaded" and cannot break at
`file:line`). So, exactly like GAS's `dwarf2_finish` when its input has `.loc`
rows but no `.debug_info`, the writer also synthesizes, per line program, a
DWARF 4 `DW_TAG_compile_unit` (`stmt_list`, `low_pc`/`high_pc` of the code
section, name, `comp_dir` = the working directory, producer, language: C99 for
`.c`/`.h`/`.i` sources, GAS's assembler code otherwise) with one
`DW_TAG_subprogram` per sized function symbol (GAS's unspecified return type,
so no debugger assumes `int`). Every cross-section value is relocated, so the
units stay correct after the linker concatenates objects. An input that brings
its own `.debug_info` or `.debug_abbrev` is left untouched.

The x86-64 linker preserves non-allocated `.debug_*`, `.zdebug_*`, `.gdb_index`,
and `.gnu_debug*` input sections. It merges same-named sections, resolves their
static relocations, and emits file-backed section headers outside `PT_LOAD` for
both executables and shared libraries. This includes GCC 16.2 DWARF 5 DIE,
abbreviation, string, range, and line sections; the regression gate checks
multi-object executable and shared links.

## Scope and limits

- LCCC currently emits source line tables with GAS-equivalent compile and
  subprogram units, not a complete compiler-generated `.debug_info` graph with
  variable locations, type DIEs, or inline-call DIEs.
- Existing DWARF sections from GCC inputs are preserved and relocated on the
  x86-64 link path; this is not a claim of complete GNU ld compatibility.
- The i686 assembler emits relocatable DWARF v4 line tables, but the current
  i686 executable linker emits no section-header table, so linked i686 debug
  sections are not yet preserved.
- `--strip-all` continues to omit debug sections on the x86-64 executable
  path.

The focused GCC 16.2 validation is
`tests/regression/check_dwarf_line_tables.py`.

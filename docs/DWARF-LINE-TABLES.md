# DWARF debug information

`-g` source locations flow through GAS-compatible `.file` and `.loc` assembly
directives. Every ELF writer (x86-64, i686, AArch64, RISC-V) serializes those
records as `.debug_line` line programs and retains a relocation from each unit
to its code section, so code layout and branch relaxation are reflected in the
final address rows.

A line table alone is invisible to debuggers: gdb, `addr2line` and `objdump -l`
reach line programs only through a compile unit's `DW_AT_stmt_list` (gdb on a
line-table-only image reports "No symbol table is loaded" and cannot break at
`file:line`). So, exactly like GAS's `dwarf2_finish` when its input has `.loc`
rows but no `.debug_info`, the writer also synthesizes, per line program, a
compile unit with `stmt_list`, `low_pc`/`high_pc` of the code section, name,
`comp_dir` (the working directory), producer and language, plus one
`DW_TAG_subprogram` per sized function symbol. A subprogram whose address range
contains a `.loc` row also records `DW_AT_decl_file`/`DW_AT_decl_line` -- the
first source row inside the function, i.e. the line a definition is declared
on. Its return type is GAS's unspecified type, so no debugger assumes `int`.
Every cross-section value is relocated, so the units stay correct after the
linker concatenates objects. An input that brings its own `.debug_info` or
`.debug_abbrev` is left untouched.

## Dialects

`-gdwarf-<N>` selects the dialect and enables debug info on its own; a bare
`-g` is version 5, which is what GCC 16.2 emits. Versions 2, 3, 4 and 5 each get
the header layout and the attribute forms their specification requires:

| version | unit header                                  | line-program header |
|---------|----------------------------------------------|---------------------|
| 2       | length, version, abbrev offset, address size | 9 standard opcodes (`opcode_base` 10), no `max_ops` |
| 3       | same                                         | 12 standard opcodes (`opcode_base` 13), no `max_ops` |
| 4       | same                                         | adds `maximum_operations_per_instruction` |
| 5       | adds `DW_UT_compile` before the address size | adds `address_size`/`segment_selector_size`, and format-described directory/file tables |

| attribute       | versions 2-3            | versions 4-5                |
|-----------------|-------------------------|-----------------------------|
| `DW_AT_stmt_list` | `DW_FORM_data4`       | `DW_FORM_sec_offset`        |
| `DW_AT_high_pc` | `DW_FORM_addr` (an address, as the spec defines it) | `DW_FORM_data8` offset from `low_pc` |
| `DW_AT_external`  | `DW_FORM_flag` + value byte | `DW_FORM_flag_present`, absent on static functions |
| `DW_AT_language`  | `DW_FORM_data1` (`data2` when the code needs it) | same |

`.debug_aranges` is version 2 in every dialect, as GCC emits it: the table is
read before the units are, so it may not depend on the dialect it describes.
Each compile unit contributes one set -- its offset in `.debug_info` and the
addresses it covers, both as relocations -- and each set's first tuple starts on
a multiple of the tuple size counted from the set's own length field. A relaxing
target (RISC-V) deliberately gets no table: its lengths are computed before the
linker deletes bytes, so a range would be slightly too long and would claim code
belonging to the next unit. A consumer without the table scans `.debug_info`,
which is correct, just slower.

Writing version 4 forms into a version 2 unit is not a cosmetic difference: a
version 2 consumer reads a `high_pc` constant as an address, which after linking
points into unrelated code, and `DW_FORM_sec_offset` (0x17) had no meaning
before version 4. Both were fixed by measuring GCC 16.2's abbreviation tables
per dialect rather than assuming one shape fits all. The same measurement
settled the remaining fields: a constant `high_pc` is `data8` (a `data4` would
cap a unit's code at 4 GiB and is not GCC's form), and `DW_AT_language` takes
the smallest constant form its code fits -- one byte for every C dialect, two
for the `DW_LANG_MIPS_ASSEMBLER` value 0x8001 that assembly units carry.
`tests/regression/check_dwarf_versions.py` now compares the declared form of
every one of these fields against GCC 16.2, because a form is invisible to a
value comparison: `DW_FORM_strp`, `DW_FORM_line_strp` and `DW_FORM_string` all
print the same text.

GCC is the reference for these choices, and it is not the only defensible one:
Clang 23.1.4 declares `DW_FORM_data2` for `DW_AT_language` and `DW_FORM_data4`
for a constant `high_pc`, and its version 5 units index their strings and
addresses (`DW_FORM_strx1`/`DW_FORM_addrx`) through `.debug_str_offsets` and
`.debug_addr` instead of pointing at them. LCCC follows GCC: `data1` is the
narrower language encoding, `data8` costs four bytes per unit and removes the
4 GiB cap, and reference forms need neither of those extra sections.

Version 5 names paths through `DW_LNCT_path`/`DW_LNCT_directory_index` entries
whose values are `DW_FORM_line_strp` offsets into `.debug_line_str`. The unit's
own name and its compilation directory are `DW_FORM_line_strp` offsets into that
same table, so the compilation directory is stored once -- directory entry 0 of
the line table and `DW_AT_comp_dir` are the same string -- while the producer
and every function name are `DW_FORM_strp` offsets into `.debug_str`. C17/C23
units carry the `DW_AT_language_name`/`DW_AT_language_version` pair, because
DWARF has no language code newer than C11. Versions 2..4 write their names
inline as `DW_FORM_string` and clamp the language to C99 -- both exactly what
GCC 16.2 does (measured for `-std=c89|c99|c11|c17|c23` across `-gdwarf-2..5`).
Both string tables intern on first use, so a unit that GCC spells with six
entries (it repeats the directory per file, and stores the full path beside its
two halves) is spelled with three.

Every string offset is a relocation against its string section, so a link that
merges the string tables of several objects cannot leave a stale offset behind.

## Directory and file tables

Both prologue flavors are built from one model, because they describe the same
thing:

- **Directory 0 is the compilation directory.** Version 5 spells it out as the
  first table entry; versions 2..4 leave it implicit in the `directory_index`
  encoding, where index 0 means "the compilation directory" and their own
  entries are numbered from 1.
- **Every other directory a recorded `.file` name mentions gets one entry**, in
  first-use order: a name under `src/` resolves through an entry `src`, a name
  below `/vendor/` through `/vendor`, and a bare name through directory 0. A
  name whose directory spells the compilation directory is folded into entry 0
  -- the resolved path is identical and the table stays minimal.
- **The file table is a list of `(directory_index, base name)`.** Version 5
  repeats the first file at entry 0, because its file register starts at 1 and
  index *i* therefore has to name file *i*; versions 2..4 number their entries
  from 1 directly.

A base name is what versions 2..4 store, and an empty one would terminate their
file table early, so a `.file` entry whose name is empty or ends in a separator
is refused with a diagnostic instead of silently truncating the table.

## Optimization

Debug info must not depend on optimization leaving source spans behind. A
function that folds to a constant at `-O2` keeps no instruction to carry a
span, so the file table is seeded with the translation unit's own file rather
than built from spans alone; the compile unit, the line program and one
subprogram per symbol then exist for every optimized build as well. A
translation unit that only declares symbols has nothing to describe, and stays
as quiet as GCC 16.2 does. `-g` on a unit that *defines* code but produced no
line rows still reports that on stderr rather than failing silently.

## Linking

The x86-64 and i686 linkers preserve non-allocated `.debug_*`, `.zdebug_*`,
`.gdb_index`, and `.gnu_debug*` input sections. They merge same-named sections,
resolve their static relocations, and emit file-backed section headers outside
`PT_LOAD` for both executables and shared libraries. This includes GCC 16.2
DWARF 5 DIE, abbreviation, string, range, and line sections; the regression gate
checks multi-object executable and shared links. AArch64 and RISC-V images carry
the same sections, and their addresses stay correct because a relaxing target
relocates every code address against a label rather than against a constant
offset (RISC-V linker relaxation deletes bytes after assembly).

## Scope and limits

- LCCC emits line tables, compile and subprogram units with their declaration
  locations, and the address map (`.debug_aranges`). It does not yet emit variable DIEs, type DIEs (`DW_TAG_base_type`,
  composite types, `DW_AT_data_member_location`) or inline-call DIEs, so a
  debugger can break at `file:line` and name a frame, but cannot print a local's
  type. Type information lives in the frontend, and the DWARF writer is fed by
  assembler records: closing this gap needs a `.debug` record vocabulary in the
  lowering, not a wider assembler emitter.
- Existing DWARF sections from GCC inputs are preserved and relocated on the
  link paths; this is not a claim of complete GNU ld compatibility.
- `--strip-all` continues to omit debug sections on the executable paths.

## What a row says, and what it costs

A subprogram is described twice, and both descriptions have to be right:

* `DW_AT_decl_line`/`DW_AT_decl_column` name the **declarator** -- the line and
  column the function's name is written on. Codegen hands that position to the
  assembler as a non-statement row (`is_stmt 0`) at the function's first
  address, followed by the body row with `is_stmt 1` restored (`is_stmt` is
  sticky). An address-to-line lookup therefore still lands on the opening brace
  -- what gcc and clang's tables read -- while the writer takes the first row
  for the declaration position. Units assembled from `.loc` rows alone declare
  no `DW_AT_decl_column`: a statement's column is not the declarator's.
* The first row after the prologue carries `prologue_end` and the row at the
  first epilogue instruction `epilogue_begin`. Both are version 3 opcodes, and
  the line program's dialect is the unit's -- gcc 16.2 writes a version 2 line
  table for `-gdwarf-2` (measured; 12.2 wrote 3, which was the older behaviour)
  -- so a `-gdwarf-2` build states neither rather than describing its own header
  with opcodes it never defined. A `.loc` whose instruction the peephole removes
  is dropped with it, so the marker is re-attached to the first row the function
  kept -- still after the prologue.

Row density is per **statement**, not per source line: two statements on one
line are two rows at their own columns, which is what makes `next` stop between
them.

The encoder picks the cheapest opcode shape for each row: a special opcode when
the address advance fits (16 or 17 units, depending on the line delta), a
`DW_LNS_advance_pc` ULEB, `DW_LNS_fixed_advance_pc` for a delta past the ULEB
window, or -- what binutils' `emit_inc_line_addr` does -- whole
`DW_LNS_const_add_pc` steps (17 units per byte) plus one final special opcode
for the remainder. Measured on gcc-generated assembly, its line program is now
byte-for-byte the size of GNU as 2.40's for the same rows, and never larger.

A target whose code the linker may relax is the exception, and by necessity: an
address there cannot be a distance, because `DW_LNS_advance_pc` would move when
the linker deletes bytes between two rows. Every row carries its own address
relocation instead (`DW_LNE_set_address`, address_size + 3 bytes) and states its
row with `advance_line` + `copy` rather than a special opcode, which is what
keeps the table correct after relaxation. The measured cost on riscv64 is 13
bytes per row above GNU as's compact encoding for the same rows, and the
line-table gate bounds a relaxing leg at exactly that much.

The focused validation is `tests/regression/check_dwarf_line_tables.py`
(line/link behaviour) and `tests/regression/check_dwarf_versions.py`
(dialect, language, directory/file tables, per-version attribute forms,
`-ffunction-sections` units and `addr2line` round-trips against GCC 16.2 for
every supported target). The latter also decodes the emitted line program back
with `readelf --debug-dump=decodedline` and requires the decoded `(file, line)`
set to be exactly the set the source asked for, on every target and dialect
(x86-64 also at `-O2`, where the peephole retires rows): a wrong operand width,
a dropped row or a mis-signed advance cannot pass that.
`tests/regression/check_dwarf_decl_positions.py` covers the declaration half:
the expected position of every function is recomputed from the fixture's own
text (return type on its own line, brace two lines down, four statements on one
line), and the object must state it, start its table at the brace, mark
`prologue_end`/`epilogue_begin` exactly once inside each function's range, and
give each statement on the shared line a row of its own -- on x86-64, i686,
aarch64 and riscv64 in versions 2 through 5.

# The DWARF campaign: what a debugger needs, what lccc ships, and the work to close the gap

Written 2026-11-05 against the `main` branch as measured in this checkout. Every
number below was produced by a command in this repository and can be reproduced
with the recipes in [§2](#2-how-every-number-here-was-produced). Where a claim
could not be measured in this harness it is marked **S** (read from source,
runtime consequence not demonstrated) instead of being stated as fact.

The campaign's single sentence: lccc already ships **correct line tables and
correct unwinding** for every architecture it supports, and now ships a **DIE
tree with types, file-scope objects, function parameters and their addresses**.
A debugger can stop at the right source line, unwind the stack, print a global
or a static, list a function's parameters with their types, `ptype` any type
those objects name, and read an object's address. What it still cannot do is
print a *local* variable or a parameter's *value*, or report the inline stack --
see [§3.5](#35-what-the-campaign-has-landed-measured) for the measured state and
D1b/D6 in [§4](#4-work-items) for the two remaining items. The work items are
ordered so that each one turns one more debugger command from "cannot" into
"works", and each one lands with a gate that fails when it regresses.

---

## 1. What a debugger actually consumes

A debugger never reads "debug info"; it answers four questions, and each is a
different section:

| Question | Section | Read by | lccc today |
|---|---|---|---|
| Which source line is this address? | `.debug_line` (via `DW_AT_stmt_list`) | `addr2line`, every stepper | **Correct**, v2–v5, x86-64/i686/AArch64/RISC-V |
| Which function is this, and where does it start? | `DW_TAG_subprogram` in `.debug_info` | backtraces, breakpoints by name | **Correct for every definition** (incl. unreferenced statics at `-O0`, since this campaign) |
| Which unit owns this address? | `.debug_aranges` | consumers mapping an address to a CU before parsing units | **Correct**: one set per unit, each naming its own `.debug_info` offset |
| How do I unwind out of it? | `.eh_frame` / `.debug_frame` | unwinders, exception edge, `bt` | **Correct**, synthesized after the peephole on x86, from directives elsewhere |
| What is the value of `x`? | `DW_TAG_variable` / `DW_TAG_formal_parameter` + `DW_AT_location` + type DIEs | `print`, `info locals`, `ptype` | **Missing** |

Everything in the last row is the campaign. The first four rows are the reason
the campaign is cheap to *finish*: the sections that a consumer reads *first*
are already right, and the writer that produces them already models versions 2–5,
relocation, string interning, range lists and abbreviation tables.

### The four questions, in the order a consumer asks them

1. `.eh_frame` — before anything else, when deciding whether a PC is inside a
   frame at all.
2. `.debug_aranges` — for `dladdr`-style lookups and for tools that must map a
   PC to a unit without parsing every unit.
3. `.debug_info` unit header, then `DW_AT_stmt_list`, then `.debug_line` rows.
4. The DIE tree under the unit, for anything with a name.

A defect in (1) or (2) makes a debugger *fail*, which is why they were audited
first. A defect in (4) makes it *answer less*, which is what this campaign fixes.

---

## 2. How every number here was produced

All commands assume the agent toolchain environment (GCC 16.2.0, binutils 2.47)
and the `fastbuild` preset:

```sh
source ~/.cache/lccc/agent-toolchains/env.sh
python3 tests/regression/check_dwarf_quality.py --lccc target/fastbuild/lccc --gcc gcc-16 \
    --json /tmp/dwarf-quality.json --markdown /tmp/dwarf-quality.md
```

That gate is the campaign's instrument: it builds programs whose source it wrote,
decodes `.debug_info`, `.debug_line`, `.debug_aranges` and `.eh_frame` with its
own reader (no `readelf`, no `pyelftools`), and compares rows, subprogram ranges
and source lines against each other. It also runs an `-m32` matrix of the checks
that do not need linked addresses (unit headers, subprogram DIEs against the
symbol table, declaration attributes, the internal-definition policy at both
tiers), skipping it with a note where the host has no 32-bit runtime — a rule
that lives in lowering or in an IR pass is target-independent, and a second
target is what turns "should be identical" into evidence. It also compares the internal-definition
policy to GCC 16.2 where a GCC 16 is installed, and reports the DIE tags a
debugger needs as named **known gaps** rather than silently ignoring them.

The tag census quoted in §3 was produced with a separate script that walks every
DIE of every compile unit and counts tags, over the first 50 programs of
`tests/benchmark/programs/`:

```python
# /tmp/coverage.py (scratch, not part of the tree): count DIEs per tag for
# `lccc -O0 -g -gdwarf-5` and `gcc-16 -O0 -g -gdwarf-5` on the same source.
```

Structural audits of the line program, the address map and the unwind tables were
run with a second scratch script over 65 files (5 hand-written probes and 60
benchmark programs) × 4 configurations (`lccc -O0 -gdwarf-5`, `lccc -O2 -gdwarf-5`,
`lccc -O0 -gdwarf-2`, `gcc-16 -O0 -gdwarf-5`): **0 failures**.

---

## 3. What is already correct, and what is measured missing

### 3.1 Correct, and gated (do not re-open)

* **Line tables, versions 2–5.** Every `.loc` attribute is version-gated:
  `prologue_end`/`epilogue_begin` appear from version 3 (measured: 5 occurrences
  at `-gdwarf-3/4/5`, 0 at `-gdwarf-2`), `is_stmt` is `1` on statement rows and
  `0` on the declarator row, `discriminator` and `isa` are present where they
  apply. `addr2line` resolves every function entry single- and multi-TU.
* **Unwinding.** x86-64/i686 CFI is synthesized by abstract interpretation after
  the peephole (GCC's `dwarf2cfi` shape), so a peephole that changes the stack
  cannot desynchronize it; AArch64/RISC-V lower it from directives. `.eh_frame`
  and `.eh_frame_hdr` are present, and the gate asserts every function's range is
  covered by an FDE.
* **Address map.** One `.debug_aranges` set per unit, each bound to its own
  `.debug_info` offset by a relocation, one tuple per code section — measured
  identical in *shape* to GCC 16.2 for single- and multi-section units, including
  `-ffunction-sections` (three tuples for three function sections, both
  compilers).
* **Dialect identity.** `DW_AT_language`/`DW_AT_language_name`/
  `DW_AT_language_version` per `-std=`, pinned by
  `tests/regression/check_c_std_diagnostics.py`.
* **Internal definitions at `-O0`** (this campaign): an unreferenced `static`
  function or object is emitted at `-O0` and dropped at `-O1`+, while an unused
  `static inline` function is dropped at every tier — GCC 16.2's measured rule in
  all three cases (the third is what keeps `sizeof(static_inline())` from pulling
  a call to a symbol that exists nowhere into the link), now gated in both
  directions. Measured on x86-64
  *and* i686 (`-m32`, symbol tables identical to GCC at both tiers); the rule
  lives in lowering and in an IR pass, so no target backend is involved.

### 3.2 A defect list that was withdrawn

Two items on an earlier draft of this list were **not** defects, and are recorded
here so no agent spends a day re-deriving them:

* *"A line row outside every subprogram's range in `glibc_memcmp.c -O2`."* The
  row is zero-width: it sits at the function's exclusive end, and the next row
  starts the next range. GCC emits the same boundary row. The audit was wrong,
  not the compiler; the gate now computes row widths before judging.
* *"`.debug_aranges` has one set spanning the whole text and attributed to unit
  0."* False. Each unit gets its own set with its own `debug_info` offset —
  verified with `readelf --debug-dump=aranges` on a linked two-TU program (sets at
  offset `0` and `0x56`) and on a `-ffunction-sections` object (three tuples).

### 3.3 The tag census, before and after

Aggregate over the first 49 benchmark programs, `-O0 -g -gdwarf-5 -c`, lccc
before this series / lccc now / GCC 16.2 (the same reader counted all three, and
the first column is the state the campaign started from):

| Tag | before | now | GCC 16.2 |
|---|---|---|---|
| `variable` | 0 | 208 → 68 | 695 |
| `formal_parameter` | 0 | 422 | 485 |
| `base_type` | 0 | 332 → 150 | 480 |
| `subprogram` | 283 | 283 | 338 |
| `pointer_type` | 0 | 323 → 63 | 122 |
| `typedef` | 0 | 2 | 103 |
| `const_type` | 0 | 30 | 98 |
| `array_type` / `subrange_type` | 0 | 137 → 48 | 74 / 78 |
| `member` | 0 | 1 330 → 54 | 63 |
| `structure_type` | 0 | 188 → 12 | 14 |
| `union_type` / `enumeration_type` / `enumerator` | 0 | 0 | 2 / 6 / 26 |
| `subroutine_type` | 0 | 1 | 2 |
| `lexical_block` | 0 | 0 | 207 |
| `unspecified_parameters` | 0 | 0 | 47 |
| `unspecified_type` | 49 | 54 | 0 |

The second number in a "now" cell is the state after the declaration policy of
[§3.5](#35-what-the-campaign-has-landed-measured) landed: 188 structures and
1 330 members were the whole of `_IO_FILE` and its friends, described because
`<stdio.h>` declares `stdin`/`stdout`/`stderr` whether or not the program names
them.

Three columns still differ from GCC, and each is a *different* item:

* `variable`/`lexical_block`: GCC describes **locals**, lccc describes
  file-scope objects and parameters. D14.
* `typedef`: GCC emits a DIE for every typedef it sees, lccc only for an object
  declared through one. A debugger prints the same types either way; the
  remaining difference is spelling of types nothing uses.
* `enumeration_type`/`enumerator`/`unspecified_parameters`: none of the corpus
  programs has a file-scope object of enum type, and lccc emits
  `DW_TAG_unspecified_parameters` only on a *defined* variadic function (the
  corpus has none). Both are gated on the quality gate's probe, which has one of
  each.

### 3.4 Measured missing: everything else

| Fact | Measurement |
|---|---|
| *Closed:* no `DW_AT_location` anywhere | every file-scope object this object file defines now carries a 9-byte `DW_OP_addr` whose address equals the address the program prints for it (gated); a parameter carries **no** location yet — D1b |
| *Closed:* a TU that is only data emits no debug info | `int pq_only_data = 42;` now gets a unit, `DW_AT_stmt_list`, a header-only line program (file table, no statements) and a `variable` DIE, byte-shape for byte-shape what GCC 16.2 writes |
| No `DW_TAG_inlined_subroutine` | at `-O2`, the row inside `main` that came from the inlined `pq_frame` claims `pq_frame`'s line, and a consumer cannot recover the inline stack |
| One line sequence for the whole unit | **matches GCC, re-measured**: five functions, `readelf --debug-dump=rawline \| grep -c 'Line Number Statements'` gives 1 for lccc *and* GCC 16.2 under `-g`, `-O0`, `-O2`, `-g3` and `-ffunction-sections` at every dialect; `-ffunction-sections` puts five sequences in that one program on both. Withdrawn as a gap — see D5 |
| `-S -g` carries no DIE tree | lccc emits 12 `.loc` and no `.debug_info`; GCC emits 10 `.loc` **and** its whole `.debug_info`/`.debug_abbrev`/`.debug_aranges` as data (70 lines). Assembling lccc's `-S` output with GAS therefore loses the subprogram DIEs GCC keeps |
| `-g3` has no `.debug_macro` | macro definitions are not recorded; `info macro` cannot work |
| `-gz` and `-gsplit-dwarf` warn "not supported" and emit plain debug info | honest, but a build that asks for compression gets a larger object: GCC's `-gz` marks `.debug_info` `SHF_COMPRESSED` (measured: 114 bytes on disk, 173 expanded) and lccc's 106 bytes are uncompressed |
| `-g3` emits no `.debug_macro` | GCC's `-g3` object carries the section; lccc's does not (measured on both) |
| DWARF 5 file table duplicates the primary file | entries 0 and 1 are both `fs.c`; harmless, 2 bytes of section, but it hides a real intent |

### 3.5 What the campaign has landed, measured

Each row is a capability, the section bytes it costs, and the gate assertion that
fails if it regresses. Sizes are `.debug_info` + `.debug_abbrev` + `.debug_str` +
`.debug_line_str` on `tests/benchmark/programs/fib.c` at `-g -gdwarf-5 -O0`:

| Capability | Evidence |
|---|---|
| Types: base, pointer, const/volatile/restrict, typedef, struct/union/enum, array/subrange, subroutine | The gate compares each `DW_AT_byte_size`, member offset, bit-field placement and enumerator value against what the **program itself prints** (`sizeof`, `offsetof`, the folded enum constants), not against a second compiler |
| File-scope objects, defined and declared | `DW_AT_location` present exactly when `.symtab` defines the object, and equal to the symbol's address (gated); a removed static keeps its DIE and loses the location, which is GCC 16.2's rule and also what stops a `DW_AT_location` relocation against a dropped symbol from breaking the link |
| Declarations only for referenced names | measured `extern int q_used` / `extern int q_missing`: GCC describes the first and not the second, and lccc now does the same; the `_IO_FILE` structure and its thirty members left the payload with it |
| Function parameters | a `DW_TAG_formal_parameter` child per parameter of every defined function, with `DW_AT_name` and a `DW_AT_type` that resolves; `DW_TAG_unspecified_parameters` on a variadic definition |
| A data-only translation unit | a unit with `DW_AT_stmt_list` and no `DW_AT_low_pc`, a line program whose header carries the file table, and one `variable` DIE per object |
| Bit-field layout in every dialect | `DW_AT_data_bit_offset` at version 5, `DW_AT_bit_offset` + `DW_AT_data_member_location` at 2–4 — GCC 16.2's own split, measured |

The cost, before and after the series (fib.c, same flags): `.debug_info`
578 → 118 bytes, `.debug_str` 482 → 35, `.debug_abbrev` 295 → 215, whole debug
payload 1 705 → 718 bytes. GCC 16.2: 1 021 bytes; Clang 23: 759. The payload got
*smaller* while gaining seven sections' worth of meaning, because the declaration
policy removed more than the new DIEs cost.

---

## 4. Work items

Ordered by value: D1–D4 are **landed** (see
[§3.5](#35-what-the-campaign-has-landed-measured)), and what remains is D1b
(parameter locations), D6 (inline instances), D14 (locals and lexical blocks),
then the `-S`/`.debug_macro`/compression items.

Format: **ID · class · evidence** — Why, Where, Plan (each step landable alone),
Gate (what the regression gate must assert; every gate is mutation-verified by
reverting the fix and watching it fail), Accept, Risks, Effort (S ≤ 1 day,
M ≤ 1 week, L > 1 week of agent time), ROI (1–5).

The one structural fact that shapes every item: **the DWARF writer lives in the
assembler, not in the code generator.** `build_dwarf_info_sections`
(`src/backend/elf/dwarf.rs`) receives line rows (`DwarfLineLocation`), a list of
sized function symbols (`DwarfFunction`), section sizes and a dialect config
(`DwarfConfig`); it has no access to the AST, the type table or the IR. Debug
information that needs a *name*, a *type* or a *location* must therefore be
carried **down to the writer as data**. Two designs are possible and the campaign
prescribes the first:

* **(A, prescribed) Extend the existing data model.** Add `DwarfVariable`,
  `DwarfType`, `DwarfSubprogram`-children to the writer's input, produced where
  the information exists (the pipeline knows types and symbols; the assembler
  knows final addresses and frame offsets). One writer keeps version gating,
  abbreviation tables, relocation and string interning in one place, which is why
  the current output is consistent across four DWARF versions.
* **(B, rejected) Build DIEs in the code generator** and append them to
  `.debug_info` as bytes from the backend. It duplicates the abbreviation,
  relocation and version machinery, or bypasses it; it also cannot see final
  frame offsets without a second pass, and it makes `-gdwarf-2`/`-gdwarf-3`
  divergence a per-site decision instead of a per-writer one.

### D1 · P0 · M/L — variables, formal parameters and lexical blocks

**Status: landed for file-scope objects and for parameters (names, types, and a resolving `DW_AT_type`); locals and lexical blocks are **D14**, parameter locations are **D1b**.**

**Why.** This is the item that turns `print x` from an error into an answer, and
it is the prerequisite for D2 (types) and D3 (locations): a variable DIE needs a
type reference and a location, so the three land as one series of commits.

**Where.** New `DwarfVariable`/`DwarfScope` types in `src/backend/elf/dwarf.rs`;
their producers in `src/driver/pipeline.rs` (it has `TypeContext` and the sema
function map) and in the x86/AArch64/RISC-V assemblers (they know final frame
offsets); abbreviation declarations in `declare_unit_abbrevs`.

**Plan.**
1. **Model.** `DwarfVariable { name, decl: SourcePosition, type_ref: DwarfTypeId,
   scope: ScopeId, location: Option<DwarfLocation>, is_parameter: bool,
   is_external: bool, artificial: bool }`, where `ScopeId` is a subprogram or a
   `DW_TAG_lexical_block` (with `DW_AT_low_pc`/`high_pc` from the block's line
   range, or omitted for the whole function). Locations are an ordered list so
   `DW_OP_piece` can describe a split value later.
2. **Emit variables before the lexical tree is needed.** Order matters for a
   consumer only in that a DIE's type reference must precede it if the writer
   uses backward references; prescribe forward `DW_FORM_ref4` references (absolute
   `.debug_info` offsets), which is simpler and what GCC uses in DWARF 4+.
3. **Parameters come from the function signature**, not from the IR: the sema
   function map already carries names and types in declaration order, which is
   the order a consumer lists `info args` in. At `-O0` the slot offset is the
   canonical one the ABI assigns; do not derive it from the register allocator
   before D3 step 3 exists.
4. **Locals come from the IR's named values**, with `DW_AT_decl_line` from the
   declaration span the AST already records for diagnostics.
5. **`DW_TAG_lexical_block`** for each scope that declares a name shadowing an
   outer one, or that has a lifetime shorter than the function (a loop body with
   a block-scoped variable). Do not emit a block per statement.
6. **Declarations of external functions and objects** (`DW_TAG_subprogram`
   without `low_pc`, `DW_TAG_variable` with `DW_AT_external` and no location) for
   every name the unit references but does not define. GCC emits these at `-O0`
   (measured: `external_fn`, `external_var`); they let a consumer print
   `external_var` even though it lives elsewhere.

**Gate.** `check_dwarf_quality.py` gains a probe with a parameter and a local per
function and asserts: a `DW_TAG_variable` per local with a `DW_AT_name` equal to
the source identifier, `DW_AT_decl_file`/`decl_line` inside the declaration's
span, a `DW_AT_type` that resolves, and a `DW_TAG_formal_parameter` per parameter
in declaration order. Delete the `variables`/`parameters` entries from
`KNOWN_GAPS` in the same commit — the gate fails if a gap is entered and does not
reproduce, which is what keeps the list honest.

**Accept.** `info locals`, `info args` and `print <name>` return the right value
at `-O0` on x86-64 for every local whose address is taken or whose type is
integer/pointer; the census's `variable` and `formal_parameter` columns reach
GCC's within 5 % on the benchmark corpus.

**Risks.** A wrong `DW_AT_location` is worse than none (it produces confident
wrong answers). Land locations (D3) only for the cases the gate can execute and
compare against the program's own output; for everything else emit no location,
which a consumer renders as `<optimized out>`.

### D2 · P0 · M — type DIEs

**Status: landed, and gated against the program's own `sizeof`/`offsetof`/enum values rather than against a second compiler.**

**Why.** Without a type a consumer can find a name but not read it, and cannot
print a struct, follow a pointer, or show a typedef. The census's zero columns for
`base_type`, `pointer_type`, `typedef`, `structure_type`, `array_type` and
`subrange_type` are all this item.

**Where.** A `DwarfType` table built in `src/driver/pipeline.rs` from
`TypeContext`, emitted by `src/backend/elf/dwarf.rs`; the same table serves
`DW_AT_type` on variables, parameters, subprogram return types and members.

**Plan.**
1. **Interning by structural identity.** `DwarfTypeId` from a canonical key
   (kind, size, encoding, element/member list) so two `int` declarations share
   one DIE while two `struct S` in different scopes do not. A debugger prints the
   name, so a `typedef` must stay a distinct DIE pointing at the underlying type
   (`DW_TAG_typedef` + `DW_AT_type`), never be collapsed into it.
2. **Base types**: `DW_AT_name` (`int`, `unsigned long`, …), `DW_AT_encoding`
   (`DW_ATE_signed`, `_unsigned`, `_signed_char`, `_float`, `_boolean`,
   `_UTF`), `DW_AT_byte_size`. Table-driven from `CType`; the name must be the
   *source* spelling a user expects (`unsigned long`, not `ulong`).
3. **Derived types**: pointer (`DW_AT_type`, byte size), const/volatile/restrict
   (`DW_TAG_const_type` etc. wrapping the qualifier), array (`DW_TAG_array_type`
   + one `DW_TAG_subrange_type` per dimension with `DW_AT_upper_bound`, using
   `DW_AT_count` only when the bound is variable), subroutine
   (`DW_TAG_subroutine_type` + `DW_TAG_unspecified_parameters` when the prototype
   is `()`).
4. **Aggregates**: `DW_TAG_structure_type`/`_union_type`/`_enumeration_type` with
   `DW_AT_name`, `DW_AT_byte_size`, `DW_AT_decl_file`/`decl_line`;
   `DW_TAG_member` with `DW_AT_data_member_location` (a constant, since layout is
   known) and bit-field `DW_AT_bit_size`/`DW_AT_bit_offset` (DWARF 4+) or
   `DW_AT_bit_offset` (v2–3, different origin — version-gate it);
   `DW_TAG_enumerator` with `DW_AT_const_value` (signed for negative enumerators).
5. **Anonymous aggregates** get `DW_AT_name: None` and a synthetic
   `DW_AT_type` chain; a consumer needs `DW_AT_declaration` absent and a size.
6. **Do not emit a type a consumer cannot use** (e.g. a struct that is only
   forward-declared): emit `DW_AT_declaration: 1` and no members.

**Gate.** A probe with a struct, a union, an enum, a typedef, a pointer, a
multi-dimensional array, a function pointer and a bit-field; the gate asserts the
DIE tree's byte sizes and member offsets against `sizeof`/`offsetof` printed by
the probe at runtime, so the DWARF is checked against the *program*, not against a
second compiler. Add `DW_AT_encoding`/`DW_AT_byte_size` presence assertions. Delete
the `types` gap.

**Accept.** `ptype struct S`, `print s.field`, `print *p`, `print a[2][1]` work at
`-O0`; the census's zero columns for type tags reach GCC's ±5 %.

**Risks.** Bit-field encodings changed between DWARF 3 and 4; a version-gated
writer must not emit v4 `DW_AT_bit_offset` into a v3 unit. The gate's
cross-version matrix (`-gdwarf-2..5`) is the defence.

### D3 · P0 · M then L — `DW_AT_location`

**Status: landed for every object the object file defines (`DW_OP_addr`, equal to the symbol's address), and for nothing else. A parameter's location is **D1b**.**

**Why.** A variable with a type but no location is a name a debugger cannot read.
At `-O0` the answer is simple (frame offsets for locals/parameters, absolute
addresses for statics); with optimization it needs register live ranges.

**Where.** `src/backend/elf/dwarf.rs` (expression emission: `DW_OP_fbreg`,
`DW_OP_addr`, `DW_OP_call_frame_cfa`, `DW_OP_piece`), the x86 assembler's frame
layout, and — for the optimized case — the register allocator's live ranges.

**Plan.**
1. **Statics and globals**: `DW_AT_location = DW_OP_addr <reloc>` in every
   version; assert the relocation lands on the object's section. This alone makes
   `print pq_global` work and is independent of frames.
2. **`-O0` locals and parameters**: `DW_OP_fbreg <offset>` relative to
   `DW_AT_frame_base = DW_OP_call_frame_cfa`, where `<offset>` is the slot's
   offset from the CFA the CFI already defines. The x86 assembler knows both, so
   this needs no new analysis.
3. **Optimized code**: `DW_AT_location` becomes a location list
   (`.debug_loclists` for v5, `.debug_loc` for v2–4) built from the register
   allocator's live ranges, with `DW_OP_regN`/`DW_OP_bregN` and `DW_OP_piece` for
   values split across registers and stack. Where no range covers the PC, emit
   nothing rather than a wrong range.
4. **Never invent**: a variable the compiler eliminated gets no `DW_AT_location`,
   which every consumer renders as `<optimized out>` (GCC's behaviour).

**Gate.** The `-O0` probe prints every variable's value and address; the gate
asserts the DIE's location expression evaluates (via its own `DW_OP_fbreg`/CFA
walk using `DW_AT_frame_base`) to the address the program printed. For step 3,
assert only *consistency*: at every statement row of a function, the location list
has exactly one covering range or none.

**Accept.** `print`/`info locals` correct at `-O0` on x86-64 and i686; at `-O2`,
≥ 90 % of scalar locals have a valid location at ≥ 1 row (the corpus check MB-10
already asks for).

**Risks.** The CFA-relative offset must be the value *after* the prologue, which
is what `cfi_synth` computes; reuse its CFA model instead of recomputing. A
mismatch here produces off-by-8 reads that look plausible.

### D4 · P0 · S — a unit with no code still needs a unit

**Status: landed: unit, `DW_AT_stmt_list`, header-only line program with its file table, and a `variable` DIE per object, matching GCC 16.2's shape.**

**Why.** A translation unit that is only data currently emits no `.debug_info` at
all, because `build_dwarf_info_sections` returns `None` when there is no code
section. A debugger attached to a program containing only such units reports "no
debug info"; `pq_only_data` cannot be printed at all.

**Where.** `build_dwarf_info_sections` (`if code_sections.is_empty() { return
Ok(None) }`), and the line-program builder the writer drives it from.

**Plan.** Emit a unit with `DW_AT_name`/`comp_dir`/`producer`/`language` and no
`low_pc`/`stmt_list` when the unit has data but no code; emit `DW_TAG_variable`
children once D1 lands. Add the CU's variables to `.debug_aranges`? No — it has no
addresses; a consumer finds it through the name index (D12) or by walking units.
Verify with GCC: `int x = 42;` yields a unit with one variable DIE (measured).

**Gate.** `check_data_only_unit` asserts a unit exists and names `pq_only_data`;
delete the `data_only_unit` gap.

**Accept.** `print pq_only_data` works from a program whose only debug source is a
data TU.

### D1b · P1 · M — a parameter's location

**Why.** A parameter DIE without `DW_AT_location` lets a debugger list `n` and
its type but not print `n = 4`; GCC 16.2 at `-O0` emits
`DW_AT_location : 2 byte block: 91 f4 (DW_OP_fbreg: -12)` for it. This is the last
piece of `info args` and the reason the quality gate carries a
`parameter_locations` known gap.

**Where.** The frame layout is the code generator's: `emit_store_params`
(`src/backend/generation.rs`) is where a parameter's home slot is chosen, and
`State::data_home_slot` (`src/backend/state.rs`) is the accessor. The writer
(`src/backend/elf/dwarf.rs`) has `DW_AT_frame_base = DW_OP_call_frame_cfa`, so a
location is `DW_OP_fbreg <slot - cfa_offset>`.

**Plan.** Carry the offsets from the instruction selector to the writer the way
line rows already travel: a `.dwarf_param <index> <cfa_offset>` directive in the
prologue, parsed by the assembler into a per-function table, consumed by the
parameter DIE writer. The directive form is what keeps `-S -g` honest as well —
the alternative (a side table passed to the in-process assembler only) silently
loses the information the moment a user assembles the text with `gas`.

**Gate.** The quality gate's `parameter_locations` gap is deleted in the same
commit; the probe runs, and every parameter's `DW_OP_fbreg` is checked against
the *program's* value by reading the byte at `CFA + offset` from the stopped
frame (`ptrace`-free: the probe prints `n` and the gate checks the location
expression resolves to the same slot the callee wrote).

**Risks.** An offset that is wrong by one register saves a wrong value into a
debugger's answer, which is the worst failure mode DWARF has. The gate must
compare values, not just decode the expression.

**Effort** M. **ROI** 4.

### D13 · P2 · S — a typedef DIE for every typedef the unit declares

**Why.** GCC 16.2 describes 103 typedefs over the corpus, lccc 2 (the ones an
object was declared through). A consumer printing a type name whose spelling
comes from an unused typedef (`iovec`, `ssize_t`) sees the underlying type
instead. Legal either way; a completeness gap, not a correctness one.

**Where.** `TypeGraph::intern_typedef` — the driver would intern each name in
`TypeContext::typedefs` that the unit's own source declared.

**Gate** A typedef-only probe: `typedef unsigned long pq_size;` with no object of
that type, asserted to produce a `DW_TAG_typedef` naming `pq_size`.

**Effort** S. **ROI** 2.

### D14 · P1 · L — local variables and lexical blocks

**Why.** GCC describes 207 lexical blocks and 695 variables over the corpus where
lccc describes file-scope objects and parameters only: `info locals`, `print x`
inside a function, and block-scoped shadowing are the remaining capability. This
is the largest item left, and it needs the same channel as D1b (a local's location
is a frame slot or a register) plus a location list for a value that moves.

**Where.** The IR carries named values and their declaration spans; the backend
knows each value's home slot (`State::data_home_slot`).

**Gate** The probe gains a function with a local of each storage class the gate
can execute; every local's value is compared against what the program prints.

**Risks.** A local that is folded away must not be described (a location that
does not exist is worse than none) — the same policy the removed-static case
already applies to file-scope objects.

**Effort** L. **ROI** 4.

### D5 · withdrawn 2026-11-05, re-measured — "one line sequence per function"

**Withdrawal, with the recipe.** The premise is false: GCC 16.2 does not emit one
sequence per function. On a five-function file,
`readelf --debug-dump=rawline obj \| grep -c 'Line Number Statements'` returns
**1** for GCC 16.2 and **1** for lccc under `-g`, `-g -O0`, `-O2`, `-g3`,
`-gdwarf-2/4/5` and `-ffunction-sections` — one line *program* in every case, and
with `-ffunction-sections` five *sequences* inside it on both compilers. A
consumer loses nothing, so there is no gap to close. The item re-surfaced once in
another pass of this document and is withdrawn again on the same evidence; the
gate records the ratio for both compilers rather than asserting a shape neither
writes.

The original argument, kept for the record:

**Why (as then believed).** lccc emits one sequence covering the unit's whole text; GCC emits one per
function. Both are legal, but a consumer that wants one function's rows must
decode every row first, and a `DW_LNE_end_sequence` is exactly the marker that
lets it stop. Measured: 5 functions → 1 sequence at every version.

**Where.** `build_dwarf_line_section` (`src/backend/elf/dwarf.rs`): the row
generator that ends a sequence only when the section changes.

**Plan.** End the sequence at each function's end (the writer already knows the
`DwarfFunction` list and its `offset`/`size`), starting a new one with
`DW_LNE_set_address`. Keep rows sorted; a sequence must not straddle a gap.

**Gate.** `sequences_vs_functions` in the gate's measurements becomes an
assertion: `sequences >= functions with code`.

**Risks.** A row emitted exactly at a function's end belongs to that function's
sequence; ending the sequence after it (not before) keeps `addr2line` answers
unchanged. Re-run the corpus audit after the change.

### D6 · P1 · L — `DW_TAG_inlined_subroutine`

**Why.** At `-O2` a row inside `main` legitimately carries the inlined callee's
line (the gate sees exactly this today); without an inlined-subroutine DIE the
debugger shows the caller at a line that does not exist in the caller, and
`step` jumps. The census cannot see this gap — it is a missing *tree*, not a
missing tag count — which is why the gate records it as a named gap.

**Where.** Inlining happens in `src/passes/inline.rs`;
`src/backend/elf/dwarf.rs` grows `DW_TAG_inlined_subroutine` children under the
subprogram, each with `DW_AT_abstract_origin` (a declaration DIE for the callee),
`DW_AT_call_file`/`call_line`, and `low_pc`/`high_pc` or `DW_AT_ranges`.

**Plan.** Have the inliner record `(caller, callee, call site span, inlined
regions)` as metadata on the function, carry it to the writer, emit one
`DW_TAG_inlined_subroutine` per inlining instance with its concrete ranges, and
nested instances as children. Emit `DW_TAG_subprogram` *declarations* for the
abstract origins (`DW_AT_inline: DW_INL_inlined`).

**Gate.** A probe with a `static inline` callee called from two sites at `-O2`;
assert two inlined-subroutine DIEs whose ranges are inside the caller, each with a
`call_line` matching the call site, and that the callee's module-level
`DW_TAG_subprogram` carries `DW_AT_inline`. Delete the `inlining` gap.

**Accept.** `bt` inside an inlined function shows both frames; `step` enters the
callee's source range.

### D7 · P1 · M — `-S -g` must carry the DIE tree

**Why.** Measured: GCC's `-S -g` output contains the complete `.debug_info`,
`.debug_abbrev`, `.debug_aranges` and an empty `.debug_line` that GAS fills from
`.loc`; lccc's contains `.loc` and nothing else. So `lccc -S -g x.c && as` yields
a whole different (worse) artifact than `lccc -c -g x.c`, and a build system that
compiles via assembly silently loses the tree.

**Where.** The `-S` emitter (`src/driver/` assembly text output) and the same
`build_*` functions the object path uses.

**Plan.** Write the sections the writer produces as `.section` + `.byte`/`.long`/
`.quad` data with symbolic relocations, exactly as GCC does, and keep `.loc` for
the line program so GAS can build `.debug_line` itself (GCC's empty `.debug_line`
is the tell that this is the intended split). One writer, two emitters.

**Gate.** The gate builds a probe through `-S` + `as` and asserts the resulting
object's DIE tree equals the direct-`-c` object's tree (same tags, names, ranges).

**Risks.** Using lccc's own assembler on its own `-S` output must not duplicate
the sections; the "continuing unit" path already handles the input that carries
`.debug_*`, and D7 makes that path the normal one, so test both assemblers.

### D8 · P2 · M — `-g3`: `.debug_macro`

**Why.** `-g3` is accepted; `info macro` cannot work without `.debug_macro`.
The preprocessor already records macro definitions for `-dM`-style dumps, so the
data exists.

**Where.** Preprocessor macro table → new `.debug_macro` writer in
`src/backend/elf/dwarf.rs`; the unit's `DW_AT_macro_info` (v2–4) or
`DW_AT_macros` (v5) attribute.

**Plan.** Emit `DW_MACRO_define_strp`/`undef_strp` (v5) or
`DW_MACINFO_define`/`undef` (v2–4) opcodes in definition order with line numbers;
one macro unit per file, imported with `DW_MACRO_import` for headers. Only at
`-g3`, and only for macros the file itself defines or undefines.

**Gate.** A probe with two macros; assert both are present with the right line
and replacement text, and that `-g2`/`-g` emits no `.debug_macro` at all.

### D9 · P2 · S — `-gz` must compress or be refused

**Why.** A `-gz` build currently warns that compression is unsupported and emits
uncompressed debug info, so a distribution that passes `-gz` (`dpkg-buildflags`
does on some profiles) gets a larger artifact than GCC produces for the same
command line. Measured: GCC's `.debug_info` is 114 bytes on disk carrying
`SHF_COMPRESSED`, 173 bytes expanded; lccc's is 106 bytes uncompressed.

**Where.** Debug section emission in the ELF writer.

**Plan.** Emit `SHF_COMPRESSED` with `ELFCOMPRESS_ZLIB` for every `.debug_*`
section, as GCC 16.2 does (measured: GCC's debug sections are compressed). If a
target or link mode cannot carry it, refuse the flag with a diagnostic instead of
ignoring it.

**Gate.** Assert `SHF_COMPRESSED` is set, and that the gate's own decoder reads
the sections back (it already decompresses).

### D10 · P2 · S — `-fdebug-prefix-map` / `-ffile-prefix-map`

**Why.** Reproducible builds need `DW_AT_name`/`DW_AT_comp_dir`/`DW_AT_decl_file`
to be rewritten, not just `__FILE__`; lccc's cross-directory determinism is a
property to protect, and a build in `/build/foo-1.2` currently embeds that path.

**Where.** The path strings the writer receives (`primary_file`, `comp_dir`,
`files`), plus `parse_file_directive` for assembler input.

**Plan.** Apply the mapping at the point strings enter the writer (one function,
so no site can forget it), and keep `-fdebug-prefix-map` independent of
`-ffile-prefix-map` (the latter also affects `__FILE__`).

**Gate.** Build the same source from two different directories with a mapping and
assert the objects are byte-identical.

### D11 · P3 · L — `-gsplit-dwarf`

**Why.** Builds with many TUs spend most of their debug size on units that are
never read; `.dwo` files let a linker drop them. Currently warned as unsupported.

**Where.** Unit emission in the ELF writer; a new `.dwo` writer; `DW_AT_dwo_name`,
`DW_AT_comp_dir`, `DW_AT_GNU_dwo_id`/`.debug_addr` indirection.

**Plan.** Emit a skeleton unit in the object (addresses only, `DW_FORM_addrx`) and
the full DIE tree into `<obj>.dwo` with `.debug_str_offsets`/`.debug_addr`;
coordinate with the driver so the `.dwo` lands next to the object.

**Gate.** Assert the object's `.debug_info` is a skeleton unit with
`DW_AT_dwo_name`, the `.dwo` exists and holds the tree, and a consumer that
follows `DW_AT_dwo_name` resolves `pq_global`.

### D12 · P3 · M — `.debug_names` (DWARF 5 name index)

**Why.** `info functions pq_` and `break pq_frame` currently walk every unit;
GCC emits a name index at `-g` on DWARF 5, which makes lookup O(log n).

**Where.** New index writer; the CU's `DW_AT_addr_base`/`DW_AT_str_offsets_base`.

**Plan.** One index for the whole object, `.debug_names` header, one bucket per
name hash, entries for subprograms and variables with `DW_IDX_compile_unit` and
`DW_IDX_die_offset`. Emit only on version 5, as GCC does.

**Gate.** Assert the index parses, that every `pq_` subprogram and variable
appears exactly once, and that a lookup by name returns the same DIE offsets a
full walk would.

---

## 5. Decisions taken, and the alternatives that were rejected

| Decision | Why | Alternative, and why not |
|---|---|---|
| Keep one DWARF writer in the assembler | Version gating, abbreviations, relocation and string interning live in one place; the current cross-version consistency is a result of that | Build DIEs in the backend: duplicates the machinery or bypasses it, and cannot see final frame offsets |
| Carry variable/type data down as a table, not as directives | A table can be validated before emission (`DwarfTypeId` must resolve), and a bad entry is a compile-time error, not a malformed section | Custom `.lccc_var` directives: GAS rejects unknown directives, so `-S` output would stop being assemblable by GAS |
| `-O0` keeps every internal **non-inline** definition | GCC 16.2 measured; an inline-asm `call` cannot be seen by the reference graph, and a debugger must find a function the user wrote. The `inline` exception is measured too, and it is not cosmetic: `static int f(void){ return missing(); }` named only by `sizeof(f())` makes GCC's `-O0` link fail, while `static inline int f(void){...}` named the same way links, because GCC drops the unused inline body | Keep pruning at `-O0`: an undefined-symbol link error GCC does not have (reproduced). Root every internal definition including inline ones: a `static inline` helper mentioned only in a `sizeof` drags a call to a symbol that exists nowhere into the link (reproduced in this repository's own `static_function_refs_vm_contexts` corpus) |
| Emit no location rather than a guessed one | A confident wrong answer is worse than `<optimized out>` | Best-effort location: silently wrong reads, which no gate can distinguish from a wrong program |
| Known gaps are listed in the gate, not silently skipped | A gap that stops reproducing is reported, so the list cannot rot | An `xfail` list in a test file: drifts from the documentation |

---

## 6. What is explicitly *not* in this campaign

* **gdb-script automation.** No `gdb` in the harness (checked: not in the
  toolchain cache, not installable offline). Every acceptance test above is
  expressed as a DWARF-structure assertion plus the program's own printed output,
  which is stronger than a gdb transcript for the purpose of a CI gate — and it
  is reproducible without a debugger.
* **`llvm-dwarfdump --verify`.** Not available offline. The gate's own decoder
  plays the role, and every decoder path has a `--self-test` on hand-built bytes,
  so the instrument is validated without an external tool.
* **DWARF 6 / `.debug_types`.** Not emitted by GCC 16.2 either
  (`-fdebug-types-section` produces `.debug_types` in v4 mode only); out of scope
  until a consumer asks.
* **BTF.** A separate concern (kernel `pahole` compatibility); it would be built
  from the same type table D2 introduces, so it is a follow-up, not part of this.

---

## 7. How to extend the gate

`tests/regression/check_dwarf_quality.py` is the campaign's instrument and its
contract:

* **Add a probe** to `PROBES` (source text is part of the file, so a probe cannot
  drift). Functions the probe wants checked are named `pq_*`.
* **Add a check** as a `check_*(report, ...)` function that calls `report.fail(...)`
  for a violation and `report.gap(key, ...)` for a documented gap; a gap key that
  never fires must be deleted from `KNOWN_GAPS`.
* **Extend the decoder**, then add a case to `self_test()` that builds the bytes by
  hand. A decoder change without a self-test case is a change nobody can review.
* **Mutation-verify**: break the compiler (or point the gate at a wrapper script
  that rewrites the flag), watch the gate fail, then fix. This is how the `-O0`
  static rule was verified in both directions.

Two properties make the instrument trustworthy, and both were learned by point
it at **GCC 16.2 instead of lccc**:

* **It applies relocations.** In an object file every `.debug_str` reference is a
  zero plus an addend, so a reader that ignores `.rela.debug_str` sees every DIE
  name as "the string at offset zero" — the producer string. The gate patches
  debug sections the way a linker does before decoding them.
* **It resolves `DW_AT_abstract_origin`.** An optimizing compiler moves a
  function's description onto an *abstract* DIE and leaves concrete DIEs carrying
  only a range and a reference to it; `decl_file`, `decl_line`, `type` and the
  parameters all live on the origin. A gate that only reads `DW_AT_name` reports
  "a function nobody describes" for correct GCC output.

With both in place, `--lccc "$(which gcc-16)"` — the gate run *on GCC 16.2* —
reports **0 failures and 0 known gaps**. That is the claim worth having: the gap
list is a statement about lccc, not about the checker. Run on lccc it reports 0
failures and the six deficits D1–D4 and D6 name.

The gate runs in CI (`ci.yml`, job `frontend-b`) and locally
(`scripts/ci_local.sh`, `dwarf-quality`); both pass `--gcc gcc-16` so the oracle
comparison runs where GCC 16 exists and degrades to structural checks elsewhere.

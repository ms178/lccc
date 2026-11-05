//! DWARF debug-information writer shared by the ELF assemblers.
//!
//! The compiler already lowers source locations to GAS-compatible `.file` and
//! `.loc` directives. This module turns those records into a `.debug_line`
//! unit, retaining relocations to the final code section so both relocatable
//! objects and linked images carry usable source line information.
//!
//! # Inputs that already carry debug sections
//!
//! `gcc -S -g` hands the assembler its compilation unit as assembly: `.debug_info`,
//! `.debug_abbrev` and `.debug_aranges` as directives, an empty `.debug_line` for
//! the assembler to fill, and `.debug_str`/`.debug_line_str` declared as
//! mergeable string pools (`"MS",@progbits,1`).  Nothing here replaces such a
//! section: the generated program is placed *after* the input's bytes and
//! `DW_AT_stmt_list` names the offset it starts at ([`place_line_build`]), the
//! two string tables continue the input's pools so an interned offset stays in
//! one coordinate system ([`DwarfStrings::continuing`]), and the section table
//! lists each name once.  The pools keep the `SHF_MERGE|SHF_STRINGS` shape GCC
//! and GAS declare, which is what lets a linker keep one copy of every path
//! across all objects of a link (measured: ten one-function units carry 126
//! bytes of `.debug_str` and 144 of `.debug_line_str`; mold keeps 43 and 60).
//!
//! Like GAS (`dwarf2_finish`), an object whose source carries `.loc` rows but
//! no `.debug_info` of its own also gets a synthesized compile unit per line
//! program — `DW_AT_stmt_list` pointing at it, the code range, name, directory,
//! producer and language — with one `DW_TAG_subprogram` per sized function
//! symbol. Debuggers reach a line table only through such a unit: without it
//! gdb reports "No symbol table is loaded" and cannot break at `file:line`.
//!
//! # Versions
//!
//! The dialect is selected by [`DwarfConfig::version`], which the driver fills
//! from `-gdwarf-<N>`; the default is 5 because that is what GCC 16.2 emits for
//! a bare `-g` (measured).  When the input brings its own `.debug_info` and the
//! command line pinned no version, the generated program adopts that unit's
//! dialect instead ([`DwarfConfig::continuing_unit`], [`unit_version`]): GAS does
//! the same -- the same assembly assembled by plain `as` yields a version-2 line
//! program from `-gdwarf-2` input and a version-5 one from `-gdwarf-5` input --
//! and `DW_AT_stmt_list` binds the two, so a consumer that trusts the unit's
//! dialect must be able to read the program it names.  Every version from 2
//! through 5 has the header layout its specification requires:
//!
//! | version | unit header                       | line header            |
//! |---------|-----------------------------------|------------------------|
//! | 2, 3    | length, version, abbrev, adsize    | no `max_ops`           |
//! | 4       | length, version, abbrev, adsize    | `max_ops` after `min_inst` |
//! | 5       | length, version, unit_type, adsize, abbrev | `address_size`, `segment_selector_size`, `max_ops`, directory and file entry formats |
//!
//! Version 5 replaces the v4 prologue's inline include-directory and file-name
//! strings with format-described tables (`DW_LNCT_path`,
//! `DW_LNCT_directory_index`) whose entries are `DW_FORM_line_strp` offsets into
//! `.debug_line_str` -- the section DWARF 5 added for exactly this data, and
//! the form GCC and Clang use there.  The unit's own name and its compilation
//! directory are `DW_FORM_line_strp` offsets into that same table, so the
//! compilation directory is stored once; the producer and every function name
//! live in `.debug_str`.  Both tables
//! ([`DwarfStrings`]) intern on first use: a translation unit repeats the same
//! twenty paths across thousands of `.loc` rows, so interning keeps the debug
//! sections proportional to the *distinct* names rather than to the row count.
//! Versions 2..4 keep inline `DW_FORM_string` names, which is what their
//! consumers expect and what GCC emits there.
//!
//! # Language
//!
//! [`DwarfConfig::for_c`] maps `__STDC_VERSION__` onto the language code GCC
//! 16.2 records for the same dialect, header version included: `DW_LANG_C89`,
//! `DW_LANG_C99`, and for version 5 `DW_LANG_C11` plus the
//! `DW_AT_language_name` (`DW_LNAME_C`) / `DW_AT_language_version` pair that
//! tells a debugger the unit is C17 or C23 (DWARF 5 has no C17 code).  Versions 2..4 clamp to C99,
//! exactly as GCC does, because DWARF 4 and older never defined a newer code.
//!
//! # Addresses
//!
//! Code addresses are normally relocated against the code section's symbol
//! plus an offset.  A target whose linker deletes code bytes (RISC-V linker
//! relaxation) passes a `symbol_at` callback instead: then every address --
//! each row's, each sequence end, each `DW_AT_low_pc`/`DW_AT_high_pc` -- is
//! relocated against a label at it, which the linker moves with the code
//! (GNU as: `DWARF2_USE_FIXED_ADVANCE_PC` under `linkrelax`).  A row then
//! costs a `DW_LNE_set_address` instead of an address advance, and
//! `DW_AT_high_pc` is an address, not a length.
//!
//! # Address map
//!
//! Each unit also contributes a set to `.debug_aranges`, the table a consumer
//! reads first so that turning a program counter into a compile unit costs a
//! binary search instead of a walk over every unit header: the set names the
//! unit's offset in `.debug_info` and lists the addresses it covers, both as
//! relocations.  A relaxing target (RISC-V) gets no map: its range lengths are
//! computed before the linker deletes bytes, and a map that is slightly too long
//! would claim code belonging to the next unit -- a scan is better than a lie.
//!
use super::dwarf_spec::{
    BaseKind, DwarfMember, DwarfType, DwarfUnitSpec, DwarfVariable, RecordKind, TypeQual,
};
use super::{ObjReloc, ObjSection, SHF_MERGE, SHF_STRINGS, SHT_PROGBITS, parse_string_literal};
use crate::common::fx_hash::FxHashMap;

/// Names the symbol at (section index, offset): see the module documentation.
/// The callback's own lifetime `'f` is separate from the borrow `'a`, so one
/// callback can be lent to several builders in turn.
pub(crate) type SymbolAt<'a, 'f> = Option<&'a mut (dyn FnMut(usize, u64) -> String + 'f)>;

/// `.file` operands: `N "name"`, or GNU as's `N "dir" "name" [md5 VALUE]`
/// (the two joined unless `name` is absolute).  `None` for the number-less
/// `.file "name"`, which names the object's source file but no line-table
/// file.
pub(crate) fn parse_file_directive(args: &str) -> Result<Option<(u32, String)>, String> {
    let args = args.trim();
    let number_end = args
        .find(|c: char| c.is_whitespace() || c == '"')
        .unwrap_or(args.len());
    if number_end == 0 {
        return Ok(None);
    }
    let (number, rest) = args.split_at(number_end);
    let number: u32 = number
        .parse()
        .map_err(|_| format!("invalid .file number: {number}"))?;
    let (first, rest) = string_operand(rest)?;
    let rest = rest.trim_start();
    let name = if rest.starts_with('"') {
        let (file, _) = string_operand(rest)?;
        if file.starts_with('/') || first.is_empty() {
            file
        } else {
            format!("{}/{}", first.trim_end_matches('/'), file)
        }
    } else {
        first
    };
    Ok(Some((number, name)))
}

/// A leading string literal, unescaped, and the text after it.
fn string_operand(s: &str) -> Result<(String, &str), String> {
    let s = s.trim_start();
    let mut escaped = false;
    let end = s
        .char_indices()
        .skip(1)
        .find(|&(_, c)| {
            let close = !escaped && c == '"';
            escaped = !escaped && c == '\\';
            close
        })
        .map(|(i, _)| i)
        .ok_or_else(|| format!("expected a string literal: {s}"))?;
    let bytes = parse_string_literal(&s[..=end])?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), &s[end + 1..]))
}

/// Row attributes carried by a GAS `.loc`. `is_stmt` and `isa` persist
/// across rows (GAS keeps them in the line-program state machine), so they
/// are `Option`s here and resolved in source order by the line builder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DwarfLocOptions {
    pub is_stmt: Option<bool>,
    pub isa: Option<u64>,
    pub basic_block: bool,
    pub prologue_end: bool,
    pub epilogue_begin: bool,
    pub discriminator: Option<u64>,
}

/// Parse `.loc FILE LINE [COLUMN] [OPTIONS]`. GNU view annotations are not
/// yet represented; retain compatibility with the previously ignored option.
pub(crate) fn parse_loc_directive(args: &str) -> Result<(u32, u32, u32, DwarfLocOptions), String> {
    let mut words = args.split_whitespace().peekable();
    let invalid = || format!("invalid .loc directive: .loc {args}");
    let file = words
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or_else(invalid)?;
    let line = words
        .next()
        .and_then(|w| w.parse().ok())
        .ok_or_else(invalid)?;
    let column = if words.peek().is_some_and(|w| w.parse::<u32>().is_ok()) {
        words
            .next()
            .and_then(|w| w.parse().ok())
            .ok_or_else(invalid)?
    } else {
        0
    };
    let mut options = DwarfLocOptions::default();
    while let Some(word) = words.next() {
        match word {
            "basic_block" => options.basic_block = true,
            "prologue_end" => options.prologue_end = true,
            "epilogue_begin" => options.epilogue_begin = true,
            "is_stmt" => match words.next() {
                Some("0") => options.is_stmt = Some(false),
                Some("1") => options.is_stmt = Some(true),
                _ => return Err(invalid()),
            },
            "isa" => {
                options.isa = Some(
                    words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(invalid)?,
                )
            }
            "discriminator" => {
                options.discriminator = Some(
                    words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(invalid)?,
                )
            }
            "view" => {
                words.next().ok_or_else(invalid)?;
            }
            _ => return Err(invalid()),
        }
    }
    Ok((file, line, column, options))
}

/// `DW_LNE_set_address`, relocated against `symbol` + `addend`; `base` is
/// where `unit` will start in the section.
fn push_set_address(
    unit: &mut Vec<u8>,
    relocs: &mut Vec<ObjReloc>,
    base: usize,
    address_size: usize,
    reloc_type: u32,
    symbol_name: String,
    addend: i64,
) {
    unit.push(0); // DW_LNE_extended_op
    write_uleb128((address_size + 1) as u64, unit);
    unit.push(2); // DW_LNE_set_address
    relocs.push(ObjReloc {
        offset: (base + unit.len()) as u64,
        reloc_type,
        symbol_name,
        addend,
    });
    unit.resize(unit.len() + address_size, 0);
}

/// A zero-filled relocation record: `size` bytes at the current end of `data`,
/// resolved by the linker against `symbol` + `addend`.
fn push_reloc(
    data: &mut Vec<u8>,
    relocs: &mut Vec<ObjReloc>,
    reloc_type: u32,
    size: usize,
    symbol: &str,
    addend: u64,
) -> Result<(), String> {
    let addend = i64::try_from(addend)
        .map_err(|_| "DWARF relocation addend exceeds signed range".to_string())?;
    relocs.push(ObjReloc {
        offset: data.len() as u64,
        reloc_type,
        symbol_name: symbol.to_string(),
        addend,
    });
    data.resize(data.len() + size, 0);
    Ok(())
}

/// `DW_FORM_strp`/`DW_FORM_line_strp`: a 32-bit offset into a string section,
/// relocated against that section so the linker can adjust it when it merges
/// the string tables of several objects (a debugger resolves these offsets
/// against the *linked* section, not the object's).
fn push_strp(
    unit: &mut Vec<u8>,
    relocs: &mut Vec<ObjReloc>,
    base: usize,
    reloc_type: u32,
    section: &str,
    offset: u32,
) {
    relocs.push(ObjReloc {
        offset: (base + unit.len()) as u64,
        reloc_type,
        symbol_name: section.to_string(),
        addend: i64::from(offset),
    });
    unit.resize(unit.len() + 4, 0);
}

/// A source row captured while processing `.loc`.
#[derive(Clone, Debug)]
pub(crate) struct DwarfLineLocation {
    pub section_index: usize,
    pub offset: u64,
    pub file_number: u32,
    pub line: u32,
    pub column: u32,
    pub options: DwarfLocOptions,
    /// Assembly source order, used to keep rows stable when offsets tie.
    pub sequence: u64,
}

/// One line-program row as the unit builder reads it: the position a function's
/// first address carries, with `is_stmt` already resolved.
#[derive(Debug, Clone, Copy)]
struct SourceRow {
    offset: u64,
    file_number: u32,
    line: u32,
    column: u32,
    is_stmt: bool,
}

/// A unit's line-program rows, grouped by the code section they describe, with
/// `is_stmt` resolved -- the view `DW_TAG_subprogram` needs.
struct SourceRows {
    by_section: Vec<Vec<SourceRow>>,
    /// True when some row states a declaration position (see
    /// [`Self::declaration`]).
    has_declarator_rows: bool,
}

impl SourceRows {
    /// Group `locations` by section and sort them into assembly order.
    fn new(locations: &[DwarfLineLocation], section_count: usize) -> Self {
        let mut by_section: Vec<Vec<SourceRow>> = vec![Vec::new(); section_count];
        let mut source_is_stmt = true;
        // `is_stmt` is sticky across `.loc`s (a directive that omits it keeps
        // the previous value), so it is resolved once here rather than re-read
        // per function.
        for row in locations {
            if let Some(value) = row.options.is_stmt {
                source_is_stmt = value;
            }
            if let Some(slot) = by_section.get_mut(row.section_index) {
                slot.push(SourceRow {
                    offset: row.offset,
                    file_number: row.file_number,
                    line: row.line,
                    column: row.column,
                    is_stmt: source_is_stmt,
                });
            }
        }
        for rows in &mut by_section {
            // Stable: two rows can share an address (the declarator row and the
            // body's opening brace both sit at a function's first byte), and the
            // declarator row is the one written first.
            rows.sort_by_key(|row| row.offset);
        }
        // A unit with at least one non-statement row is compiler output: that
        // row states a declaration position, which is what
        // `DW_AT_decl_line`/`DW_AT_decl_column` name.  Otherwise the first row
        // of a function is its body, and the column of the first *statement* is
        // not the declarator's -- a wrong column is worse than none, so the
        // attribute is declared only where a declarator row can supply it.
        let has_declarator_rows = locations
            .iter()
            .any(|row| row.options.is_stmt == Some(false));
        Self {
            by_section,
            has_declarator_rows,
        }
    }

    /// Where the subprogram at `offset..offset + size` is declared: its first
    /// row, at the one file numbering the unit's dialect uses.
    ///
    /// The column is stated only when that first row is a declarator row; a
    /// statement's column would be a different position altogether.
    fn declaration(
        &self,
        file_table: &[(u32, &str)],
        version: u8,
        section_index: usize,
        offset: u64,
        size: u64,
    ) -> Option<SourcePosition> {
        let rows = self.by_section.get(section_index)?;
        let end = offset.checked_add(size)?;
        let first = rows.partition_point(|row| row.offset < offset);
        let row = *rows.get(first)?;
        // The row must belong to this function: the first row at or after its
        // end describes the next one, and claiming it would attribute a
        // function to its neighbour's source line.
        if row.offset >= end || row.line == 0 || row.file_number == 0 {
            return None;
        }
        // The same numbering the line program uses (see `file_index`): the
        // number itself for version 5, the 1-based slot before it.
        let position = file_table.iter().position(|(n, _)| *n == row.file_number)?;
        let file = if version >= 5 {
            u64::from(row.file_number)
        } else {
            position as u64 + 1
        };
        Some(SourcePosition {
            file,
            line: u64::from(row.line),
            column: if row.is_stmt { 0 } else { row.column },
        })
    }
}

/// Where a subprogram is declared, as its `DW_TAG_subprogram` states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourcePosition {
    /// The unit's file number for the declaration's file.
    file: u64,
    line: u64,
    /// `DW_AT_decl_column`; 0 when the row is a statement and so cannot state
    /// the declarator's column.
    column: u32,
}

/// A sized function symbol, for its `DW_TAG_subprogram`.
pub(crate) struct DwarfFunction<'a> {
    pub name: &'a str,
    pub section_index: usize,
    pub offset: u64,
    pub size: u64,
    pub external: bool,
}

/// The final size/name of an assembler section, after branch relaxation.
pub(crate) struct DwarfSectionInfo<'a> {
    pub name: &'a str,
    pub size: u64,
    pub is_executable: bool,
}

// ---------------------------------------------------------------------------
// Dialect selection
// ---------------------------------------------------------------------------

/// The oldest and newest DWARF versions this writer emits.  GCC 16.2 accepts
/// `-gdwarf-2` through `-gdwarf-5` and rejects `-gdwarf-1`/`-gdwarf-6`
/// (measured), so the driver's grammar and this writer's range agree.
pub(crate) const DWARF_MIN_VERSION: u8 = 2;
pub(crate) const DWARF_MAX_VERSION: u8 = 5;
/// GCC 16.2's default for a bare `-g` is DWARF 5 (measured), so that is the
/// default here: the same flag produces the same dialect from both compilers.
pub(crate) const DWARF_DEFAULT_VERSION: u8 = 5;

/// The dialect a debug section is written in: which DWARF version, and which
/// language a compilation unit declares.
///
/// The two halves travel together because they are decided by the same two
/// command-line facts (`-gdwarf-<N>` and `-std=`), and because the language
/// answer *depends* on the version: DWARF 4 and older have no code for C11, so
/// a C23 unit is recorded as C99 there, which is exactly what GCC 16.2 does
/// (measured for `-gdwarf-2..4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DwarfConfig {
    pub version: u8,
    /// `DW_AT_language`: `DW_LANG_C89`, `DW_LANG_C99`, `DW_LANG_C11` or
    /// `DW_LANG_Mips_Assembler`.
    pub language: u16,
    /// `DW_AT_language_name`, emitted for C17 and newer on version 5 (GCC
    /// 16.2 emits `DW_LNAME_C` there because DWARF has no C17 code).
    pub language_name: Option<u16>,
    /// `DW_AT_language_version`: the `__STDC_VERSION__` year, so a debugger can
    /// tell C17 from C23 inside a `DW_LANG_C11` unit.
    pub language_version: Option<u32>,
    /// Whether the command line pinned the version (`-gdwarf-<N>`), in which
    /// case it is used exactly as given.  Unpinned, the dialect follows the unit
    /// the input already carries -- see [`DwarfConfig::continuing_unit`].
    pub version_pinned: bool,
}

impl DwarfConfig {
    /// Debug info for an assembly source: GAS records the assembler language
    /// for `.s` input, and no version defines a `language_name` for it.
    pub(crate) const fn assembler(version: u8) -> Self {
        Self {
            version: clamp_version(version),
            language: DW_LANG_MIPS_ASSEMBLER,
            language_name: None,
            language_version: None,
            version_pinned: false,
        }
    }

    /// This configuration, adopting the dialect of the unit the input already
    /// declares when the command line did not pin one.
    ///
    /// `DW_AT_stmt_list` binds a unit to its line program, so the program has to
    /// be readable by whatever reads that unit: `gcc -S -g -gdwarf-2` hands over
    /// a version-2 unit, and GAS derives its own default dialect from that unit
    /// (measured on binutils 2.47: the same assembly assembled with plain `as`
    /// yields a version-2 program, and `-gdwarf-5` assembly a version-5 one).
    /// lccc therefore follows the unit too -- an explicit `-gdwarf-<N>` still
    /// wins over it, because that is the caller saying which dialect they want.
    pub(crate) fn continuing_unit(mut self, unit: Option<u8>) -> Self {
        if let (Some(version), false) = (unit, self.version_pinned) {
            self.version = clamp_version(version);
        }
        self
    }

    /// Debug info for a C translation unit.
    ///
    /// `stdc_version` is `__STDC_VERSION__`'s spelling (`"202311L"`), or `None`
    /// for C89, which never defined the macro. The mapping is the one GCC 16.2
    /// records for the same dialect and version (measured for
    /// `-std=c89|c99|c11|c17|c23` × `-gdwarf-2..5`):
    ///
    /// | dialect | v2..4 | v5 |
    /// |---------|-------|----|
    /// | C89     | C89   | C89 |
    /// | C99     | C99   | C99 |
    /// | C11     | C99   | C11 |
    /// | C17+    | C99   | C11 + name/version |
    pub(crate) fn for_c(version: u8, stdc_version: Option<Option<&str>>) -> Self {
        let version = clamp_version(version);
        let year = stdc_year(stdc_version);
        let c99_or_newer = year.is_some_and(|year| year >= 199901);
        if version < 5 {
            // DWARF 4 and older stop at `DW_LANG_C99`; GCC clamps here too.
            return Self {
                version,
                language: if c99_or_newer {
                    DW_LANG_C99
                } else {
                    DW_LANG_C89
                },
                language_name: None,
                language_version: None,
                // A C translation unit is compiled here and now: there is no
                // input unit whose dialect could be adopted.
                version_pinned: true,
            };
        }
        let c11_or_newer = year.is_some_and(|year| year >= 201112);
        Self {
            version,
            language: if c11_or_newer {
                DW_LANG_C11
            } else if c99_or_newer {
                DW_LANG_C99
            } else {
                DW_LANG_C89
            },
            // DWARF 5 has no code for C17/C23; the version attribute is what
            // distinguishes them, and GCC emits it only from C17 on.
            language_name: year.filter(|year| *year >= 201710).map(|_| DW_LNAME_C),
            language_version: year.filter(|year| *year >= 201710),
            // See the version-4 arm: a C unit is always written here.
            version_pinned: true,
        }
    }
}

const fn clamp_version(version: u8) -> u8 {
    if version < DWARF_MIN_VERSION {
        DWARF_MIN_VERSION
    } else if version > DWARF_MAX_VERSION {
        DWARF_MAX_VERSION
    } else {
        version
    }
}

/// The year in `__STDC_VERSION__`'s spelling: `"202311L"` is 202311.
fn stdc_year(stdc_version: Option<Option<&str>>) -> Option<u32> {
    let text = stdc_version??;
    text.strip_suffix('L').unwrap_or(text).parse().ok()
}

// ---------------------------------------------------------------------------
// String table
// ---------------------------------------------------------------------------

/// A DWARF string table: NUL-terminated strings, interned on first use, and
/// written as `.debug_str` or `.debug_line_str` when it is not empty.
///
/// Interning matters: a `.loc`-rich translation unit repeats the same handful
/// of paths thousands of times, and one offset table turns that into one copy
/// per *distinct* name.  The section is written only when a version that can
/// reference it was emitted, so DWARF 4 output stays self-contained.
#[derive(Default)]
pub(crate) struct DwarfStrings {
    data: Vec<u8>,
    offsets: FxHashMap<String, u32>,
}

impl DwarfStrings {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A table that continues a section the input already built.
    ///
    /// `gcc -S -g` writes `.debug_info`, `.debug_abbrev` and `.debug_aranges`
    /// itself and leaves `.debug_line` empty for the assembler to fill, so the
    /// generated program is appended to the caller's bytes.  The same holds for
    /// the two string tables: interning then reuses a name that is already
    /// there and appends only new ones, which keeps every offset in one
    /// coordinate system -- the section's.
    ///
    /// Only positions that *start* a string (offset 0, or just after a NUL) can
    /// be referenced by a `DW_FORM_strp`, so those are the ones indexed; a
    /// caller that hands over bytes without a terminating NUL gets one, because
    /// appending to an unterminated name would silently extend it.
    pub(crate) fn continuing(existing: &[u8]) -> Self {
        let mut this = Self {
            data: existing.to_vec(),
            offsets: FxHashMap::default(),
        };
        if this.data.last().is_some_and(|&byte| byte != 0) {
            this.data.push(0);
        }
        let mut start = 0;
        while start < this.data.len() {
            match this.data[start..].iter().position(|&byte| byte == 0) {
                Some(0) => {
                    start += 1;
                    continue;
                }
                Some(length) => {
                    let text =
                        String::from_utf8_lossy(&this.data[start..start + length]).into_owned();
                    this.offsets.entry(text).or_insert(start as u32);
                    start += length + 1;
                }
                None => break,
            }
        }
        this
    }

    /// The section offset of `text`, appending it on first use.
    fn intern(&mut self, text: &str) -> Result<u32, String> {
        if let Some(&offset) = self.offsets.get(text) {
            return Ok(offset);
        }
        // A NUL inside a name would split one string into two and silently
        // point every later reference at a suffix, so refuse it here rather
        // than emit a section no debugger can interpret.
        if text.as_bytes().contains(&0) {
            return Err(format!("DWARF string contains a NUL byte: {text:?}"));
        }
        let offset = u32::try_from(self.data.len())
            .map_err(|_| "DWARF .debug_str exceeds 4 GiB".to_string())?;
        self.data.extend_from_slice(text.as_bytes());
        self.data.push(0);
        self.offsets.insert(text.to_string(), offset);
        Ok(offset)
    }

    /// The finished section under `name`, or `None` when nothing was
    /// referenced (so a dialect that never points at it emits nothing).
    pub(crate) fn into_section(self, name: &str) -> Option<ObjSection> {
        (!self.data.is_empty()).then(|| debug_section(name, self.data, Vec::new()))
    }
}

// ---------------------------------------------------------------------------
// Line programs
// ---------------------------------------------------------------------------

/// A built `.debug_line` section: one line program, covering `code_sections`.
///
/// One compilation unit has exactly one line program -- its header carries one
/// directory table and one file table -- so the sections an object's code lives
/// in become one *sequence* each inside that single program, which is the shape
/// GAS and GCC both emit.  A unit per section instead would repeat the whole
/// prologue for every function and hand a consumer one compilation unit per
/// function: measured on a 121-function translation unit built with
/// `-ffunction-sections`, `.debug_line` grew to 10,111 bytes for 478 rows where
/// GCC needed 8,844 for 1,453 rows in one unit.
pub(crate) struct DwarfLineBuild {
    pub line: ObjSection,
    /// The code section each sequence describes, in emission order.
    pub code_sections: Vec<usize>,
}

/// The assembler's `.file` records as a DWARF file table.
///
/// DWARF numbers file entries from 1 and so does the line program's file
/// register, so the table is `(assembler number, name)` in first-declaration
/// order with GAS's replacement rule (a later `.file N "x"` renames entry N,
/// it does not append).  The line program and `DW_AT_decl_file` share this
/// mapping, which is why it is computed once and handed to both.
fn debug_file_table(files: &[(u32, String)]) -> Result<Vec<(u32, &str)>, String> {
    let mut table: Vec<(u32, &str)> = Vec::new();
    for (number, filename) in files {
        if filename.as_bytes().contains(&0) {
            return Err(format!(
                "DWARF source filename for .file {number} contains a NUL byte"
            ));
        }
        // Versions 2..4 terminate the file table with an empty name, so an
        // entry with an empty base name would silently end the table early and
        // hide every file after it. Refuse instead of emitting a section that
        // a consumer must misread.
        if split_path(filename).1.is_empty() {
            return Err(format!(
                "DWARF source filename for .file {number} is empty or ends in a directory separator"
            ));
        }
        if let Some((_, existing)) = table.iter_mut().find(|(n, _)| n == number) {
            *existing = filename;
        } else {
            table.push((*number, filename));
        }
    }
    // Both tables are addressed by number -- a version 5 table's slot *i* is
    // `.file i`, and versions 2..4 number their slots from 1 -- so the numbers
    // have to run `0, 1, 2, ...` or `1, 2, 3, ...` in declaration order.  A
    // skipped or reordered number would silently renumber every file after it
    // and point every `DW_AT_decl_file` an input unit carries at the wrong
    // name; GAS refuses such input ("unassigned file number 2", measured).
    let starts_at_zero = table.first().is_some_and(|(number, _)| *number == 0);
    if let Some((_, (number, _))) = table
        .iter()
        .enumerate()
        .find(|(position, (number, _))| *number != *position as u32 + u32::from(!starts_at_zero))
    {
        return Err(format!(
            "DWARF .file {number} is out of order: the numbers must run \
             {} (a gap would renumber every file after it)",
            if starts_at_zero {
                "0, 1, 2, ...".to_string()
            } else {
                "1, 2, 3, ...".to_string()
            }
        ));
    }
    Ok(table)
}

/// Split a recorded path into `(directory, base name)`.
///
/// The directory is empty for a name that carries none, which is the recorded
/// form of "this file lives in the compilation directory"; `/a.c` has the root
/// as its directory and `a.c` as its base.
fn split_path(name: &str) -> (&str, &str) {
    match name.rfind('/') {
        Some(0) => ("/", &name[1..]),
        Some(index) => (&name[..index], &name[index + 1..]),
        None => ("", name),
    }
}

/// The directory and file tables of a line program, as its prologue needs them.
struct LineTables<'a> {
    /// Directories other than the compilation directory, in first-use order.
    /// Directory index `i` is `directories[i - 1]`: versions 2..4 number their
    /// own entries from 1, and a version 5 table repeats the compilation
    /// directory at entry 0.
    directories: Vec<&'a str>,
    /// `(directory index, base name)` per recorded file, in `.file` order.
    files: Vec<(u64, &'a str)>,
    /// Whether the first recorded file is `.file 0`, which GCC's version-5
    /// assembly declares (`.file 0 "dir" "unit.c"`): a version-5 table then
    /// starts at that entry instead of repeating it.
    primary_is_zero: bool,
}

/// Split every recorded file name into the directory index it resolves through
/// and the base name a debugger appends to that directory.
///
/// Directory index 0 is the compilation directory for every version: versions
/// 2..4 leave it implicit and number their own entries from 1, version 5 spells
/// it out as the first table entry.  A name whose directory *spells* the
/// compilation directory is folded into entry 0 -- the resolved path is
/// identical, and the table stays minimal.
fn line_tables<'a>(file_table: &[(u32, &'a str)], comp_dir: &str) -> LineTables<'a> {
    let mut directories: Vec<&str> = Vec::new();
    let mut files: Vec<(u64, &str)> = Vec::with_capacity(file_table.len());
    for (_, name) in file_table {
        let (directory, base) = split_path(name);
        let index = if directory.is_empty() || directory == comp_dir {
            0
        } else if let Some(index) = directories.iter().position(|entry| *entry == directory) {
            index as u64 + 1
        } else {
            directories.push(directory);
            directories.len() as u64
        };
        files.push((index, base));
    }
    LineTables {
        directories,
        files,
        primary_is_zero: file_table.first().is_some_and(|(number, _)| *number == 0),
    }
}

/// A version-5 file table is addressed by number: slot *i* holds `.file i`.
///
/// The file register a row sets and the `DW_AT_decl_file` a unit records are
/// indexes into this table, and GAS builds it as "slot = number" in every
/// dialect -- `as -g` on `.file 0 "d" "p.c"` / `.file 1 "a.c"` / `.loc 1 ...`
/// resolves the row to `a.c` and keeps `p.c` in slot 0.  A table that renumbers
/// the files one slot higher (a duplicate primary at 0 and the files after it)
/// stays self-consistent for the rows it writes itself, but it rewrites the
/// meaning of every `DW_AT_decl_file` an *input* unit already carries.  Measured
/// on `gcc -gdwarf-5 -S` assembly of a unit that includes a header --
/// `.file 0 "dir" "t.c"`, `.file 1 "h.h"`, `.file 2 "t.c"`, with the inlined
/// helper recorded as `DW_AT_decl_file 1`: the renumbered table resolved that to
/// `t.c`, GAS and this table to `h.h`.
///
/// Slot 0 is the primary file: `.file 0` itself when the input declares one
/// (GCC does), and otherwise a repeat of the first declared file -- GCC's own
/// tables repeat the unit's file there, and the default file register 1 has to
/// name something.
fn version_5_file_slots<'a>(tables: &LineTables<'a>) -> Vec<(u64, &'a str)> {
    let Some(&primary) = tables.files.first() else {
        return Vec::new();
    };
    if tables.primary_is_zero {
        // `.file 0` is the primary, so the list already starts at slot 0.
        return tables.files.clone();
    }
    // Slot 0 repeats the first file and the files follow it, so slot *i* is
    // `.file i` here too.
    let mut slots = Vec::with_capacity(tables.files.len() + 1);
    slots.push(primary);
    slots.extend(tables.files.iter().copied());
    slots
}

/// The fixed fields of a line-program prologue, up to but not including the
/// directory and file tables.
///
/// Two of them depend on the dialect.  DWARF 4 inserted
/// `maximum_operations_per_instruction` before `default_is_stmt`; writing one
/// into a version 2 or 3 prologue shifts every table that follows, and binutils
/// then reports a bogus line.  DWARF 3 added `DW_LNS_set_prologue_end`,
/// `_set_epilogue_begin` and `_set_isa`, which move `opcode_base` from 10 to 13
/// -- a version 2 consumer cannot decode the opcodes that number would promise.
fn write_line_prologue_header(unit: &mut Vec<u8>, config: DwarfConfig, opcode_base: u8) {
    unit.push(1); // minimum_instruction_length
    if config.version >= 4 {
        unit.push(1); // maximum_operations_per_instruction
    }
    unit.push(1); // default_is_stmt
    unit.push((-5i8) as u8); // line_base
    unit.push(14); // line_range
    unit.push(opcode_base);
    if config.version >= 3 {
        unit.extend_from_slice(&STANDARD_OPCODE_LENGTHS);
    } else {
        unit.extend_from_slice(&STANDARD_OPCODE_LENGTHS[..usize::from(opcode_base) - 1]);
    }
}

/// Append the directory and file tables of a prologue, and return the
/// relocations the version 5 path offsets need, with offsets relative to the
/// appended bytes.
///
/// Version 5 describes both tables with entry formats whose path columns are
/// `DW_FORM_line_strp` offsets into `line_strings`; versions 2..4 carry the names
/// inline and terminate each table with an empty name.  The caller rebases the
/// returned relocations, because a string offset travels as a relocation against
/// `.debug_line_str` and a relocation names the *section* offset.
fn append_line_prologue_tables(
    unit: &mut Vec<u8>,
    config: DwarfConfig,
    tables: &LineTables<'_>,
    comp_dir: &str,
    abs32: u32,
    line_strings: &mut DwarfStrings,
) -> Result<Vec<ObjReloc>, String> {
    let mut relocs = Vec::new();
    if config.version >= 5 {
        // Directory table: one column, `DW_LNCT_path` as a string offset. Entry
        // 0 is the compilation directory, so the table is one longer than the
        // list of extra directories.
        unit.push(1); // directory_entry_format_count
        write_uleb128(DW_LNCT_PATH, unit);
        write_uleb128(DW_FORM_LINE_STRP, unit);
        let count = u64::try_from(tables.directories.len() + 1)
            .map_err(|_| "DWARF directory table too large".to_string())?;
        write_uleb128(count, unit);
        for directory in std::iter::once(comp_dir).chain(tables.directories.iter().copied()) {
            let offset = line_strings.intern(directory)?;
            push_strp(unit, &mut relocs, 0, abs32, ".debug_line_str", offset);
        }
        // File name table: `DW_LNCT_directory_index` then `DW_LNCT_path`, one
        // entry per file with entry *i* being `.file i` (see
        // [`version_5_file_slots`] for why the numbering is the index).
        unit.push(2); // file_name_entry_format_count
        write_uleb128(DW_LNCT_DIRECTORY_INDEX, unit);
        write_uleb128(DW_FORM_UDATA, unit);
        write_uleb128(DW_LNCT_PATH, unit);
        write_uleb128(DW_FORM_LINE_STRP, unit);
        let slots = version_5_file_slots(tables);
        write_uleb128(slots.len() as u64, unit);
        for (index, base) in slots {
            write_uleb128(index, unit);
            let offset = line_strings.intern(base)?;
            push_strp(unit, &mut relocs, 0, abs32, ".debug_line_str", offset);
        }
    } else {
        // Versions 2..4 carry both tables inline, and their directory entries
        // are numbered from 1: entry `i` is `directories[i - 1]`, while
        // directory index 0 is the (unlisted) compilation directory.
        for directory in &tables.directories {
            unit.extend_from_slice(directory.as_bytes());
            unit.push(0);
        }
        unit.push(0); // empty directory name terminates the table
        for (index, base) in &tables.files {
            unit.extend_from_slice(base.as_bytes());
            unit.push(0);
            write_uleb128(*index, unit); // directory_index
            write_uleb128(0, unit); // modification_time
            write_uleb128(0, unit); // file_size
        }
        unit.push(0); // empty file name terminates the file table
    }
    Ok(relocs)
}

/// Build a `.debug_line` section from assembler source records.
///
/// `address_size` is the target ELF address width (4 or 8), and
/// `absolute_relocation` is the matching absolute relocation type; `abs32` is
/// the 32-bit absolute relocation that carries string-table offsets. Every
/// line program starts with a relocated `DW_LNE_set_address` to its code
/// section; its remaining rows use section-relative address advances, or,
/// given `symbol_at`, a relocated `DW_LNE_set_address` each.
///
/// Both flavors of prologue describe the same thing, so both are built from
/// one model: directory 0 is `comp_dir`, and every other directory a recorded
/// `.file` name mentions is one more entry in first-use order. Version 5
/// names the tables through `line_strings`, the `.debug_line_str` section, and
/// spells directory 0 out as the first entry; versions 2..4 carry the names
/// inline and leave directory 0 implicit, as their prologues require.
#[allow(clippy::too_many_arguments)]
/// Group the `.loc` records by the code section each one describes, with the
/// `is_stmt` and ISA state resolved.
///
/// GAS carries both of those across section switches: a `.loc` that omits them
/// keeps the value of the previous row, and the state is per *program*, not per
/// section.  Resolving it in source order here -- before the rows are split
/// into one line program per section, each of which starts from the DWARF
/// defaults -- keeps every program's rows self-consistent.
///
/// A record is dropped when it names a section this object has no executable
/// body for, when it points past the section's end, or when the line program's
/// file table cannot name its file: the first two are input that describes code
/// the object does not contain, and the third would need a file index that does
/// not exist.  A record naming a section that is not in the object at all is a
/// bug in the caller, not in the input.
fn group_rows_by_section<'a>(
    locations: &'a [DwarfLineLocation],
    sections: &[DwarfSectionInfo<'_>],
    file_index: &dyn Fn(u32) -> Option<u64>,
) -> Result<Vec<Vec<(&'a DwarfLineLocation, bool, u64)>>, String> {
    let mut rows_by_section: Vec<Vec<(&DwarfLineLocation, bool, u64)>> =
        (0..sections.len()).map(|_| Vec::new()).collect();
    let mut source_is_stmt = true;
    let mut source_isa = 0;
    for location in locations {
        if let Some(value) = location.options.is_stmt {
            source_is_stmt = value;
        }
        if let Some(value) = location.options.isa {
            source_isa = value;
        }
        let Some(section) = sections.get(location.section_index) else {
            return Err(format!(
                "internal: DWARF line record refers to missing section {}",
                location.section_index
            ));
        };
        if !section.is_executable || location.offset > section.size {
            continue;
        }
        if file_index(location.file_number).is_some() {
            rows_by_section[location.section_index].push((location, source_is_stmt, source_isa));
        }
    }
    Ok(rows_by_section)
}

pub(crate) fn build_dwarf_line_section(
    files: &[(u32, String)],
    locations: &[DwarfLineLocation],
    sections: &[DwarfSectionInfo<'_>],
    address_size: usize,
    absolute_relocation: u32,
    abs32: u32,
    comp_dir: &str,
    config: DwarfConfig,
    line_strings: &mut DwarfStrings,
    mut symbol_at: SymbolAt<'_, '_>,
) -> Result<Option<DwarfLineBuild>, String> {
    if !matches!(address_size, 4 | 8) {
        return Err(format!(
            "DWARF line table address size must be 4 or 8, got {address_size}"
        ));
    }
    if files.is_empty() {
        return Ok(None);
    }
    // A unit with no rows still gets a program *header*: the file table it
    // carries is what every `DW_AT_decl_file` in the unit indexes, so a data-only
    // translation unit needs it even though it has no address to name.  GCC 16.2
    // emits exactly that shape (measured: a 50-byte version 5 header, two file
    // entries and "No Line Number Statements"), and it is what makes
    // `DW_AT_stmt_list` a valid, if row-free, program.

    let file_table = debug_file_table(files)?;
    // Version 5 addresses its file table by number: slot 0 is the primary file,
    // slot *i* is `.file i`.  Versions 2..4 number their entries from 1, so the
    // register is the entry's position in that table -- which the contiguity
    // rule in [`debug_file_table`] makes equal to the number as well.
    let five = config.version >= 5;
    let file_index = |number: u32| -> Option<u64> {
        if five && number == 0 {
            return Some(0); // the primary file, always slot 0
        }
        let position = file_table
            .iter()
            .position(|(candidate, _)| *candidate == number)?;
        Some(if five {
            u64::from(number)
        } else {
            position as u64 + 1
        })
    };

    let mut rows_by_section = group_rows_by_section(locations, sections, &file_index)?;

    let tables = line_tables(&file_table, comp_dir);

    // One compilation unit owns exactly one line program -- its header carries
    // one directory table and one file table -- so the sections this object's
    // code lives in become one *sequence* each inside that single program,
    // which is what GAS and GCC emit (measured: `as` 2.47 and `gcc` 16.2 write
    // one unit with three sequences for three code sections, while a unit per
    // section would repeat the prologue and the tables for every one of them).
    //
    // `DW_LNE_end_sequence` resets every row register to its initial value
    // (DWARF 5 §6.2.5.3), so the encoder's state starts from the defaults at
    // each sequence -- the same rule a consumer applies when it decodes them.
    let opcode_base = if config.version >= 3 {
        OPCODE_BASE
    } else {
        OPCODE_BASE_DWARF2 as u8
    };
    let mut unit = Vec::new();
    let mut relocs = Vec::new();
    let mut code_sections = Vec::new();
    unit.extend_from_slice(&[0; 4]); // unit_length, patched below
    unit.extend_from_slice(&(config.version as u16).to_le_bytes());
    if config.version >= 5 {
        // DWARF 5's line-program header names its own address size and
        // segment selector size; there is no unit type here (unlike
        // `.debug_info`).
        unit.push(address_size as u8);
        unit.push(0); // segment_selector_size
    }
    let header_length_at = unit.len();
    unit.extend_from_slice(&[0; 4]); // header_length, patched below
    let header_start = unit.len();

    write_line_prologue_header(&mut unit, config, opcode_base);
    // The version 5 tables name their paths through relocations written against
    // the unit's own start, and the unit *is* the section: offset 0, so the
    // offsets need no rebasing.
    relocs.extend(append_line_prologue_tables(
        &mut unit,
        config,
        &tables,
        comp_dir,
        abs32,
        line_strings,
    )?);
    let header_length = unit.len() - header_start;
    let header_length = u32::try_from(header_length)
        .map_err(|_| "DWARF line-table header exceeds 32-bit length".to_string())?;
    unit[header_length_at..header_length_at + 4].copy_from_slice(&header_length.to_le_bytes());

    for (section_index, rows) in rows_by_section.iter_mut().enumerate() {
        if rows.is_empty() {
            continue;
        }
        code_sections.push(section_index);
        rows.sort_by_key(|(row, _, _)| (row.offset, row.sequence));
        let code_section = &sections[section_index];
        // A sequence starts by naming its first address; the linker resolves
        // the relocation against the code section's symbol and folds in the row
        // offset.
        let first_offset = rows[0].0.offset;
        let (symbol_name, addend) = match symbol_at.as_deref_mut() {
            Some(symbol_at) => (symbol_at(section_index, first_offset), 0),
            None => (
                code_section.name.to_string(),
                i64::try_from(first_offset).map_err(|_| {
                    "DWARF line-table code offset exceeds signed addend range".to_string()
                })?,
            ),
        };
        push_set_address(
            &mut unit,
            &mut relocs,
            0,
            address_size,
            absolute_relocation,
            symbol_name,
            addend,
        );

        let mut address = first_offset;
        let mut current_file = 1u64;
        let mut current_line = 1i64;
        let mut current_column = 0u32;
        let mut current_is_stmt = true;
        let mut current_isa = 0u64;
        for &(row, is_stmt, isa) in rows.iter() {
            let next_file = file_index(row.file_number).unwrap_or(1);
            if row.offset < address {
                return Err("internal: DWARF line rows are not address ordered".to_string());
            }
            let advance = row.offset - address;
            let next_line = row.line as i64;
            let line_delta = next_line - current_line;
            // Relocatable per-row addresses (a relaxing target) cannot ride in a
            // special opcode: each row names its own symbol, so it keeps a
            // `DW_LNE_set_address`.
            let motion = match symbol_at.as_deref_mut() {
                // A relaxing target names every row's address with a label: the
                // linker deletes bytes between rows, so a distance would move.
                // A row sitting at the address already named -- the sequence
                // start, or the previous row -- needs no second relocation.
                Some(symbol_at) => {
                    if advance != 0 {
                        let symbol_name = symbol_at(section_index, row.offset);
                        push_set_address(
                            &mut unit,
                            &mut relocs,
                            0,
                            address_size,
                            absolute_relocation,
                            symbol_name,
                            0,
                        );
                    }
                    if line_delta != 0 {
                        unit.push(3); // DW_LNS_advance_line
                        write_sleb128(line_delta, &mut unit);
                    }
                    None
                }
                None => {
                    let motion = RowMotion::plan(advance, line_delta, u64::from(opcode_base));
                    if let Some(delta) = motion.line_advance {
                        unit.push(3); // DW_LNS_advance_line
                        write_sleb128(delta, &mut unit);
                    }
                    motion.address.push(&mut unit);
                    Some(motion)
                }
            };
            address = row.offset;
            if next_file != current_file {
                unit.push(4); // DW_LNS_set_file
                write_uleb128(next_file, &mut unit);
                current_file = next_file;
            }
            if row.column != current_column {
                unit.push(5); // DW_LNS_set_column
                write_uleb128(row.column as u64, &mut unit);
                current_column = row.column;
            }
            current_line = next_line;
            if is_stmt != current_is_stmt {
                unit.push(6); // DW_LNS_negate_stmt
                current_is_stmt = is_stmt;
            }
            // `set_isa`, `set_prologue_end` and `set_epilogue_begin` are
            // version 3 opcodes, and `set_discriminator` is a version 4
            // extended opcode: a unit of an older version cannot express them,
            // so the state is dropped rather than written in a vocabulary its
            // consumer does not have.  GAS writes the opcode into a version 2
            // or 3 unit anyway (measured on 2.47: `-gdwarf-2` assembly with
            // `.loc ... discriminator 1` comes back with a `DW_LNE_`
            // discriminator record), which leaves that unit describing an
            // opcode its own dialect never defined; a consumer that trusts the
            // header cannot decode it.  Dropping the state keeps the program
            // readable by the consumer the header announces.
            if isa != current_isa && config.version >= 3 {
                unit.push(12); // DW_LNS_set_isa
                write_uleb128(isa, &mut unit);
                current_isa = isa;
            }
            if row.options.basic_block {
                unit.push(7);
            }
            if row.options.prologue_end && config.version >= 3 {
                unit.push(10);
            }
            if row.options.epilogue_begin && config.version >= 3 {
                unit.push(11);
            }
            if let Some(discriminator) = row.options.discriminator
                && config.version >= 4
            {
                // ULEB128 needs at least one byte, even for zero. Write it
                // directly into the unit without a per-row allocation.
                let bits = (u64::BITS - discriminator.leading_zeros()).max(1);
                let operand_len = bits.div_ceil(7);
                unit.push(0); // DW_LNE_extended_op
                write_uleb128(1 + u64::from(operand_len), &mut unit);
                unit.push(4); // DW_LNE_set_discriminator
                write_uleb128(discriminator, &mut unit);
            }
            // A special opcode emits the row; otherwise copy does so.
            unit.push(motion.map_or(1, |motion| motion.row));
        }

        let end_offset = code_section.size;
        if end_offset < address {
            return Err(format!(
                "internal: DWARF line row at {address:#x} is past section '{}' end {end_offset:#x}",
                code_section.name
            ));
        }
        if end_offset != address {
            if let Some(symbol_at) = symbol_at.as_deref_mut() {
                let symbol_name = symbol_at(section_index, end_offset);
                push_set_address(
                    &mut unit,
                    &mut relocs,
                    0,
                    address_size,
                    absolute_relocation,
                    symbol_name,
                    0,
                );
            } else {
                unit.push(2); // DW_LNS_advance_pc
                write_uleb128(end_offset - address, &mut unit);
            }
        }
        unit.extend_from_slice(&[0, 1, 1]); // DW_LNE_end_sequence
    }

    // A program with no sequence is still a program: its header carries the
    // directory and file tables, which is what a unit with no code needs in order
    // to name the file its objects were declared in.  GCC 16.2 writes exactly
    // that for a translation unit that is only data (measured: a 50-byte version
    // 5 header and no line number statements).
    let unit_length = u32::try_from(unit.len() - 4)
        .map_err(|_| "DWARF line-table unit exceeds 32-bit length".to_string())?;
    unit[..4].copy_from_slice(&unit_length.to_le_bytes());
    Ok(Some(DwarfLineBuild {
        line: debug_section(".debug_line", unit, relocs),
        code_sections,
    }))
}

// ---------------------------------------------------------------------------
// Compilation units
// ---------------------------------------------------------------------------

/// The `.debug_info`, `.debug_abbrev` and `.debug_aranges` sections of a
/// synthesized object.  `aranges` is `None` when the target's addresses cannot
/// be stated exactly (see [`build_debug_aranges`]).
pub(crate) struct DwarfInfoBuild {
    pub info: ObjSection,
    pub abbrev: ObjSection,
    pub aranges: Option<ObjSection>,
    /// `.debug_ranges`/`.debug_rnglists`, present only when the unit's code is
    /// not one contiguous section (see [`build_debug_ranges`]).
    pub ranges: Option<ObjSection>,
}

/// Every attribute form one object's units use, decided once from the dialect.
///
/// The forms follow the dialect, not the writer's convenience, and each one is
/// the form GCC 16.2 was measured to emit for the same field:
///
/// * from version 5 the unit's name and compilation directory are
///   `DW_FORM_line_strp` offsets into `.debug_line_str`, which already holds both
///   -- the compilation directory as directory entry 0 and the unit's file as
///   file entry 1 -- so the directory is stored once;
/// * the producer and every function name have no home in the line table and are
///   `DW_FORM_strp` offsets into `.debug_str` from version 5, and inline
///   `DW_FORM_string` before it, which is the smallest encoding for a unit that
///   has no other string section;
/// * `DW_AT_high_pc` is an address through version 3 and a `DW_FORM_udata`
///   constant offset from `low_pc` from version 4 on.  A version 2 or 3 consumer
///   reads a constant as an address, which after linking names unrelated code,
///   so those versions keep the relocated address; from version 4 the offset is
///   a ULEB -- one byte for a function under 128 bytes, seven fewer than the
///   `data8` this writer used to spend on every subprogram of every object;
/// * `DW_FORM_sec_offset` and `DW_FORM_flag_present` arrived with version 4: the
///   same four bytes are a `DW_FORM_data4` before it, and the only flag form a
///   version 2 or 3 consumer has is `DW_FORM_flag`, whose value byte says whether
///   a function is external;
/// * `DW_AT_language` takes the smallest constant form its code fits, which is
///   what GCC does: every C code is one byte, and the assembler code GCC also
///   emits (`DW_LANG_MIPS_ASSEMBLER`, 0x8001) needs two.
#[derive(Clone, Copy)]
struct UnitForms {
    /// `DW_AT_name` and `DW_AT_comp_dir`.
    unit_path: u64,
    /// `DW_AT_producer` and the `DW_AT_name` of every subprogram.
    string: u64,
    high_pc: u64,
    stmt_list: u64,
    external: u64,
    language: u64,
    /// `DW_AT_ranges`: a section offset from version 4 on, the same four bytes
    /// as a constant before it (the attribute is a GNU extension in DWARF 2 and
    /// a standard one from DWARF 3, where its form is `DW_FORM_data4`).
    ranges: u64,
}

impl UnitForms {
    /// `labels` is true when code addresses are symbols rather than section
    /// offsets (a relaxing target), which forces a relocated `high_pc` address.
    fn new(config: DwarfConfig, labels: bool) -> Self {
        let five = config.version >= 5;
        let four = config.version >= 4;
        Self {
            unit_path: if five {
                DW_FORM_LINE_STRP
            } else {
                DW_FORM_STRING
            },
            string: if five { DW_FORM_STRP } else { DW_FORM_STRING },
            // `DW_AT_high_pc` is an address in DWARF 2 and 3; from 4 on a
            // constant form means "offset from `low_pc`".  A ULEB is the
            // shortest form that holds a function's size (one byte below 128
            // against `DW_FORM_data8`'s eight) and is what GNU as 2.40 emits on
            // its own -- `DW_FORM_data8` here was costing seven bytes per
            // subprogram on every optimised object.
            high_pc: if labels || !four {
                DW_FORM_ADDR
            } else {
                DW_FORM_UDATA
            },
            stmt_list: if four {
                DW_FORM_SEC_OFFSET
            } else {
                DW_FORM_DATA4
            },
            external: if four {
                DW_FORM_FLAG_PRESENT
            } else {
                DW_FORM_FLAG
            },
            language: if u8::try_from(config.language).is_ok() {
                DW_FORM_DATA1
            } else {
                DW_FORM_DATA2
            },
            ranges: if four {
                DW_FORM_SEC_OFFSET
            } else {
                DW_FORM_DATA4
            },
        }
    }
}

/// The abbreviation codes one unit's DIEs use, by shape.
///
/// Named rather than returned as a tuple of arrays because four of the six are
/// indexed the same way and a caller that mixes two of them would still compile.
struct UnitAbbrevs {
    /// The compile-unit DIE.
    cu_code: u64,
    /// A subprogram without children, `[has no declaration][is external]`.
    subprogram: [[u64; 2]; 2],
    /// The same subprogram shapes with a child list: the one a defined function
    /// with parameters needs, so a debugger can list the arguments it takes.
    subprogram_with_children: [[u64; 2]; 2],
    /// `DW_TAG_formal_parameter` with a name, and without one (a prototype).
    formal_parameter_named: Option<u64>,
    formal_parameter_unnamed: Option<u64>,
    /// The same two shapes with a `DW_AT_location`: a parameter the frame layout
    /// proved a home for.  Two extra codes rather than an attribute on every
    /// parameter, because the compressor of a table is its shape count: a
    /// parameter without a home (a prototype's, or one the layout could not
    /// place) must not pay for an attribute it cannot fill.
    ///
    /// `None` in a unit that places *no* parameter at all -- every function in a
    /// `-O2` unit whose parameters live in registers, or any unit built without a
    /// frame -- because an abbreviation no DIE uses is bytes in every object and
    /// a parse for every consumer.
    formal_parameter_named_located: Option<u64>,
    formal_parameter_unnamed_located: Option<u64>,
    /// `DW_TAG_unspecified_parameters`: the list continues with `...`.
    unspecified_parameters: u64,
    next_code: u64,
}

/// The parameter shapes a unit's DIEs take.
///
/// Read from the unit before the table is written, so the table declares the
/// shapes that will be used and no others: at `-O0` a named parameter always has
/// a home, which leaves the unlocated shape dead weight, while in a unit whose
/// parameters all live in registers the located one is.  The unnamed axis is
/// independent: only a prototype (or a C23 definition that names no parameter)
/// takes an unnamed shape at all.
#[derive(Default, Clone, Copy)]
struct ParameterShapes {
    named_unlocated: bool,
    unnamed_unlocated: bool,
    named_located: bool,
    unnamed_located: bool,
}

/// Declare every abbreviation of a unit and return the codes its DIEs use.
///
/// One subprogram abbreviation per shape, numbered from 3.  `DW_FORM_flag_present`
/// says "this function is external" by its presence, so from version 4 on a
/// static function omits the attribute and needs its own abbreviation; versions 2
/// and 3 carry the flag's value and serve both shapes with one.  A shape is
/// declared when the unit has a DIE that uses it, which for the parameter shapes
/// is decided by [`ParameterShapes`]: a unit that places no parameter declares no
/// located shape, and one that places all of them declares no unlocated one.
fn declare_unit_abbrevs(
    abbrev: &mut Vec<u8>,
    config: DwarfConfig,
    forms: UnitForms,
    range_list: bool,
    declarator_rows: bool,
    has_code: bool,
    has_stmt_list: bool,
    parameters: ParameterShapes,
) -> UnitAbbrevs {
    let mut cu_attrs = vec![
        (DW_AT_NAME, forms.unit_path),
        (DW_AT_COMP_DIR, forms.unit_path),
        (DW_AT_PRODUCER, forms.string),
        (DW_AT_LANGUAGE, forms.language),
    ];
    if config.language_name.is_some() {
        cu_attrs.push((DW_AT_LANGUAGE_NAME, DW_FORM_DATA1));
    }
    if config.language_version.is_some() {
        cu_attrs.push((DW_AT_LANGUAGE_VERSION, DW_FORM_DATA4));
    }
    if has_code {
        if range_list {
            // Code in several sections cannot be described by one low/high pair,
            // so the unit names a range list instead; the pair form would have to
            // span the gaps between sections, claiming code that belongs to
            // another unit -- and, once the linker places `.text.foo` and
            // `.text.bar` apart, to unrelated code entirely.
            cu_attrs.push((DW_AT_RANGES, forms.ranges));
            cu_attrs.push((DW_AT_LOW_PC, DW_FORM_ADDR));
        } else {
            cu_attrs
                .extend_from_slice(&[(DW_AT_LOW_PC, DW_FORM_ADDR), (DW_AT_HIGH_PC, forms.high_pc)]);
        }
    }
    // The line program is where `DW_AT_decl_file` finds its file table, so a unit
    // names it whenever it has one -- a data-only unit included.  GCC 16.2 does
    // the same (measured: a header-only program from a translation unit that is
    // only data).
    if has_stmt_list {
        cu_attrs.push((DW_AT_STMT_LIST, forms.stmt_list));
    }
    // A unit with no code states neither an address nor a line program: there
    // is no address to name and no row to point at.  GCC 16.2 does the same for
    // a translation unit that is only data (measured), and a consumer then finds
    // the unit by walking `.debug_info` rather than through `.debug_aranges`.
    declare_abbrev(abbrev, 1, DW_TAG_COMPILE_UNIT, true, &cu_attrs);
    // The unspecified return type GAS uses, so a debugger never assumes `int`.
    // Only a unit with code has subprograms to attach it to.
    if has_code {
        declare_abbrev(abbrev, 2, DW_TAG_UNSPECIFIED_TYPE, false, &[]);
    }
    let mut subprogram_code = [[3u64; 2]; 2];
    let mut subprogram_with_children = [[3u64; 2]; 2];
    let mut next_code = 3;
    for has_declaration in [true, false] {
        let mut attrs = vec![(DW_AT_NAME, forms.string)];
        if config.version < 4 {
            attrs.push((DW_AT_EXTERNAL, forms.external));
        }
        if has_declaration {
            // `DW_FORM_udata` is never wider than the `DW_FORM_data1` GCC uses
            // and never truncates either: it is the shortest form that holds any
            // file index or line number.
            attrs.push((DW_AT_DECL_FILE, DW_FORM_UDATA));
            attrs.push((DW_AT_DECL_LINE, DW_FORM_UDATA));
            if declarator_rows {
                // Only where a declarator row can supply it: `udata` 0 is the
                // standard spelling of "column unknown", and units assembled
                // from `.loc` rows alone have nothing better to say.
                attrs.push((DW_AT_DECL_COLUMN, DW_FORM_UDATA));
            }
        }
        attrs.extend_from_slice(&[
            // `DW_FORM_ref4`, not `ref_udata`: the compiler's type table is laid
            // out after the subprograms, so a variable-width reference could not
            // be written before its target's offset was known.
            (DW_AT_TYPE, DW_FORM_REF4),
            (DW_AT_LOW_PC, DW_FORM_ADDR),
            (DW_AT_HIGH_PC, forms.high_pc),
        ]);
        if config.version >= 3 {
            // Every subprogram this writer describes has code (the walk below
            // skips functions whose size is zero), and the assembler gives all
            // of it an FDE, so the CFA expression is always evaluable.
            attrs.push((DW_AT_FRAME_BASE, DW_FORM_EXPRLOC));
        }
        // Two declarations of every shape: one without children (a function with
        // no parameter to list) and one with, which the caller terminates with a
        // zero code after the parameter DIEs.  A subprogram that declares
        // children and has none would be a different byte stream for the same
        // meaning, so the caller picks by the parameter list it has.
        for children in [false, true] {
            let slot = if children {
                &mut subprogram_with_children[usize::from(!has_declaration)]
            } else {
                &mut subprogram_code[usize::from(!has_declaration)]
            };
            if config.version >= 4 {
                let mut external_atts = attrs.clone();
                external_atts.insert(1, (DW_AT_EXTERNAL, forms.external));
                declare_abbrev(
                    abbrev,
                    next_code,
                    DW_TAG_SUBPROGRAM,
                    children,
                    &external_atts,
                );
                slot[1] = next_code;
                next_code += 1;
                declare_abbrev(abbrev, next_code, DW_TAG_SUBPROGRAM, children, &attrs);
                slot[0] = next_code;
                next_code += 1;
            } else {
                declare_abbrev(abbrev, next_code, DW_TAG_SUBPROGRAM, children, &attrs);
                *slot = [next_code, next_code];
                next_code += 1;
            }
        }
    }
    // The parameter shapes.  A named parameter states where the user wrote it and
    // what type it has; an unnamed one (a prototype's `void f(int)`) states only
    // the type, which is the difference `DW_AT_name`'s presence encodes.
    let formal_parameter_named = if parameters.named_unlocated {
        declare_abbrev(
            abbrev,
            next_code,
            DW_TAG_FORMAL_PARAMETER,
            false,
            &[(DW_AT_NAME, forms.string), (DW_AT_TYPE, DW_FORM_REF4)],
        );
        next_code += 1;
        Some(next_code - 1)
    } else {
        None
    };
    let formal_parameter_unnamed = if parameters.unnamed_unlocated {
        declare_abbrev(
            abbrev,
            next_code,
            DW_TAG_FORMAL_PARAMETER,
            false,
            &[(DW_AT_TYPE, DW_FORM_REF4)],
        );
        next_code += 1;
        Some(next_code - 1)
    } else {
        None
    };
    let formal_parameter_named_located = if parameters.named_located {
        declare_abbrev(
            abbrev,
            next_code,
            DW_TAG_FORMAL_PARAMETER,
            false,
            &[
                (DW_AT_NAME, forms.string),
                (DW_AT_TYPE, DW_FORM_REF4),
                (DW_AT_LOCATION, DW_FORM_BLOCK1),
            ],
        );
        next_code += 1;
        Some(next_code - 1)
    } else {
        None
    };
    let formal_parameter_unnamed_located = if parameters.unnamed_located {
        declare_abbrev(
            abbrev,
            next_code,
            DW_TAG_FORMAL_PARAMETER,
            false,
            &[(DW_AT_TYPE, DW_FORM_REF4), (DW_AT_LOCATION, DW_FORM_BLOCK1)],
        );
        next_code += 1;
        Some(next_code - 1)
    } else {
        None
    };
    declare_abbrev(abbrev, next_code, DW_TAG_UNSPECIFIED_PARAMETERS, false, &[]);
    let unspecified_parameters = next_code;
    next_code += 1;
    // The end-of-table marker is written by the caller, after the compiler's own
    // DIE tables have added their abbreviations: a terminator here would hide
    // every one of them from a consumer.
    UnitAbbrevs {
        cu_code: 1,
        subprogram: subprogram_code,
        subprogram_with_children,
        formal_parameter_named,
        formal_parameter_unnamed,
        formal_parameter_named_located,
        formal_parameter_unnamed_located,
        unspecified_parameters,
        next_code,
    }
}

/// The code sections a unit describes, in section order.
///
/// That is every section its line program has a sequence for, plus every
/// section that holds a sized function symbol without a row: a function that
/// optimization folded to nothing still has an address a debugger may search
/// for, and a range list has to cover every address the unit's DIEs name.
/// A section neither list names is not this unit's code.  One pass over the
/// section table and one over the symbols; a name that does not resolve is a
/// writer bug, not input, because both lists are built from this same table.
///
/// Returns the coverage as a bitmap indexed by section (the subprogram walk
/// asks it again) and the covered sections in section order (the unit's address
/// map walks those).
fn covered_code_sections<'a, 's>(
    sections: &'s [DwarfSectionInfo<'a>],
    code_sections: &[usize],
    functions: &[DwarfFunction<'a>],
) -> Result<(Vec<bool>, Vec<&'s DwarfSectionInfo<'a>>), String> {
    let mut covered = vec![false; sections.len()];
    for &index in code_sections {
        let slot = covered
            .get_mut(index)
            .ok_or_else(|| format!("internal: DWARF line build names missing section {index}"))?;
        *slot = true;
    }
    for function in functions {
        if function.size == 0 {
            continue;
        }
        let slot = covered.get_mut(function.section_index).ok_or_else(|| {
            format!(
                "internal: DWARF function '{}' names missing section {}",
                function.name, function.section_index
            )
        })?;
        *slot = true;
    }
    let listed: Vec<&DwarfSectionInfo<'a>> = covered
        .iter()
        .enumerate()
        .filter(|&(_, inside)| *inside)
        .map(|(index, _)| &sections[index])
        .collect();
    Ok((covered, listed))
}

/// Build `.debug_info` and `.debug_abbrev` for the line programs `units`.
///
/// Each unit gets one `DW_TAG_compile_unit` spanning its code section, with a
/// `DW_TAG_subprogram` per sized function symbol in that section (the
/// unspecified return type GAS uses, so a debugger never assumes `int`).  A
/// function whose address range contains a `.loc` row also records the file and
/// line it starts on (`DW_AT_decl_file`/`decl_line`), which is what lets a
/// debugger list the function and report a backtrace frame's location without
/// walking the line program.  The language comes from `config` (see
/// [`DwarfConfig`]), never from the file name: `-std=` decides it, and a `.c`
/// file compiled as C23 must not claim C99 on DWARF 5.
///
/// `abs32` relocates the 32-bit section offsets (`DW_AT_stmt_list`, the
/// abbreviation offset) and `abs_addr` the `address_size`-byte code addresses,
/// so every value stays correct after the linker concatenates objects.  Given
/// `symbol_at`, code addresses are relocated against labels and
/// `DW_AT_high_pc` is an address (see the module documentation).  Version 5
/// hangs the unit's own name and compilation directory on `line_strings`
/// (`.debug_line_str`) and every function name on `strings` (`.debug_str`);
/// versions 2..4 write all of them inline, exactly as their consumers expect.
///
/// The forms follow the dialect, not the writer's convenience, and each one is
/// the form GCC 16.2 was measured to emit for the same dialect:
/// `DW_AT_stmt_list` is a `DW_FORM_sec_offset` only from version 4 on
/// (`DW_FORM_data4` before), `DW_AT_high_pc` is an address through version 3
/// and a `DW_FORM_udata` `low_pc`-relative constant from version 4 on,
/// `DW_AT_external` becomes a zero-byte `DW_FORM_flag_present` from version 4
/// on (which is why the abbreviation set has one shape per externality), and
/// `DW_AT_language` takes the smallest constant form that holds its code.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_dwarf_info_sections(
    code_sections: &[usize],
    stmt_list: Option<u64>,
    files: &[(u32, String)],
    locations: &[DwarfLineLocation],
    sections: &[DwarfSectionInfo<'_>],
    functions: &[DwarfFunction<'_>],
    primary_file: &str,
    comp_dir: &str,
    producer: &str,
    address_size: usize,
    abs32: u32,
    abs_addr: u32,
    config: DwarfConfig,
    strings: &mut DwarfStrings,
    line_strings: &mut DwarfStrings,
    unit: &DwarfUnitSpec,
    // Objects this very object file defines: a variable the unit declares but
    // does not define has no address here to point at.
    defined_objects: &FxHashMap<String, ()>,
    mut symbol_at: SymbolAt<'_, '_>,
) -> Result<Option<DwarfInfoBuild>, String> {
    // A unit with no code is still a unit: a translation unit that holds only
    // data names its objects here, and a debugger that cannot find them reports
    // "no debugging information" for a program that has some.  Only a unit with
    // neither code nor objects has nothing to describe.
    if code_sections.is_empty() && unit.variables.is_empty() && unit.types.is_empty() {
        return Ok(None);
    }
    if !matches!(address_size, 4 | 8) {
        return Err(format!(
            "DWARF address size must be 4 or 8, got {address_size}"
        ));
    }
    for text in [primary_file, comp_dir, producer] {
        if text.as_bytes().contains(&0) {
            return Err(format!("DWARF string contains a NUL byte: {text:?}"));
        }
    }

    let labels = symbol_at.is_some();
    let forms = UnitForms::new(config, labels);

    // Declarations come from the first row inside each function's range. The
    // line program already had to sort these, but it consumes them per section
    // and in address order of its own, so keep a per-section copy here.
    let file_table = debug_file_table(files)?;
    let source_rows = SourceRows::new(locations, sections.len());
    let has_declarator_rows = source_rows.has_declarator_rows;
    let declaration = |section_index: usize, offset: u64, size: u64| {
        source_rows.declaration(&file_table, config.version, section_index, offset, size)
    };

    let (covered, code) = covered_code_sections(sections, code_sections, functions)?;
    let has_code = !code.is_empty();
    // One range covers the unit exactly when its code is one contiguous
    // section; anything else needs a range list.
    let range_list = code.len() > 1;

    // Which parameter shapes this unit's DIEs take, so the table declares those
    // and no others.
    let mut parameters = ParameterShapes::default();
    for function in unit.parameters.values() {
        for parameter in &function.parameters {
            match (parameter.name.is_some(), parameter.location.is_some()) {
                (true, true) => parameters.named_located = true,
                (false, true) => parameters.unnamed_located = true,
                (true, false) => parameters.named_unlocated = true,
                (false, false) => parameters.unnamed_unlocated = true,
            }
        }
    }
    let mut abbrev = Vec::new();
    let abbrevs = declare_unit_abbrevs(
        &mut abbrev,
        config,
        forms,
        range_list,
        has_declarator_rows,
        has_code,
        stmt_list.is_some(),
        parameters,
    );
    let cu_code = abbrevs.cu_code;
    // The abbreviation table grows as the compiler's DIE tables are written: a
    // record with five members shares one member abbreviation, and a repeated
    // shape is declared exactly once.
    let mut dies = DieWriter::new(abbrev, abbrevs.next_code, unit.types.len());

    let mut info = Vec::new();
    let mut relocs = Vec::new();
    let length32 = |value: u64, what: &str| {
        u32::try_from(value).map_err(|_| format!("DWARF {what} exceeds 32 bits"))
    };
    let start = info.len();
    info.extend_from_slice(&[0; 4]); // unit_length, patched below
    info.extend_from_slice(&(config.version as u16).to_le_bytes());
    if config.version >= 5 {
        info.push(DW_UT_COMPILE);
        info.push(address_size as u8);
        push_reloc(&mut info, &mut relocs, abs32, 4, ".debug_abbrev", 0)?;
    } else {
        push_reloc(&mut info, &mut relocs, abs32, 4, ".debug_abbrev", 0)?;
        info.push(address_size as u8);
    }

    write_uleb128(cu_code, &mut info);
    // Unit name, compilation directory and producer, in abbreviation order.
    for (text, form) in [
        (primary_file, forms.unit_path),
        (comp_dir, forms.unit_path),
        (producer, forms.string),
    ] {
        write_string_attribute(
            text,
            form,
            &mut info,
            &mut relocs,
            abs32,
            strings,
            line_strings,
        )?;
    }
    if forms.language == DW_FORM_DATA1 {
        info.push(u8::try_from(config.language).map_err(|_| {
            format!(
                "DWARF language code {} does not fit in a byte",
                config.language
            )
        })?);
    } else {
        info.extend_from_slice(&config.language.to_le_bytes());
    }
    if let Some(name) = config.language_name {
        info.push(u8::try_from(name).map_err(|_| "DW_AT_language_name too large")?);
    }
    if let Some(version) = config.language_version {
        info.extend_from_slice(&version.to_le_bytes());
    }
    if let Some(first) = code.first() {
        if range_list {
            // The offset of the first range list.  A version 5 table starts with
            // its twelve-byte header, so its first list -- and therefore this
            // attribute -- is 12; `.debug_ranges` has no header and starts at 0.
            let (section, addend) = if config.version >= 5 {
                (".debug_rnglists", 12)
            } else {
                (".debug_ranges", 0)
            };
            push_reloc(&mut info, &mut relocs, abs32, 4, section, addend)?;
        }
        // The unit's base address: the start of its first range.  With a range
        // list present this is the base a consumer may offset ranges against
        // (DWARF 5 §2.17.3); without one it is simply the start of the code.
        push_reloc(
            &mut info,
            &mut relocs,
            abs_addr,
            address_size,
            first.name,
            0,
        )?;
        if !range_list {
            if forms.high_pc == DW_FORM_ADDR {
                if let Some(symbol_at) = symbol_at.as_deref_mut() {
                    let end = symbol_at(code_sections[0], first.size);
                    push_reloc(&mut info, &mut relocs, abs_addr, address_size, &end, 0)?;
                } else {
                    push_reloc(
                        &mut info,
                        &mut relocs,
                        abs_addr,
                        address_size,
                        first.name,
                        first.size,
                    )?;
                }
            } else {
                push_high_pc(&mut info, forms.high_pc, first.size);
            }
        }
    }
    // The object has one line program; `stmt_list` is where it starts, which is
    // zero unless the input had already begun a `.debug_line` section of its own
    // (`gcc -S -g` leaves an empty one for the assembler to fill).  A unit with no
    // code still names it: the program's file table is what `DW_AT_decl_file`
    // indexes.
    if let Some(stmt_list) = stmt_list {
        push_reloc(&mut info, &mut relocs, abs32, 4, ".debug_line", stmt_list)?;
    }

    let void_die = length32((info.len() - start) as u64, "unit")?;
    if has_code {
        write_uleb128(2, &mut info);
        emit_subprograms(
            SubprogramSink {
                info: &mut info,
                relocs: &mut relocs,
                dies: &mut dies,
                strings: &mut *strings,
                line_strings: &mut *line_strings,
                symbol_at,
                config,
                forms,
                abs32,
                abs_addr,
                address_size,
                has_declarator_rows,
            },
            functions,
            sections,
            &covered,
            unit,
            &abbrevs,
            &declaration,
            void_die,
        )?;
    }
    if !unit.types.is_empty() || !unit.variables.is_empty() {
        let mut ctx = DieContext {
            info: &mut info,
            relocs: &mut relocs,
            strings,
            line_strings,
            config,
            forms,
            abs32,
            abs_addr,
            address_size,
            unit_start: start,
            writer: &mut dies,
        };
        emit_types(&mut ctx, unit)?;
        emit_variables(&mut ctx, &unit.variables, files, defined_objects)?;
        ctx.patch_type_refs()?;
    }
    dies.abbrev.push(0); // end of the abbreviation table
    info.push(0); // end of the unit's children

    let unit_length = length32((info.len() - start - 4) as u64, "unit")?;
    info[start..start + 4].copy_from_slice(&unit_length.to_le_bytes());
    Ok(Some(DwarfInfoBuild {
        info: debug_section(".debug_info", info, relocs),
        abbrev: debug_section(".debug_abbrev", dies.abbrev, Vec::new()),
        aranges: build_debug_aranges(start as u64, &code, address_size, abs32, abs_addr, labels)?,
        ranges: build_debug_ranges(config, &code, address_size, abs_addr)?,
    }))
}

/// The three outputs a subprogram DIE appends to, and the dialect it must be
/// spelled in.
///
/// Passing this as one value is what keeps [`emit_subprograms`] readable: the
/// alternative is fifteen arguments, four of them mutable, in every recursive
/// helper.  `symbol_at` is moved in because the caller has no further use for
/// it — the unit's `low_pc` was already relocated against a label.
struct SubprogramSink<'a, 's, 'f> {
    info: &'a mut Vec<u8>,
    relocs: &'a mut Vec<ObjReloc>,
    dies: &'a mut DieWriter,
    strings: &'a mut DwarfStrings,
    line_strings: &'a mut DwarfStrings,
    symbol_at: SymbolAt<'s, 'f>,
    config: DwarfConfig,
    forms: UnitForms,
    abs32: u32,
    abs_addr: u32,
    address_size: usize,
    has_declarator_rows: bool,
}

/// Every function of the unit, as `DW_TAG_subprogram` DIEs in address order.
///
/// The sort key is `(section, offset, name)` rather than the input order because
/// a consumer binary-searches ranges; `name` only breaks ties between two
/// functions that start at the same address, which keeps the output
/// byte-reproducible when two sections alias.
///
/// A function is described when it has a size and lies inside a section the
/// unit covers: a zero-size symbol is an alias or an assembler label, and
/// describing it would produce a DIE whose `low_pc`/`high_pc` range is empty.
fn emit_subprograms(
    sink: SubprogramSink<'_, '_, '_>,
    functions: &[DwarfFunction<'_>],
    sections: &[DwarfSectionInfo<'_>],
    covered: &[bool],
    unit: &DwarfUnitSpec,
    abbrevs: &UnitAbbrevs,
    declaration: &dyn Fn(usize, u64, u64) -> Option<SourcePosition>,
    void_die: u32,
) -> Result<(), String> {
    // Destructured into plain locals, so the emission below reads exactly as it
    // did when it lived inline in the section builder.
    let SubprogramSink {
        info,
        relocs,
        dies,
        strings,
        line_strings,
        mut symbol_at,
        config,
        forms,
        abs32,
        abs_addr,
        address_size,
        has_declarator_rows,
    } = sink;
    let mut members: Vec<&DwarfFunction<'_>> = functions
        .iter()
        .filter(|f| f.size != 0 && covered[f.section_index])
        .filter(|f| {
            sections[f.section_index]
                .size
                .checked_sub(f.size)
                .is_some_and(|limit| f.offset <= limit)
        })
        .collect();
    members.sort_by(|a, b| {
        (a.section_index, a.offset, a.name).cmp(&(b.section_index, b.offset, b.name))
    });
    for function in members {
        if function.name.as_bytes().contains(&0) {
            return Err(format!(
                "DWARF function name contains a NUL byte: {:?}",
                function.name
            ));
        }
        let known = declaration(function.section_index, function.offset, function.size);
        // A function with parameters needs a subprogram abbreviation that declares
        // children; one without keeps the compact shape, so the common case costs
        // no extra byte.
        let parameters = unit.parameters_of(function.name);
        let shape = if parameters.is_some() {
            &abbrevs.subprogram_with_children
        } else {
            &abbrevs.subprogram
        };
        let abbrev_code = shape[usize::from(known.is_none())][usize::from(function.external)];
        write_uleb128(abbrev_code, info);
        write_string_attribute(
            function.name,
            forms.string,
            info,
            relocs,
            abs32,
            strings,
            line_strings,
        )?;
        if config.version < 4 {
            // `DW_FORM_flag`: the byte after the name says whether the
            // function is external. From DWARF 4 on the attribute is a
            // `flag_present` and only exists on external functions.
            info.push(u8::from(function.external));
        }
        if let Some(position) = known {
            write_uleb128(position.file, info);
            write_uleb128(position.line, info);
            if has_declarator_rows {
                write_uleb128(u64::from(position.column), info);
            }
        }
        // The return type, when the compiler knows it.  `DW_AT_type` is a
        // four-byte reference in both cases -- a type reference cannot be a
        // ULEB here, because the type table is laid out after the subprograms
        // and a `ref_udata` would change width when the offset moved.
        match unit.return_types.get(function.name) {
            Some(&index) => {
                dies.patches.push((info.len(), index));
                info.extend_from_slice(&[0; 4]);
            }
            None => {
                // The `DW_TAG_unspecified_type` the unit opened with: a debugger
                // then never assumes `int` for an unknown return type.
                info.extend_from_slice(&void_die.to_le_bytes());
            }
        }
        if let Some(symbol_at) = symbol_at.as_deref_mut() {
            let start_at = symbol_at(function.section_index, function.offset);
            let end_at = symbol_at(function.section_index, function.offset + function.size);
            push_reloc(info, relocs, abs_addr, address_size, &start_at, 0)?;
            push_reloc(info, relocs, abs_addr, address_size, &end_at, 0)?;
        } else {
            let section = &sections[function.section_index];
            push_reloc(
                info,
                relocs,
                abs_addr,
                address_size,
                section.name,
                function.offset,
            )?;
            if forms.high_pc == DW_FORM_ADDR {
                let end = function.offset.checked_add(function.size).ok_or_else(|| {
                    "internal: DWARF function range overflows the code section".to_string()
                })?;
                push_reloc(info, relocs, abs_addr, address_size, section.name, end)?;
            } else {
                push_high_pc(info, forms.high_pc, function.size);
            }
        }
        if config.version >= 3 {
            // A one-byte expression block: `uleb` length, then the opcode.
            info.push(1);
            info.push(DW_OP_CALL_FRAME_CFA);
        }
        // The parameter list, as children of the subprogram.  A consumer lists a
        // function's arguments from here; the `DW_TAG_formal_parameter` DIEs
        // inside a `DW_TAG_subroutine_type` describe a *type* and are anonymous,
        // so they cannot answer "what is `n` in this frame".
        if let Some(list) = parameters {
            for parameter in &list.parameters {
                // Four shapes: named or not, located or not.  The abbreviation
                // says which attributes follow, so a parameter the frame could not
                // place simply takes the shorter shape instead of writing an empty
                // location block -- an empty block is not "unknown", it is an
                // expression of length zero, which a consumer must reject.
                let shape = match (parameter.name.is_some(), parameter.location.is_some()) {
                    (true, true) => abbrevs.formal_parameter_named_located,
                    (false, true) => abbrevs.formal_parameter_unnamed_located,
                    (true, false) => abbrevs.formal_parameter_named,
                    (false, false) => abbrevs.formal_parameter_unnamed,
                };
                // A located parameter exists only in a unit that declared the
                // shape for it, so a missing code is a writer bug and not input.
                let code = shape.ok_or_else(|| {
                    "internal: DWARF parameter has a location but the unit declared no \
                     abbreviation for it"
                        .to_string()
                })?;
                write_uleb128(code, info);
                if let Some(name) = parameter.name.as_deref() {
                    if name.as_bytes().contains(&0) {
                        return Err(format!(
                            "DWARF parameter name contains a NUL byte: {name:?}"
                        ));
                    }
                    write_string_attribute(
                        name,
                        forms.string,
                        info,
                        relocs,
                        abs32,
                        strings,
                        line_strings,
                    )?;
                }
                // The type reference is patched once the type table has offsets.
                dies.patches.push((info.len(), parameter.ty));
                info.extend_from_slice(&[0; 4]);
                if let Some(location) = parameter.location {
                    // `DW_OP_fbreg <delta>` in a `block1`: one opcode, then a
                    // signed LEB128.  The frame base is `DW_OP_call_frame_cfa`, so
                    // this reads "the caller's stack pointer at the call, plus
                    // `delta`", which is the slot the prologue writes and the
                    // address `&parameter` yields.
                    let expression = location.expression();
                    let length = u8::try_from(expression.len()).map_err(|_| {
                        "DWARF parameter location expression exceeds 255 bytes".to_string()
                    })?;
                    info.push(length);
                    info.extend_from_slice(&expression);
                }
            }
            if list.variadic {
                write_uleb128(abbrevs.unspecified_parameters, info);
            }
            info.push(0); // end of this subprogram's children
        }
    }
    Ok(())
}

/// Move a built line program to where it starts inside the section it belongs
/// to, and answer that offset.
///
/// The builder writes one program starting at zero, which is the whole section
/// for an assembler-generated object.  An input that declares `.debug_line`
/// itself -- `gcc -S -g` emits `.section .debug_line` and an empty
/// `.Ldebug_line0`, expecting the assembler to fill it, exactly as GAS does --
/// keeps its bytes and the generated program follows them; every relocation
/// offset then moves by the prefix length, and the program's offset is the
/// `DW_AT_stmt_list` value of the unit that describes it.
pub(crate) fn place_line_build(build: &mut DwarfLineBuild, prefix: &[u8]) -> Result<u64, String> {
    let base = u64::try_from(prefix.len())
        .map_err(|_| "debug section prefix exceeds the addressable size".to_string())?;
    if base == 0 {
        return Ok(0);
    }
    for reloc in &mut build.line.relocs {
        reloc.offset = reloc
            .offset
            .checked_add(base)
            .ok_or_else(|| "DWARF line-table relocation offset overflows".to_string())?;
    }
    let mut data = Vec::with_capacity(prefix.len() + build.line.data.len());
    data.extend_from_slice(prefix);
    data.append(&mut build.line.data);
    build.line.data = data;
    Ok(base)
}

/// Write one string attribute in the encoding its form names: inline for
/// versions 2..4, a relocated offset into `.debug_line_str` for a version-5
/// unit path, and a relocated offset into `.debug_str` otherwise.  The offset
/// always travels as a relocation, because the linker merges the string tables
/// of every object in the link and only it knows the final offsets.
fn write_string_attribute(
    text: &str,
    form: u64,
    info: &mut Vec<u8>,
    relocs: &mut Vec<ObjReloc>,
    abs32: u32,
    strings: &mut DwarfStrings,
    line_strings: &mut DwarfStrings,
) -> Result<(), String> {
    match form {
        DW_FORM_STRING => {
            info.extend_from_slice(text.as_bytes());
            info.push(0);
        }
        DW_FORM_LINE_STRP => {
            let offset = line_strings.intern(text)?;
            push_strp(info, relocs, 0, abs32, ".debug_line_str", offset);
        }
        DW_FORM_STRP => {
            let offset = strings.intern(text)?;
            push_strp(info, relocs, 0, abs32, ".debug_str", offset);
        }
        other => return Err(format!("internal: {other:#x} is not a string form")),
    }
    Ok(())
}

/// Declare one abbreviation: its code, its tag, whether it has children, and
/// every `(attribute, form)` pair, terminated as the section requires.
fn declare_abbrev(abbrev: &mut Vec<u8>, code: u64, tag: u64, children: bool, attrs: &[(u64, u64)]) {
    write_uleb128(code, abbrev);
    write_uleb128(tag, abbrev);
    abbrev.push(u8::from(children));
    for &(at, form) in attrs {
        write_uleb128(at, abbrev);
        write_uleb128(form, abbrev);
    }
    abbrev.extend_from_slice(&[0, 0]);
}

// ---------------------------------------------------------------------------
// DIEs: types and variables
// ---------------------------------------------------------------------------
//
// The unit described above is a skeleton -- a compile unit and one subprogram
// per function, which is everything an assembler can infer from the text it was
// given.  A compiler knows more, and a debugger needs it: the type of a name and
// the address of its bytes.  Both arrive as a [`DwarfUnitSpec`] (see
// `dwarf_spec`), which the driver builds from the semantic analysis, and are
// written here as `DW_TAG_base_type`-style DIEs and `DW_TAG_variable` DIEs.
//
// The emission is a small table-driven writer rather than a hand-written byte
// dumper, because the abbreviation has to declare exactly the attributes the
// DIE writes.  One list of `Attr` values produces both, so the two cannot drift;
// that failure mode (an abbreviation that promises an attribute the bytes never
// contain) makes every later DIE in the unit unreadable, and it is the one a
// debug-info writer most easily ships.

/// The value of one DIE attribute, in the shape the writer has to produce.
enum AttrValue {
    /// A string in the unit's dialect: `DW_FORM_strp` from version 5, inline
    /// before it.
    Text(String),
    Udata(u64),
    Data1(u8),
    Sdata(i64),
    /// A reference to a DIE in this unit's type table, patched once the table
    /// is laid out: a type may name a type that is written after it.
    TypeRef(u32),
    /// `DW_AT_location` of an object at a fixed address: `DW_OP_addr` and a
    /// relocation against the symbol the linker will place.
    LocationAddr(String),
    /// `DW_AT_location` of a value the frame layout placed: a location
    /// expression, written as a length-prefixed block.
    ///
    /// `DW_FORM_block1` is what GCC 16.2 writes at `-O0` (`2 byte block: 91 5c`)
    /// and what every dialect from 2 to 5 accepts, so one form serves them all;
    /// `DW_FORM_exprloc` would say the same thing one version later for no gain.
    LocationExpr(Vec<u8>),
    /// `DW_AT_data_member_location`: a constant from version 4 on,
    /// `DW_OP_plus_uconst` in a block before it.
    MemberLocation(u64),
    /// A flag whose value is always true: `DW_FORM_flag_present` from version 4
    /// on, which costs no bytes, and `DW_FORM_flag` before it.
    FlagPresent,
}

struct Attr {
    at: u64,
    value: AttrValue,
}

impl Attr {
    /// The form this attribute takes in the unit's dialect.  The abbreviation and
    /// the value writer both call this, so a form cannot be declared for one
    /// shape and written in another.
    fn form(&self, config: DwarfConfig, forms: UnitForms) -> u64 {
        match &self.value {
            AttrValue::Text(_) => forms.string,
            AttrValue::Udata(_) => DW_FORM_UDATA,
            AttrValue::MemberLocation(_) if config.version >= 4 => DW_FORM_UDATA,
            AttrValue::MemberLocation(_) => DW_FORM_BLOCK1,
            AttrValue::Data1(_) => DW_FORM_DATA1,
            AttrValue::Sdata(_) => DW_FORM_SDATA,
            AttrValue::TypeRef(_) => DW_FORM_REF4,
            AttrValue::LocationExpr(_) => DW_FORM_BLOCK1,
            AttrValue::LocationAddr(_) => {
                if config.version >= 4 {
                    DW_FORM_EXPRLOC
                } else {
                    DW_FORM_BLOCK1
                }
            }
            AttrValue::FlagPresent => {
                if config.version >= 4 {
                    DW_FORM_FLAG_PRESENT
                } else {
                    DW_FORM_FLAG
                }
            }
        }
    }
}

/// The abbreviation table and the type table's layout, shared by every DIE the
/// unit writes.
struct DieWriter {
    /// `.debug_abbrev` bytes, including the unit's own declarations.
    abbrev: Vec<u8>,
    /// `(tag, has_children, attributes)` to abbreviation code, so a shape that
    /// repeats -- 40 `int` members, say -- is declared once.
    codes: FxHashMap<(u64, bool, Vec<(u64, u64)>), u64>,
    next_code: u64,
    /// Where each type DIE starts, relative to the unit, filled as the table is
    /// written.
    type_offsets: Vec<Option<u64>>,
    /// `(position, type index)` for every `DW_AT_type` written, patched once
    /// every type offset is known.
    patches: Vec<(usize, u32)>,
}

impl DieWriter {
    fn new(abbrev: Vec<u8>, next_code: u64, types: usize) -> Self {
        Self {
            abbrev,
            codes: FxHashMap::default(),
            next_code,
            type_offsets: vec![None; types],
            patches: Vec::new(),
        }
    }

    fn code_for(&mut self, tag: u64, children: bool, attrs: &[(u64, u64)]) -> u64 {
        let key = (tag, children, attrs.to_vec());
        if let Some(&code) = self.codes.get(&key) {
            return code;
        }
        let code = self.next_code;
        self.next_code += 1;
        declare_abbrev(&mut self.abbrev, code, tag, children, attrs);
        self.codes.insert(key, code);
        code
    }
}

/// Everything one DIE needs to write its bytes: the output buffers, the
/// dialect, and the relocation parameters of the unit.
struct DieContext<'a, 'b> {
    info: &'a mut Vec<u8>,
    relocs: &'a mut Vec<ObjReloc>,
    strings: &'a mut DwarfStrings,
    line_strings: &'a mut DwarfStrings,
    config: DwarfConfig,
    forms: UnitForms,
    abs32: u32,
    abs_addr: u32,
    address_size: usize,
    /// Where the unit starts, so a reference can be made unit-relative.
    unit_start: usize,
    writer: &'b mut DieWriter,
}

impl DieContext<'_, '_> {
    /// Write one DIE: its abbreviation code, then each attribute value.
    fn write(&mut self, tag: u64, children: bool, attrs: &[Attr]) -> Result<(), String> {
        let forms: Vec<(u64, u64)> = attrs
            .iter()
            .map(|attr| (attr.at, attr.form(self.config, self.forms)))
            .collect();
        let code = self.writer.code_for(tag, children, &forms);
        write_uleb128(code, self.info);
        for attr in attrs {
            self.value(&attr.value)?;
        }
        Ok(())
    }

    fn value(&mut self, value: &AttrValue) -> Result<(), String> {
        match value {
            AttrValue::Text(text) => write_string_attribute(
                text,
                self.forms.string,
                self.info,
                self.relocs,
                self.abs32,
                self.strings,
                self.line_strings,
            ),
            AttrValue::Udata(number) => {
                write_uleb128(*number, self.info);
                Ok(())
            }
            AttrValue::MemberLocation(offset) if self.config.version >= 4 => {
                // From version 4 the offset is a constant; the value is a byte
                // offset into the record, which is exactly what the layout
                // computed.
                write_uleb128(*offset, self.info);
                Ok(())
            }
            AttrValue::MemberLocation(offset) => {
                // Versions 2 and 3 have no constant form for a member offset: the
                // attribute is an expression that adds the offset to the record's
                // address.  `DW_OP_plus_uconst` in a one-byte-length block is the
                // spelling GCC 16.2 uses for the same dialect (measured).
                let mut block = vec![DW_OP_PLUS_UCONST];
                write_uleb128(*offset, &mut block);
                let length = u8::try_from(block.len())
                    .map_err(|_| "DWARF member location block exceeds 255 bytes".to_string())?;
                self.info.push(length);
                self.info.extend_from_slice(&block);
                Ok(())
            }
            AttrValue::Data1(byte) => {
                self.info.push(*byte);
                Ok(())
            }
            AttrValue::Sdata(number) => {
                write_sleb128(*number, self.info);
                Ok(())
            }
            AttrValue::TypeRef(index) => {
                // Four zero bytes now, the real offset once every type DIE has a
                // position: a record may contain a pointer to itself, so the table
                // cannot always be written in dependency order.
                self.writer.patches.push((self.info.len(), *index));
                self.info.extend_from_slice(&[0; 4]);
                Ok(())
            }
            AttrValue::LocationAddr(symbol) => {
                let length = 1usize
                    .checked_add(self.address_size)
                    .ok_or_else(|| "DWARF location expression overflows".to_string())?;
                if self.config.version >= 4 {
                    write_uleb128(
                        u64::try_from(length).map_err(|_| "DWARF expression length")?,
                        self.info,
                    );
                } else {
                    self.info.push(
                        u8::try_from(length)
                            .map_err(|_| "DWARF expression exceeds 255 bytes".to_string())?,
                    );
                }
                self.info.push(DW_OP_ADDR);
                push_reloc(
                    self.info,
                    self.relocs,
                    self.abs_addr,
                    self.address_size,
                    symbol,
                    0,
                )
            }
            AttrValue::LocationExpr(expression) => {
                // `DW_FORM_block1`: a one-byte length, then the expression. A
                // parameter's home is at most six bytes (`DW_OP_fbreg` plus a
                // five-byte LEB128), so the length always fits; anything longer
                // would be a writer bug, and is reported rather than truncated.
                let length = u8::try_from(expression.len())
                    .map_err(|_| "DWARF location expression exceeds 255 bytes".to_string())?;
                self.info.push(length);
                self.info.extend_from_slice(expression);
                Ok(())
            }
            AttrValue::FlagPresent => {
                if self.config.version < 4 {
                    self.info.push(1);
                }
                Ok(())
            }
        }
    }

    /// Patch every `DW_AT_type` now that the type table has a position for each
    /// entry.
    fn patch_type_refs(&mut self) -> Result<(), String> {
        let unit_start = self.unit_start;
        for &(position, index) in &self.writer.patches {
            let offset = self
                .writer
                .type_offsets
                .get(index as usize)
                .copied()
                .flatten()
                .ok_or_else(|| format!("internal: DWARF type {index} was never written"))?;
            let relative = u32::try_from(offset)
                .map_err(|_| "DWARF type offset exceeds 32 bits".to_string())?;
            let end = position
                .checked_add(4)
                .ok_or_else(|| "DWARF reference position overflows".to_string())?;
            if end > self.info.len() {
                return Err("internal: DWARF reference points past the unit".to_string());
            }
            self.info[position..end].copy_from_slice(&relative.to_le_bytes());
        }
        let _ = unit_start;
        Ok(())
    }
}

/// The index base type of an array: `DW_AT_type` of every `DW_TAG_subrange_type`.
///
/// DWARF gives a subrange a default (a signed type of the target's address
/// size); GCC 16.2 states the unsigned width instead, so the same is done here
/// -- an upper bound is never negative.
fn subrange_base(pointer_size: usize) -> BaseKind {
    if pointer_size == 8 {
        BaseKind::ULong
    } else {
        BaseKind::UInt
    }
}

/// Write the type table: one DIE per interned type, in table order, each with
/// its children, and remember where each landed.
fn emit_types(ctx: &mut DieContext<'_, '_>, spec: &DwarfUnitSpec) -> Result<(), String> {
    for (index, ty) in spec.types.iter().enumerate() {
        let offset = u64::try_from(ctx.info.len() - ctx.unit_start)
            .map_err(|_| "DWARF type offset exceeds 64 bits".to_string())?;
        if let Some(slot) = ctx.writer.type_offsets.get_mut(index) {
            *slot = Some(offset);
        }
        emit_type(ctx, spec, ty)?;
    }
    Ok(())
}

fn emit_type(
    ctx: &mut DieContext<'_, '_>,
    spec: &DwarfUnitSpec,
    ty: &DwarfType,
) -> Result<(), String> {
    // `DW_AT_data_bit_offset` is legal from DWARF 4 on, but GCC 16.2 keeps the
    // `bit_offset` + `data_member_location` pair there and switches only for 5
    // (measured on both dialects).  Matching that is free and buys consumers
    // that never learned the newer attribute -- and a byte-for-byte oracle.
    let five = ctx.config.version >= 5;
    match ty {
        DwarfType::Base { kind, size } => ctx.write(
            DW_TAG_BASE_TYPE,
            false,
            &[
                Attr {
                    at: DW_AT_BYTE_SIZE,
                    value: AttrValue::Udata(*size),
                },
                Attr {
                    at: DW_AT_ENCODING,
                    value: AttrValue::Data1(
                        u8::try_from(kind.encoding())
                            .map_err(|_| "DWARF encoding does not fit a byte".to_string())?,
                    ),
                },
                Attr {
                    at: DW_AT_NAME,
                    value: AttrValue::Text(kind.spelling().to_string()),
                },
            ],
        ),
        DwarfType::Pointer { pointee, size } => {
            let mut attrs = vec![Attr {
                at: DW_AT_BYTE_SIZE,
                value: AttrValue::Udata(*size),
            }];
            if let Some(pointee) = pointee {
                attrs.push(Attr {
                    at: DW_AT_TYPE,
                    value: AttrValue::TypeRef(*pointee),
                });
            }
            ctx.write(DW_TAG_POINTER_TYPE, false, &attrs)
        }
        DwarfType::Qualified { qual, target } => {
            let tag = match qual {
                TypeQual::Const => DW_TAG_CONST_TYPE,
                TypeQual::Volatile => DW_TAG_VOLATILE_TYPE,
                TypeQual::Restrict => DW_TAG_RESTRICT_TYPE,
                // `DW_TAG_atomic_type` is DWARF 5; before it, `const` is the
                // truthful approximation of a C11 `_Atomic` object's storage.
                TypeQual::Atomic => DW_TAG_CONST_TYPE,
            };
            ctx.write(
                tag,
                false,
                &[Attr {
                    at: DW_AT_TYPE,
                    value: AttrValue::TypeRef(*target),
                }],
            )
        }
        DwarfType::Typedef { name, target } => ctx.write(
            DW_TAG_TYPEDEF,
            false,
            &[
                Attr {
                    at: DW_AT_NAME,
                    value: AttrValue::Text(name.clone()),
                },
                Attr {
                    at: DW_AT_TYPE,
                    value: AttrValue::TypeRef(*target),
                },
            ],
        ),
        DwarfType::Array {
            element,
            bounds,
            vector,
        } => {
            let mut attrs = vec![Attr {
                at: DW_AT_TYPE,
                value: AttrValue::TypeRef(*element),
            }];
            if *vector {
                attrs.push(Attr {
                    at: DW_AT_GNU_VECTOR,
                    value: AttrValue::FlagPresent,
                });
            }
            ctx.write(DW_TAG_ARRAY_TYPE, true, &attrs)?;
            // Every subrange names the same unsigned type of the target's
            // address size, which is what GCC 16.2 states and what a consumer
            // uses to print an index; the spec interns it, so the reference is a
            // lookup rather than a second table.
            let index_type = spec.base(subrange_base(ctx.address_size));
            for bound in bounds {
                let mut subrange = Vec::new();
                if let Some(index_type) = index_type {
                    subrange.push(Attr {
                        at: DW_AT_TYPE,
                        value: AttrValue::TypeRef(index_type),
                    });
                }
                if let Some(upper) = bound {
                    subrange.push(Attr {
                        at: DW_AT_UPPER_BOUND,
                        value: AttrValue::Udata(*upper),
                    });
                }
                ctx.write(DW_TAG_SUBRANGE_TYPE, false, &subrange)?;
            }
            ctx.info.push(0);
            Ok(())
        }
        DwarfType::Record {
            name,
            kind,
            size,
            complete,
            members,
        } => emit_record(
            ctx,
            spec,
            name.as_deref(),
            *kind,
            *size,
            *complete,
            members,
            five,
        ),
        DwarfType::Enum {
            name,
            size,
            underlying,
            complete,
            enumerators,
        } => {
            let mut attrs = Vec::new();
            if let Some(name) = name {
                attrs.push(Attr {
                    at: DW_AT_NAME,
                    value: AttrValue::Text(name.clone()),
                });
            }
            attrs.push(Attr {
                at: DW_AT_ENCODING,
                value: AttrValue::Data1(
                    u8::try_from(BaseKind::Int.encoding())
                        .map_err(|_| "DWARF encoding does not fit a byte".to_string())?,
                ),
            });
            attrs.push(Attr {
                at: DW_AT_BYTE_SIZE,
                value: AttrValue::Udata(*size),
            });
            attrs.push(Attr {
                at: DW_AT_TYPE,
                value: AttrValue::TypeRef(*underlying),
            });
            if !*complete {
                attrs.push(Attr {
                    at: DW_AT_DECLARATION,
                    value: AttrValue::FlagPresent,
                });
            }
            ctx.write(DW_TAG_ENUMERATION_TYPE, true, &attrs)?;
            for (name, value) in enumerators {
                ctx.write(
                    DW_TAG_ENUMERATOR,
                    false,
                    &[
                        Attr {
                            at: DW_AT_NAME,
                            value: AttrValue::Text(name.clone()),
                        },
                        Attr {
                            at: DW_AT_CONST_VALUE,
                            value: AttrValue::Sdata(*value),
                        },
                    ],
                )?;
            }
            ctx.info.push(0);
            Ok(())
        }
        DwarfType::Subroutine {
            ret,
            params,
            variadic,
            prototyped,
        } => {
            let mut attrs = Vec::new();
            if let Some(ret) = ret {
                attrs.push(Attr {
                    at: DW_AT_TYPE,
                    value: AttrValue::TypeRef(*ret),
                });
            }
            if *prototyped {
                attrs.push(Attr {
                    at: DW_AT_PROTOTYPED,
                    value: AttrValue::FlagPresent,
                });
            }
            ctx.write(DW_TAG_SUBROUTINE_TYPE, true, &attrs)?;
            for param in params {
                ctx.write(
                    DW_TAG_FORMAL_PARAMETER,
                    false,
                    &[Attr {
                        at: DW_AT_TYPE,
                        value: AttrValue::TypeRef(*param),
                    }],
                )?;
            }
            if *variadic {
                ctx.write(DW_TAG_UNSPECIFIED_PARAMETERS, false, &[])?;
            }
            ctx.info.push(0);
            Ok(())
        }
        DwarfType::Unspecified { name } => ctx.write(
            DW_TAG_UNSPECIFIED_TYPE,
            false,
            &[Attr {
                at: DW_AT_NAME,
                value: AttrValue::Text((*name).to_string()),
            }],
        ),
    }
}

/// Write the unit's file-scope objects as `DW_TAG_variable` DIEs.
///
/// An object the unit defines carries `DW_OP_addr` and a relocation against its
/// symbol, so the location evaluates to wherever the linker put it; a pure
/// declaration (`extern int x;`) carries `DW_AT_declaration` and no location,
/// because this object emits no storage for it.  Inventing a location for such
/// an object would make a debugger read address zero and print confident
/// nonsense.
/// One `DW_TAG_structure_type`/`union_type` DIE and its members.
///
/// The member loop is the reason this is a function rather than three lines
/// inside the dispatch: the bit-field attributes differ between DWARF 5
/// (`data_bit_offset`, counted from the least significant bit of the object) and
/// 2–4 (`bit_offset`, counted from the most significant bit of the *storage
/// unit*, which is why the unit's width has to be looked up), and both encodings
/// belong next to the record they annotate rather than in the middle of a
/// nine-way match.
fn emit_record(
    ctx: &mut DieContext<'_, '_>,
    spec: &DwarfUnitSpec,
    name: Option<&str>,
    kind: RecordKind,
    size: u64,
    complete: bool,
    members: &[DwarfMember],
    five: bool,
) -> Result<(), String> {
    let tag = match kind {
        RecordKind::Struct => DW_TAG_STRUCTURE_TYPE,
        RecordKind::Union => DW_TAG_UNION_TYPE,
    };
    let mut attrs = Vec::new();
    if let Some(name) = name {
        attrs.push(Attr {
            at: DW_AT_NAME,
            value: AttrValue::Text(name.to_string()),
        });
    }
    attrs.push(Attr {
        at: DW_AT_BYTE_SIZE,
        value: AttrValue::Udata(size),
    });
    if !complete {
        // A declaration is not an empty struct: without this flag a consumer
        // would report every member as missing instead of "incomplete type".
        attrs.push(Attr {
            at: DW_AT_DECLARATION,
            value: AttrValue::FlagPresent,
        });
    }
    ctx.write(tag, true, &attrs)?;
    for member in members {
        emit_member(ctx, spec, kind, member, five)?;
    }
    ctx.info.push(0);
    Ok(())
}

/// One `DW_TAG_member`, in the encoding its dialect uses.
fn emit_member(
    ctx: &mut DieContext<'_, '_>,
    spec: &DwarfUnitSpec,
    kind: RecordKind,
    member: &DwarfMember,
    five: bool,
) -> Result<(), String> {
    let mut attrs = Vec::new();
    if let Some(name) = &member.name {
        attrs.push(Attr {
            at: DW_AT_NAME,
            value: AttrValue::Text(name.clone()),
        });
    }
    attrs.push(Attr {
        at: DW_AT_TYPE,
        value: AttrValue::TypeRef(member.ty),
    });
    match member.bit {
        None => {
            // A union member sits at offset zero by definition, so the attribute
            // is not written at all -- GCC 16.2 does the same (measured).
            if matches!(kind, RecordKind::Struct) {
                attrs.push(Attr {
                    at: DW_AT_DATA_MEMBER_LOCATION,
                    value: AttrValue::MemberLocation(member.offset),
                });
            }
        }
        Some((width, bit_offset)) => {
            if five {
                // Version 5 counts from the least significant bit of the
                // object, which is the layout's own numbering.
                let bit = member.offset * 8 + u64::from(bit_offset);
                attrs.push(Attr {
                    at: DW_AT_BIT_SIZE,
                    value: AttrValue::Udata(u64::from(width)),
                });
                attrs.push(Attr {
                    at: DW_AT_DATA_BIT_OFFSET,
                    value: AttrValue::Udata(bit),
                });
            } else {
                // Versions 2 and 3 count from the most significant bit of the
                // *storage unit*, so the width of that unit is needed:
                // `sizeof(field.ty) * 8 - bit_offset - width`.
                let unit_bits = spec_type_size(spec, member.ty)
                    .and_then(|bytes| bytes.checked_mul(8))
                    .ok_or_else(|| "DWARF bit-field storage unit overflows".to_string())?;
                let offset = unit_bits
                    .checked_sub(u64::from(bit_offset) + u64::from(width))
                    .ok_or_else(|| {
                        "DWARF bit-field offset underflows its storage unit".to_string()
                    })?;
                attrs.push(Attr {
                    at: DW_AT_BYTE_SIZE,
                    value: AttrValue::Udata(unit_bits / 8),
                });
                attrs.push(Attr {
                    at: DW_AT_BIT_SIZE,
                    value: AttrValue::Udata(u64::from(width)),
                });
                attrs.push(Attr {
                    at: DW_AT_BIT_OFFSET,
                    value: AttrValue::Udata(offset),
                });
                attrs.push(Attr {
                    at: DW_AT_DATA_MEMBER_LOCATION,
                    value: AttrValue::MemberLocation(member.offset),
                });
            }
        }
    }
    ctx.write(DW_TAG_MEMBER, false, &attrs)?;
    Ok(())
}

fn emit_variables(
    ctx: &mut DieContext<'_, '_>,
    variables: &[DwarfVariable],
    files: &[(u32, String)],
    defined: &FxHashMap<String, ()>,
) -> Result<(), String> {
    // `DW_AT_decl_file` indexes the line program's file table, whose numbers are
    // contiguous and start at one (`debug_file_table` guarantees both), so the
    // `.file` number a declaration carries *is* its index -- in version 5 too,
    // where slot 0 is the primary file and slot `n` is `.file n`.  A number the
    // table does not list would point at the wrong file, so it falls back to the
    // first entry rather than being written blindly.
    let table = debug_file_table(files)?;
    let fallback = table.first().map_or(1, |(number, _)| *number);
    // A name the table lists under any spelling -- the compiler passes the
    // path the preprocessor resolved, the object may spell it relative -- is
    // matched by its last component, which is what a debugger shows anyway.
    let number_for = |name: &str| -> Option<u32> {
        if let Some((number, _)) = table.iter().find(|(_, listed)| *listed == name) {
            return Some(*number);
        }
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        table
            .iter()
            .find(|(_, listed)| listed.rsplit(['/', '\\']).next() == Some(base))
            .map(|(number, _)| *number)
    };
    for variable in variables {
        let mut attrs = vec![Attr {
            at: DW_AT_NAME,
            value: AttrValue::Text(variable.name.clone()),
        }];
        let file_number = variable
            .decl_file
            .as_deref()
            .and_then(&number_for)
            .unwrap_or(fallback);
        attrs.push(Attr {
            at: DW_AT_DECL_FILE,
            value: AttrValue::Udata(u64::from(file_number)),
        });
        // The line and column belong to that file: without a file index a
        // consumer would resolve them against the unit's, which need not be the
        // same one.
        if let Some(line) = variable.decl_line.filter(|_| !variable.declaration || true) {
            attrs.push(Attr {
                at: DW_AT_DECL_LINE,
                value: AttrValue::Udata(u64::from(line)),
            });
            if let Some(column) = variable.decl_column {
                attrs.push(Attr {
                    at: DW_AT_DECL_COLUMN,
                    value: AttrValue::Udata(u64::from(column)),
                });
            }
        }
        attrs.push(Attr {
            at: DW_AT_TYPE,
            value: AttrValue::TypeRef(variable.ty),
        });
        if variable.external {
            attrs.push(Attr {
                at: DW_AT_EXTERNAL,
                value: AttrValue::FlagPresent,
            });
        }
        if variable.declaration {
            attrs.push(Attr {
                at: DW_AT_DECLARATION,
                value: AttrValue::FlagPresent,
            });
        } else if defined.contains_key(&variable.name) {
            attrs.push(Attr {
                at: DW_AT_LOCATION,
                value: AttrValue::LocationAddr(variable.name.clone()),
            });
        }
        // An object the optimizer removed keeps its DIE and states no location,
        // which every consumer renders as "optimized out".  A location here would
        // also be a *relocation* against a symbol the object does not define, and
        // the link would fail on it -- measured: GCC 16.2 keeps the DIE and drops
        // the location for an unused static at `-O2`.
        ctx.write(DW_TAG_VARIABLE, false, &attrs)?;
    }
    Ok(())
}

/// The size of an interned type, for the version 2 and 3 bit-field encoding
/// that needs the storage unit's width.
fn spec_type_size(spec: &DwarfUnitSpec, index: u32) -> Option<u64> {
    match spec.types.get(index as usize)? {
        DwarfType::Base { size, .. }
        | DwarfType::Record { size, .. }
        | DwarfType::Enum { size, .. }
        | DwarfType::Pointer { size, .. } => Some(*size),
        DwarfType::Qualified { target, .. } | DwarfType::Typedef { target, .. } => {
            spec_type_size(spec, *target)
        }
        DwarfType::Array {
            element, bounds, ..
        } => {
            let element_size = spec_type_size(spec, *element)?;
            // The product of the dimensions, each of which is an *upper* bound.
            // An incomplete array has no size to report.
            let mut total = element_size;
            for bound in bounds {
                total = total.checked_mul((*bound)?.checked_add(1)?)?;
            }
            Some(total)
        }
        // A function type has no object size, and an unspecified type has no
        // bytes a consumer may read.
        DwarfType::Subroutine { .. } | DwarfType::Unspecified { .. } => None,
    }
}

/// Build `.debug_aranges`: the address map a consumer reads *before* the
/// units, so mapping an address to a compile unit costs one binary search
/// instead of a walk over every unit header.
///
/// One set per compilation unit, exactly as GCC emits: the set names the unit's
/// offset in `.debug_info` (a relocation, because the linker concatenates the
/// sections of every object) and then lists every address range the unit
/// covers, one `(address, length)` tuple per code section -- followed by the
/// zero tuple that ends the set.  The specification requires the first tuple to
/// start on a multiple of the tuple size counted from the set's own length
/// field, which is 12 bytes of header on a 64-bit target and therefore four
/// bytes of padding: the same shape GCC 16.2 writes (measured: length 76, three
/// tuples and a terminator for a three-section unit).
///
/// A relaxing target gets no table.  Its lengths are computed before the linker
/// deletes bytes (RISC-V relaxation), so a range would be slightly too long and
/// would claim code that belongs to the next unit: an address map that is wrong
/// is worse than the scan a consumer falls back to.  `labels` is that case.
fn build_debug_aranges(
    info_offset: u64,
    code: &[&DwarfSectionInfo<'_>],
    address_size: usize,
    abs32: u32,
    abs_addr: u32,
    labels: bool,
) -> Result<Option<ObjSection>, String> {
    if labels {
        return Ok(None);
    }
    // A unit whose sections all hold no bytes covers no address; an empty tuple
    // would only give a consumer something to skip.
    let ranges: Vec<&&DwarfSectionInfo<'_>> = code.iter().filter(|c| c.size != 0).collect();
    if ranges.is_empty() {
        return Ok(None);
    }
    let tuple = 2 * address_size;
    let mut data = Vec::new();
    let mut relocs = Vec::new();
    data.extend_from_slice(&[0; 4]); // unit_length, patched below
    data.extend_from_slice(&2u16.to_le_bytes()); // version: always 2
    push_reloc(&mut data, &mut relocs, abs32, 4, ".debug_info", info_offset)?;
    data.push(u8::try_from(address_size).map_err(|_| "DWARF address size")?);
    data.push(0); // segment_selector_size: no segments
    let padding = (tuple - data.len() % tuple) % tuple;
    data.resize(data.len() + padding, 0);
    for section in ranges {
        push_reloc(
            &mut data,
            &mut relocs,
            abs_addr,
            address_size,
            section.name,
            0,
        )?;
        // The length is a constant tuple field, so it is `address_size` bytes
        // wide. A section larger than the target's address space is a malformed
        // input, not something to truncate silently.
        let size = section.size.to_le_bytes();
        if size[address_size..].iter().any(|&byte| byte != 0) {
            return Err(format!(
                "DWARF address range length {:#x} does not fit the target's {}-byte \
                 addresses",
                section.size, address_size
            ));
        }
        data.extend_from_slice(&size[..address_size]);
    }
    data.resize(data.len() + 2 * address_size, 0); // the terminating zero tuple
    let length = u32::try_from(data.len() - 4)
        .map_err(|_| "DWARF address range set exceeds 32 bits".to_string())?;
    data[..4].copy_from_slice(&length.to_le_bytes());
    Ok(Some(debug_section(".debug_aranges", data, relocs)))
}

/// Build the range list a unit with code in more than one section needs:
/// `.debug_rnglists` for version 5 and `.debug_ranges` for the versions that
/// predate it.
///
/// Version 5's section is a table with a twelve-byte header -- unit length,
/// version, address size, segment selector size and an offset table count of
/// zero -- followed by one list.  Each range is `DW_RLE_start_length`: the
/// start is an `address_size` relocation against the code section (the linker
/// decides where the section lands, so an offset could not be known here) and
/// the length is a constant, which is a ULEB128 and therefore never truncates.
/// `DW_RLE_end_of_list` ends it.  The three ranges of a three-section unit cost
/// 43 bytes, the size GCC 16.2 emits for the same input.
///
/// Before version 5 the ranges are pairs of `address_size` addresses followed by
/// a zero pair, with no header: the unit points at the list directly, so the
/// same three ranges cost 64 bytes on a 64-bit target.
///
/// A single contiguous section needs none of this -- its unit carries a
/// `DW_AT_low_pc`/`DW_AT_high_pc` pair, which is smaller than a section offset
/// plus this table -- so `code.len() < 2` writes nothing at all.
fn build_debug_ranges(
    config: DwarfConfig,
    code: &[&DwarfSectionInfo<'_>],
    address_size: usize,
    abs_addr: u32,
) -> Result<Option<ObjSection>, String> {
    if code.len() < 2 {
        return Ok(None);
    }
    let mut data = Vec::new();
    let mut relocs = Vec::new();
    if config.version >= 5 {
        data.extend_from_slice(&[0; 4]); // unit_length, patched below
        data.extend_from_slice(&5u16.to_le_bytes());
        data.push(u8::try_from(address_size).map_err(|_| "DWARF address size")?);
        data.push(0); // segment_selector_size
        data.extend_from_slice(&0u32.to_le_bytes()); // offset_entry_count
        for section in code {
            if section.size == 0 {
                continue;
            }
            data.push(DW_RLE_START_LENGTH);
            push_reloc(
                &mut data,
                &mut relocs,
                abs_addr,
                address_size,
                section.name,
                0,
            )?;
            write_uleb128(section.size, &mut data);
        }
        data.push(DW_RLE_END_OF_LIST);
        let length = u32::try_from(data.len() - 4)
            .map_err(|_| "DWARF range list exceeds 32 bits".to_string())?;
        data[..4].copy_from_slice(&length.to_le_bytes());
        Ok(Some(debug_section(".debug_rnglists", data, relocs)))
    } else {
        for section in code {
            if section.size == 0 {
                continue;
            }
            // A base-address pair would cost a third relocation and buy
            // nothing: the entries are absolute addresses either way.
            push_reloc(
                &mut data,
                &mut relocs,
                abs_addr,
                address_size,
                section.name,
                0,
            )?;
            push_reloc(
                &mut data,
                &mut relocs,
                abs_addr,
                address_size,
                section.name,
                section.size,
            )?;
        }
        data.resize(data.len() + 2 * address_size, 0); // terminating zero pair
        Ok(Some(debug_section(".debug_ranges", data, relocs)))
    }
}

/// The DWARF version the input's `.debug_info` declares, if it declares one.
///
/// The first unit header is `length` (4 or 12 bytes), then the version as a
/// 2-byte little-endian number; a section too short to hold that much states no
/// version, and a number outside 2..=5 is not a DWARF unit version at all.  A
/// section holding several units is read through its first, which is the one an
/// inputs' `.loc` rows belong to.
pub(crate) fn unit_version(info: &[u8]) -> Option<u8> {
    let low = *info.get(4)?;
    let high = *info.get(5)?;
    let version = u16::from_le_bytes([low, high]);
    u8::try_from(version)
        .ok()
        .filter(|version| (2..=5).contains(version))
}

/// The string pools among the debug sections.
///
/// `.debug_str` and `.debug_line_str` are NUL-separated pools of whole strings
/// that a relocation may only point *into at a string's first byte*, which is
/// exactly the invariant `SHF_MERGE|SHF_STRINGS` states -- so GCC, GAS and
/// every other producer flag them mergeable (`@progbits,1`), and a linker is
/// then free to keep one copy of each string across all objects of a link.
/// lccc marks them the same way: an object built from GCC's own assembly
/// declares `.section .debug_str,"MS",@progbits,1`, and dropping that attribute
/// (as a plain `PROGBITS` with flags 0 does) costs every multi-object link one
/// copy of the producer, directory and file-name strings per object, and makes
/// lccc objects the only ones a linker cannot dedupe.
fn debug_section_flags(name: &str) -> u64 {
    match name {
        ".debug_str" | ".debug_line_str" => SHF_MERGE | SHF_STRINGS,
        _ => 0,
    }
}

fn debug_section(name: &str, data: Vec<u8>, relocs: Vec<ObjReloc>) -> ObjSection {
    ObjSection {
        name: name.to_string(),
        sh_type: SHT_PROGBITS,
        sh_flags: debug_section_flags(name),
        sh_size: None,
        data,
        sh_addralign: 1,
        relocs,
        comdat_group: None,
    }
}

/// The standard opcodes' operand counts, in opcode order: `copy`,
/// `advance_pc`, `advance_line`, `set_file`, `set_column`, `negate_stmt`,
/// `set_basic_block`, `const_add_pc`, `fixed_advance_pc`, `set_prologue_end`,
/// `set_epilogue_begin`, `set_isa`.
const STANDARD_OPCODE_LENGTHS: [u8; 12] = [0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1];
/// `opcode_base` for DWARF 3 and newer (twelve standard opcodes).
const OPCODE_BASE: u8 = 13;
/// `opcode_base` for DWARF 2, which has only the first nine standard opcodes.
const OPCODE_BASE_DWARF2: usize = 10;

/// `DW_UT_compile`: the unit type a version-5 compilation unit header names.
const DW_UT_COMPILE: u8 = 0x01;
const DW_TAG_COMPILE_UNIT: u64 = 0x11;
const DW_TAG_SUBPROGRAM: u64 = 0x2e;
const DW_TAG_UNSPECIFIED_TYPE: u64 = 0x3b;
const DW_TAG_MEMBER: u64 = 0x0d;
const DW_TAG_POINTER_TYPE: u64 = 0x0f;
const DW_TAG_CONST_TYPE: u64 = 0x26;
const DW_TAG_VOLATILE_TYPE: u64 = 0x35;
const DW_TAG_RESTRICT_TYPE: u64 = 0x37;
const DW_TAG_BASE_TYPE: u64 = 0x24;
const DW_TAG_TYPEDEF: u64 = 0x16;
const DW_TAG_STRUCTURE_TYPE: u64 = 0x13;
const DW_TAG_UNION_TYPE: u64 = 0x17;
const DW_TAG_ENUMERATION_TYPE: u64 = 0x04;
const DW_TAG_ENUMERATOR: u64 = 0x28;
const DW_TAG_ARRAY_TYPE: u64 = 0x01;
const DW_TAG_SUBRANGE_TYPE: u64 = 0x21;
const DW_TAG_SUBROUTINE_TYPE: u64 = 0x15;
const DW_TAG_FORMAL_PARAMETER: u64 = 0x05;
const DW_TAG_UNSPECIFIED_PARAMETERS: u64 = 0x18;
const DW_TAG_VARIABLE: u64 = 0x34;
const DW_AT_NAME: u64 = 0x03;
const DW_AT_STMT_LIST: u64 = 0x10;
const DW_AT_LOW_PC: u64 = 0x11;
const DW_AT_RANGES: u64 = 0x55;
const DW_AT_HIGH_PC: u64 = 0x12;
const DW_AT_LANGUAGE: u64 = 0x13;
const DW_AT_COMP_DIR: u64 = 0x1b;
const DW_AT_PRODUCER: u64 = 0x25;
const DW_AT_EXTERNAL: u64 = 0x3f;
const DW_AT_FRAME_BASE: u64 = 0x40;
const DW_AT_TYPE: u64 = 0x49;
/// `DW_AT_decl_file` and `DW_AT_decl_line`.
const DW_AT_DECL_FILE: u64 = 0x3a;
const DW_AT_DECL_LINE: u64 = 0x3b;
const DW_AT_DECL_COLUMN: u64 = 0x39;
/// `DW_AT_language_name` and `DW_AT_language_version`: the DWARF 6 attributes
/// GCC 14 and newer emit so a unit can say "C, 202311" where no language code
/// exists yet.
const DW_AT_LANGUAGE_NAME: u64 = 0x90;
const DW_AT_LANGUAGE_VERSION: u64 = 0x91;
const DW_AT_LOCATION: u64 = 0x02;
const DW_AT_BYTE_SIZE: u64 = 0x0b;
const DW_AT_ENCODING: u64 = 0x3e;
const DW_AT_DATA_MEMBER_LOCATION: u64 = 0x38;
const DW_AT_BIT_SIZE: u64 = 0x0d;
/// `DW_AT_bit_offset` is the version 2..3 spelling: the offset of the field's
/// most significant bit, counted from the most significant bit of the storage
/// unit.  `DW_AT_data_bit_offset` (version 4) counts from the least significant
/// bit of the object instead, which is the layout's own numbering.
const DW_AT_BIT_OFFSET: u64 = 0x0c;
const DW_AT_DATA_BIT_OFFSET: u64 = 0x6b;
const DW_AT_UPPER_BOUND: u64 = 0x2f;
const DW_AT_COUNT: u64 = 0x37;
const DW_AT_PROTOTYPED: u64 = 0x27;
const DW_AT_DECLARATION: u64 = 0x3c;
const DW_AT_CONST_VALUE: u64 = 0x1c;
/// `DW_AT_GNU_vector`: the object is a `vector_size` array whose lanes are not
/// subscriptable as an ordinary array.
const DW_AT_GNU_VECTOR: u64 = 0x2107;
const DW_FORM_ADDR: u64 = 0x01;
const DW_FORM_DATA1: u64 = 0x0b;
const DW_FORM_DATA2: u64 = 0x05;
const DW_FORM_DATA4: u64 = 0x06;
const DW_FORM_DATA8: u64 = 0x07;
const DW_FORM_STRING: u64 = 0x08;
const DW_FORM_FLAG: u64 = 0x0c;
const DW_FORM_UDATA: u64 = 0x0f;
const DW_FORM_REF4: u64 = 0x13;
/// `DW_FORM_ref_udata`: a ULEB offset from the unit's start -- the shortest
/// spelling of a reference to a DIE whose offset is only known once the unit is
/// laid out.
const DW_FORM_REF_UDATA: u64 = 0x15;
const DW_FORM_SEC_OFFSET: u64 = 0x17;
const DW_FORM_STRP: u64 = 0x0e;
/// `DW_FORM_line_strp`: an offset into `.debug_line_str`, the table a version-5
/// line program names its paths in.
const DW_FORM_LINE_STRP: u64 = 0x1f;
/// `DW_FORM_flag_present`: the attribute is true by its mere presence (DWARF 4).
const DW_FORM_FLAG_PRESENT: u64 = 0x19;
const DW_FORM_EXPRLOC: u64 = 0x18;
/// `DW_FORM_block1`: a one-byte length followed by the bytes -- the block form
/// versions 2 and 3 have for an expression-valued attribute.
const DW_FORM_BLOCK1: u64 = 0x0a;
/// `DW_FORM_sdata`: a signed LEB128, the one form that can carry a negative
/// enumerator value in every version.
const DW_FORM_SDATA: u64 = 0x0d;
/// `DW_OP_addr`: the address of an object, as a relocation the linker resolves.
const DW_OP_ADDR: u8 = 0x03;
/// `DW_OP_fbreg`: a signed displacement from the frame base of the enclosing
/// subprogram.  Version 2 and later, so every dialect this writer emits can use
/// it, and GCC 16.2 uses it for the same slots at `-O0`.
pub(crate) const DW_OP_FBREG: u8 = 0x91;
/// `DW_OP_plus_uconst`: the member offset a version 2 or 3 unit writes as a
/// block rather than as a constant.
const DW_OP_PLUS_UCONST: u8 = 0x23;
/// `DW_OP_call_frame_cfa`: "the frame base is the canonical frame address".
///
/// The assembler already synthesizes the `.eh_frame` FDE that defines the CFA
/// at every address of the function, so the expression costs one byte and stays
/// true wherever the frame pointer is omitted -- unlike the `DW_OP_reg6` clang
/// writes for `-O0`, which describes a register the optimized code may not
/// keep.  The opcode is DWARF 3, so a version 2 unit states no frame base at
/// all rather than describing its own prologue with an opcode it never defined;
/// gcc 16.2 pays a `.debug_loc` list for the same information there.
const DW_OP_CALL_FRAME_CFA: u8 = 0x9c;
/// `DW_LNCT_path` and `DW_LNCT_directory_index`: the content types of the
/// version-5 directory and file name tables.
const DW_LNCT_PATH: u64 = 0x01;
const DW_LNCT_DIRECTORY_INDEX: u64 = 0x02;
/// `DW_RLE_start_length` and `DW_RLE_end_of_list`: the two range-list entry
/// kinds a unit with several code sections needs.
const DW_RLE_START_LENGTH: u8 = 0x07;
const DW_RLE_END_OF_LIST: u8 = 0x00;
/// Write a `DW_AT_high_pc` constant in the form the unit declared for it.
fn push_high_pc(info: &mut Vec<u8>, form: u64, size: u64) {
    match form {
        DW_FORM_UDATA => write_uleb128(size, info),
        // `DW_FORM_addr` reaches here only with a relocation; the constant
        // shapes left are the fixed-width ones.
        _ => info.extend_from_slice(&size.to_le_bytes()),
    }
}

/// `DW_LANG_C89`, `DW_LANG_C99`, `DW_LANG_C11`.
const DW_LANG_C89: u16 = 0x0001;
const DW_LANG_C99: u16 = 0x000c;
const DW_LANG_C11: u16 = 0x001d;
/// `DW_LNAME_C`: the source language *name* code for C
/// (<https://dwarfstd.org/languages-v6.html>), paired with
/// `DW_AT_language_version`.  This is the name-namespace code, not
/// `DW_LANG_C` (0x0002): `DW_AT_language_name` speaks the former.
const DW_LNAME_C: u16 = 0x0003;
/// `DW_LANG_Mips_Assembler`, what GAS records for assembly sources.
const DW_LANG_MIPS_ASSEMBLER: u16 = 0x8001;

/// `DW_LNS_copy`, `DW_LNS_advance_pc`, `DW_LNS_advance_line`,
/// `DW_LNS_const_add_pc` and `DW_LNS_fixed_advance_pc`.
const DW_LNS_COPY: u8 = 1;
const DW_LNS_ADVANCE_PC: u8 = 2;
const DW_LNS_ADVANCE_LINE: u8 = 3;
const DW_LNS_CONST_ADD_PC: u8 = 8;
const DW_LNS_FIXED_ADVANCE_PC: u8 = 9;
/// The header this writer emits: `minimum_instruction_length` 1, `line_base`
/// -5, `line_range` 14 (see [`write_line_prologue_header`]).
const LINE_BASE: i64 = -5;
const LINE_RANGE: u64 = 14;

/// How one row's motion -- `advance` address units and `line_delta` lines -- is
/// encoded, and how the row itself is emitted.
///
/// A special opcode does both jobs in one byte, but only while the address
/// advance is small: `advance * line_range + (line_delta - line_base) +
/// opcode_base` must fit in a byte, which caps the advance at 16 or 17 units.
/// Real functions move further than that between statements, and then the choice
/// is not obvious: `DW_LNS_advance_pc` costs 2-4 bytes for the whole advance,
/// while `n` copies of `DW_LNS_const_add_pc` cost `n` bytes and move the machine
/// by `n * (255 - opcode_base) / line_range` (17 units) each, leaving a remainder
/// small enough for a final special opcode.  Measured against GNU as 2.40 on
/// gcc-generated assembly, the greedy "subtract 17 at a time" split -- what
/// binutils' `emit_inc_line_addr` does -- is 1 to 2 bytes per row smaller than
/// advancing the whole delta explicitly; this planner picks the cheapest of
/// every shape that exists, including that split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowMotion {
    /// A `DW_LNS_advance_line` the row needs first, when the final opcode cannot
    /// carry the line delta itself (it is outside `line_base..line_base +
    /// line_range`).
    line_advance: Option<i64>,
    /// The opcode that moves the machine to the row's address, if any.
    address: AddressMotion,
    /// The one-byte opcode that emits the row.
    row: u8,
}

/// The address half of a [`RowMotion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddressMotion {
    /// A special opcode moves the machine and emits the row.
    Special,
    /// Nothing to move.
    None,
    /// `DW_LNS_advance_pc` by `delta`.
    AdvancePc(u64),
    /// `DW_LNS_const_add_pc` `count` times, then a special opcode for the
    /// remainder.
    ConstAddPc(u32),
    /// `DW_LNS_fixed_advance_pc` by `delta` (3 bytes for any 16-bit delta).
    FixedAdvancePc(u64),
}

impl AddressMotion {
    fn push(self, out: &mut Vec<u8>) {
        match self {
            Self::Special | Self::None => {}
            Self::AdvancePc(delta) => {
                out.push(DW_LNS_ADVANCE_PC);
                write_uleb128(delta, out);
            }
            Self::FixedAdvancePc(delta) => {
                out.push(DW_LNS_FIXED_ADVANCE_PC);
                out.extend_from_slice(&(delta as u16).to_le_bytes());
            }
            Self::ConstAddPc(count) => {
                for _ in 0..count {
                    out.push(DW_LNS_CONST_ADD_PC);
                }
            }
        }
    }

    fn cost(self) -> usize {
        match self {
            Self::Special | Self::None => 0,
            Self::AdvancePc(delta) => 1 + uleb128_len(delta),
            Self::FixedAdvancePc(_) => 3,
            Self::ConstAddPc(count) => count as usize,
        }
    }
}

/// Bytes `value` occupies as ULEB128.
fn uleb128_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

/// Bytes `value` occupies as SLEB128.
fn sleb128_len(mut value: i64) -> usize {
    let mut len = 1;
    loop {
        let byte = (value as u8) & 0x7f;
        value >>= 7;
        let sign_bit = byte & 0x40 != 0;
        if (value == 0 && !sign_bit) || (value == -1 && sign_bit) {
            return len;
        }
        len += 1;
    }
}

/// Address units one `DW_LNS_const_add_pc` moves the machine by.
fn const_add_pc_step(opcode_base: u64) -> u64 {
    (255 - opcode_base) / LINE_RANGE
}

impl RowMotion {
    /// The cheapest encoding of a row at `advance` address units and
    /// `line_delta` lines from the current position.
    fn plan(advance: u64, line_delta: i64, opcode_base: u64) -> Self {
        // The shape that can state any position: `DW_LNS_advance_pc`, plus
        // `DW_LNS_advance_line` when the line moves, plus `DW_LNS_copy`.  It is
        // the starting bid, so the planner always has an answer without an
        // unreachable branch.
        let fallback = Self {
            line_advance: (line_delta != 0).then_some(line_delta),
            address: if advance == 0 {
                AddressMotion::None
            } else {
                AddressMotion::AdvancePc(advance)
            },
            row: DW_LNS_COPY,
        };
        let mut best = (Self::cost_of(&fallback), fallback);
        let mut consider = |cost: usize, motion: Self| {
            if cost < best.0 {
                best = (cost, motion);
            }
        };
        // 1. One opcode for both: the special opcode.  2. One opcode for the
        //    line and a special for the address, when the line does not fit.
        if let Some(op) = line_special_opcode(advance, line_delta, opcode_base) {
            consider(
                1,
                Self {
                    line_advance: None,
                    address: AddressMotion::Special,
                    row: op,
                },
            );
        }
        // 3. Address by `DW_LNS_const_add_pc` steps plus a special opcode.
        for (line_advance, delta) in [(None, line_delta), (Some(line_delta), 0)] {
            if line_advance.is_some() && line_delta == 0 {
                continue;
            }
            let line_cost = line_advance.map_or(0, |delta| 1 + sleb128_len(delta));
            // Every remainder the split can leave behind, cheapest count first.
            let step = const_add_pc_step(opcode_base);
            if step > 0 {
                // `k` steps leave `advance - k * step`; the remainder has to fit
                // a special opcode, and a smaller `k` is only worth it when the
                // remainder still fits.
                let max_tail = (0..=step)
                    .rev()
                    .find(|tail| line_special_opcode(*tail, delta, opcode_base).is_some());
                if let Some(max_tail) = max_tail {
                    let count = advance.saturating_sub(max_tail).div_ceil(step);
                    let tail = advance - count * step;
                    if count <= u32::MAX as u64
                        && advance >= count * step
                        && let Some(op) = line_special_opcode(tail, delta, opcode_base)
                    {
                        consider(
                            line_cost + count as usize + 1,
                            Self {
                                line_advance,
                                address: AddressMotion::ConstAddPc(count as u32),
                                row: op,
                            },
                        );
                    }
                }
            }
            // 4. Address by one explicit opcode, row by a special or a copy.
            let address_ops = [
                AddressMotion::AdvancePc(advance),
                AddressMotion::FixedAdvancePc(advance),
            ];
            for address in address_ops {
                if let AddressMotion::FixedAdvancePc(delta) = address
                    && delta > u64::from(u16::MAX)
                {
                    continue;
                }
                // The special opcode still carries the line when the address is
                // moved explicitly; otherwise the line was advanced above.
                let row_and_line = line_special_opcode(0, delta, opcode_base);
                if let Some(op) = row_and_line {
                    consider(
                        line_cost + address.cost() + 1,
                        Self {
                            line_advance,
                            address,
                            row: op,
                        },
                    );
                }
                if delta == 0 {
                    consider(
                        line_cost + address.cost() + 1,
                        Self {
                            line_advance,
                            address,
                            row: DW_LNS_COPY,
                        },
                    );
                }
            }
        }
        // 5. Nothing to move at all: `DW_LNS_copy` states the row.
        if advance == 0 && line_delta == 0 {
            consider(
                1,
                Self {
                    line_advance: None,
                    address: AddressMotion::None,
                    row: DW_LNS_COPY,
                },
            );
        }
        best.1
    }

    /// Bytes `self` occupies, one byte for the row opcode plus its motion.
    fn cost_of(motion: &Self) -> usize {
        1 + motion.line_advance.map_or(0, |d| 1 + sleb128_len(d)) + motion.address.cost()
    }
}

/// Encode a special opcode for the header this writer emits: minimum
/// instruction length 1, line base -5, line range 14, and `opcode_base`
/// (13 since DWARF 3, 10 in DWARF 2). Return None when the pair does not fit.
fn line_special_opcode(address_delta: u64, line_delta: i64, opcode_base: u64) -> Option<u8> {
    if !(LINE_BASE..LINE_BASE + LINE_RANGE as i64).contains(&line_delta) {
        return None;
    }
    address_delta
        .checked_mul(14)?
        .checked_add((line_delta + 5) as u64)?
        .checked_add(opcode_base)
        .and_then(|value| u8::try_from(value).ok())
}

fn write_uleb128(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

pub(crate) fn write_sleb128(mut value: i64, out: &mut Vec<u8>) {
    loop {
        let byte = (value as u8) & 0x7f;
        value >>= 7;
        let sign_bit = byte & 0x40 != 0;
        let done = (value == 0 && !sign_bit) || (value == -1 && sign_bit);
        out.push(if done { byte } else { byte | 0x80 });
        if done {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABS32: u32 = 10; // R_X86_64_32
    const ABS64: u32 = 1; // R_X86_64_64

    /// A unit with nothing in it: what `-g` on hand-written assembly describes.
    fn no_unit() -> DwarfUnitSpec {
        DwarfUnitSpec::default()
    }

    /// The object file defines no object, so every variable DIE is a declaration.
    fn no_defined_objects() -> FxHashMap<String, ()> {
        FxHashMap::default()
    }

    fn text_section() -> Vec<DwarfSectionInfo<'static>> {
        vec![DwarfSectionInfo {
            name: ".text",
            size: 0x40,
            is_executable: true,
        }]
    }

    /// Two code sections, the shape `-ffunction-sections` produces: `.text` of
    /// 0x40 bytes and a 0x20-byte `.text.unlikely` after it.
    fn split_sections() -> Vec<DwarfSectionInfo<'static>> {
        vec![
            DwarfSectionInfo {
                name: ".text",
                size: 0x40,
                is_executable: true,
            },
            DwarfSectionInfo {
                name: ".text.unlikely",
                size: 0x20,
                is_executable: true,
            },
        ]
    }

    /// One row per code section, at offset 0 and 0x10.
    fn split_rows() -> Vec<DwarfLineLocation> {
        vec![
            row(0, 1, 1),
            DwarfLineLocation {
                section_index: 1,
                offset: 0x10,
                file_number: 1,
                line: 3,
                column: 0,
                options: DwarfLocOptions::default(),
                sequence: 1,
            },
        ]
    }

    fn row(offset: u64, file_number: u32, line: u32) -> DwarfLineLocation {
        DwarfLineLocation {
            section_index: 0,
            offset,
            file_number,
            line,
            column: 0,
            options: DwarfLocOptions::default(),
            sequence: offset,
        }
    }

    fn read_uleb(data: &[u8], at: &mut usize) -> u64 {
        let mut value = 0u64;
        let mut shift = 0;
        loop {
            let byte = data[*at];
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    }

    fn read_u16(data: &[u8], at: &mut usize) -> u16 {
        let value = u16::from_le_bytes([data[*at], data[*at + 1]]);
        *at += 2;
        value
    }

    fn read_u32(data: &[u8], at: &mut usize) -> u32 {
        let value = u32::from_le_bytes(data[*at..*at + 4].try_into().unwrap());
        *at += 4;
        value
    }

    fn read_u64(data: &[u8], at: &mut usize) -> u64 {
        let value = u64::from_le_bytes(data[*at..*at + 8].try_into().unwrap());
        *at += 8;
        value
    }

    /// Relocation addends of a section, keyed by the offset they patch.  A
    /// `DW_FORM_strp`/`DW_FORM_line_strp` field is zero in the section body and
    /// its real string offset lives in a relocation (which is what lets the
    /// linker merge two objects' string tables), so a reader has to resolve it
    /// the way a linker does.
    fn reloc_addends(section: &ObjSection, symbol: &str) -> Vec<(u64, i64)> {
        section
            .relocs
            .iter()
            .filter(|reloc| reloc.symbol_name == symbol)
            .map(|reloc| (reloc.offset, reloc.addend))
            .collect()
    }

    /// The two string sections a unit can point into.  `.debug_info` holds the
    /// relocations that carry the real offsets, so the table a form addresses is
    /// chosen by the form, exactly as a debugger does it.
    struct Strings<'a> {
        debug_str: &'a [u8],
        debug_str_addends: Vec<(u64, i64)>,
        line_str: &'a [u8],
        line_addends: Vec<(u64, i64)>,
    }

    impl<'a> Strings<'a> {
        /// Both tables as the linker would see them, from the `.debug_info`
        /// relocations that patch each `strp`/`line_strp` field.
        fn of(
            info: &ObjSection,
            debug_str: Option<&'a ObjSection>,
            line_str: Option<&'a ObjSection>,
        ) -> Self {
            Self {
                debug_str: debug_str.map_or(&[], |section| &section.data[..]),
                debug_str_addends: reloc_addends(info, ".debug_str"),
                line_str: line_str.map_or(&[], |section| &section.data[..]),
                line_addends: reloc_addends(info, ".debug_line_str"),
            }
        }
    }

    /// The string a `DW_FORM_strp`/`DW_FORM_line_strp` field names.
    fn read_strp(data: &[u8], at: &mut usize, strings: &[u8], addends: &[(u64, i64)]) -> String {
        let field = *at as u64;
        let inline = read_u32(data, at) as usize;
        let offset = addends
            .iter()
            .find(|(offset, _)| *offset == field)
            .map_or(inline, |(_, addend)| *addend as usize);
        let end = strings[offset..]
            .iter()
            .position(|&b| b == 0)
            .map(|end: usize| offset + end)
            .unwrap_or(strings.len());
        String::from_utf8_lossy(&strings[offset..end]).into_owned()
    }

    /// Skip one attribute of `form`, returning `(numeric value, string)`.
    fn skip_attribute(
        form: u64,
        data: &[u8],
        at: &mut usize,
        strings: &Strings<'_>,
        address_size: usize,
    ) -> (u64, Option<String>) {
        match form {
            DW_FORM_STRING => {
                let end = data[*at..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|end: usize| *at + end)
                    .unwrap();
                let text = String::from_utf8_lossy(&data[*at..end]).into_owned();
                *at = end + 1;
                (0, Some(text))
            }
            // The form, not the writer, says which table the offset addresses.
            DW_FORM_STRP => {
                let text = read_strp(data, at, strings.debug_str, &strings.debug_str_addends);
                (0, Some(text))
            }
            DW_FORM_LINE_STRP => {
                let text = read_strp(data, at, strings.line_str, &strings.line_addends);
                (0, Some(text))
            }
            DW_FORM_ADDR => (read_u64(data, at), None),
            DW_FORM_FLAG_PRESENT => (1, None),
            DW_FORM_DATA1 | DW_FORM_FLAG => {
                let value = u64::from(data[*at]);
                *at += 1;
                (value, None)
            }
            DW_FORM_UDATA | DW_FORM_REF_UDATA => (read_uleb(data, at), None),
            // A block is a length and that many bytes; the writer emits exactly
            // one (`DW_OP_call_frame_cfa`), and the reader hands back the
            // expression so a test can assert on it.
            DW_FORM_EXPRLOC => {
                let length = read_uleb(data, at) as usize;
                let expression = data[*at..].get(..length).unwrap_or_default();
                let value = expression
                    .iter()
                    .fold(0u64, |value, &byte| (value << 8) | u64::from(byte));
                *at += length;
                (value, None)
            }
            DW_FORM_DATA2 => (u64::from(read_u16(data, at)), None),
            DW_FORM_DATA4 | DW_FORM_SEC_OFFSET | DW_FORM_REF4 => {
                (u64::from(read_u32(data, at)), None)
            }
            DW_FORM_DATA8 => (read_u64(data, at), None),
            other => panic!(
                "test reader has no rule for form {other:#x} at offset {at} (address size {address_size})"
            ),
        }
    }

    /// One `DW_TAG_subprogram` of the first unit, as the sections encode it.
    #[derive(Debug, PartialEq, Eq)]
    struct Subprogram {
        name: String,
        external: bool,
        declaration: Option<(u64, u64)>,
    }

    /// Every abbreviation the section declares:
    /// `(code, tag, [(attribute, form)])`.
    fn abbreviation_table(abbrev: &[u8]) -> Vec<(u64, u64, Vec<(u64, u64)>)> {
        let mut table = Vec::new();
        let mut cursor = 0;
        loop {
            let code = read_uleb(abbrev, &mut cursor);
            if code == 0 {
                return table;
            }
            let tag = read_uleb(abbrev, &mut cursor);
            cursor += 1; // children flag
            let mut attributes = Vec::new();
            loop {
                let at_code = read_uleb(abbrev, &mut cursor);
                let form = read_uleb(abbrev, &mut cursor);
                if at_code == 0 && form == 0 {
                    break;
                }
                attributes.push((at_code, form));
            }
            table.push((code, tag, attributes));
        }
    }

    /// `(tag, name, declares children)` of every DIE of the first unit, in
    /// order and with children flattened, plus a `TERMINATOR` entry for each
    /// child list that ends.  `unit_contents` stops at the first terminator,
    /// which is all a unit of parameterless functions has; a unit with
    /// parameters needs the children themselves.
    fn die_sequence(
        info: &[u8],
        abbrev: &[u8],
        strings: &Strings<'_>,
        address_size: usize,
    ) -> Vec<(u64, String, bool)> {
        let mut table = Vec::new();
        let mut children = FxHashMap::default();
        let mut cursor = 0;
        loop {
            let code = read_uleb(abbrev, &mut cursor);
            if code == 0 {
                break;
            }
            let tag = read_uleb(abbrev, &mut cursor);
            children.insert(code, abbrev[cursor] != 0);
            cursor += 1;
            let mut attributes = Vec::new();
            loop {
                let at_code = read_uleb(abbrev, &mut cursor);
                let form = read_uleb(abbrev, &mut cursor);
                if at_code == 0 && form == 0 {
                    break;
                }
                attributes.push((at_code, form));
            }
            table.push((code, tag, attributes));
        }
        let lookup = |code: u64| {
            table
                .iter()
                .find(|(declared, _, _)| *declared == code)
                .unwrap_or_else(|| panic!("no abbreviation {code} declared"))
        };
        let mut at = 0;
        let _unit_length = read_u32(info, &mut at);
        let version = read_u16(info, &mut at);
        if version >= 5 {
            at += 2;
        }
        let _abbrev_offset = read_u32(info, &mut at);
        if version < 5 {
            at += 1;
        }
        let mut out = Vec::new();
        loop {
            let code = read_uleb(info, &mut at);
            if code == 0 {
                out.push((TERMINATOR, String::new(), false));
                if at >= info.len() {
                    return out;
                }
                continue;
            }
            let (_, tag, attributes) = lookup(code);
            let mut name = String::new();
            for (at_code, form) in attributes.clone() {
                let (_, text) = skip_attribute(form, info, &mut at, strings, address_size);
                if at_code == DW_AT_NAME && text.is_some() {
                    name = text.unwrap();
                }
            }
            out.push((*tag, name, children[&code]));
        }
    }

    /// The tag a DIE list ends with; no DWARF tag is `u64::MAX`.
    const TERMINATOR: u64 = u64::MAX;

    /// A defined function's parameters are children of its subprogram DIE, with
    /// the name and the type a debugger prints; a function that takes none
    /// declares no children, so the common shape stays one byte shorter.
    #[test]
    fn a_subprogram_lists_its_parameters_as_children() {
        use crate::backend::elf::dwarf_spec::{
            BaseKind, DwarfFunctionParameters, DwarfParameter, DwarfType,
        };
        let files = [(1, "b.c".to_string())];
        let rows = [row(0, 1, 2)];
        let functions = [
            DwarfFunction {
                name: "with_params",
                section_index: 0,
                offset: 0,
                size: 0x10,
                external: true,
            },
            DwarfFunction {
                name: "no_params",
                section_index: 0,
                offset: 0x10,
                size: 0x10,
                external: true,
            },
            DwarfFunction {
                name: "variadic",
                section_index: 0,
                offset: 0x20,
                size: 0x20,
                external: true,
            },
        ];
        // `int`, `char`, `char *`: enough for `int with_params(int a, char *b)`.
        let types = vec![
            DwarfType::Base {
                kind: BaseKind::Int,
                size: 4,
            },
            DwarfType::Base {
                kind: BaseKind::Char,
                size: 1,
            },
            DwarfType::Pointer {
                pointee: Some(1),
                size: 8,
            },
        ];
        let mut parameters: FxHashMap<String, DwarfFunctionParameters> = FxHashMap::default();
        parameters.insert(
            "with_params".to_string(),
            DwarfFunctionParameters {
                parameters: vec![
                    DwarfParameter {
                        name: Some("a".to_string()),
                        ty: 0,
                        location: None,
                    },
                    DwarfParameter {
                        name: Some("b".to_string()),
                        ty: 2,
                        location: None,
                    },
                ],
                variadic: false,
            },
        );
        parameters.insert(
            "variadic".to_string(),
            DwarfFunctionParameters {
                parameters: vec![DwarfParameter {
                    name: Some("count".to_string()),
                    ty: 0,
                    location: None,
                }],
                variadic: true,
            },
        );
        let unit = DwarfUnitSpec {
            types,
            variables: Vec::new(),
            return_types: FxHashMap::default(),
            parameters,
        };
        for version in 2..=5 {
            let config = DwarfConfig::for_c(version, None);
            let mut strings = DwarfStrings::new();
            let mut line_strings = DwarfStrings::new();
            let line = build_dwarf_line_section(
                &files,
                &rows,
                &text_section(),
                8,
                ABS64,
                ABS32,
                "/src",
                config,
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap();
            let info = build_dwarf_info_sections(
                &line.code_sections,
                Some(0),
                &files,
                &rows,
                &text_section(),
                &functions,
                "b.c",
                "/src",
                "p",
                8,
                ABS32,
                ABS64,
                config,
                &mut strings,
                &mut line_strings,
                &unit,
                &no_defined_objects(),
                None,
            )
            .unwrap()
            .unwrap();
            // DWARF 2 and 3 spell a string inline, so neither section exists there.
            let strings = strings.into_section(".debug_str");
            let line_strings = line_strings.into_section(".debug_line_str");
            let tables = Strings::of(&info.info, strings.as_ref(), line_strings.as_ref());
            let dies = die_sequence(&info.info.data, &info.abbrev.data, &tables, 8);
            // The whole unit, in order: the unit DIE, the `unspecified_type` the
            // unit's unknown return types point at, the subprograms with their
            // children, then the type table and the unit's terminator.
            let expected: [(u64, &str, bool); 15] = [
                (DW_TAG_COMPILE_UNIT, "b.c", true),
                (DW_TAG_UNSPECIFIED_TYPE, "", false),
                (DW_TAG_SUBPROGRAM, "with_params", true),
                (DW_TAG_FORMAL_PARAMETER, "a", false),
                (DW_TAG_FORMAL_PARAMETER, "b", false),
                (TERMINATOR, "", false),
                (DW_TAG_SUBPROGRAM, "no_params", false),
                (DW_TAG_SUBPROGRAM, "variadic", true),
                (DW_TAG_FORMAL_PARAMETER, "count", false),
                (DW_TAG_UNSPECIFIED_PARAMETERS, "", false),
                (TERMINATOR, "", false),
                (DW_TAG_BASE_TYPE, "int", false),
                (DW_TAG_BASE_TYPE, "char", false),
                (DW_TAG_POINTER_TYPE, "", false),
                (TERMINATOR, "", false),
            ];
            let found: Vec<(u64, &str, bool)> = dies
                .iter()
                .map(|(tag, name, children)| (*tag, name.as_str(), *children))
                .collect();
            assert_eq!(found, expected, "version {version}: {dies:?}");
        }
    }

    /// Walk the first unit's `DW_TAG_compile_unit` and its subprograms, so a
    /// test can assert the *values* a debugger would read.
    fn unit_contents(
        info: &[u8],
        abbrev: &[u8],
        strings: &Strings<'_>,
        address_size: usize,
    ) -> (Vec<(u64, u64)>, Vec<Subprogram>) {
        let table = abbreviation_table(abbrev);
        let mut at = 0;
        let _unit_length = read_u32(info, &mut at);
        let version = read_u16(info, &mut at);
        if version >= 5 {
            at += 2; // unit_type, address_size
        }
        let _abbrev_offset = read_u32(info, &mut at);
        if version < 5 {
            at += 1; // address_size
        }
        let attributes = |code: u64| {
            table
                .iter()
                .find(|(declared, _, _)| *declared == code)
                .unwrap_or_else(|| panic!("no abbreviation {code} declared"))
                .2
                .clone()
        };
        let tag_of = |code: u64| {
            table
                .iter()
                .find(|(declared, _, _)| *declared == code)
                .map(|(_, tag, _)| *tag)
                .unwrap_or_else(|| panic!("no abbreviation {code} declared"))
        };

        let cu_code = read_uleb(info, &mut at);
        let mut cu = Vec::new();
        for (at_code, form) in attributes(cu_code) {
            cu.push((
                at_code,
                skip_attribute(form, info, &mut at, strings, address_size).0,
            ));
        }

        let mut subprograms = Vec::new();
        loop {
            let code = read_uleb(info, &mut at);
            if code == 0 {
                break;
            }
            let (mut name, mut external, mut declaration) = (String::new(), false, [0u64; 2]);
            let mut has_declaration = false;
            let is_subprogram = tag_of(code) == DW_TAG_SUBPROGRAM;
            for (at_code, form) in attributes(code) {
                let (value, text) = skip_attribute(form, info, &mut at, strings, address_size);
                match at_code {
                    DW_AT_NAME => name = text.unwrap(),
                    DW_AT_EXTERNAL => external = value != 0,
                    DW_AT_DECL_FILE => {
                        declaration[0] = value;
                        has_declaration = true;
                    }
                    DW_AT_DECL_LINE => declaration[1] = value,
                    _ => {}
                }
            }
            if is_subprogram {
                subprograms.push(Subprogram {
                    name,
                    external,
                    declaration: has_declaration.then_some((declaration[0], declaration[1])),
                });
            }
        }
        (cu, subprograms)
    }

    /// The row planner states each position with as few bytes as the dialect
    /// allows, and the bytes it plans are the bytes it writes.  Every shape a
    /// line program can hold is walked, and each plan is replayed against the
    /// header's own constants: it must land exactly on the requested position,
    /// cost exactly what it claims, and never lose to the three-opcode fallback
    /// (`DW_LNS_advance_pc`, `DW_LNS_advance_line`, `DW_LNS_copy`) that can state
    /// anything.
    #[test]
    fn row_motion_round_trips_and_never_loses_to_advance_pc() {
        for opcode_base in [10u64, 13] {
            for advance in [0u64, 1, 16, 17, 18, 33, 100, 255, 1000, 70_000] {
                for line_delta in [-7i64, -5, -1, 0, 1, 8, 9, 100, -100] {
                    let plan = RowMotion::plan(advance, line_delta, opcode_base);
                    let mut bytes = Vec::new();
                    if let Some(delta) = plan.line_advance {
                        bytes.push(DW_LNS_ADVANCE_LINE);
                        write_sleb128(delta, &mut bytes);
                    }
                    plan.address.push(&mut bytes);
                    bytes.push(plan.row);
                    let planned: usize = 1
                        + plan.line_advance.map_or(0, |d| 1 + sleb128_len(d))
                        + plan.address.cost();
                    assert_eq!(bytes.len(), planned, "{advance} {line_delta}: {plan:?}");
                    let mut address = 0u64;
                    let mut line = 0i64;
                    if let Some(delta) = plan.line_advance {
                        line += delta;
                    }
                    match plan.address {
                        AddressMotion::AdvancePc(delta) | AddressMotion::FixedAdvancePc(delta) => {
                            address += delta;
                        }
                        AddressMotion::ConstAddPc(count) => {
                            address += u64::from(count) * const_add_pc_step(opcode_base);
                        }
                        AddressMotion::Special | AddressMotion::None => {}
                    }
                    if plan.row != DW_LNS_COPY {
                        let adjusted = u64::from(plan.row) - opcode_base;
                        address += adjusted / LINE_RANGE;
                        line += LINE_BASE + (adjusted % LINE_RANGE) as i64;
                    }
                    assert_eq!(
                        (address, line),
                        (advance, line_delta),
                        "{advance} {line_delta} {opcode_base}: {plan:?}"
                    );
                    let naive: usize = 1
                        + uleb128_len(advance)
                        + if line_delta != 0 {
                            1 + sleb128_len(line_delta)
                        } else {
                            0
                        }
                        + 1;
                    assert!(
                        bytes.len() <= naive,
                        "{advance} {line_delta}: {} bytes loses to the naive {naive}: {plan:?}",
                        bytes.len()
                    );
                }
            }
        }
    }

    #[test]
    fn file_and_loc_operands_parse_like_gnu_as() {
        assert_eq!(
            parse_file_directive("1 \"a.c\"").unwrap(),
            Some((1, "a.c".to_string()))
        );
        // The escapes the compiler writes (`escape_dwarf_path`) come back.
        assert_eq!(
            parse_file_directive(r#"2 "dir\\sub/q\"x.c""#).unwrap(),
            Some((2, r#"dir\sub/q"x.c"#.to_string()))
        );
        assert_eq!(
            parse_file_directive("3 \"/src/\" \"b.c\" md5 0x00").unwrap(),
            Some((3, "/src/b.c".to_string()))
        );
        assert_eq!(
            parse_file_directive("4 \"/src\" \"/abs/c.c\"").unwrap(),
            Some((4, "/abs/c.c".to_string()))
        );
        assert_eq!(parse_file_directive("\"unit.c\"").unwrap(), None);
        assert!(parse_file_directive("x \"a.c\"").is_err());
        assert!(parse_file_directive("1 \"open").is_err());
        assert_eq!(
            parse_loc_directive("1 7 3").unwrap(),
            (1, 7, 3, DwarfLocOptions::default())
        );
        assert!(
            parse_loc_directive("1 7 prologue_end")
                .unwrap()
                .3
                .prologue_end
        );
        assert_eq!(
            parse_loc_directive("2 9 0 is_stmt 0").unwrap().3.is_stmt,
            Some(false)
        );
        let (_, _, _, options) = parse_loc_directive(
            "2 9 4 basic_block prologue_end epilogue_begin discriminator 257 isa 3 is_stmt 0",
        )
        .unwrap();
        assert_eq!(
            options,
            DwarfLocOptions {
                is_stmt: Some(false),
                isa: Some(3),
                basic_block: true,
                prologue_end: true,
                epilogue_begin: true,
                discriminator: Some(257),
            }
        );
        for invalid in ["1", "1 2 is_stmt 2", "1 2 isa", "1 2 discriminator -1"] {
            assert!(parse_loc_directive(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn line_special_opcode_matches_the_header_it_encodes_against() {
        // line_base -5, line_range 14; opcode_base 13 from DWARF 3 on, 10 for
        // DWARF 2 (which has three fewer standard opcodes).
        assert_eq!(line_special_opcode(0, -5, 13), Some(13));
        assert_eq!(line_special_opcode(0, -5, 10), Some(10));
        assert_eq!(line_special_opcode(1, -5, 13), Some(27));
        assert_eq!(line_special_opcode(1, -5, 10), Some(24));
        // The largest encodable row: opcode 255 at both opcode bases, three
        // address advances further apart than the version 3+ table allows.
        assert_eq!(line_special_opcode(16, 8, 13), Some(250));
        assert_eq!(line_special_opcode(17, -5, 13), Some(251));
        assert_eq!(line_special_opcode(17, -2, 13), Some(254));
        assert_eq!(line_special_opcode(17, -1, 13), Some(255));
        assert_eq!(line_special_opcode(17, 0, 13), None);
        assert_eq!(line_special_opcode(17, 8, 13), None);
        assert_eq!(line_special_opcode(18, -5, 13), None);
        assert_eq!(line_special_opcode(0, -6, 13), None);
        assert_eq!(line_special_opcode(0, 9, 13), None);
        assert_eq!(line_special_opcode(u64::MAX, 0, 13), None);
        // With nine standard opcodes the same row is three lower, so 17
        // advances reach 255 at `line_delta` 2 where 18 advances cannot.
        assert_eq!(line_special_opcode(17, -5, 10), Some(248));
        assert_eq!(line_special_opcode(17, 2, 10), Some(255));
        assert_eq!(line_special_opcode(17, 3, 10), None);
        assert_eq!(line_special_opcode(18, -5, 10), None);
    }

    /// DWARF 5 is the default dialect, so the same records produce the v5 unit
    /// and prologue headers: unit type, address size and format-described
    /// tables in the unit header, `DW_AT_language_name`/`_version` in the
    /// compile unit, and string-table references throughout.
    #[test]
    fn a_synthesized_unit_points_at_its_line_program_and_code() {
        let files = [(1, "b.c".to_string())];
        // The single row sits on `helper`'s entry, so it becomes that
        // subprogram's declaration.
        let rows = [DwarfLineLocation {
            section_index: 0,
            offset: 0x10,
            file_number: 1,
            line: 2,
            column: 5,
            options: DwarfLocOptions::default(),
            sequence: 0,
        }];
        let config = DwarfConfig::for_c(5, Some(Some("202311L")));
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &text_section(),
            8,
            ABS64,
            ABS32,
            "/src",
            config,
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(line.code_sections, [0], "one sequence for one code section");
        let mut at = 4;
        assert_eq!(read_u16(&line.line.data, &mut at), 5, "DWARF 5 line table");
        assert_eq!(line.line.data[at], 8, "address size");
        assert_eq!(line.line.data[at + 1], 0, "segment selector size");

        let functions = [
            DwarfFunction {
                name: "helper",
                section_index: 0,
                offset: 0x10,
                size: 0x30,
                external: true,
            },
            // Unsized and out-of-range symbols get no subprogram.
            DwarfFunction {
                name: "nosize",
                section_index: 0,
                offset: 0,
                size: 0,
                external: false,
            },
            DwarfFunction {
                name: "past_end",
                section_index: 0,
                offset: 0x30,
                size: 0x20,
                external: false,
            },
        ];
        let info = build_dwarf_info_sections(
            &line.code_sections,
            Some(0),
            &files,
            &rows,
            &text_section(),
            &functions,
            "b.c",
            "/src",
            "LCCC test",
            8,
            ABS32,
            ABS64,
            config,
            &mut strings,
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            None,
        )
        .unwrap()
        .unwrap();
        let d = &info.info.data;
        let unit_length = u32::from_le_bytes(d[..4].try_into().unwrap()) as usize;
        assert_eq!(unit_length + 4, d.len(), "one unit spans the section");
        assert_eq!(u16::from_le_bytes([d[4], d[5]]), 5, "DWARF 5");
        assert_eq!(d[6], DW_UT_COMPILE, "DW_UT_compile");
        assert_eq!(d[7], 8, "address size");
        let relocs: Vec<_> = info
            .info
            .relocs
            .iter()
            .map(|r| (r.offset, r.reloc_type, r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(relocs[0], (8, ABS32, ".debug_abbrev", 0));
        // The string fields are zero in the section body; their offsets travel
        // as relocations against the string sections, so only address and
        // section values are compared here, and only by target rather than by
        // byte offset so that a form choice cannot break the test.
        let address_relocs: Vec<_> = relocs
            .iter()
            .filter(|(_, _, symbol, _)| !matches!(*symbol, ".debug_str" | ".debug_line_str"))
            .map(|(_, reloc_type, symbol, addend)| (*reloc_type, *symbol, *addend))
            .collect();
        assert_eq!(
            address_relocs,
            [
                (ABS32, ".debug_abbrev", 0),
                (ABS64, ".text", 0),
                (ABS32, ".debug_line", 0),
                (ABS64, ".text", 0x10),
            ],
            "unit low_pc, line program, and `helper` alone: {relocs:?}"
        );
        assert!(
            relocs.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "relocations are emitted in ascending offset order: {relocs:?}"
        );
        // A version 5 unit names itself and its compilation directory through
        // the line string table: `b.c` is file entry 1 there and `/src` is
        // directory entry 0, so both are already interned.
        let line_string_relocs: Vec<_> = relocs
            .iter()
            .filter(|(_, _, symbol, _)| *symbol == ".debug_line_str")
            .map(|(offset, _, _, addend)| (*offset, *addend))
            .collect();
        assert_eq!(
            line_string_relocs,
            [(13, 5), (17, 0)],
            "unit name and compilation directory share `.debug_line_str`: {relocs:?}"
        );
        // The producer and every function name have no home in the line table.
        let string_relocs: Vec<_> = relocs
            .iter()
            .filter(|(_, _, symbol, _)| *symbol == ".debug_str")
            .map(|(offset, _, _, addend)| (*offset, *addend))
            .collect();
        assert_eq!(
            string_relocs,
            [(21, 0), (46, 10)],
            // `helper` returns nothing, so its `DW_AT_type` is a ULEB128
            // reference to the unit's one-byte `void` DIE instead of a
            // relocation-sized address: the name that follows sits seven bytes
            // earlier than it would otherwise.
            "producer then function name: {relocs:?}"
        );

        let strings = strings.into_section(".debug_str").expect(".debug_str");
        let line_strings = line_strings
            .into_section(".debug_line_str")
            .expect(".debug_line_str");
        let tables = Strings::of(&info.info, Some(&strings), Some(&line_strings));
        let (cu, subprograms) = unit_contents(d, &info.abbrev.data, &tables, 8);
        assert_eq!(
            cu,
            [
                (DW_AT_NAME, 0),
                (DW_AT_COMP_DIR, 0),
                (DW_AT_PRODUCER, 0),
                (DW_AT_LANGUAGE, u64::from(DW_LANG_C11)),
                (DW_AT_LANGUAGE_NAME, u64::from(DW_LNAME_C)),
                (DW_AT_LANGUAGE_VERSION, 202311),
                (DW_AT_LOW_PC, 0),
                (DW_AT_HIGH_PC, 0x40),
                (DW_AT_STMT_LIST, 0),
            ]
        );
        assert_eq!(
            subprograms,
            [Subprogram {
                name: "helper".to_string(),
                external: true,
                declaration: Some((1, 2)),
            }],
            "no subprogram for unsized or out-of-range symbols"
        );
        let text = String::from_utf8_lossy(&strings.data);
        assert_eq!(
            text, "LCCC test\0helper\0",
            "only names without a home in the line table"
        );
        assert_eq!(
            String::from_utf8_lossy(&line_strings.data),
            "/src\0b.c\0",
            "the compilation directory and the unit's file are stored once"
        );
        assert!(!text.contains("nosize") && !text.contains("past_end"));
        assert_eq!(*info.abbrev.data.last().unwrap(), 0, "abbrev terminated");
        assert!(info.abbrev.relocs.is_empty());
        let cu_forms: Vec<_> = abbreviation_table(&info.abbrev.data)
            .into_iter()
            .find(|(_, tag, _)| *tag == DW_TAG_COMPILE_UNIT)
            .map(|(_, _, attrs)| attrs)
            .unwrap();
        let form_of = |attribute: u64| {
            cu_forms
                .iter()
                .find(|(at, _)| *at == attribute)
                .map(|(_, form)| *form)
        };
        // The unit's own paths are line-string offsets, the producer is a
        // `.debug_str` offset, and a C language code is one byte: the forms GCC
        // 16.2 declares for the same unit.
        assert_eq!(form_of(DW_AT_NAME), Some(DW_FORM_LINE_STRP));
        assert_eq!(form_of(DW_AT_COMP_DIR), Some(DW_FORM_LINE_STRP));
        assert_eq!(form_of(DW_AT_PRODUCER), Some(DW_FORM_STRP));
        assert_eq!(form_of(DW_AT_LANGUAGE), Some(DW_FORM_DATA1));
        assert_eq!(form_of(DW_AT_HIGH_PC), Some(DW_FORM_UDATA));
    }

    /// Versions 2..5 differ exactly where their specifications say: the unit
    /// header grows `DW_UT_compile` and the address size at version 5, the line
    /// prologue grows `max_ops` at version 3 and the address-size/segment
    /// fields plus entry-format tables at version 5.
    #[test]
    fn every_version_writes_its_own_unit_and_prologue_header() {
        let files = [(1, "b.c".to_string())];
        let rows = [row(0, 1, 2)];
        // Prologue bytes before the tables: the fixed header fields
        // (`minimum_instruction_length`, `default_is_stmt`, `line_base`,
        // `line_range`, `opcode_base`, the standard opcode lengths, and
        // `maximum_operations_per_instruction` from version 4 on, with nine
        // standard opcodes in version 2 and twelve after).
        let fixed = [(2, 14usize), (3, 17), (4, 18), (5, 18)];
        // Name `b.c`: inline (`b.c\0` + 3 ULEB zeros) for v2..4.
        let tables = [(2, 1 + 7 + 1), (3, 1 + 7 + 1), (4, 1 + 7 + 1)];
        // v5: one directory (format count, `DW_LNCT_path`+`DW_FORM_strp`, count,
        // one 4-byte offset) and two file entries (format count, two content
        // type/form pairs, count, then `directory_index`+`path` each).
        let v5_tables = (1 + 2 + 1 + 4) + (1 + 2 + 2 + 1 + (1 + 4) * 2);
        for version in 2..=5 {
            let config = DwarfConfig::for_c(version, Some(Some("202311L")));
            let mut strings = DwarfStrings::new();
            let mut line_strings = DwarfStrings::new();
            let line = build_dwarf_line_section(
                &files,
                &rows,
                &text_section(),
                8,
                ABS64,
                ABS32,
                "/src",
                config,
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap();
            let data = &line.line.data;
            let mut at = 0;
            let unit_length = read_u32(data, &mut at) as usize;
            assert_eq!(unit_length + 4, data.len(), "version {version} length");
            assert_eq!(read_u16(data, &mut at), version as u16);
            if version >= 5 {
                // Version 5's line header names the address size and the
                // segment selector size before its length.
                assert_eq!(data[at], 8, "address_size in the line header");
                assert_eq!(data[at + 1], 0, "segment_selector_size");
                at += 2;
            }
            let header_length = read_u32(data, &mut at) as usize;
            let expected = match version {
                2..=4 => fixed[version as usize - 2].1 + tables[version as usize - 2].1,
                _ => fixed[3].1 + v5_tables,
            };
            assert_eq!(header_length, expected, "version {version} prologue");
            // After `minimum_instruction_length` come `default_is_stmt`,
            // `line_base`, `line_range` and `opcode_base`; DWARF 4 inserted
            // `maximum_operations_per_instruction` before `default_is_stmt`.
            assert_eq!(data[at], 1, "minimum_instruction_length");
            let extra = usize::from(version >= 4);
            assert_eq!(data[at + 1 + extra], 1, "default_is_stmt");
            let table = at + 5 + extra;
            if version >= 3 {
                assert_eq!(data[table - 1], 13, "opcode_base");
                assert_eq!(
                    &data[table..table + 12],
                    &STANDARD_OPCODE_LENGTHS,
                    "version {version} standard opcode table"
                );
            } else {
                // A version 2 line program has nine standard opcodes, and gcc
                // 16.2 writes exactly that for `-gdwarf-2` (measured): the
                // table's dialect follows the unit's.
                assert_eq!(data[table - 1], 10, "version 2 opcode_base");
                assert_eq!(
                    &data[table..table + 9],
                    &STANDARD_OPCODE_LENGTHS[..9],
                    "DWARF 2 has nine standard opcodes"
                );
            }
            if version >= 4 {
                assert_eq!(data[at + 1], 1, "maximum_operations_per_instruction");
                assert_eq!(data[at + 2], 1, "default_is_stmt");
            } else {
                assert_eq!(data[at + 1], 1, "default_is_stmt");
            }

            let info = build_dwarf_info_sections(
                &line.code_sections,
                Some(0),
                &files,
                &rows,
                &text_section(),
                &[],
                "b.c",
                "/src",
                "p",
                8,
                ABS32,
                ABS64,
                config,
                &mut strings,
                &mut line_strings,
                &no_unit(),
                &no_defined_objects(),
                None,
            )
            .unwrap()
            .unwrap();
            let d = &info.info.data;
            let mut at = 0;
            let unit_length = read_u32(d, &mut at) as usize;
            assert_eq!(unit_length + 4, d.len(), "version {version} unit length");
            assert_eq!(read_u16(d, &mut at), version as u16);
            if version >= 5 {
                assert_eq!(d[at], DW_UT_COMPILE);
                assert_eq!(d[at + 1], 8);
                at += 2;
                assert_eq!(read_u32(d, &mut at), 0, "abbrev offset relocation");
            } else {
                assert_eq!(read_u32(d, &mut at), 0, "abbrev offset relocation");
                assert_eq!(d[at], 8, "address size after the unit header");
            }
        }
    }

    /// A subprogram states the frame base its own CFI already defines: one byte
    /// of `DW_OP_call_frame_cfa` from version 3 on.  A version 2 unit cannot
    /// decode that opcode, so it states nothing rather than describing its
    /// prologue with an opcode it never defined -- gcc 16.2 pays a `.debug_loc`
    /// list for the same information there (measured), and clang 23 writes a
    /// bare `DW_OP_reg6`, which stops being the frame base as soon as the frame
    /// pointer is omitted.
    #[test]
    fn a_subprogram_states_a_frame_base_where_the_dialect_allows_one() {
        let files = [(1, "b.c".to_string())];
        let rows = [row(0, 1, 2)];
        let functions = [DwarfFunction {
            name: "f",
            section_index: 0,
            offset: 0,
            size: 0x40,
            external: true,
        }];
        for version in 2..=5 {
            let config = DwarfConfig::for_c(version, None);
            let mut strings = DwarfStrings::new();
            let mut line_strings = DwarfStrings::new();
            let line = build_dwarf_line_section(
                &files,
                &rows,
                &text_section(),
                8,
                ABS64,
                ABS32,
                "/src",
                config,
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap();
            let info = build_dwarf_info_sections(
                &line.code_sections,
                Some(0),
                &files,
                &rows,
                &text_section(),
                &functions,
                "b.c",
                "/src",
                "p",
                8,
                ABS32,
                ABS64,
                config,
                &mut strings,
                &mut line_strings,
                &no_unit(),
                &no_defined_objects(),
                None,
            )
            .unwrap()
            .unwrap();
            let declared: Vec<u64> = abbreviation_table(&info.abbrev.data)
                .into_iter()
                .filter(|(_, tag, _)| *tag == DW_TAG_SUBPROGRAM)
                .flat_map(|(_, _, attributes)| attributes)
                .filter(|(at, _)| *at == DW_AT_FRAME_BASE)
                .map(|(_, form)| form)
                .collect();
            // The unit's last child is the only subprogram; the expression is
            // its last attribute, followed by the unit terminator.
            let expression = [1, DW_OP_CALL_FRAME_CFA, 0];
            if version >= 3 {
                assert!(
                    !declared.is_empty() && declared.iter().all(|&form| form == DW_FORM_EXPRLOC),
                    "version {version}: DW_AT_FRAME_BASE is declared as {declared:?}"
                );
                assert!(
                    info.info.data.ends_with(&expression),
                    "version {version}: the subprogram does not end in a one-byte \
                     DW_OP_call_frame_cfa block: {:?}",
                    &info.info.data[info.info.data.len().saturating_sub(8)..]
                );
            } else {
                assert!(
                    declared.is_empty(),
                    "version {version}: a version 2 unit states no frame base"
                );
                assert!(
                    !info.info.data.ends_with(&expression),
                    "version {version}: the opcode leaked into a version 2 unit"
                );
            }
        }
    }

    /// `.debug_aranges` is the first thing a consumer reads, so its set has to
    /// be exactly what the specification describes: the unit's offset in
    /// `.debug_info` (a relocation, because the linker concatenates the
    /// sections), the address size, the first tuple on a multiple of the tuple
    /// size counted from the set's own length field, and the zero tuple that
    /// ends it.
    #[test]
    fn the_address_map_names_each_unit_and_its_code() {
        let files = [(1, "b.c".to_string())];
        let rows = [row(0, 1, 2)];
        let functions = [DwarfFunction {
            name: "f",
            section_index: 0,
            offset: 0,
            size: 0x40,
            external: true,
        }];
        for (address_size, abs_addr) in [(4usize, ABS32), (8, ABS64)] {
            let mut strings = DwarfStrings::new();
            let mut line_strings = DwarfStrings::new();
            let line = build_dwarf_line_section(
                &files,
                &rows,
                &text_section(),
                address_size,
                abs_addr,
                ABS32,
                "/src",
                DwarfConfig::for_c(5, None),
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap();
            let info = build_dwarf_info_sections(
                &line.code_sections,
                Some(0),
                &files,
                &rows,
                &text_section(),
                &functions,
                "b.c",
                "/src",
                "p",
                address_size,
                ABS32,
                abs_addr,
                DwarfConfig::for_c(5, None),
                &mut strings,
                &mut line_strings,
                &no_unit(),
                &no_defined_objects(),
                None,
            )
            .unwrap()
            .unwrap();
            let aranges = info.aranges.expect("a fixed-layout target gets a map");
            assert_eq!(aranges.name, ".debug_aranges");
            let data = &aranges.data;
            // length, version, the unit's offset, address size and segment size;
            // the tuple that follows is aligned from the set's first byte.
            let tuple = 2 * address_size;
            let header = 4 + 2 + 4 + 1 + 1;
            let padding = (tuple - header % tuple) % tuple;
            assert_eq!(padding, 4, "the header is 12 bytes at either size");
            assert_eq!(
                data.len(),
                header + padding + tuple + tuple,
                "one range plus the terminator"
            );
            assert_eq!(
                u32::from_le_bytes(data[..4].try_into().unwrap()) as usize,
                data.len() - 4,
                "the length covers the set but not itself"
            );
            assert_eq!(u16::from_le_bytes([data[4], data[5]]), 2, "version 2");
            assert_eq!(data[10], address_size as u8);
            assert_eq!(data[11], 0, "no segment selector");
            assert!(
                data[header..header + padding].iter().all(|&b| b == 0),
                "alignment padding is zero"
            );
            assert!(
                data[header + padding + tuple..].iter().all(|&b| b == 0),
                "the terminating tuple is zero"
            );
            let relocs: Vec<_> = aranges
                .relocs
                .iter()
                .map(|r| (r.offset, r.reloc_type, r.symbol_name.as_str(), r.addend))
                .collect();
            assert_eq!(
                relocs,
                [
                    (6, ABS32, ".debug_info", 0),
                    (header as u64 + padding as u64, abs_addr, ".text", 0)
                ],
                "the unit offset and the code address travel as relocations"
            );
        }
    }

    /// Code in more than one section is *one* line program holding one
    /// sequence per section -- the shape GAS 2.47 and GCC 16.2 both write --
    /// not one program per section.  Each sequence starts with a relocated
    /// `DW_LNE_set_address`, so the number of address relocations is the number
    /// of sequences.
    #[test]
    fn one_line_program_holds_a_sequence_per_code_section() {
        let files = [(1, "s.c".to_string())];
        let rows = split_rows();
        let sections = split_sections();
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &sections,
            8,
            ABS64,
            ABS32,
            "/src",
            DwarfConfig::for_c(5, None),
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(line.code_sections, [0, 1], "one sequence per code section");
        let addresses = line
            .line
            .relocs
            .iter()
            .filter(|r| r.reloc_type == ABS64)
            .map(|r| (r.symbol_name.as_str(), r.addend))
            .collect::<Vec<_>>();
        assert_eq!(
            addresses,
            [(".text", 0), (".text.unlikely", 0x10)],
            "each sequence names its own first address"
        );
        // The section is one unit: its length field spans everything that
        // follows, prologue included.
        let unit_length = u32::from_le_bytes(line.line.data[..4].try_into().unwrap()) as usize;
        assert_eq!(unit_length + 4, line.line.data.len());
        // One prologue, not one per section: a second copy would at least
        // double the section, since the prologue dominates two sequences.
        let single = build_dwarf_line_section(
            &files,
            &rows[..1],
            &text_section(),
            8,
            ABS64,
            ABS32,
            "/src",
            DwarfConfig::for_c(5, None),
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap()
        .line
        .data;
        assert!(
            line.line.data.len() < single.len() + 0x40,
            "the second sequence costs its statements, not a second prologue"
        );
    }

    /// Code in several sections cannot be one `DW_AT_low_pc`/`high_pc` pair, so
    /// the unit names a range list: `.debug_rnglists` from version 5 and
    /// `.debug_ranges` before it.
    #[test]
    fn split_code_gets_a_range_list_the_unit_points_at() {
        let files = [(1, "s.c".to_string())];
        let rows = split_rows();
        let sections = split_sections();
        let one_section = text_section();
        for (version, range_section, form) in [
            (2u8, ".debug_ranges", DW_FORM_DATA4),
            (3, ".debug_ranges", DW_FORM_DATA4),
            (4, ".debug_ranges", DW_FORM_SEC_OFFSET),
            (5, ".debug_rnglists", DW_FORM_SEC_OFFSET),
        ] {
            let config = DwarfConfig::for_c(version, None);
            let mut strings = DwarfStrings::new();
            let mut line_strings = DwarfStrings::new();
            let line = build_dwarf_line_section(
                &files,
                &rows,
                &sections,
                8,
                ABS64,
                ABS32,
                "/src",
                config,
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap();
            let info = build_dwarf_info_sections(
                &line.code_sections,
                Some(0),
                &files,
                &rows,
                &sections,
                &[],
                "s.c",
                "/src",
                "p",
                8,
                ABS32,
                ABS64,
                config,
                &mut strings,
                &mut line_strings,
                &no_unit(),
                &no_defined_objects(),
                None,
            )
            .unwrap()
            .unwrap();
            let ranges = info.ranges.as_ref().expect("split code needs a range list");
            assert_eq!(ranges.name, range_section, "version {version}");
            // The unit's own attributes name the list, never a high PC.
            let attrs: Vec<(u64, u64)> = abbreviation_table(&info.abbrev.data)
                .into_iter()
                .find(|(_, tag, _)| *tag == DW_TAG_COMPILE_UNIT)
                .map(|(_, _, attrs)| attrs)
                .expect("the unit abbreviation");
            assert_eq!(
                attrs
                    .iter()
                    .find(|(at, _)| *at == DW_AT_RANGES)
                    .map(|(_, f)| *f),
                Some(form),
                "version {version}: {attrs:?}"
            );
            assert!(
                !attrs.iter().any(|(at, _)| *at == DW_AT_HIGH_PC),
                "version {version}: a range list replaces the pair"
            );
            let list_offset = if version >= 5 { 12u64 } else { 0 };
            let range_reloc = info
                .info
                .relocs
                .iter()
                .find(|r| r.symbol_name == range_section)
                .map(|r| (r.reloc_type, r.addend))
                .expect("the list offset travels as a relocation");
            assert_eq!(
                range_reloc,
                (ABS32, list_offset as i64),
                "version {version}"
            );
            if version >= 5 {
                let data = &ranges.data;
                assert_eq!(
                    u32::from_le_bytes(data[..4].try_into().unwrap()) as usize + 4,
                    data.len()
                );
                assert_eq!(u16::from_le_bytes([data[4], data[5]]), 5);
                assert_eq!(data[6], 8, "address size");
                assert_eq!(data[7], 0, "no segments");
                assert_eq!(
                    u32::from_le_bytes(data[8..12].try_into().unwrap()),
                    0,
                    "no offset table: the list is named by section offset"
                );
                // Two `start_length` entries and the end marker: a kind byte,
                // an `address_size` address and a ULEB128 length each, then the
                // end marker.
                assert_eq!(data[12], DW_RLE_START_LENGTH);
                assert_eq!(data[21], 0x40, "the first section's length");
                assert_eq!(data[22], DW_RLE_START_LENGTH);
                assert_eq!(data[31], 0x20, "the second section's length");
                assert_eq!(data[32], DW_RLE_END_OF_LIST);
                assert_eq!(data.len(), 33);
                let relocs: Vec<_> = ranges
                    .relocs
                    .iter()
                    .map(|r| (r.offset, r.symbol_name.as_str(), r.addend))
                    .collect();
                assert_eq!(
                    relocs,
                    [(13, ".text", 0), (23, ".text.unlikely", 0)],
                    "each range starts at a relocated address"
                );
            } else {
                // Two address pairs and the terminating zero pair,
                // `address_size` bytes per address.
                assert_eq!(ranges.data.len(), 6 * 8);
                let relocs: Vec<_> = ranges
                    .relocs
                    .iter()
                    .map(|r| (r.offset, r.symbol_name.as_str(), r.addend))
                    .collect();
                assert_eq!(
                    relocs,
                    [
                        (0, ".text", 0),
                        (8, ".text", 0x40),
                        (16, ".text.unlikely", 0),
                        (24, ".text.unlikely", 0x20)
                    ]
                );
                assert!(ranges.data[32..].iter().all(|&b| b == 0));
            }
            // A single contiguous section needs none of this.
            let mut single_info = build_dwarf_info_sections(
                &[0],
                Some(0),
                &files,
                &[row(0, 1, 1)],
                &one_section,
                &[],
                "s.c",
                "/src",
                "p",
                8,
                ABS32,
                ABS64,
                config,
                &mut DwarfStrings::new(),
                &mut DwarfStrings::new(),
                &no_unit(),
                &no_defined_objects(),
                None,
            )
            .unwrap()
            .unwrap();
            assert!(single_info.ranges.is_none(), "version {version}");
            let single_attrs: Vec<(u64, u64)> = abbreviation_table(&single_info.abbrev.data)
                .into_iter()
                .find(|(_, tag, _)| *tag == DW_TAG_COMPILE_UNIT)
                .map(|(_, _, attrs)| attrs)
                .unwrap();
            assert!(!single_attrs.iter().any(|(at, _)| *at == DW_AT_RANGES));
            single_info.info.data.clear();
        }
    }

    /// The address map carries one tuple per code section of the unit, in one
    /// set -- exactly the shape GCC 16.2 writes for a split unit.
    #[test]
    fn the_address_map_lists_every_section_of_a_split_unit() {
        let files = [(1, "s.c".to_string())];
        let rows = split_rows();
        let sections = split_sections();
        let config = DwarfConfig::for_c(5, None);
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &sections,
            8,
            ABS64,
            ABS32,
            "/src",
            config,
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        let info = build_dwarf_info_sections(
            &line.code_sections,
            Some(0),
            &files,
            &rows,
            &sections,
            &[],
            "s.c",
            "/src",
            "p",
            8,
            ABS32,
            ABS64,
            config,
            &mut DwarfStrings::new(),
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            None,
        )
        .unwrap()
        .unwrap();
        let aranges = info.aranges.expect("a fixed-layout target gets a map");
        // 4 length + 2 version + 4 unit offset + 1 address size + 1 segment
        // size, padded to 16, then two tuples and the terminating tuple.
        assert_eq!(aranges.data.len(), 4 + 12 + 16 * 3);
        assert_eq!(
            u32::from_le_bytes(aranges.data[..4].try_into().unwrap()) as usize + 4,
            aranges.data.len()
        );
        let relocs: Vec<_> = aranges
            .relocs
            .iter()
            .map(|r| (r.offset, r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(
            relocs,
            [
                (6, ".debug_info", 0),
                (16, ".text", 0),
                (32, ".text.unlikely", 0)
            ]
        );
        assert!(aranges.data[48..].iter().all(|&b| b == 0));
    }

    /// A range list's lengths are constants, so they are ULEB128 and cannot
    /// truncate: a section larger than 4 GiB is described exactly, where the
    /// address-map tuple would have to truncate it.
    #[test]
    fn a_range_list_length_never_truncates() {
        let huge = [
            DwarfSectionInfo {
                name: ".text",
                size: u64::from(u32::MAX) + 1,
                is_executable: true,
            },
            DwarfSectionInfo {
                name: ".text.unlikely",
                size: 0x10,
                is_executable: true,
            },
        ];
        let ranges =
            build_debug_ranges(DwarfConfig::for_c(5, None), &[&huge[0], &huge[1]], 8, ABS64)
                .unwrap()
                .unwrap();
        assert_eq!(ranges.name, ".debug_rnglists");
        // The length is the five-byte ULEB128 of 0x1_0000_0000.
        assert_eq!(&ranges.data[21..26], &[0x80, 0x80, 0x80, 0x80, 0x10]);
    }

    /// A range length is a constant tuple field of `address_size` bytes.  A
    /// section larger than the target's address space cannot be described by
    /// one, and truncating it would claim a range that ends somewhere else
    /// entirely, so it is refused instead.
    #[test]
    fn an_address_range_that_does_not_fit_the_target_is_refused() {
        let files = [(1, "b.c".to_string())];
        let rows = [row(0, 1, 2)];
        let huge = [DwarfSectionInfo {
            name: ".text",
            size: u64::from(u32::MAX) + 1,
            is_executable: true,
        }];
        let config = DwarfConfig::for_c(5, None);
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &huge,
            4,
            ABS32,
            ABS32,
            "/src",
            config,
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        let built = build_dwarf_info_sections(
            &line.code_sections,
            Some(0),
            &files,
            &rows,
            &huge,
            &[],
            "b.c",
            "/src",
            "p",
            4,
            ABS32,
            ABS32,
            config,
            &mut strings,
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            None,
        );
        let error = match built {
            Err(error) => error,
            Ok(_) => panic!("a range longer than the address space must be refused"),
        };
        assert!(
            error.contains("does not fit") && error.contains("4-byte"),
            "{error}"
        );
    }

    /// A unit that covers no bytes gets no set: an empty tuple would only give a
    /// consumer something to skip, and `readelf` numbers sets, not units.
    #[test]
    fn a_unit_without_code_gets_no_address_map() {
        let files = [(1, "b.c".to_string())];
        let rows = [row(0, 1, 2)];
        let empty = [DwarfSectionInfo {
            name: ".text",
            size: 0,
            is_executable: true,
        }];
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &empty,
            8,
            ABS64,
            ABS32,
            "/src",
            DwarfConfig::for_c(5, None),
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        let info = build_dwarf_info_sections(
            &line.code_sections,
            Some(0),
            &files,
            &rows,
            &empty,
            &[],
            "b.c",
            "/src",
            "p",
            8,
            ABS32,
            ABS64,
            DwarfConfig::for_c(5, None),
            &mut strings,
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            None,
        )
        .unwrap()
        .unwrap();
        assert!(info.aranges.is_none(), "no bytes, no addresses to map");
    }

    /// The language a unit declares follows `-std=` and the dialect, exactly as
    /// GCC 16.2 records it (measured for every C dialect on every version).
    #[test]
    fn the_language_follows_the_dialect_and_version() {
        let case = |version, stdc| DwarfConfig::for_c(version, stdc);
        // C89 is the absence of `__STDC_VERSION__`.
        assert_eq!(case(5, None).language, DW_LANG_C89);
        assert_eq!(case(5, Some(None)).language, DW_LANG_C89);
        assert_eq!(case(5, Some(Some("199409L"))).language, DW_LANG_C89);
        assert_eq!(case(5, Some(Some("199901L"))).language, DW_LANG_C99);
        assert_eq!(case(5, Some(Some("201112L"))).language, DW_LANG_C11);
        assert_eq!(
            (
                case(5, Some(Some("201112L"))).language_name,
                case(5, Some(Some("201112L"))).language_version
            ),
            (None, None),
            "C11 has its own DWARF code, so no name/version pair",
        );
        for (stdc, year) in [
            ("201710L", 201710u32),
            ("202311L", 202311),
            ("202400L", 202400),
        ] {
            let config = case(5, Some(Some(stdc)));
            assert_eq!(config.language, DW_LANG_C11, "{stdc}");
            assert_eq!(config.language_name, Some(DW_LNAME_C), "{stdc}");
            assert_eq!(config.language_version, Some(year), "{stdc}");
        }
        // Versions 2..4 stop at DW_LANG_C99, as GCC does for every newer dialect.
        for version in 2..=4 {
            for stdc in [None, Some(None), Some(Some("199901L"))] {
                let expected = if stdc.is_some_and(|stdc| stdc.is_some()) {
                    DW_LANG_C99
                } else {
                    DW_LANG_C89
                };
                assert_eq!(
                    case(version, stdc).language,
                    expected,
                    "v{version} {stdc:?}"
                );
                assert_eq!(case(version, stdc).language_name, None);
                assert_eq!(case(version, stdc).language_version, None);
            }
            for stdc in ["201112L", "201710L", "202311L"] {
                let config = case(version, Some(Some(stdc)));
                assert_eq!(config.language, DW_LANG_C99, "v{version} {stdc}");
                assert_eq!(config.language_name, None);
                assert_eq!(config.language_version, None);
            }
        }
        // Assembly keeps GAS's language code, and the version is clamped to the
        // range the writer implements.
        assert_eq!(DwarfConfig::assembler(5).language, DW_LANG_MIPS_ASSEMBLER);
        assert_eq!(DwarfConfig::assembler(1).version, DWARF_MIN_VERSION);
        assert_eq!(DwarfConfig::assembler(9).version, DWARF_MAX_VERSION);
        assert_eq!(DwarfConfig::for_c(0, None).version, DWARF_MIN_VERSION);
        assert_eq!(DwarfConfig::for_c(u8::MAX, None).version, DWARF_MAX_VERSION);
    }

    /// The attribute *numbers* are the contract with a debugger, so they are
    /// pinned to the ones GCC 16.2 writes.
    ///
    /// Measured on GCC 16.2.0 for `-std=c17 -g -gdwarf-5`: the compile unit's
    /// abbreviation table declares `DW_AT_language` 0x13, then 0x90 and 0x91
    /// for the name/version pair -- DWARF 6's numbers, which binutils calls
    /// `Unknown AT value` and GDB decodes.  Asserting the constants against
    /// themselves would pass with any number, so the numbers are spelled out.
    #[test]
    fn dialect_attribute_numbers_match_gcc() {
        assert_eq!(DW_AT_LANGUAGE, 0x13);
        assert_eq!(DW_AT_LANGUAGE_NAME, 0x90);
        assert_eq!(DW_AT_LANGUAGE_VERSION, 0x91);
        assert_eq!(DW_AT_PRODUCER, 0x25);
        assert_eq!(DW_LANG_C89, 0x0001);
        assert_eq!(DW_LANG_C99, 0x000c);
        assert_eq!(DW_LANG_C11, 0x001d);
        assert_eq!(DW_LNAME_C, 0x0003);
    }

    /// A version-5 line prologue names its directory and file table through
    /// `DW_LNCT_*` entry formats, and every path is one interned `.debug_str`
    /// entry no matter how many rows or files repeat it.
    #[test]
    fn version_5_names_paths_through_the_string_table() {
        let files = [(1, "/src/b.c".to_string()), (2, "/src/inc/h.h".to_string())];
        let rows = [row(0, 1, 3), row(4, 2, 9), row(8, 1, 4)];
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &text_section(),
            8,
            ABS64,
            ABS32,
            "/src",
            DwarfConfig::for_c(5, None),
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        let data = &line.line.data;
        let mut at = 0;
        let _length = read_u32(data, &mut at);
        assert_eq!(read_u16(data, &mut at), 5);
        assert_eq!(data[at], 8, "address_size in the line header");
        assert_eq!(data[at + 1], 0, "segment_selector_size");
        at += 2;
        let header_length = read_u32(data, &mut at) as usize;
        let prologue_end = at + header_length;
        assert_eq!(data[at], 1, "minimum_instruction_length");
        assert_eq!(data[at + 1], 1, "maximum_operations_per_instruction");
        assert_eq!(data[at + 2], 1, "default_is_stmt");
        assert_eq!(data[at + 3] as i8, -5, "line_base");
        assert_eq!(data[at + 4], 14, "line_range");
        assert_eq!(data[at + 5], 13, "opcode_base");
        let mut cursor = at + 6 + 12; // fixed fields + standard opcode lengths

        assert_eq!(data[cursor], 1, "one directory entry format");
        cursor += 1;
        assert_eq!(read_uleb(data, &mut cursor), DW_LNCT_PATH);
        assert_eq!(read_uleb(data, &mut cursor), DW_FORM_LINE_STRP);
        let directories = read_uleb(data, &mut cursor);
        assert_eq!(directories, 2, "comp_dir and the include directory");
        let strings_data = line_strings.data.clone();
        let addends = reloc_addends(&line.line, ".debug_line_str");
        assert_eq!(
            read_strp(data, &mut cursor, &strings_data, &addends),
            "/src"
        );
        assert_eq!(
            read_strp(data, &mut cursor, &strings_data, &addends),
            "/src/inc"
        );

        assert_eq!(data[cursor], 2, "two file entry format columns");
        cursor += 1;
        assert_eq!(read_uleb(data, &mut cursor), DW_LNCT_DIRECTORY_INDEX);
        assert_eq!(read_uleb(data, &mut cursor), DW_FORM_UDATA);
        assert_eq!(read_uleb(data, &mut cursor), DW_LNCT_PATH);
        assert_eq!(read_uleb(data, &mut cursor), DW_FORM_LINE_STRP);
        assert_eq!(
            read_uleb(data, &mut cursor),
            3,
            "entry 0 duplicates the first file so index 1 is file 1"
        );
        for expected in [(0, "b.c"), (0, "b.c"), (1, "h.h")] {
            assert_eq!(read_uleb(data, &mut cursor), expected.0);
            assert_eq!(
                read_strp(data, &mut cursor, &strings_data, &addends),
                expected.1
            );
        }
        assert_eq!(cursor, prologue_end, "prologue ends at the opcodes");
        // Rows and the end sequence follow the prologue.
        assert!(data.len() > prologue_end);

        let line_strings = line_strings.into_section(".debug_line_str").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&line_strings.data),
            "/src\0/src/inc\0b.c\0h.h\0",
            "one entry per distinct string"
        );
    }

    /// Decode one version-5 directory or file table, returning every entry's
    /// columns: the format list is read first, so the reader follows whatever
    /// the writer chose instead of assuming a shape.
    fn version_5_table(
        data: &[u8],
        cursor: &mut usize,
        strings: &[u8],
        addends: &[(u64, i64)],
    ) -> Vec<Vec<String>> {
        let columns = read_uleb(data, cursor);
        let mut forms = Vec::new();
        for _ in 0..columns {
            read_uleb(data, cursor); // DW_LNCT_* content type
            forms.push(read_uleb(data, cursor));
        }
        let mut rows = Vec::new();
        for _ in 0..read_uleb(data, cursor) {
            let mut row = Vec::with_capacity(forms.len());
            for &form in &forms {
                row.push(match form {
                    DW_FORM_UDATA => read_uleb(data, cursor).to_string(),
                    DW_FORM_LINE_STRP => read_strp(data, cursor, strings, addends),
                    other => panic!("test reader has no rule for form {other:#x}"),
                });
            }
            rows.push(row);
        }
        rows
    }

    /// The directory and file tables of a version-5 prologue.
    fn version_5_tables(
        build: &DwarfLineBuild,
        line_strings: &DwarfStrings,
    ) -> (Vec<String>, Vec<(u64, String)>) {
        let data = &build.line.data;
        let mut at = 4;
        assert_eq!(read_u16(data, &mut at), 5, "line program version");
        at += 2; // address_size, segment_selector_size
        let header_length = read_u32(data, &mut at) as usize;
        let prologue_end = at + header_length;
        at += 5; // minimum_instruction_length .. line_range
        let opcode_base = usize::from(data[at]);
        at += opcode_base; // opcode_base byte and its standard-opcode lengths
        let addends = reloc_addends(&build.line, ".debug_line_str");
        let mut cursor = at;
        let directories = version_5_table(data, &mut cursor, &line_strings.data, &addends)
            .into_iter()
            .map(|row| row[0].clone())
            .collect();
        let files = version_5_table(data, &mut cursor, &line_strings.data, &addends)
            .into_iter()
            .map(|row| (row[0].parse().unwrap(), row[1].clone()))
            .collect();
        assert_eq!(cursor, prologue_end, "tables end at the opcodes");
        (directories, files)
    }

    /// The directory and file tables of a version 2..4 prologue, read the way
    /// a consumer reads them: NUL-terminated names and LEB128 indices.
    fn inline_tables(data: &[u8], unit_version: u16) -> (Vec<String>, Vec<(u64, String)>) {
        let version = unit_version;
        let mut at = 4;
        assert_eq!(read_u16(data, &mut at), version, "line program version");
        let header_length = read_u32(data, &mut at) as usize;
        let mut cursor = at + 1; // minimum_instruction_length
        if version >= 4 {
            cursor += 1; // maximum_operations_per_instruction
        }
        cursor += 3; // default_is_stmt, line_base, line_range
        let opcode_base = usize::from(data[cursor]);
        cursor += opcode_base; // the opcode_base byte and its lengths
        let prologue_end = at + header_length;
        let mut directories = Vec::new();
        while data[cursor] != 0 {
            let end = cursor + data[cursor..].iter().position(|&b| b == 0).unwrap();
            directories.push(String::from_utf8_lossy(&data[cursor..end]).into_owned());
            cursor = end + 1;
        }
        cursor += 1;
        let mut files = Vec::new();
        while data[cursor] != 0 {
            let end = cursor + data[cursor..].iter().position(|&b| b == 0).unwrap();
            let name = String::from_utf8_lossy(&data[cursor..end]).into_owned();
            cursor = end + 1;
            let index = read_uleb(data, &mut cursor);
            assert_eq!(read_uleb(data, &mut cursor), 0, "modification_time");
            assert_eq!(read_uleb(data, &mut cursor), 0, "file_size");
            files.push((index, name));
        }
        assert_eq!(cursor + 1, prologue_end, "tables end at the opcodes");
        (directories, files)
    }

    /// Resolve a recorded path the way a debugger does: an absolute directory
    /// stands on its own, a relative one and a bare base name hang off the
    /// compilation directory.
    fn resolve_path(comp_dir: &str, directory: &str, base: &str) -> String {
        if directory.is_empty() {
            format!("{comp_dir}/{base}")
        } else if directory.starts_with('/') {
            format!("{directory}/{base}")
        } else {
            format!("{comp_dir}/{directory}/{base}")
        }
    }

    /// A base name is what a version 2..4 file entry stores, and an empty one
    /// would terminate the table early, so it is refused rather than written.
    #[test]
    fn split_path_names_directories_and_bases() {
        assert_eq!(split_path("a.c"), ("", "a.c"));
        assert_eq!(split_path("src/a.c"), ("src", "a.c"));
        assert_eq!(split_path("src/inc/a.c"), ("src/inc", "a.c"));
        assert_eq!(split_path("/a.c"), ("/", "a.c"));
        assert_eq!(split_path("/src/a.c"), ("/src", "a.c"));
        // A trailing separator leaves no base name to record.
        assert_eq!(split_path("src/"), ("src", ""));
        assert_eq!(split_path(""), ("", ""));

        let mut files = [(1u32, "ok.c".to_string())];
        assert_eq!(debug_file_table(&files).unwrap(), [(1, "ok.c")]);
        for bad in ["", "src/"] {
            files[0].1 = bad.to_string();
            let error = debug_file_table(&files).unwrap_err();
            assert!(
                error.contains(".file 1") && error.contains("directory separator"),
                "{error}"
            );
        }
        files[0].1 = "a\u{0}b.c".to_string();
        assert!(debug_file_table(&files).unwrap_err().contains("NUL"));
    }

    /// The same recorded names describe the same files at every version, and
    /// each prologue uses the numbering its specification defines: directory
    /// 0 is the compilation directory everywhere, version 5 spells it out as
    /// table entry 0, and versions 2..4 leave it implicit and number their own
    /// entries from 1. A name in a subdirectory, an absolute one, and one that
    /// spells the compilation directory all resolve.
    #[test]
    fn every_version_resolves_recorded_paths_to_the_same_file() {
        let files = [
            (1, "src/a.c".to_string()),
            (2, "/vendor/inc/b.h".to_string()),
            (3, "/work/c.c".to_string()),
            (4, "d.c".to_string()),
            (5, "src/e.c".to_string()),
        ];
        let rows = [row(0, 1, 3), row(4, 2, 9)];
        let comp_dir = "/work";
        // File number -> the path a debugger must resolve it to.
        let expected = [
            (1u64, "/work/src/a.c"),
            (2, "/vendor/inc/b.h"),
            (3, "/work/c.c"),
            (4, "/work/d.c"),
            (5, "/work/src/e.c"),
        ];
        // Version 5 repeats the first file at entry 0 so that file 1 is entry
        // 1; every version shares the same directory numbering from 1.
        let files_inline = [
            (1u64, "a.c"),
            (2, "b.h"),
            (0, "c.c"),
            (0, "d.c"),
            (1, "e.c"),
        ];
        let files_v5 = [
            (1u64, "a.c"),
            (1, "a.c"),
            (2, "b.h"),
            (0, "c.c"),
            (0, "d.c"),
            (1, "e.c"),
        ];
        for version in 2..=5u8 {
            let mut line_strings = DwarfStrings::new();
            let line = build_dwarf_line_section(
                &files,
                &rows,
                &text_section(),
                8,
                ABS64,
                ABS32,
                comp_dir,
                DwarfConfig::for_c(version, None),
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap();
            let (directories, entries, table) = if version >= 5 {
                let (directories, entries) = version_5_tables(&line, &line_strings);
                (directories, entries, &files_v5[..])
            } else {
                let (directories, entries) = inline_tables(&line.line.data, u16::from(version));
                (directories, entries, &files_inline[..])
            };
            let expected_directories: &[&str] = if version >= 5 {
                &["/work", "src", "/vendor/inc"]
            } else {
                &["src", "/vendor/inc"]
            };
            assert_eq!(
                directories, expected_directories,
                "version {version} directory table"
            );
            assert_eq!(
                entries,
                table
                    .iter()
                    .map(|(index, base)| (*index, base.to_string()))
                    .collect::<Vec<_>>(),
                "version {version} file table"
            );
            for (number, want) in expected {
                // Version 5 indexes files directly; older prologues start at 1.
                let position = if version >= 5 { number } else { number - 1 } as usize;
                let (directory_index, base) = &entries[position];
                let index = *directory_index as usize;
                let directory = if version >= 5 {
                    directories[index].clone()
                } else if index == 0 {
                    String::new()
                } else {
                    directories[index - 1].clone()
                };
                assert_eq!(
                    resolve_path(comp_dir, &directory, base),
                    want,
                    "version {version} file {number}"
                );
            }
        }
    }

    /// A function records where it was declared: the first row inside its own
    /// address range, and never a row that belongs to its neighbour.
    #[test]
    fn declarations_come_from_the_first_row_inside_each_function() {
        let files = [(1, "a.c".to_string()), (2, "b.c".to_string())];
        let rows = [
            row(0x8, 1, 5),
            row(0x10, 1, 7),
            row(0x14, 1, 8),
            row(0x20, 2, 9),
            // Past both functions: belongs to whatever follows, not to `g`.
            row(0x30, 2, 11),
        ];
        let functions = [
            DwarfFunction {
                name: "f",
                section_index: 0,
                offset: 0x10,
                size: 0x10,
                external: true,
            },
            DwarfFunction {
                name: "g",
                section_index: 0,
                offset: 0x20,
                size: 0x4,
                external: false,
            },
        ];
        let config = DwarfConfig::for_c(5, None);
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &text_section(),
            8,
            ABS64,
            ABS32,
            "/src",
            config,
            &mut line_strings,
            None,
        )
        .unwrap()
        .unwrap();
        let info = build_dwarf_info_sections(
            &line.code_sections,
            Some(0),
            &files,
            &rows,
            &text_section(),
            &functions,
            "a.c",
            "/src",
            "p",
            8,
            ABS32,
            ABS64,
            config,
            &mut strings,
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            None,
        )
        .unwrap()
        .unwrap();
        let strings = strings.into_section(".debug_str").unwrap();
        let line_strings = line_strings.into_section(".debug_line_str").unwrap();
        let tables = Strings::of(&info.info, Some(&strings), Some(&line_strings));
        let (_, subprograms) = unit_contents(&info.info.data, &info.abbrev.data, &tables, 8);
        assert_eq!(
            subprograms,
            [
                Subprogram {
                    name: "f".to_string(),
                    external: true,
                    declaration: Some((1, 7)),
                },
                Subprogram {
                    name: "g".to_string(),
                    external: false,
                    declaration: Some((2, 9)),
                },
            ]
        );
    }

    #[test]
    fn an_assembly_source_records_the_assembler_language() {
        let units = [0usize];
        let config = DwarfConfig::assembler(4);
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let info = build_dwarf_info_sections(
            &units,
            Some(0),
            &[],
            &[],
            &text_section(),
            &[],
            "start.S",
            "/",
            "p",
            4,
            1,
            1,
            config,
            &mut strings,
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            None,
        )
        .unwrap()
        .unwrap();
        let d = &info.info.data;
        // `DW_AT_language` follows the inline producer, and it is
        // `DW_FORM_data2`: `DW_LANG_MIPS_ASSEMBLER` is 0x8001, which does not
        // fit in the byte a C language code needs.
        let at = d.windows(2).position(|w| w == b"p\0").unwrap() + 2;
        assert_eq!(
            u16::from_le_bytes([d[at], d[at + 1]]),
            DW_LANG_MIPS_ASSEMBLER
        );
        let forms: Vec<_> = abbreviation_table(&info.abbrev.data)
            .into_iter()
            .filter(|(_, tag, _)| *tag == DW_TAG_COMPILE_UNIT)
            .flat_map(|(_, _, attrs)| attrs)
            .collect();
        assert_eq!(
            forms
                .iter()
                .find(|(at, _)| *at == DW_AT_LANGUAGE)
                .map(|(_, form)| *form),
            Some(DW_FORM_DATA2),
            "{forms:?}"
        );
        assert_eq!(d[10], 4, "i686 address size");
        assert!(
            strings.into_section(".debug_str").is_none(),
            "DWARF 4 keeps its strings inline"
        );
    }

    /// Relaxation-safe addresses: every row, the sequence end and both PCs
    /// of the unit and each subprogram are label relocations, never
    /// section+offset or a length the linker could not adjust.
    #[test]
    fn label_addresses_survive_code_deletion() {
        let files = [(1, "r.c".to_string())];
        let row = |offset, line, sequence| DwarfLineLocation {
            section_index: 0,
            offset,
            file_number: 1,
            line,
            column: 0,
            options: DwarfLocOptions::default(),
            sequence,
        };
        let rows = [row(0, 1, 0), row(8, 2, 1), row(8, 3, 2), row(0x20, 4, 3)];
        let mut asked = Vec::new();
        let mut label = |section: usize, offset: u64| {
            asked.push((section, offset));
            format!("L{offset:x}")
        };
        let config = DwarfConfig::for_c(5, None);
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let line = build_dwarf_line_section(
            &files,
            &rows,
            &text_section(),
            8,
            ABS64,
            ABS32,
            "/",
            config,
            &mut line_strings,
            Some(&mut label),
        )
        .unwrap()
        .unwrap();
        // The v5 path table contributes string-offset relocations; the code
        // addresses are the label ones, and every one of them must be a label.
        let targets: Vec<_> = line
            .line
            .relocs
            .iter()
            .filter(|reloc| reloc.symbol_name != ".debug_line_str")
            .map(|reloc| (reloc.symbol_name.as_str(), reloc.addend))
            .collect();
        assert_eq!(targets, [("L0", 0), ("L8", 0), ("L20", 0), ("L40", 0)]);
        assert!(
            line.line
                .relocs
                .iter()
                .filter(|reloc| reloc.symbol_name == ".debug_line_str")
                .all(|reloc| reloc.reloc_type == ABS32),
            "path offsets are 32-bit relocations against .debug_line_str"
        );
        let functions = [DwarfFunction {
            name: "f",
            section_index: 0,
            offset: 8,
            size: 0x18,
            external: true,
        }];
        let info = build_dwarf_info_sections(
            &line.code_sections,
            Some(0),
            &files,
            &rows,
            &text_section(),
            &functions,
            "r.c",
            "/",
            "p",
            8,
            ABS32,
            ABS64,
            config,
            &mut strings,
            &mut line_strings,
            &no_unit(),
            &no_defined_objects(),
            Some(&mut label),
        )
        .unwrap()
        .unwrap();
        // String offsets are relocations too (into `.debug_str` and, from
        // version 5, into `.debug_line_str`); what has to survive deletion is
        // the code addresses, so only those are compared here.
        let targets: Vec<_> = info
            .info
            .relocs
            .iter()
            .filter(|r| !matches!(r.symbol_name.as_str(), ".debug_str" | ".debug_line_str"))
            .map(|r| (r.symbol_name.as_str(), r.addend))
            .collect();
        assert_eq!(
            targets,
            [
                (".debug_abbrev", 0),
                (".text", 0),
                ("L40", 0),
                (".debug_line", 0),
                ("L8", 0),
                ("L20", 0)
            ]
        );
        // Every abbreviation addresses `high_pc` with `DW_FORM_addr`: labels
        // are in use, so the value has to be a relocated address.
        let table = abbreviation_table(&info.abbrev.data);
        assert!(!table.is_empty());
        for (code, tag, attrs) in &table {
            if *tag != DW_TAG_SUBPROGRAM {
                continue;
            }
            assert_eq!(
                attrs
                    .iter()
                    .find(|(at, _)| *at == DW_AT_HIGH_PC)
                    .map(|(_, form)| *form),
                Some(DW_FORM_ADDR),
                "abbreviation {code} lists {attrs:?}"
            );
        }
        assert!(asked.iter().all(|&(section, _)| section == 0));
        // RISC-V relaxation deletes bytes after assembly, so a range length
        // computed here would claim code that belongs to the next unit: the
        // address map is omitted rather than approximated.
        assert!(
            info.aranges.is_none(),
            "a relaxing target must not publish an approximate address map"
        );
    }

    /// `.debug_str` and `.debug_line_str` are NUL-separated pools of whole
    /// strings, which is exactly what `SHF_MERGE|SHF_STRINGS` says to a linker;
    /// GCC and GAS flag both that way (`.section .debug_str,"MS",@progbits,1`).
    /// Measured on ten one-function units: 126 bytes of `.debug_str` and 144 of
    /// `.debug_line_str` unmerged, 43 and 60 after mold's merge -- and only for
    /// objects that carry the flags.  Nothing else may claim the flags: a
    /// mergeable section promises its entries are strings, and a linker that
    /// believes a byte stream is one rewrites offsets that are not string
    /// starts.
    #[test]
    fn the_string_pools_are_marked_mergeable_and_nothing_else_is() {
        for name in [".debug_str", ".debug_line_str"] {
            let mut strings = DwarfStrings::new();
            assert_eq!(strings.intern("producer").unwrap(), 0);
            let section = strings.into_section(name).expect("non-empty pool");
            assert_eq!(section.name, name);
            assert_eq!(section.sh_flags, SHF_MERGE | SHF_STRINGS, "{name}");
            assert_eq!(section.sh_addralign, 1, "{name} holds bytes");
            assert_eq!(section.sh_type, SHT_PROGBITS, "{name}");
            assert_eq!(
                debug_section_flags(name) & !(SHF_MERGE | SHF_STRINGS),
                0,
                "{name} carries no other flag"
            );
        }
        for name in [
            ".debug_info",
            ".debug_abbrev",
            ".debug_line",
            ".debug_aranges",
            ".debug_rnglists",
            ".debug_ranges",
            ".debug_str_offsets",
        ] {
            assert_eq!(
                debug_section(name, vec![0], Vec::new()).sh_flags,
                0,
                "{name} is a byte stream, not a string pool"
            );
        }
    }

    /// An input that already declares `.debug_str`/`.debug_line_str` (GCC's own
    /// assembly does) is *continued*, never replaced: the object then holds one
    /// pool whose head is the input's bytes, and a name the input already had
    /// keeps its offset, because the input's `.debug_info` -- copied verbatim --
    /// refers to it by number.
    #[test]
    fn an_input_string_pool_is_continued_not_replaced() {
        let mut strings = DwarfStrings::continuing(b"producer\0/tmp\0");
        assert_eq!(strings.intern("producer").unwrap(), 0, "reuses the input's");
        assert_eq!(strings.intern("/tmp").unwrap(), 9, "reuses the input's");
        assert_eq!(strings.intern("new").unwrap(), 14, "appends after it");
        assert_eq!(
            strings.into_section(".debug_str").unwrap().data,
            b"producer\0/tmp\0new\0",
            "one pool, input first, and it still ends in a NUL"
        );
        // Without the closing NUL an appended name would fuse with the input's
        // last one and every later reference would read a suffix.
        let mut open = DwarfStrings::continuing(b"abc");
        assert_eq!(open.intern("d").unwrap(), 4);
        assert_eq!(open.into_section(".debug_str").unwrap().data, b"abc\0d\0");
        // A pool that already holds its NUL is left exactly as it was.
        let mut closed = DwarfStrings::continuing(b"abc\0");
        assert_eq!(closed.intern("abc").unwrap(), 0);
        assert_eq!(closed.into_section(".debug_str").unwrap().data, b"abc\0");
    }

    /// An input that declares `.debug_line` (GCC writes an empty one; hand
    /// written assembly may carry a program) keeps that program: the generated
    /// one is written *after* the input's bytes, `DW_AT_stmt_list` is the offset
    /// it starts at, and every relocation moves with it.  Dropping the input's
    /// program instead would leave its `DW_AT_stmt_list` pointing at the new
    /// header's first byte, which decodes as whatever the two have in common.
    #[test]
    fn an_input_line_program_is_continued_not_replaced() {
        let files = [(1u32, "ms.c".to_string())];
        let rows = [DwarfLineLocation {
            section_index: 0,
            offset: 0,
            file_number: 1,
            line: 1,
            column: 3,
            options: DwarfLocOptions::default(),
            sequence: 0,
        }];
        let build = || {
            let mut line_strings = DwarfStrings::new();
            build_dwarf_line_section(
                &files,
                &rows,
                &text_section(),
                8,
                ABS64,
                ABS32,
                "/src",
                DwarfConfig::for_c(5, None),
                &mut line_strings,
                None,
            )
            .unwrap()
            .unwrap()
        };
        let original = build();
        assert!(
            !original.line.relocs.is_empty(),
            "the row's address is a relocation"
        );
        let offsets: Vec<u64> = original.line.relocs.iter().map(|r| r.offset).collect();

        // A program the input already carries, 7 bytes of it.
        let prefix = b"\x07\0\0\0\x05\0\x01";
        let mut continued = build();
        assert_eq!(place_line_build(&mut continued, prefix).unwrap(), 7);
        assert_eq!(
            &continued.line.data[..7],
            prefix,
            "the input's program leads"
        );
        assert_eq!(
            &continued.line.data[7..],
            &original.line.data[..],
            "the generated program follows it byte for byte"
        );
        let moved: Vec<u64> = continued.line.relocs.iter().map(|r| r.offset).collect();
        assert_eq!(
            moved,
            offsets.iter().map(|o| o + 7).collect::<Vec<_>>(),
            "every relocation moves with the program it belongs to"
        );

        // No input program: nothing moves, and the offset is zero.
        let mut plain = build();
        assert_eq!(place_line_build(&mut plain, b"").unwrap(), 0);
        assert_eq!(plain.line.data, original.line.data);
        assert_eq!(
            plain
                .line
                .relocs
                .iter()
                .map(|r| r.offset)
                .collect::<Vec<_>>(),
            offsets
        );
    }
}

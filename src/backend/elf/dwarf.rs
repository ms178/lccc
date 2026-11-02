//! DWARF debug-information writer shared by the ELF assemblers.
//!
//! The compiler already lowers source locations to GAS-compatible `.file` and
//! `.loc` directives. This module turns those records into a `.debug_line`
//! unit, retaining relocations to the final code section so both relocatable
//! objects and linked images carry usable source line information.
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
//! a bare `-g` (measured). Every version from 2 through 5 has the header layout
//! its specification requires:
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
use super::{ObjReloc, ObjSection, SHT_PROGBITS, parse_string_literal};
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

/// One line program in a built `.debug_line`: its byte offset and the code
/// section its rows describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DwarfLineUnit {
    pub offset: u64,
    pub section_index: usize,
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
        }
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

/// A built `.debug_line` section and the units it holds.
pub(crate) struct DwarfLineBuild {
    pub line: ObjSection,
    pub units: Vec<DwarfLineUnit>,
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
    LineTables { directories, files }
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
        // File name table: `DW_LNCT_directory_index` then `DW_LNCT_path`. Entry
        // 0 repeats the first file, because the line-program file register
        // starts at 1: index *i* of the table is file *i*, and nothing may fall
        // off the front.
        unit.push(2); // file_name_entry_format_count
        write_uleb128(DW_LNCT_DIRECTORY_INDEX, unit);
        write_uleb128(DW_FORM_UDATA, unit);
        write_uleb128(DW_LNCT_PATH, unit);
        write_uleb128(DW_FORM_LINE_STRP, unit);
        write_uleb128(tables.files.len() as u64 + 1, unit);
        for (index, base) in std::iter::once(&tables.files[0]).chain(tables.files.iter()) {
            write_uleb128(*index, unit);
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
    if files.is_empty() || locations.is_empty() {
        return Ok(None);
    }

    let file_table = debug_file_table(files)?;
    let file_index = |number: u32| -> Option<u64> {
        file_table
            .iter()
            .position(|(candidate, _)| *candidate == number)
            .map(|i| i as u64 + 1)
    };

    // GAS carries is_stmt and ISA across section switches. Resolve those
    // persistent states in source order before sorting rows into per-section
    // line programs; each program itself starts with DWARF defaults.
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

    let tables = line_tables(&file_table, comp_dir);

    let mut data = Vec::new();
    let mut relocs = Vec::new();
    let mut units = Vec::new();
    for (section_index, rows) in rows_by_section.iter_mut().enumerate() {
        if rows.is_empty() {
            continue;
        }
        units.push(DwarfLineUnit {
            offset: data.len() as u64,
            section_index,
        });
        rows.sort_by_key(|(row, _, _)| (row.offset, row.sequence));
        let code_section = &sections[section_index];
        let opcode_base = if config.version >= 3 {
            OPCODE_BASE
        } else {
            OPCODE_BASE_DWARF2 as u8
        };
        let mut unit = Vec::new();
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
        let table_relocs =
            append_line_prologue_tables(&mut unit, config, &tables, comp_dir, abs32, line_strings)?;
        // The tables name their paths through relocations written against the
        // unit's own start; the section needs the offsets the unit will have
        // once it is appended to it.
        relocs.extend(table_relocs.into_iter().map(|mut reloc| {
            reloc.offset += data.len() as u64;
            reloc
        }));
        let header_length = unit.len() - header_start;
        let header_length = u32::try_from(header_length)
            .map_err(|_| "DWARF line-table header exceeds 32-bit length".to_string())?;
        unit[header_length_at..header_length_at + 4].copy_from_slice(&header_length.to_le_bytes());

        // Set the initial code address. The linker resolves this relocation
        // against the executable section symbol and folds in the row offset.
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
            data.len(),
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
            // Special opcodes both advance the machine and emit the row in
            // one byte. Relocatable per-row addresses (used for linker
            // relaxation) must keep their set_address relocations instead.
            let special = if symbol_at.is_none() {
                line_special_opcode(advance, line_delta, u64::from(opcode_base))
            } else {
                None
            };
            if advance != 0 && special.is_none() {
                if let Some(symbol_at) = symbol_at.as_deref_mut() {
                    let symbol_name = symbol_at(section_index, row.offset);
                    push_set_address(
                        &mut unit,
                        &mut relocs,
                        data.len(),
                        address_size,
                        absolute_relocation,
                        symbol_name,
                        0,
                    );
                } else {
                    unit.push(2); // DW_LNS_advance_pc
                    write_uleb128(advance, &mut unit);
                }
            }
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
            if line_delta != 0 && special.is_none() {
                unit.push(3); // DW_LNS_advance_line
                write_sleb128(line_delta, &mut unit);
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
            // consumer does not have (GCC drops them the same way).
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
            unit.push(special.unwrap_or(1));
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
                    data.len(),
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

        let unit_length = u32::try_from(unit.len() - 4)
            .map_err(|_| "DWARF line-table unit exceeds 32-bit length".to_string())?;
        unit[..4].copy_from_slice(&unit_length.to_le_bytes());
        data.extend_from_slice(&unit);
    }

    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(DwarfLineBuild {
        line: debug_section(".debug_line", data, relocs),
        units,
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
/// * `DW_AT_high_pc` is an address through version 3 and a `DW_FORM_data8`
///   constant offset from `low_pc` from version 4 on.  A version 2 or 3 consumer
///   reads the constant as an address, which after linking names unrelated code,
///   and `data4` would cap a unit's code at 4 GiB;
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
            high_pc: if labels || !four {
                DW_FORM_ADDR
            } else {
                DW_FORM_DATA8
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
        }
    }
}

/// Declare every abbreviation of a unit and return the subprogram abbreviation
/// codes, indexed by `[has no declaration][is external]`.
///
/// One subprogram abbreviation per shape, numbered from 3.  `DW_FORM_flag_present`
/// says "this function is external" by its presence, so from version 4 on a
/// static function omits the attribute and needs its own abbreviation; versions 2
/// and 3 carry the flag's value and serve both shapes with one.
fn declare_unit_abbrevs(
    abbrev: &mut Vec<u8>,
    config: DwarfConfig,
    forms: UnitForms,
) -> [[u64; 2]; 2] {
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
    cu_attrs.extend_from_slice(&[
        (DW_AT_LOW_PC, DW_FORM_ADDR),
        (DW_AT_HIGH_PC, forms.high_pc),
        (DW_AT_STMT_LIST, forms.stmt_list),
    ]);
    declare_abbrev(abbrev, 1, DW_TAG_COMPILE_UNIT, true, &cu_attrs);
    // The unspecified return type GAS uses, so a debugger never assumes `int`.
    declare_abbrev(abbrev, 2, DW_TAG_UNSPECIFIED_TYPE, false, &[]);
    let mut subprogram_code = [[3u64; 2]; 2];
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
        }
        attrs.extend_from_slice(&[
            (DW_AT_TYPE, DW_FORM_REF4),
            (DW_AT_LOW_PC, DW_FORM_ADDR),
            (DW_AT_HIGH_PC, forms.high_pc),
        ]);
        if config.version >= 4 {
            let mut external_attrs = attrs.clone();
            external_attrs.insert(1, (DW_AT_EXTERNAL, forms.external));
            declare_abbrev(abbrev, next_code, DW_TAG_SUBPROGRAM, false, &external_attrs);
            subprogram_code[usize::from(!has_declaration)][1] = next_code;
            next_code += 1;
        }
        declare_abbrev(abbrev, next_code, DW_TAG_SUBPROGRAM, false, &attrs);
        let slot = &mut subprogram_code[usize::from(!has_declaration)];
        *slot = if config.version >= 4 {
            [next_code, slot[1]] // the static shape, then the external one
        } else {
            [next_code, next_code] // one shape carries the flag's value
        };
        next_code += 1;
    }
    abbrev.push(0);
    subprogram_code
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
/// and a `DW_FORM_data8` `low_pc`-relative constant from version 4 on,
/// `DW_AT_external` becomes a zero-byte `DW_FORM_flag_present` from version 4
/// on (which is why the abbreviation set has one shape per externality), and
/// `DW_AT_language` takes the smallest constant form that holds its code.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_dwarf_info_sections(
    units: &[DwarfLineUnit],
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
    mut symbol_at: SymbolAt<'_, '_>,
) -> Result<Option<DwarfInfoBuild>, String> {
    if units.is_empty() {
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
    let mut rows_by_section: Vec<Vec<(u64, u32, u32, u32)>> = vec![Vec::new(); sections.len()];
    for row in locations {
        if let Some(slot) = rows_by_section.get_mut(row.section_index) {
            slot.push((row.offset, row.file_number, row.line, row.column));
        }
    }
    for rows in &mut rows_by_section {
        rows.sort_unstable_by_key(|row| row.0);
    }
    // The assembler is told where code is, not where a declarator is, so the
    // declaration it can state honestly is the function's *first* source row:
    // for a definition that is the line the function is declared on, which is
    // what a debugger lists.  `DW_AT_decl_column` is deliberately omitted --
    // the first instruction's column is not the declarator's, and a wrong
    // column is worse than none.
    let declaration = |section_index: usize, offset: u64, size: u64| {
        let rows = rows_by_section.get(section_index)?;
        let end = offset.checked_add(size)?;
        let first = rows.partition_point(|row| row.0 < offset);
        let (row_offset, file_number, line, _column) = *rows.get(first)?;
        // The row must belong to this function: the first row at or after its
        // end describes the next one, and claiming it would attribute a
        // function to its neighbour's source line.
        if row_offset >= end || line == 0 || file_number == 0 {
            return None;
        }
        let file = file_table.iter().position(|(n, _)| *n == file_number)? + 1;
        Some((file as u64, u64::from(line)))
    };

    let mut abbrev = Vec::new();
    let subprogram_code = declare_unit_abbrevs(&mut abbrev, config, forms);

    let mut info = Vec::new();
    let mut relocs = Vec::new();
    // One address range per unit, keyed by the unit's offset in `.debug_info`.
    let mut ranges: Vec<(u64, &DwarfSectionInfo<'_>)> = Vec::with_capacity(units.len());
    let length32 = |value: u64, what: &str| {
        u32::try_from(value).map_err(|_| format!("DWARF {what} exceeds 32 bits"))
    };
    for unit in units {
        let code = sections.get(unit.section_index).ok_or_else(|| {
            format!(
                "internal: DWARF line unit refers to missing section {}",
                unit.section_index
            )
        })?;
        let start = info.len();
        ranges.push((start as u64, code));
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

        write_uleb128(1, &mut info);
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
        push_reloc(&mut info, &mut relocs, abs_addr, address_size, code.name, 0)?;
        if forms.high_pc == DW_FORM_ADDR {
            if let Some(symbol_at) = symbol_at.as_deref_mut() {
                let end = symbol_at(unit.section_index, code.size);
                push_reloc(&mut info, &mut relocs, abs_addr, address_size, &end, 0)?;
            } else {
                push_reloc(
                    &mut info,
                    &mut relocs,
                    abs_addr,
                    address_size,
                    code.name,
                    code.size,
                )?;
            }
        } else {
            info.extend_from_slice(&code.size.to_le_bytes());
        }
        push_reloc(&mut info, &mut relocs, abs32, 4, ".debug_line", unit.offset)?;

        let void_die = length32((info.len() - start) as u64, "unit")?;
        write_uleb128(2, &mut info);

        let mut members: Vec<&DwarfFunction<'_>> = functions
            .iter()
            .filter(|f| f.section_index == unit.section_index && f.size != 0)
            .filter(|f| {
                f.offset
                    .checked_add(f.size)
                    .is_some_and(|end| end <= code.size)
            })
            .collect();
        members.sort_by(|a, b| (a.offset, a.name).cmp(&(b.offset, b.name)));
        for function in members {
            if function.name.as_bytes().contains(&0) {
                return Err(format!(
                    "DWARF function name contains a NUL byte: {:?}",
                    function.name
                ));
            }
            let known = declaration(unit.section_index, function.offset, function.size);
            let abbrev_code =
                subprogram_code[usize::from(known.is_none())][usize::from(function.external)];
            write_uleb128(abbrev_code, &mut info);
            write_string_attribute(
                function.name,
                forms.string,
                &mut info,
                &mut relocs,
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
            if let Some((file, line)) = known {
                write_uleb128(file, &mut info);
                write_uleb128(line, &mut info);
            }
            info.extend_from_slice(&void_die.to_le_bytes());
            if let Some(symbol_at) = symbol_at.as_deref_mut() {
                let start = symbol_at(unit.section_index, function.offset);
                let end = symbol_at(unit.section_index, function.offset + function.size);
                push_reloc(&mut info, &mut relocs, abs_addr, address_size, &start, 0)?;
                push_reloc(&mut info, &mut relocs, abs_addr, address_size, &end, 0)?;
            } else {
                push_reloc(
                    &mut info,
                    &mut relocs,
                    abs_addr,
                    address_size,
                    code.name,
                    function.offset,
                )?;
                if forms.high_pc == DW_FORM_ADDR {
                    let end = function.offset.checked_add(function.size).ok_or_else(|| {
                        "internal: DWARF function range overflows the code section".to_string()
                    })?;
                    push_reloc(
                        &mut info,
                        &mut relocs,
                        abs_addr,
                        address_size,
                        code.name,
                        end,
                    )?;
                } else {
                    info.extend_from_slice(&function.size.to_le_bytes());
                }
            }
        }
        info.push(0); // end of the unit's children

        let unit_length = length32((info.len() - start - 4) as u64, "unit")?;
        info[start..start + 4].copy_from_slice(&unit_length.to_le_bytes());
    }
    Ok(Some(DwarfInfoBuild {
        info: debug_section(".debug_info", info, relocs),
        abbrev: debug_section(".debug_abbrev", abbrev, Vec::new()),
        aranges: build_debug_aranges(&ranges, address_size, abs32, abs_addr, labels)?,
    }))
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

/// Build `.debug_aranges`: the address map a consumer reads *before* the units,
/// so mapping an address to a compile unit costs one binary search instead of a
/// walk over every unit header.
///
/// One set per unit, exactly as GCC emits: the set names its unit's offset in
/// `.debug_info` (a relocation, because the linker concatenates the sections of
/// every object) and then lists the addresses the unit covers.  Addresses are
/// `address_size` bytes and the length that follows each one is the same size, so
/// for a compile unit that spans its code section there is one `(address,
/// length)` tuple -- followed by the zero tuple that ends the set.
///
/// A relaxing target gets no table.  Its lengths are computed before the linker
/// deletes bytes (RISC-V relaxation), so a range would be slightly too long and
/// would claim code that belongs to the next unit: an address map that is wrong
/// is worse than the scan a consumer falls back to.  `labels` is that case.
fn build_debug_aranges(
    ranges: &[(u64, &DwarfSectionInfo<'_>)],
    address_size: usize,
    abs32: u32,
    abs_addr: u32,
    labels: bool,
) -> Result<Option<ObjSection>, String> {
    if labels || ranges.is_empty() {
        return Ok(None);
    }
    // A tuple is an address and a length, and the specification requires the
    // first tuple of a set to start on a multiple of that size counted from the
    // set's own length field.
    let tuple = 2 * address_size;
    let mut data = Vec::new();
    let mut relocs = Vec::new();
    for &(info_offset, code) in ranges {
        if code.size == 0 {
            // A unit without bytes covers no address; an empty tuple would only
            // give a consumer something to skip.
            continue;
        }
        let set_start = data.len();
        data.extend_from_slice(&[0; 4]); // unit_length, patched below
        data.extend_from_slice(&2u16.to_le_bytes()); // version: always 2
        push_reloc(&mut data, &mut relocs, abs32, 4, ".debug_info", info_offset)?;
        data.push(u8::try_from(address_size).map_err(|_| "DWARF address size")?);
        data.push(0); // segment_selector_size: no segments
        let header = data.len() - set_start;
        let padding = (tuple - header % tuple) % tuple;
        data.resize(data.len() + padding, 0);
        push_reloc(&mut data, &mut relocs, abs_addr, address_size, code.name, 0)?;
        // The length is a constant tuple field, so it is `address_size` bytes
        // wide. A section larger than the target's address space is a malformed
        // input, not something to truncate silently.
        let size = code.size.to_le_bytes();
        if size[address_size..].iter().any(|&byte| byte != 0) {
            return Err(format!(
                "DWARF address range length {:#x} does not fit the target's {}-byte \
                 addresses",
                code.size, address_size
            ));
        }
        data.extend_from_slice(&size[..address_size]);
        data.resize(data.len() + tuple, 0); // the terminating zero tuple
        let length = u32::try_from(data.len() - set_start - 4)
            .map_err(|_| "DWARF address range set exceeds 32 bits".to_string())?;
        data[set_start..set_start + 4].copy_from_slice(&length.to_le_bytes());
    }
    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(debug_section(".debug_aranges", data, relocs)))
}

fn debug_section(name: &str, data: Vec<u8>, relocs: Vec<ObjReloc>) -> ObjSection {
    ObjSection {
        name: name.to_string(),
        sh_type: SHT_PROGBITS,
        sh_flags: 0,
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
const DW_AT_NAME: u64 = 0x03;
const DW_AT_STMT_LIST: u64 = 0x10;
const DW_AT_LOW_PC: u64 = 0x11;
const DW_AT_HIGH_PC: u64 = 0x12;
const DW_AT_LANGUAGE: u64 = 0x13;
const DW_AT_COMP_DIR: u64 = 0x1b;
const DW_AT_PRODUCER: u64 = 0x25;
const DW_AT_EXTERNAL: u64 = 0x3f;
const DW_AT_TYPE: u64 = 0x49;
/// `DW_AT_decl_file` and `DW_AT_decl_line`.
const DW_AT_DECL_FILE: u64 = 0x3a;
const DW_AT_DECL_LINE: u64 = 0x3b;
/// `DW_AT_language_name` and `DW_AT_language_version`: the DWARF 6 attributes
/// GCC 14 and newer emit so a unit can say "C, 202311" where no language code
/// exists yet.
const DW_AT_LANGUAGE_NAME: u64 = 0x90;
const DW_AT_LANGUAGE_VERSION: u64 = 0x91;
const DW_FORM_ADDR: u64 = 0x01;
const DW_FORM_DATA1: u64 = 0x0b;
const DW_FORM_DATA2: u64 = 0x05;
const DW_FORM_DATA4: u64 = 0x06;
const DW_FORM_DATA8: u64 = 0x07;
const DW_FORM_STRING: u64 = 0x08;
const DW_FORM_FLAG: u64 = 0x0c;
const DW_FORM_UDATA: u64 = 0x0f;
const DW_FORM_REF4: u64 = 0x13;
const DW_FORM_SEC_OFFSET: u64 = 0x17;
const DW_FORM_STRP: u64 = 0x0e;
/// `DW_FORM_line_strp`: an offset into `.debug_line_str`, the table a version-5
/// line program names its paths in.
const DW_FORM_LINE_STRP: u64 = 0x1f;
/// `DW_FORM_flag_present`: the attribute is true by its mere presence (DWARF 4).
const DW_FORM_FLAG_PRESENT: u64 = 0x19;
/// `DW_LNCT_path` and `DW_LNCT_directory_index`: the content types of the
/// version-5 directory and file name tables.
const DW_LNCT_PATH: u64 = 0x01;
const DW_LNCT_DIRECTORY_INDEX: u64 = 0x02;
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

/// Encode a special opcode for the header this writer emits: minimum
/// instruction length 1, line base -5, line range 14, and `opcode_base`
/// (13 since DWARF 3, 10 in DWARF 2). Return None when the pair does not fit.
fn line_special_opcode(address_delta: u64, line_delta: i64, opcode_base: u64) -> Option<u8> {
    if !(-5..=8).contains(&line_delta) {
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

fn write_sleb128(mut value: i64, out: &mut Vec<u8>) {
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

    fn text_section() -> Vec<DwarfSectionInfo<'static>> {
        vec![DwarfSectionInfo {
            name: ".text",
            size: 0x40,
            is_executable: true,
        }]
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
            DW_FORM_UDATA => (read_uleb(data, at), None),
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
        assert_eq!(
            line.units,
            [DwarfLineUnit {
                offset: 0,
                section_index: 0
            }]
        );
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
            &line.units,
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
            [(21, 0), (53, 10)],
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
        assert_eq!(form_of(DW_AT_HIGH_PC), Some(DW_FORM_DATA8));
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
                &line.units,
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
                &line.units,
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
            &line.units,
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
            &line.units,
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
    fn inline_tables(data: &[u8], version: u16) -> (Vec<String>, Vec<(u64, String)>) {
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
            &line.units,
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
        let units = [DwarfLineUnit {
            offset: 0,
            section_index: 0,
        }];
        let config = DwarfConfig::assembler(4);
        let mut strings = DwarfStrings::new();
        let mut line_strings = DwarfStrings::new();
        let info = build_dwarf_info_sections(
            &units,
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
            &line.units,
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
}

//! The debug-information model the compiler hands to the object writer.
//!
//! A debugger answers `print x` from three linked facts: a `DW_TAG_variable`
//! with a name, a `DW_AT_type` that describes the object's bytes, and a
//! `DW_AT_location` that says where those bytes are. The writer that emits the
//! `.debug_info` bytes knows assembly, not C, and the compiler knows C, not
//! section offsets -- so the facts are converted to plain data here, in one
//! place, and the writer's job stays mechanical.
//!
//! # Why a separate model instead of passing `CType` down
//!
//! The writer lives in the backend and the type system in `common`; naming
//! `CType` from the writer would drag struct layouts, typedef resolution and
//! the target's pointer width into a module that must stay byte-oriented. A
//! second reason is testability: the conversion from `CType` to DIEs is where
//! every "the debugger prints the wrong number" bug lives, and it is worth
//! having as a value that a unit test can assert on without an ELF file.
//!
//! # Interning
//!
//! Two `int` declarations must share one `DW_TAG_base_type`, while two
//! `struct S` definitions in different scopes must not: a debugger prints the
//! name, so type identity reaches the user. Every node is interned by its
//! structure, with children already interned, and a record reserves its slot
//! before its members are built so that `struct S { struct S *next; }` -- the
//! one cycle C can spell -- terminates.
//!
//! A typedef is a node of its own (`DW_TAG_typedef`) pointing at the type it
//! names, never a collapse into it: `typedef int myint; myint x;` must print
//! `myint`, and GCC 16.2 emits exactly that (measured: `DW_AT_type` of `x` is
//! the typedef DIE).

use std::sync::Arc;

use super::dwarf::DwarfConfig;
use crate::common::fx_hash::FxHashMap;
use crate::common::types::{CType, EnumType, FunctionType, PtrQuals, StructLayoutProvider};

/// A DWARF base type's identity: the source spelling, the encoding and the
/// size all come from one table entry, so two spellings cannot disagree with
/// their encoding by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BaseKind {
    Bool,
    Char,
    SChar,
    UChar,
    PlainUChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    Int128,
    UInt128,
    Float,
    Double,
    LongDouble,
    Float16,
    BFloat16,
    Float128,
    Decimal32,
    Decimal64,
    Decimal128,
    ComplexFloat,
    ComplexDouble,
    ComplexLongDouble,
}

/// `DW_ATE_*` encodings, as numbers because the writer's byte emitter takes
/// numbers; the names are here so the table below reads like the standard.
mod ate {
    pub(super) const ADDRESS: u16 = 0x01;
    pub(super) const BOOLEAN: u16 = 0x02;
    pub(super) const COMPLEX_FLOAT: u16 = 0x03;
    pub(super) const FLOAT: u16 = 0x04;
    pub(super) const SIGNED: u16 = 0x05;
    pub(super) const SIGNED_CHAR: u16 = 0x06;
    pub(super) const UNSIGNED: u16 = 0x07;
    pub(super) const UNSIGNED_CHAR: u16 = 0x08;
    pub(super) const DECIMAL_FLOAT: u16 = 0x0f;
}

impl BaseKind {
    /// The spelling GCC 16.2 emits for this type, measured one type per
    /// `-gdwarf-5` object (`tests/regression/check_dwarf_quality.py` pins the
    /// same strings).  A debugger shows this text, so it is the *source*
    /// spelling -- `short unsigned int`, not `ushort`, and `bool` rather than
    /// `_Bool`, which is what GCC writes.
    pub(crate) const fn spelling(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Char => "char",
            Self::SChar => "signed char",
            Self::UChar => "unsigned char",
            // Plain `char` on a target where it is unsigned: still spelled
            // `char`, but its encoding is unsigned.
            Self::PlainUChar => "char",
            Self::Short => "short int",
            Self::UShort => "short unsigned int",
            Self::Int => "int",
            Self::UInt => "unsigned int",
            Self::Long => "long int",
            Self::ULong => "long unsigned int",
            Self::LongLong => "long long int",
            Self::ULongLong => "long long unsigned int",
            Self::Int128 => "__int128",
            Self::UInt128 => "__int128 unsigned",
            Self::Float => "float",
            Self::Double => "double",
            Self::LongDouble => "long double",
            Self::Float16 => "_Float16",
            Self::BFloat16 => "__bf16",
            Self::Float128 => "_Float128",
            Self::Decimal32 => "_Decimal32",
            Self::Decimal64 => "_Decimal64",
            Self::Decimal128 => "_Decimal128",
            Self::ComplexFloat => "complex float",
            Self::ComplexDouble => "complex double",
            Self::ComplexLongDouble => "complex long double",
        }
    }

    pub(crate) const fn encoding(self) -> u16 {
        match self {
            Self::Bool => ate::BOOLEAN,
            Self::Char | Self::SChar => ate::SIGNED_CHAR,
            Self::UChar | Self::PlainUChar => ate::UNSIGNED_CHAR,
            Self::Short | Self::Int | Self::Long | Self::LongLong | Self::Int128 => ate::SIGNED,
            Self::UShort | Self::UInt | Self::ULong | Self::ULongLong | Self::UInt128 => {
                ate::UNSIGNED
            }
            Self::Float
            | Self::Double
            | Self::LongDouble
            | Self::Float16
            | Self::BFloat16
            | Self::Float128 => ate::FLOAT,
            Self::Decimal32 | Self::Decimal64 | Self::Decimal128 => ate::DECIMAL_FLOAT,
            Self::ComplexFloat | Self::ComplexDouble | Self::ComplexLongDouble => {
                ate::COMPLEX_FLOAT
            }
        }
    }
}

/// One address space qualifier that a DIE has to wrap a type in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TypeQual {
    Const,
    Volatile,
    Restrict,
    Atomic,
}

/// One member of a record, with the layout the compiler already computed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DwarfMember {
    pub name: Option<String>,
    pub ty: u32,
    /// Byte offset of the storage unit that holds the member.
    pub offset: u64,
    /// For a bit-field: its width and its bit offset inside that storage unit
    /// (`StructFieldLayout::bit_offset`, which is the layout's own answer).
    pub bit: Option<(u32, u32)>,
}

/// Whether a record is a struct or a union.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RecordKind {
    Struct,
    Union,
}

/// An interned type.  Children are indices into [`DwarfUnitSpec::types`], so
/// interning a node never re-walks a subgraph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DwarfType {
    Base {
        kind: BaseKind,
        size: u64,
    },
    Pointer {
        pointee: Option<u32>,
        size: u64,
    },
    /// `DW_TAG_const_type` and friends: a wrapper with no size of its own.
    Qualified {
        qual: TypeQual,
        target: u32,
    },
    Typedef {
        name: String,
        target: u32,
    },
    /// A C array.  The bounds are the *upper* bounds, outermost dimension
    /// first, because that is the order `int a[2][3]` prints and the order
    /// GCC 16.2 numbers its subranges (measured); `None` is the incomplete
    /// `int a[]`.
    Array {
        element: u32,
        bounds: Vec<Option<u64>>,
        /// `DW_AT_GNU_vector`: the array is a `vector_size` object, which a
        /// consumer must not treat as a subscriptable array.
        vector: bool,
    },
    Record {
        name: Option<String>,
        kind: RecordKind,
        size: u64,
        /// A declaration (`struct S;`) has no members and says so with
        /// `DW_AT_declaration`, so a consumer does not conclude `S` is empty.
        complete: bool,
        members: Vec<DwarfMember>,
    },
    Enum {
        name: Option<String>,
        size: u64,
        /// The integer type the enumerators live in, as `DW_AT_type`.
        underlying: u32,
        complete: bool,
        enumerators: Vec<(String, i64)>,
    },
    Subroutine {
        ret: Option<u32>,
        params: Vec<u32>,
        variadic: bool,
        prototyped: bool,
    },
    /// A type no consumer can evaluate -- C23's `nullptr_t`, whose only value
    /// is the null pointer and which has no integer representation.
    Unspecified {
        name: &'static str,
    },
}

/// A file-scope object: the unit's `DW_TAG_variable`.
#[derive(Clone, Debug)]
pub(crate) struct DwarfVariable {
    pub name: String,
    pub ty: u32,
    /// `DW_AT_external`: the object has external linkage, so the value lives in
    /// (or is visible to) another object.  GCC 16.2 emits the attribute for
    /// every non-static file-scope object and for nothing else (measured).
    pub external: bool,
    /// `DW_AT_declaration`: the unit only declares the object (`extern int x;`)
    /// and emits no storage, so there is no address to point at.
    pub declaration: bool,
    /// The source file the declaration sits in, by name: the writer owns the
    /// line program's file table and resolves the name to its index there, which
    /// keeps one numbering for `DW_AT_decl_file` and the rows.
    pub decl_file: Option<String>,
    pub decl_line: Option<u32>,
    pub decl_column: Option<u32>,
    /// The spelling to show for a `typedef`-declared object, so `print x`
    /// answers `myint` rather than `unsigned int`.
    pub file_scope: bool,
}

/// Renumber one type index, or report an unreachable one as an internal fault.
fn remap_of(remap: &[u32], index: u32) -> u32 {
    match remap.get(index as usize) {
        Some(&mapped) if mapped != u32::MAX => mapped,
        // A reference the mark phase missed is a bug in this file, not input:
        // the writer would otherwise emit a DIE naming a type that is not there.
        _ => unreachable!("DWARF type {index} was reachable but not marked"),
    }
}

impl DwarfType {
    /// Every type this one names, for the reachability walk in
    /// [`DwarfUnitSpec::with_referenced`].
    fn children(&self) -> Vec<u32> {
        match self {
            DwarfType::Base { .. } | DwarfType::Unspecified { .. } => Vec::new(),
            DwarfType::Pointer { pointee, .. } => pointee.iter().copied().collect(),
            DwarfType::Qualified { target, .. } => vec![*target],
            DwarfType::Typedef { target, .. } => vec![*target],
            DwarfType::Array { element, .. } => vec![*element],
            DwarfType::Record { members, .. } => members.iter().map(|member| member.ty).collect(),
            DwarfType::Enum { underlying, .. } => vec![*underlying],
            DwarfType::Subroutine { ret, params, .. } => {
                ret.iter().copied().chain(params.iter().copied()).collect()
            }
        }
    }

    /// Rewrite every index this type holds through `remap`.
    fn remap(&mut self, remap: &[u32]) {
        let fix = |index: &mut u32| *index = remap_of(remap, *index);
        match self {
            DwarfType::Base { .. } | DwarfType::Unspecified { .. } => {}
            DwarfType::Pointer { pointee, .. } => {
                if let Some(pointee) = pointee {
                    fix(pointee);
                }
            }
            DwarfType::Qualified { target, .. } => fix(target),
            DwarfType::Typedef { target, .. } => fix(target),
            DwarfType::Array { element, .. } => fix(element),
            DwarfType::Record { members, .. } => {
                for member in members {
                    fix(&mut member.ty);
                }
            }
            DwarfType::Enum { underlying, .. } => fix(underlying),
            DwarfType::Subroutine { ret, params, .. } => {
                if let Some(ret) = ret {
                    fix(ret);
                }
                for parameter in params {
                    fix(parameter);
                }
            }
        }
    }
}

/// Where a value's bytes are, as a `DW_AT_location` expression says it.
///
/// A location is a *promise*: a consumer prints whatever it finds there and
/// never re-checks it, so the only form this type names is one the frame layout
/// proved for the whole function body.  That is a constant displacement from the
/// canonical frame address (`DW_OP_fbreg`, the expression `DW_AT_frame_base =
/// DW_OP_call_frame_cfa` makes readable): the frame address and the slot are both
/// fixed once the prologue has run, so their difference is a compile-time
/// constant that stays true at every instruction -- no location list, no
/// `DW_OP_piece`, and nothing to get wrong when the function has a branch.
///
/// A value the layout cannot promise (a parameter homed in a register, an
/// over-aligned automatic object whose address is aligned at run time, a frame
/// with no stable anchor) gets `None` instead.  A consumer renders that as
/// `<optimized out>`; a wrong expression would render as a confident wrong
/// number, which is the one failure mode DWARF cannot recover from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DwarfLocation {
    /// `DW_OP_fbreg <delta>`: the value starts `delta` bytes from the canonical
    /// frame address, whose rule is `DW_AT_frame_base` (`DW_OP_call_frame_cfa`).
    FrameOffset(i32),
}

impl DwarfLocation {
    /// The expression's bytes, without the length that a block form puts in
    /// front: `DW_OP_fbreg` (one opcode) then a signed LEB128 displacement.
    pub(crate) fn expression(&self) -> Vec<u8> {
        match *self {
            DwarfLocation::FrameOffset(delta) => {
                let mut out = Vec::with_capacity(6);
                out.push(super::dwarf::DW_OP_FBREG);
                super::dwarf::write_sleb128(i64::from(delta), &mut out);
                out
            }
        }
    }
}

/// One parameter of a function this unit defines: the `DW_TAG_formal_parameter`
/// a debugger lists `pq_frame` as taking.
///
/// The type is what makes the DIE worth emitting -- an unnamed parameter is a
/// prototype's parameter and names a type, never a value -- and the name is what
/// lets a consumer ask for it by the spelling the user wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DwarfParameter {
    /// `None` for an unnamed parameter: `void f(int)` declares a type, not a
    /// value, and GCC 16.2 omits `DW_AT_name` there (measured).
    pub name: Option<String>,
    pub ty: u32,
    /// Where the value is, once the code generator has laid the frame out.  The
    /// analyser builds the parameter list before any frame exists, so this is
    /// filled in later, by [`DwarfUnitSpec::apply_frame_facts`], and stays
    /// `None` for a parameter the frame does not give a trustworthy home.
    pub location: Option<DwarfLocation>,
}

/// The parameter list of one defined function.
#[derive(Clone, Debug, Default)]
pub(crate) struct DwarfFunctionParameters {
    pub parameters: Vec<DwarfParameter>,
    /// `DW_TAG_unspecified_parameters`: the function takes more arguments than
    /// the ones listed, so a debugger must not conclude the list is complete.
    pub variadic: bool,
}

impl DwarfFunctionParameters {
    pub(crate) fn is_empty(&self) -> bool {
        self.parameters.is_empty() && !self.variadic
    }
}

/// Everything the writer needs to describe a translation unit's data.
#[derive(Clone, Debug, Default)]
pub(crate) struct DwarfUnitSpec {
    pub types: Vec<DwarfType>,
    pub variables: Vec<DwarfVariable>,
    /// Return type of each function the unit defines, by symbol name.
    pub return_types: FxHashMap<String, u32>,
    /// Parameter list of each function the unit defines, by symbol name.  A
    /// function with no parameters has no entry, so the writer knows it can
    /// describe the subprogram without children.
    pub parameters: FxHashMap<String, DwarfFunctionParameters>,
}

impl DwarfUnitSpec {
    pub(crate) fn is_empty(&self) -> bool {
        self.variables.is_empty() && self.types.is_empty()
    }

    /// The parameter list of a defined function, when it has one.
    pub(crate) fn parameters_of(&self, name: &str) -> Option<&DwarfFunctionParameters> {
        self.parameters.get(name).filter(|list| !list.is_empty())
    }

    /// Give every parameter the home the code generator's frame layout gave it.
    ///
    /// The parameter *list* is built from the analysis, which runs before any
    /// frame exists; the offsets come from the instruction selector, which is the
    /// only component that knows where a parameter's bytes ended up.  Both sides
    /// are keyed by the emitted symbol name, and the offsets are applied
    /// positionally inside a list.
    ///
    /// A length mismatch is treated as "no facts" rather than as a partial
    /// match.  The two lists come from different layers (a source-level
    /// parameter list and an IR-level one, which a hidden return pointer could
    /// grow), and applying a prefix of one to the other would attach the third
    /// parameter's slot to the fourth parameter's name -- the exact class of
    /// confident lie this whole module is arranged to avoid.  Nothing is lost by
    /// declining: the DIEs simply carry no location.
    pub(crate) fn apply_frame_facts(&mut self, facts: &crate::backend::debug_frame::DebugFrameFacts) {
        for (name, offsets) in &facts.parameters {
            let Some(list) = self.parameters.get_mut(name) else {
                continue;
            };
            if offsets.len() != list.parameters.len() {
                continue;
            }
            for (parameter, offset) in list.parameters.iter_mut().zip(offsets) {
                parameter.location = offset.map(DwarfLocation::FrameOffset);
            }
        }
    }

    /// Drop the declarations this object does not reference, and every type that
    /// only those declarations named.
    ///
    /// A C translation unit sees the declarations of every header it includes --
    /// `<stdio.h>` alone declares `stdin`, `stdout`, `stderr`, `sys_errlist` and
    /// friends, and each of those names the whole `_IO_FILE` structure. Describing
    /// all of them costs a debug payload a program never asked for: measured on
    /// `tests/benchmark/programs/fib.c`, lccc's `.debug_info` was 578 bytes
    /// against GCC 16.2's 249 and Clang 23's 133, almost all of it `_IO_FILE`'s
    /// 216-byte layout and its thirty members, reachable only from an `extern
    /// FILE *stdout` the program never mentions.
    ///
    /// GCC 16.2's rule (measured on `extern int q_missing;` next to a used
    /// `extern int q_used;`) is: a declaration gets a `DW_TAG_variable` exactly
    /// when the unit's code references it. That is precisely the set of names the
    /// object file relocates against, which is what `referenced` holds. A
    /// definition is kept either way -- it has storage in this file, and a
    /// debugger must be able to name it.
    pub(crate) fn with_referenced(&self, referenced: &FxHashMap<String, ()>) -> DwarfUnitSpec {
        if self.variables.iter().all(|variable| !variable.declaration) {
            // Nothing to drop; the type table is exactly as reachable as it was.
            return self.clone();
        }
        let variables: Vec<DwarfVariable> = self
            .variables
            .iter()
            .filter(|variable| !variable.declaration || referenced.contains_key(&variable.name))
            .cloned()
            .collect();
        // The types reachable from what survived: every object's type, every
        // function's return type, every parameter's type, and -- transitively --
        // whatever those name.
        let mut keep = vec![false; self.types.len()];
        let mut work: Vec<u32> = Vec::new();
        let mut root = |index: u32| {
            if let Some(slot) = keep.get_mut(index as usize)
                && !*slot
            {
                *slot = true;
                work.push(index);
            }
        };
        for variable in &variables {
            root(variable.ty);
        }
        for index in self.return_types.values() {
            root(*index);
        }
        for list in self.parameters.values() {
            for parameter in &list.parameters {
                root(parameter.ty);
            }
        }
        while let Some(index) = work.pop() {
            let Some(ty) = self.types.get(index as usize) else {
                continue;
            };
            for child in ty.children() {
                if let Some(slot) = keep.get_mut(child as usize)
                    && !*slot
                {
                    *slot = true;
                    work.push(child);
                }
            }
        }
        // Renumber: the DIEs hold indices, so dropping one shifts every later
        // reference.
        let mut remap = vec![u32::MAX; self.types.len()];
        let mut types = Vec::with_capacity(keep.iter().filter(|&&kept| kept).count());
        for (index, &kept) in keep.iter().enumerate() {
            if kept {
                remap[index] = u32::try_from(types.len()).unwrap_or(u32::MAX);
                types.push(self.types[index].clone());
            }
        }
        for ty in &mut types {
            ty.remap(&remap);
        }
        let variables = variables
            .into_iter()
            .map(|mut variable| {
                variable.ty = remap_of(&remap, variable.ty);
                variable
            })
            .collect();
        let return_types = self
            .return_types
            .iter()
            .map(|(name, index)| (name.clone(), remap_of(&remap, *index)))
            .collect();
        let parameters = self
            .parameters
            .iter()
            .map(|(name, list)| {
                let mut list = list.clone();
                for parameter in &mut list.parameters {
                    parameter.ty = remap_of(&remap, parameter.ty);
                }
                (name.clone(), list)
            })
            .collect();
        DwarfUnitSpec {
            types,
            variables,
            return_types,
            parameters,
        }
    }

    /// The index of the one `void`-ish type a subprogram's unknown return type
    /// already points at, if the unit has no better answer.
    pub(crate) fn base(&self, kind: BaseKind) -> Option<u32> {
        self.types
            .iter()
            .position(|ty| matches!(ty, DwarfType::Base { kind: found, .. } if *found == kind))
            .and_then(|index| u32::try_from(index).ok())
    }
}

/// What a compiler hands the object writer when `-g` is on: the dialect the
/// sections are written in, plus the unit description the writer could not have
/// inferred from the text.
///
/// Bundled rather than passed as two arguments because a dialect without a unit
/// is a valid request (`-g` on hand-written assembly) while a unit without a
/// dialect cannot be written at all, so the type makes the useless state
/// unrepresentable for the compiler's own paths.
#[derive(Clone, Debug)]
pub struct DwarfDebug {
    pub config: DwarfConfig,
    /// `None` for input that is only assembly: its `.loc` rows are all the debug
    /// information there is.
    pub unit: Option<Arc<DwarfUnitSpec>>,
}

impl DwarfDebug {
    /// A dialect with no analysed unit: what an assembler-only input gets.
    pub(crate) fn dialect(config: DwarfConfig) -> Self {
        Self { config, unit: None }
    }

    pub(crate) fn with_unit(config: DwarfConfig, unit: DwarfUnitSpec) -> Self {
        Self {
            config,
            unit: Some(Arc::new(unit)),
        }
    }
}

/// Builds the interned type table for one translation unit.
pub(crate) struct TypeGraph<'a> {
    layouts: &'a dyn StructLayoutProvider,
    typedefs: &'a FxHashMap<String, CType>,
    types: Vec<DwarfType>,
    index: FxHashMap<DwarfType, u32>,
    /// Records and enums already reserved, keyed by their `CType` identity:
    /// a member that names its own record finds the slot before it is filled,
    /// which is what makes the one cycle C can spell terminate.
    reserved: FxHashMap<String, u32>,
    pointer_size: u64,
    long_size: u64,
    char_is_signed: bool,
}

impl<'a> TypeGraph<'a> {
    pub(crate) fn new(
        layouts: &'a dyn StructLayoutProvider,
        typedefs: &'a FxHashMap<String, CType>,
    ) -> Self {
        // The target's own answers, not the host's: a 32-bit unit describes
        // 4-byte pointers and 4-byte longs.
        let pointer_size = CType::Pointer(Box::new(CType::Void), PtrQuals::GENERIC).size() as u64;
        let long_size = CType::Long.size() as u64;
        Self {
            layouts,
            typedefs,
            types: Vec::new(),
            index: FxHashMap::default(),
            reserved: FxHashMap::default(),
            pointer_size,
            long_size,
            char_is_signed: matches!(CType::plain_char(), CType::Char),
        }
    }

    pub(crate) fn into_types(self) -> Vec<DwarfType> {
        self.types
    }

    fn intern(&mut self, ty: DwarfType) -> u32 {
        if let Some(&index) = self.index.get(&ty) {
            return index;
        }
        let index = u32::try_from(self.types.len()).expect("a unit with 4 billion types");
        self.types.push(ty.clone());
        self.index.insert(ty, index);
        index
    }

    fn reserve(&mut self, key: &str, ty: DwarfType) -> (u32, bool) {
        if let Some(&index) = self.reserved.get(key) {
            return (index, false);
        }
        let index = u32::try_from(self.types.len()).expect("a unit with 4 billion types");
        self.types.push(ty);
        self.reserved.insert(key.to_string(), index);
        (index, true)
    }

    fn fill(&mut self, index: u32, ty: DwarfType) {
        self.types[index as usize] = ty.clone();
        self.index.insert(ty, index);
    }

    /// The DIE index for a C type, building every node it needs.
    pub(crate) fn intern_ctype(&mut self, ty: &CType) -> u32 {
        match ty {
            CType::Void => self.intern(DwarfType::Unspecified { name: "void" }),
            CType::Bool => self.base(BaseKind::Bool, 1),
            CType::Char => {
                if self.char_is_signed {
                    self.base(BaseKind::Char, 1)
                } else {
                    self.base(BaseKind::PlainUChar, 1)
                }
            }
            CType::SChar => self.base(BaseKind::SChar, 1),
            CType::UChar => self.base(BaseKind::UChar, 1),
            CType::PlainUChar => self.base(BaseKind::PlainUChar, 1),
            CType::Short => self.base(BaseKind::Short, 2),
            CType::UShort => self.base(BaseKind::UShort, 2),
            CType::Int => self.base(BaseKind::Int, 4),
            CType::UInt => self.base(BaseKind::UInt, 4),
            CType::Long => self.base(BaseKind::Long, self.long_size),
            CType::ULong => self.base(BaseKind::ULong, self.long_size),
            CType::LongLong => self.base(BaseKind::LongLong, 8),
            CType::ULongLong => self.base(BaseKind::ULongLong, 8),
            CType::Int128 => self.base(BaseKind::Int128, 16),
            CType::UInt128 => self.base(BaseKind::UInt128, 16),
            CType::Float => self.base(BaseKind::Float, 4),
            CType::Double => self.base(BaseKind::Double, 8),
            CType::LongDouble => self.base(BaseKind::LongDouble, ty.size() as u64),
            CType::Float16 => self.base(BaseKind::Float16, 2),
            CType::BFloat16 => self.base(BaseKind::BFloat16, 2),
            CType::Float128 => self.base(BaseKind::Float128, 16),
            CType::Decimal32 => self.base(BaseKind::Decimal32, 4),
            CType::Decimal64 => self.base(BaseKind::Decimal64, 8),
            CType::Decimal128 => self.base(BaseKind::Decimal128, 16),
            CType::ComplexFloat => self.base(BaseKind::ComplexFloat, 8),
            CType::ComplexDouble => self.base(BaseKind::ComplexDouble, 16),
            CType::ComplexLongDouble => self.base(BaseKind::ComplexLongDouble, ty.size() as u64),
            CType::Pointer(pointee, quals) => {
                let inner = self.intern_ctype(pointee);
                let sized = self.inner(inner);
                let size = self.pointer_size;
                // The qualifiers of a pointer declaration (`const char *p`)
                // belong to the *pointee*, which is why the wrapper goes
                // inside the pointer DIE: a consumer that dereferences `p`
                // must see `const char`.
                let target = self.wrap_quals(sized, *quals);
                self.intern(DwarfType::Pointer {
                    pointee: Some(target),
                    size,
                })
            }
            CType::NullPtr => self.intern(DwarfType::Unspecified { name: "nullptr_t" }),
            CType::Array(element, count) => {
                let element = self.intern_ctype(element);
                let bounds = vec![count.map(|count| (count as u64).saturating_sub(1))];
                self.intern(DwarfType::Array {
                    element,
                    bounds,
                    vector: false,
                })
            }
            CType::Function(function) => self.subroutine(function),
            CType::Struct(key) => self.record(key, RecordKind::Struct),
            CType::Union(key) => self.record(key, RecordKind::Union),
            CType::Enum(enumerated) => self.enumeration(enumerated),
            CType::Vector(element, bytes) => {
                let element_index = self.intern_ctype(element);
                let lane = element.size().max(1) as u64;
                let count = (*bytes as u64).div_ceil(lane).max(1);
                self.intern(DwarfType::Array {
                    element: element_index,
                    bounds: vec![Some(count - 1)],
                    vector: true,
                })
            }
            CType::ExtVector(element, layout) => {
                let element_index = self.intern_ctype(element);
                let count = layout.lanes.max(1) as u64;
                self.intern(DwarfType::Array {
                    element: element_index,
                    bounds: vec![Some(count - 1)],
                    vector: true,
                })
            }
        }
    }

    /// The DIE for a declared type, spelled with the typedef its declaration
    /// wrote.
    ///
    /// `CType` erases the alias: `typedef unsigned long size_t; void f(size_t)`
    /// and `void f(unsigned long)` are one type, and the analyser hands the
    /// writer that type plus, separately, the name the declaration spelled.
    /// This puts the name back on the DIE -- `size_t` rather than
    /// `long unsigned int` -- and reapplies the derived layers the declaration
    /// had above it, so `pair_t *p` describes a pointer to `pair_t` and not a
    /// flattened `struct pair *`.
    ///
    /// The alias is used only when it still names the type the declaration had:
    /// one unit may spell `typedef int myint;` and later
    /// `typedef double myint;`, and describing the second one's objects as
    /// `myint` while the field says `double` would be a confident lie.  In that
    /// case the structural description wins, which is still correct, just less
    /// legible -- the same rule the object path always used, now expressed once.
    pub(crate) fn intern_written(&mut self, ty: &CType, alias: Option<&str>) -> u32 {
        let Some(alias) = alias else {
            return self.intern_ctype(ty);
        };
        // The alias the declaration wrote has to still name what the declaration
        // had: one unit may write `typedef int myint;` and later
        // `typedef double myint;`, and the table holds only the second.  A DIE
        // that says `myint` over a `DW_AT_type` of `double`, attached to an `int`
        // object, is worse than a structural description -- it is wrong.
        let Some(named) = self.typedefs.get(alias).cloned() else {
            return self.intern_ctype(ty);
        };
        // The declaration's type, exactly.  Checked before anything is peeled:
        // an alias can name a pointer itself (`typedef int (*fn_t)(int)`), and
        // looking inside it would find the signature rather than the alias.
        if *ty == named {
            let target = self.intern_ctype(&named);
            return self.intern_typedef(alias, target);
        }
        // Otherwise the declaration added layers of its own above the name.  Only
        // pointer and array layers are transparent here, and both are rebuilt
        // around the alias below, innermost first, so `const pair_t *` describes a
        // pointer to a `const pair_t` and not a `pair_t` to a `const struct`.
        enum Layer {
            Pointer(PtrQuals),
            Array(Option<usize>),
        }
        let mut layers: Vec<Layer> = Vec::new();
        let mut core: &CType = ty;
        loop {
            match core {
                CType::Pointer(inner, quals) => {
                    layers.push(Layer::Pointer(*quals));
                    core = inner;
                }
                CType::Array(inner, count) => {
                    layers.push(Layer::Array(*count));
                    core = inner;
                }
                _ => break,
            }
        }
        if *core != named {
            // The declaration named a type the alias does not describe -- a
            // conversion through a different derived form.  The structural DIE is
            // correct, so it wins.
            return self.intern_ctype(ty);
        }
        let target = self.intern_ctype(&named);
        let mut index = self.intern_typedef(alias, target);
        for layer in layers.iter().rev() {
            index = match layer {
                Layer::Pointer(quals) => {
                    let sized = self.inner(index);
                    let target = self.wrap_quals(sized, *quals);
                    self.intern(DwarfType::Pointer {
                        pointee: Some(target),
                        size: self.pointer_size,
                    })
                }
                Layer::Array(count) => self.intern(DwarfType::Array {
                    element: index,
                    bounds: vec![count.map(|count| (count as u64).saturating_sub(1))],
                    vector: false,
                }),
            };
        }
        index
    }

    /// A `typedef` visible to this unit: a `DW_TAG_typedef` over `target`, so
    /// a debugger prints the alias the source used.
    pub(crate) fn intern_typedef(&mut self, name: &str, target: u32) -> u32 {
        self.intern(DwarfType::Typedef {
            name: name.to_string(),
            target,
        })
    }

    /// The index of a type whose name the caller resolves itself (a typedef of
    /// a C type by name, for example).
    pub(crate) fn resolve_typedef(&self, name: &str) -> Option<&CType> {
        self.typedefs.get(name)
    }

    fn inner(&self, index: u32) -> u32 {
        index
    }

    fn base(&mut self, kind: BaseKind, size: u64) -> u32 {
        self.intern(DwarfType::Base { kind, size })
    }

    fn wrap_quals(&mut self, target: u32, quals: PtrQuals) -> u32 {
        // Outermost-first wrapping keeps the DIE readable: `const volatile T`
        // reads const(volatile(T)).  The bits are only meaningful when the
        // declaration recorded them; an unrecorded qualifier set emits no
        // wrapper rather than a guess.
        let Some(bits) = quals.cvr() else {
            return target;
        };
        let mut index = target;
        for (flag, qual) in [
            (PtrQuals::VOLATILE, TypeQual::Volatile),
            (PtrQuals::CONST, TypeQual::Const),
            (PtrQuals::RESTRICT, TypeQual::Restrict),
        ] {
            if bits & flag != 0 {
                let wrapped = self.intern(DwarfType::Qualified {
                    qual,
                    target: index,
                });
                index = wrapped;
            }
        }
        index
    }

    fn subroutine(&mut self, function: &FunctionType) -> u32 {
        // C's `void` return has no type in DWARF: a subroutine DIE with no
        // `DW_AT_type` says "returns nothing", and a consumer then refuses to
        // print a value rather than inventing one.
        let ret = match &function.return_type {
            CType::Void => None,
            other => Some(self.intern_ctype(other)),
        };
        let mut params = Vec::with_capacity(function.params.len());
        for (ty, _) in &function.params {
            params.push(self.intern_ctype(ty));
        }
        self.intern(DwarfType::Subroutine {
            ret,
            params,
            variadic: function.variadic,
            prototyped: function.has_prototype,
        })
    }

    fn record(&mut self, key: &str, kind: RecordKind) -> u32 {
        let placeholder = DwarfType::Record {
            name: None,
            kind,
            size: 0,
            complete: false,
            members: Vec::new(),
        };
        let (index, fresh) = self.reserve(key, placeholder);
        if !fresh {
            return index;
        }
        let Some(layout) = self.layouts.get_struct_layout(key) else {
            // A record whose layout this unit does not know cannot be
            // described: an incomplete type is the truthful answer, and the
            // name is the one the key spells.
            self.fill(
                index,
                DwarfType::Record {
                    name: record_name(key),
                    kind,
                    size: 0,
                    complete: false,
                    members: Vec::new(),
                },
            );
            return index;
        };
        let mut members = Vec::with_capacity(layout.fields.len());
        for field in &layout.fields {
            let ty = self.intern_written(&field.ty, field.written_typedef.as_deref());
            members.push(DwarfMember {
                name: (!field.name.is_empty()).then(|| field.name.clone()),
                ty,
                offset: field.offset as u64,
                bit: match (field.bit_width, field.bit_offset) {
                    (Some(width), Some(offset)) if width > 0 => Some((width, offset)),
                    _ => None,
                },
            });
        }
        self.fill(
            index,
            DwarfType::Record {
                name: record_name(key),
                kind,
                size: layout.size as u64,
                complete: layout.is_complete,
                members,
            },
        );
        index
    }

    fn enumeration(&mut self, enumerated: &EnumType) -> u32 {
        // The enumerators' storage type is the enum's own compatible type --
        // `int` for a plain C enum, `unsigned int` for an all-non-negative
        // `-fshort-enums` one -- so the DIE never contradicts the width the
        // compiler gave the object.
        let compatible = enumerated.compatible_ctype();
        let underlying = self.intern_ctype(&compatible);
        let size = compatible.size() as u64;
        let key = match &enumerated.name {
            Some(name) => format!("enum.{name}"),
            None => format!("enum@{}", enumerated.identity.map_or(0, |id| id.get())),
        };
        let placeholder = DwarfType::Enum {
            name: enumerated.name.clone(),
            size,
            underlying,
            complete: false,
            enumerators: Vec::new(),
        };
        let (index, fresh) = self.reserve(&key, placeholder);
        if !fresh {
            return index;
        }
        self.fill(
            index,
            DwarfType::Enum {
                name: enumerated.name.clone(),
                size,
                underlying,
                complete: !enumerated.variants.is_empty(),
                enumerators: enumerated.variants.clone(),
            },
        );
        index
    }
}

/// The user-visible tag of a record key: `struct.S`/`union.U` and the `#N`
/// scoping suffix are the compiler's internal spelling, not the source's.
pub(crate) fn record_name(key: &str) -> Option<String> {
    let tag = key
        .strip_prefix("struct.")
        .or_else(|| key.strip_prefix("union."))
        .unwrap_or(key);
    // Anonymous records get a generated key; a consumer must see no name
    // rather than `__anon_struct_7`.
    if tag.starts_with("__anon") || tag.starts_with('.') || tag.is_empty() {
        return None;
    }
    let name = match tag.find('#') {
        Some(hash) => &tag[..hash],
        None => tag,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, SsoMode, StructFieldLayout, StructLayout};
    use std::rc::Rc;

    fn empty_layouts() -> FxHashMap<String, Rc<StructLayout>> {
        FxHashMap::default()
    }

    fn graph<'a>(
        layouts: &'a FxHashMap<String, Rc<StructLayout>>,
        typedefs: &'a FxHashMap<String, CType>,
    ) -> TypeGraph<'a> {
        TypeGraph::new(layouts, typedefs)
    }

    #[test]
    fn two_ints_share_one_die() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let first = types.intern_ctype(&CType::Int);
        let second = types.intern_ctype(&CType::Int);
        assert_eq!(first, second, "an interned base type is one DIE");
        assert_eq!(types.into_types().len(), 1);
    }

    #[test]
    fn signedness_is_part_of_identity() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let signed = types.intern_ctype(&CType::Int);
        let unsigned = types.intern_ctype(&CType::UInt);
        assert_ne!(signed, unsigned, "int and unsigned int are distinct DIEs");
    }

    #[test]
    fn a_record_reserves_its_slot_before_its_members() {
        // `struct Node { struct Node *next; };`: the member's pointee is the
        // record itself, so the walk must find the reserved slot rather than
        // recurse for ever.
        let mut layouts = empty_layouts();
        layouts.insert(
            "Node".to_string(),
            Rc::new(StructLayout {
                is_complete: true,
                fields: vec![StructFieldLayout {
                    written_typedef: None,
                    name: "next".to_string(),
                    offset: 0,
                    ty: CType::Pointer(Box::new(CType::Struct("Node".into())), PtrQuals::GENERIC),
                    bit_offset: None,
                    bit_width: None,
                    sso: SsoMode::None,
                    sso_storage_ty: None,
                    base_volatile: false,
                    base_const: false,
                    object_volatile: false,
                    object_const: false,
                    counted_by: None,
                    pointer_quals: Vec::new(),
                    own_cvr: None,
                }],
                size: 8,
                align: 8,
                is_union: false,
                is_transparent_union: false,
                requires_designators: false,
                reverse_sso: false,
                user_align: false,
                member_align: 4,
            }),
        );
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let node = types.intern_ctype(&CType::Struct("Node".into()));
        let built = types.into_types();
        let DwarfType::Record {
            name,
            complete,
            members,
            size,
            ..
        } = &built[node as usize]
        else {
            panic!("expected a record, got {:?}", built[node as usize]);
        };
        assert_eq!(name.as_deref(), Some("Node"));
        assert!(complete);
        assert_eq!(*size, 8);
        assert_eq!(members.len(), 1);
        let DwarfType::Pointer { pointee, size } = &built[members[0].ty as usize] else {
            panic!("expected a pointer");
        };
        assert_eq!(
            *pointee,
            Some(node),
            "the pointer's pointee is the record itself"
        );
        assert_eq!(*size, 8);
    }

    #[test]
    fn a_typedef_stays_a_die_of_its_own() {
        let layouts = empty_layouts();
        let mut typedefs = FxHashMap::default();
        typedefs.insert("myuint".to_string(), CType::UInt);
        let mut types = graph(&layouts, &typedefs);
        let inner = types.intern_ctype(&CType::UInt);
        let alias = types.intern_typedef("myuint", inner);
        assert_ne!(
            alias, inner,
            "collapsing the alias would print `unsigned int`"
        );
        let built = types.into_types();
        assert!(matches!(
            &built[alias as usize],
            DwarfType::Typedef { name, target } if name == "myuint" && *target == inner
        ));
    }

    #[test]
    fn a_written_typedef_becomes_a_typedef_die() {
        let layouts = empty_layouts();
        let mut typedefs = FxHashMap::default();
        typedefs.insert("size_t".to_string(), CType::ULong);
        let mut types = graph(&layouts, &typedefs);
        let written = types.intern_written(&CType::ULong, Some("size_t"));
        let plain = types.intern_ctype(&CType::ULong);
        let built = types.into_types();
        let (written, plain) = (&built[written as usize], &built[plain as usize]);
        assert_ne!(
            written, plain,
            "a `size_t` parameter must not collapse into `long unsigned int`"
        );
        assert!(
            matches!(written, DwarfType::Typedef { name, .. } if name == "size_t"),
            "{written:?}"
        );
        assert!(
            matches!(plain, DwarfType::Base { .. }),
            "the structural type is still a base type"
        );
    }

    #[test]
    fn a_written_typedef_keeps_the_pointer_above_it() {
        let layouts = empty_layouts();
        let mut typedefs = FxHashMap::default();
        typedefs.insert("myint".to_string(), CType::Int);
        let mut types = graph(&layouts, &typedefs);
        let pointer = CType::Pointer(Box::new(CType::Int), PtrQuals::GENERIC);
        let index = types.intern_written(&pointer, Some("myint"));
        let built = types.into_types();
        let DwarfType::Pointer { pointee, .. } = &built[index as usize] else {
            panic!(
                "`myint *` is a pointer, not a typedef: {:?}",
                &built[index as usize]
            );
        };
        let pointee = &built[pointee.expect("a pointer DIE names what it points at") as usize];
        assert!(
            matches!(pointee, DwarfType::Typedef { name, .. } if name == "myint"),
            "the alias belongs under the pointer, not around it: {pointee:?}"
        );
    }

    #[test]
    fn a_written_typedef_keeps_the_pointee_qualifier_outside_it() {
        let layouts = empty_layouts();
        let mut typedefs = FxHashMap::default();
        typedefs.insert("myint".to_string(), CType::Int);
        let mut types = graph(&layouts, &typedefs);
        let pointer = CType::Pointer(
            Box::new(CType::Int),
            PtrQuals::recorded(AddressSpace::Default, PtrQuals::CONST),
        );
        let index = types.intern_written(&pointer, Some("myint"));
        let built = types.into_types();
        let DwarfType::Pointer { pointee, .. } = &built[index as usize] else {
            panic!("`const myint *` is a pointer");
        };
        let qualified = &built[pointee.unwrap() as usize];
        let DwarfType::Qualified { target, .. } = qualified else {
            panic!("`const` belongs on the pointee: {qualified:?}");
        };
        assert!(
            matches!(&built[*target as usize], DwarfType::Typedef { name, .. } if name == "myint"),
            "`const myint` is a qualified typedef, not a qualified int"
        );
    }

    #[test]
    fn a_typedef_that_stopped_naming_the_declared_type_is_not_used() {
        let layouts = empty_layouts();
        let mut typedefs = FxHashMap::default();
        // `typedef int myint;` came first, and a later `typedef double myint;`
        // won the table: describing an `int` object as `myint` would now lie.
        typedefs.insert("myint".to_string(), CType::Double);
        let mut types = graph(&layouts, &typedefs);
        let written = types.intern_written(&CType::Int, Some("myint"));
        let plain = types.intern_ctype(&CType::Int);
        let built = types.into_types();
        assert_eq!(
            written, plain,
            "a stale alias describes a different type, so the structural DIE wins"
        );
        assert!(
            !built
                .iter()
                .any(|node| matches!(node, DwarfType::Typedef { name, .. } if name == "myint")),
            "no typedef DIE may claim an alias that names something else"
        );
    }

    #[test]
    fn a_written_typedef_of_an_array_keeps_the_array() {
        let layouts = empty_layouts();
        let mut typedefs = FxHashMap::default();
        typedefs.insert("myint".to_string(), CType::Int);
        let mut types = graph(&layouts, &typedefs);
        let array = CType::Array(Box::new(CType::Int), Some(4));
        let index = types.intern_written(&array, Some("myint"));
        let built = types.into_types();
        let DwarfType::Array {
            element, bounds, ..
        } = &built[index as usize]
        else {
            panic!("`myint[4]` is an array");
        };
        assert_eq!(bounds, &vec![Some(3)]);
        assert!(
            matches!(&built[*element as usize], DwarfType::Typedef { name, .. } if name == "myint"),
            "the element keeps the alias the declaration wrote"
        );
    }

    #[test]
    fn a_member_keeps_the_typedef_its_declaration_named() {
        // `struct holder { size_t count; };` -- the layout carries the spelling
        // the field declaration wrote, and the member DIE must point at the
        // typedef rather than at `long unsigned int`.
        let mut layouts = empty_layouts();
        layouts.insert(
            "holder".to_string(),
            Rc::new(StructLayout {
                is_complete: true,
                fields: vec![StructFieldLayout {
                    written_typedef: Some("size_t".to_string()),
                    name: "count".to_string(),
                    offset: 0,
                    ty: CType::ULong,
                    bit_offset: None,
                    bit_width: None,
                    sso: SsoMode::None,
                    sso_storage_ty: None,
                    base_volatile: false,
                    base_const: false,
                    object_volatile: false,
                    object_const: false,
                    counted_by: None,
                    pointer_quals: Vec::new(),
                    own_cvr: None,
                }],
                size: 8,
                align: 8,
                is_union: false,
                is_transparent_union: false,
                requires_designators: false,
                reverse_sso: false,
                user_align: false,
                member_align: 8,
            }),
        );
        let mut typedefs = FxHashMap::default();
        typedefs.insert("size_t".to_string(), CType::ULong);
        let mut types = graph(&layouts, &typedefs);
        let record = types.intern_ctype(&CType::Struct("holder".into()));
        let built = types.into_types();
        let DwarfType::Record { members, .. } = &built[record as usize] else {
            panic!("a struct is a record DIE");
        };
        let member = &built[members[0].ty as usize];
        assert!(
            matches!(member, DwarfType::Typedef { name, .. } if name == "size_t"),
            "the member's `size_t` spelling is lost: {member:?}"
        );
    }

    #[test]
    fn multidimension_arrays_keep_declaration_order() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let inner = types.intern_ctype(&CType::Array(Box::new(CType::Int), Some(3)));
        let outer = types.intern_ctype(&CType::Array(
            Box::new(CType::Array(Box::new(CType::Int), Some(3))),
            Some(2),
        ));
        let built = types.into_types();
        let (inner, outer) = (&built[inner as usize], &built[outer as usize]);
        assert!(matches!(
            inner,
            DwarfType::Array { bounds, vector: false, .. } if bounds == &vec![Some(2)]
        ));
        assert!(
            matches!(
                outer,
                DwarfType::Array { bounds, vector: false, .. } if bounds == &vec![Some(1)]
            ),
            "the outer dimension is its own array type: {outer:?}"
        );
    }

    #[test]
    fn an_incomplete_array_has_no_upper_bound() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let id = types.intern_ctype(&CType::Array(Box::new(CType::Char), None));
        let built = types.into_types();
        assert!(matches!(
            &built[id as usize],
            DwarfType::Array { bounds, .. } if bounds == &vec![None]
        ));
    }

    #[test]
    fn a_bitfield_keeps_its_layout_offsets() {
        let mut layouts = empty_layouts();
        layouts.insert(
            "Bits".to_string(),
            Rc::new(StructLayout {
                is_complete: true,
                fields: vec![StructFieldLayout {
                    written_typedef: None,
                    name: "flag".to_string(),
                    offset: 4,
                    ty: CType::UInt,
                    bit_offset: Some(3),
                    bit_width: Some(5),
                    sso: SsoMode::None,
                    sso_storage_ty: None,
                    base_volatile: false,
                    base_const: false,
                    object_volatile: false,
                    object_const: false,
                    counted_by: None,
                    pointer_quals: Vec::new(),
                    own_cvr: None,
                }],
                size: 8,
                align: 4,
                is_union: false,
                is_transparent_union: false,
                requires_designators: false,
                reverse_sso: false,
                user_align: false,
                member_align: 4,
            }),
        );
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let id = types.intern_ctype(&CType::Struct("Bits".into()));
        let built = types.into_types();
        let DwarfType::Record { members, .. } = &built[id as usize] else {
            panic!("expected a record");
        };
        assert_eq!(members[0].offset, 4);
        assert_eq!(members[0].bit, Some((5, 3)));
    }

    #[test]
    fn an_anonymous_record_has_no_name() {
        assert_eq!(record_name("__anon_struct_7"), None);
        assert_eq!(record_name("struct.Node"), Some("Node".to_string()));
        assert_eq!(
            record_name("struct.S#1"),
            Some("S".to_string()),
            "a shadowed tag still prints the source's tag"
        );
        assert_eq!(record_name("union.U"), Some("U".to_string()));
    }

    #[test]
    fn a_vector_is_a_gnu_vector_array() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let id = types.intern_ctype(&CType::Vector(Box::new(CType::Int), 16));
        let built = types.into_types();
        assert!(matches!(
            &built[id as usize],
            DwarfType::Array { bounds, vector: true, .. } if bounds == &vec![Some(3)]
        ));
    }

    #[test]
    fn a_subroutine_records_its_parameters_and_variadic_tail() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let id = types.intern_ctype(&CType::Function(Box::new(FunctionType {
            calling_convention: Default::default(),
            return_type: CType::Void,
            params: vec![(CType::Int, None)],
            variadic: true,
            has_prototype: true,
        })));
        let built = types.into_types();
        let DwarfType::Subroutine {
            ret,
            params,
            variadic,
            prototyped,
        } = &built[id as usize]
        else {
            panic!("expected a subroutine");
        };
        assert_eq!(*ret, None, "a void return has no DW_AT_type");
        assert_eq!(params.len(), 1);
        assert!(*variadic);
        assert!(*prototyped);
    }

    #[test]
    fn pointer_size_follows_the_target() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let id = types.intern_ctype(&CType::Pointer(Box::new(CType::Int), PtrQuals::GENERIC));
        let built = types.into_types();
        let DwarfType::Pointer { size, .. } = &built[id as usize] else {
            panic!("expected a pointer");
        };
        assert_eq!(*size, crate::common::types::target_ptr_size() as u64);
    }

    #[test]
    fn an_unreferenced_declaration_and_its_type_are_dropped() {
        // `extern struct OnlyDecl *unused;` next to a used `extern int used;`:
        // the declaration is never referred to, so neither it nor the record only
        // it names belongs in the unit.
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let int = types.intern_ctype(&CType::Int);
        let only_decl = types.intern_ctype(&CType::Struct("OnlyDecl".into()));
        let mut unit = DwarfUnitSpec {
            types: types.into_types(),
            variables: vec![
                DwarfVariable {
                    name: "used".to_string(),
                    ty: int,
                    external: true,
                    declaration: true,
                    decl_file: None,
                    decl_line: None,
                    decl_column: None,
                    file_scope: true,
                },
                DwarfVariable {
                    name: "unused".to_string(),
                    ty: only_decl,
                    external: true,
                    declaration: true,
                    decl_file: None,
                    decl_line: None,
                    decl_column: None,
                    file_scope: true,
                },
            ],
            return_types: FxHashMap::default(),
            parameters: FxHashMap::default(),
        };
        let mut referenced = FxHashMap::default();
        referenced.insert("used".to_string(), ());
        let pruned = unit.with_referenced(&referenced);
        assert_eq!(pruned.variables.len(), 1);
        assert_eq!(pruned.variables[0].name, "used");
        assert_eq!(
            pruned.variables[0].ty, 0,
            "the surviving type keeps index 0"
        );
        assert_eq!(pruned.types.len(), 1, "the record only it named is gone");
        assert!(matches!(unit.variables.len(), 2), "the input is untouched");

        // A definition stays even when nothing refers to it: it has storage.
        unit.variables[1].declaration = false;
        let pruned = unit.with_referenced(&referenced);
        assert_eq!(pruned.variables.len(), 2);
        assert_eq!(pruned.types.len(), 2);
    }

    #[test]
    fn a_const_pointee_is_wrapped_inside_the_pointer() {
        let layouts = empty_layouts();
        let typedefs = FxHashMap::default();
        let mut types = graph(&layouts, &typedefs);
        let id = types.intern_ctype(&CType::Pointer(
            Box::new(CType::Char),
            PtrQuals::recorded(AddressSpace::Default, PtrQuals::CONST),
        ));
        let built = types.into_types();
        let DwarfType::Pointer { pointee, .. } = &built[id as usize] else {
            panic!("expected a pointer");
        };
        let pointee = pointee.expect("a pointer to a complete type names it");
        assert!(matches!(
            &built[pointee as usize],
            DwarfType::Qualified {
                qual: TypeQual::Const,
                ..
            }
        ));
    }
}

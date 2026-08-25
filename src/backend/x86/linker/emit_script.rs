//! Linker-script-driven ELF executable emission (x86-64).
//!
//! Implements the layout semantics of GNU ld for full `SECTIONS` scripts:
//! the location counter walks the script top-to-bottom, input sections are
//! assigned to output sections by glob pattern (first match wins, in script
//! order), symbols are defined at their point of assignment, PHDRS are
//! honored, and `AT()` gives independent load addresses (LMA).
//!
//! This is what links the Linux kernel's `vmlinux.lds`. It produces a fully
//! static ET_EXEC image with no dynamic sections; relocation types are the
//! static x86-64 set (PC32/PLT32/32/32S/64 and TLS LE forms).

use crate::backend::elf::{elf64_sym_entry, push_strtab_name};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

use super::elf::*;
use crate::backend::linker_common::{
    self,
    linker_script::{
        self, eval_expr, glob_match, AssignOp, Assignment, EvalCtx, EvalError, LinkerScript,
        OutputSecDef, SecItem, SectionsItem, SortKind,
    },
};

type Object = linker_common::Elf64Object;

const PT_LOAD_: u32 = 1;
const PT_TLS_: u32 = 7;
const PT_NOTE_: u32 = 4;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptMachine {
    X86_64,
    I386,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ScriptExprHome {
    Absolute,
    Section(String),
    /// The expression is relocatable, but its output section is not known yet
    /// (for example `marker = .` before the first output-section command).
    Unresolved,
}

/// Classify the section-relative semantics of an assignment expression made
/// at SECTIONS scope.  GNU ld does not decide this from the final numeric
/// address: `end = .` after an output section remains section-relative, while
/// `size = end - start` and `ABSOLUTE(.)` are SHN_ABS.
fn script_expr_home(
    expr: &linker_script::Expr,
    dot_home: Option<&str>,
    sym_home: &FxHashMap<String, String>,
    absolute_syms: &FxHashSet<String>,
) -> ScriptExprHome {
    use linker_script::{BinOp, Expr};

    let recurse = |e| script_expr_home(e, dot_home, sym_home, absolute_syms);
    match expr {
        Expr::Dot => dot_home
            .map(|name| ScriptExprHome::Section(name.to_string()))
            .unwrap_or(ScriptExprHome::Unresolved),
        Expr::Sym(name) => {
            if let Some(home) = sym_home.get(name) {
                ScriptExprHome::Section(home.clone())
            } else if absolute_syms.contains(name) {
                ScriptExprHome::Absolute
            } else {
                ScriptExprHome::Unresolved
            }
        }
        Expr::AddrOf(name) | Expr::LoadAddrOf(name) => ScriptExprHome::Section(name.clone()),
        Expr::Absolute(_)
        | Expr::Num(_)
        | Expr::SizeOf(_)
        | Expr::AlignOf(_)
        | Expr::DefinedSym(_)
        | Expr::SegmentStart(_, _) => ScriptExprHome::Absolute,
        Expr::Align1(_) => dot_home
            .map(|name| ScriptExprHome::Section(name.to_string()))
            .unwrap_or(ScriptExprHome::Unresolved),
        Expr::Align2(value, _) => recurse(value),
        Expr::Bin(op, lhs, rhs) => {
            let left = recurse(lhs);
            let right = recurse(rhs);
            match op {
                BinOp::Add => match (left, right) {
                    (ScriptExprHome::Section(s), ScriptExprHome::Absolute)
                    | (ScriptExprHome::Absolute, ScriptExprHome::Section(s)) => {
                        ScriptExprHome::Section(s)
                    }
                    (ScriptExprHome::Unresolved, ScriptExprHome::Absolute)
                    | (ScriptExprHome::Absolute, ScriptExprHome::Unresolved) => {
                        ScriptExprHome::Unresolved
                    }
                    (ScriptExprHome::Absolute, ScriptExprHome::Absolute) => {
                        ScriptExprHome::Absolute
                    }
                    _ => ScriptExprHome::Absolute,
                },
                BinOp::Sub => match (left, right) {
                    (ScriptExprHome::Section(s), ScriptExprHome::Absolute) => {
                        ScriptExprHome::Section(s)
                    }
                    // GNU ld preserves the location counter's section when
                    // subtracting a symbol in that same section (`. - start`),
                    // but `start - start` is an absolute section difference.
                    (ScriptExprHome::Section(s), ScriptExprHome::Section(r))
                        if matches!(lhs.as_ref(), Expr::Dot) && s == r =>
                    {
                        ScriptExprHome::Section(s)
                    }
                    (ScriptExprHome::Unresolved, ScriptExprHome::Absolute) => {
                        ScriptExprHome::Unresolved
                    }
                    _ => ScriptExprHome::Absolute,
                },
                _ => ScriptExprHome::Absolute,
            }
        }
        Expr::Ternary(cond, yes, no) => {
            if let Some(value) = linker_script::eval_const(cond) {
                recurse(if value != 0 { yes } else { no })
            } else {
                let yes = recurse(yes);
                let no = recurse(no);
                if yes == no {
                    yes
                } else {
                    ScriptExprHome::Absolute
                }
            }
        }
        Expr::Min(yes, no) | Expr::Max(yes, no) => {
            let yes = recurse(yes);
            let no = recurse(no);
            if yes == no {
                yes
            } else {
                ScriptExprHome::Absolute
            }
        }
        Expr::Assert(value, _) => recurse(value),
        Expr::Neg(_) | Expr::Not(_) => ScriptExprHome::Absolute,
    }
}

/// Within an output-section body GNU ld gives ordinary expressions that
/// section's home, even constants and same-section differences.  ABSOLUTE is
/// the explicit escape hatch and its absolute type propagates through the
/// containing expression.
fn expr_contains_absolute(expr: &linker_script::Expr) -> bool {
    use linker_script::Expr;
    match expr {
        Expr::Absolute(_) => true,
        Expr::SegmentStart(_, value)
        | Expr::Neg(value)
        | Expr::Not(value)
        | Expr::Align1(value)
        | Expr::Assert(value, _) => expr_contains_absolute(value),
        Expr::Bin(_, lhs, rhs)
        | Expr::Align2(lhs, rhs)
        | Expr::Min(lhs, rhs)
        | Expr::Max(lhs, rhs) => expr_contains_absolute(lhs) || expr_contains_absolute(rhs),
        Expr::Ternary(cond, yes, no) => {
            if let Some(value) = linker_script::eval_const(cond) {
                expr_contains_absolute(if value != 0 { yes } else { no })
            } else {
                expr_contains_absolute(yes) || expr_contains_absolute(no)
            }
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScriptValueForm {
    Address,
    Offset,
    Unknown,
}

/// The numeric evaluator uses absolute VMAs for Dot/Sym/ADDR, but constants,
/// SIZEOF, and differences are offsets.  When GNU ld assigns an offset-valued
/// expression inside an output-section body, it adds the section VMA because
/// the resulting symbol is section-relative (`inside = 5` at 0x400000 has
/// st_value 0x400005, not 5).
fn script_value_form(
    expr: &linker_script::Expr,
    sym_home: &FxHashMap<String, String>,
    absolute_syms: &FxHashSet<String>,
) -> ScriptValueForm {
    use linker_script::{BinOp, Expr};
    let recurse = |e| script_value_form(e, sym_home, absolute_syms);
    match expr {
        Expr::Dot | Expr::AddrOf(_) | Expr::LoadAddrOf(_) | Expr::Align1(_) => {
            ScriptValueForm::Address
        }
        Expr::Sym(name) => {
            if sym_home.contains_key(name) {
                ScriptValueForm::Address
            } else if absolute_syms.contains(name) {
                ScriptValueForm::Offset
            } else {
                ScriptValueForm::Unknown
            }
        }
        Expr::Num(_)
        | Expr::SizeOf(_)
        | Expr::AlignOf(_)
        | Expr::DefinedSym(_)
        | Expr::SegmentStart(_, _) => ScriptValueForm::Offset,
        Expr::Absolute(_) => ScriptValueForm::Address,
        Expr::Align2(value, _) => recurse(value),
        Expr::Bin(op, lhs, rhs) => {
            let left = recurse(lhs);
            let right = recurse(rhs);
            match op {
                BinOp::Add => match (left, right) {
                    (ScriptValueForm::Address, ScriptValueForm::Offset)
                    | (ScriptValueForm::Offset, ScriptValueForm::Address) => {
                        ScriptValueForm::Address
                    }
                    (ScriptValueForm::Offset, ScriptValueForm::Offset) => ScriptValueForm::Offset,
                    _ => ScriptValueForm::Unknown,
                },
                BinOp::Sub => match (left, right) {
                    (ScriptValueForm::Address, ScriptValueForm::Offset) => ScriptValueForm::Address,
                    (ScriptValueForm::Address, ScriptValueForm::Address)
                    | (ScriptValueForm::Offset, ScriptValueForm::Offset) => ScriptValueForm::Offset,
                    _ => ScriptValueForm::Unknown,
                },
                _ => ScriptValueForm::Offset,
            }
        }
        Expr::Ternary(cond, yes, no) => {
            if let Some(value) = linker_script::eval_const(cond) {
                recurse(if value != 0 { yes } else { no })
            } else {
                let yes = recurse(yes);
                let no = recurse(no);
                if yes == no {
                    yes
                } else {
                    ScriptValueForm::Unknown
                }
            }
        }
        Expr::Min(yes, no) | Expr::Max(yes, no) => {
            let yes = recurse(yes);
            let no = recurse(no);
            if yes == no {
                yes
            } else {
                ScriptValueForm::Unknown
            }
        }
        Expr::Assert(value, _) => recurse(value),
        Expr::Neg(_) | Expr::Not(_) => ScriptValueForm::Offset,
    }
}

/// SIZEOF of the output section currently being built cannot be evaluated
/// until its closing brace.  The generic evaluator deliberately returns zero
/// for a genuinely absent section, so the layout driver must distinguish this
/// forward reference and defer it explicitly.
fn expr_references_sizeof(expr: &linker_script::Expr, section: &str) -> bool {
    use linker_script::Expr;
    match expr {
        Expr::SizeOf(name) => name == section,
        Expr::SegmentStart(_, value)
        | Expr::Neg(value)
        | Expr::Not(value)
        | Expr::Align1(value)
        | Expr::Absolute(value)
        | Expr::Assert(value, _) => expr_references_sizeof(value, section),
        Expr::Bin(_, lhs, rhs)
        | Expr::Align2(lhs, rhs)
        | Expr::Min(lhs, rhs)
        | Expr::Max(lhs, rhs) => {
            expr_references_sizeof(lhs, section) || expr_references_sizeof(rhs, section)
        }
        Expr::Ternary(cond, yes, no) => {
            expr_references_sizeof(cond, section)
                || expr_references_sizeof(yes, section)
                || expr_references_sizeof(no, section)
        }
        _ => false,
    }
}

impl ScriptMachine {
    fn ehdr_size(self) -> u64 {
        match self {
            Self::X86_64 => 64,
            Self::I386 => 52,
        }
    }
    fn phdr_size(self) -> u64 {
        match self {
            Self::X86_64 => 56,
            Self::I386 => 32,
        }
    }
    fn shdr_size(self) -> u64 {
        match self {
            Self::X86_64 => 64,
            Self::I386 => 40,
        }
    }
    fn sym_size(self) -> usize {
        match self {
            Self::X86_64 => 24,
            Self::I386 => 16,
        }
    }
    fn rel_size(self) -> usize {
        match self {
            Self::X86_64 => 24,
            Self::I386 => 8,
        }
    }
}

/// Bytes occupied by the ELF header plus all program headers.
///
/// This is what `SIZEOF_HEADERS` evaluates to, so it must agree exactly with
/// the number of program headers actually written. `extra_phdrs` accounts for
/// headers the linker synthesises rather than the script declaring them (today:
/// PT_TLS). Undercounting places the first section on top of the last program
/// header, and the entry point then lands in header bytes -- observed as an
/// immediate SIGILL.
fn script_header_size_with(
    script: &LinkerScript,
    extra_phdrs: usize,
    machine: ScriptMachine,
) -> u64 {
    machine.ehdr_size() + machine.phdr_size() * (script.phdrs.len().max(1) + extra_phdrs) as u64
}

/// Synthetic dynamic-link information required by an ET_DYN linker-script
/// image. Linux's vDSO script names `.dynsym`, `.dynstr`, `.hash`,
/// `.gnu.hash`, `.gnu.version*`, and `.dynamic`, but those sections are linker
/// products rather than compiler inputs. Treating unmatched patterns as empty
/// creates an ET_DYN file that looks linked yet cannot be resolved by ld.so.
struct ScriptDynamic {
    object_index: usize,
    exports: Vec<String>,
    name_offsets: Vec<u32>,
    /// Named version nodes are also absolute dynsyms, as in GNU ld.
    version_symbols: FxHashSet<String>,
    soname_offset: Option<u32>,
    verdef_count: u64,
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn build_sysv_hash(names: &[String]) -> Vec<u8> {
    let buckets = names.len().next_power_of_two().max(1);
    let mut heads = vec![0u32; buckets];
    let mut tails = vec![0u32; buckets];
    let mut chains = vec![0u32; names.len() + 1];
    for (i, name) in names.iter().enumerate() {
        let index = (i + 1) as u32;
        let bucket = linker_common::sysv_hash(name.as_bytes()) as usize % buckets;
        if heads[bucket] == 0 {
            heads[bucket] = index;
        } else {
            chains[tails[bucket] as usize] = index;
        }
        tails[bucket] = index;
    }
    let mut out = Vec::with_capacity(8 + (buckets + chains.len()) * 4);
    push_u32(&mut out, buckets as u32);
    push_u32(&mut out, chains.len() as u32);
    for value in heads {
        push_u32(&mut out, value);
    }
    for value in chains {
        push_u32(&mut out, value);
    }
    out
}

/// Build a GNU hash table and return the required dynsym order. GNU hash
/// requires all symbols belonging to a bucket to be contiguous.
fn build_gnu_hash(mut names: Vec<String>, machine: ScriptMachine) -> (Vec<u8>, Vec<String>) {
    // Bucket count: a power of two near the symbol count, so chains stay
    // short. GNU ld uses a load factor around 1 symbol per bucket for exactly
    // this reason -- every ld.so lookup walks a chain, so an undersized table
    // is a permanent runtime tax on the consumer, not a link-time saving.
    //
    // A fixed cap (an earlier revision clamped this to 64) is a trap: it looks
    // harmless on a vDSO with four exports and silently degrades to ~78-symbol
    // chains at 5 000 exports, versus ~1.5 uncapped -- measured. The table
    // costs 4 bytes per bucket, so sizing it properly is cheap.
    let nbuckets = names.len().next_power_of_two().max(1);
    // Decorate once: `sort_by_key(... name.clone())` allocated and copied a
    // String on every comparator invocation (O(n log n) heap traffic), then
    // hashed every name again.  Hash once and compare borrowed names.
    let mut hashed: Vec<(String, u32)> = names
        .drain(..)
        .map(|name| {
            let hash = linker_common::gnu_hash(name.as_bytes());
            (name, hash)
        })
        .collect();
    hashed.sort_unstable_by(|(a_name, a_hash), (b_name, b_hash)| {
        (a_hash % nbuckets as u32)
            .cmp(&(b_hash % nbuckets as u32))
            .then(a_hash.cmp(b_hash))
            .then(a_name.cmp(b_name))
    });
    let (names, hashes): (Vec<String>, Vec<u32>) = hashed.into_iter().unzip();
    let word_bits = if machine == ScriptMachine::X86_64 {
        64usize
    } else {
        32usize
    };
    let bloom_size = ((names.len() + word_bits - 1) / word_bits)
        .next_power_of_two()
        .max(1);
    let bloom_shift = 6u32;
    // Store bloom words in u64 while building; ELF32 serialises their low 32
    // bits as Elf32_Addr words.
    let mut bloom = vec![0u64; bloom_size];
    for &hash in &hashes {
        let word = (hash as usize / word_bits) % bloom_size;
        bloom[word] |= 1u64 << (hash as usize % word_bits);
        bloom[word] |= 1u64 << ((hash >> bloom_shift) as usize % word_bits);
    }
    let mut buckets = vec![0u32; nbuckets];
    let mut chains = vec![0u32; names.len()];
    for (i, &hash) in hashes.iter().enumerate() {
        let bucket = hash as usize % nbuckets;
        if buckets[bucket] == 0 {
            buckets[bucket] = (i + 1) as u32;
        }
        let last =
            i + 1 == hashes.len() || hashes[i + 1] % nbuckets as u32 != hash % nbuckets as u32;
        chains[i] = (hash & !1) | u32::from(last);
    }
    let mut out = Vec::new();
    push_u32(&mut out, nbuckets as u32);
    push_u32(&mut out, 1); // first hashed dynsym follows the null entry
    push_u32(&mut out, bloom_size as u32);
    push_u32(&mut out, bloom_shift);
    for value in bloom {
        if machine == ScriptMachine::X86_64 {
            out.extend_from_slice(&value.to_le_bytes());
        } else {
            push_u32(&mut out, value as u32);
        }
    }
    for value in buckets {
        push_u32(&mut out, value);
    }
    for value in chains {
        push_u32(&mut out, value);
    }
    (out, names)
}

fn push_verdef(
    buf: &mut Vec<u8>,
    index: u16,
    flags: u16,
    name: &str,
    name_off: u32,
    parent_off: Option<u32>,
    next: u32,
) {
    // Elf{32,64}_Verdef is class-independent: a 20-byte header followed by
    // one Verdaux for the node name and, for inherited nodes, one for the
    // parent. `vd_cnt` and the first Verdaux's vda_next must describe that
    // second auxiliary entry or the loader silently loses the version edge.
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&flags.to_le_bytes());
    buf.extend_from_slice(&index.to_le_bytes());
    buf.extend_from_slice(&(if parent_off.is_some() { 2u16 } else { 1u16 }).to_le_bytes());
    push_u32(buf, linker_common::sysv_hash(name.as_bytes()));
    push_u32(buf, 20);
    push_u32(buf, next);
    push_u32(buf, name_off);
    push_u32(buf, if parent_off.is_some() { 8 } else { 0 });
    if let Some(parent_off) = parent_off {
        push_u32(buf, parent_off);
        push_u32(buf, 0);
    }
}

fn synthetic_section(
    name: &str,
    sh_type: u32,
    flags: u64,
    align: u64,
    entsize: u64,
    data: Vec<u8>,
) -> (linker_common::Elf64Section, Vec<u8>) {
    let size = data.len() as u64;
    (
        linker_common::Elf64Section {
            name_idx: 0,
            name: name.into(),
            sh_type,
            flags,
            addr: 0,
            offset: 0,
            size,
            link: 0,
            info: 0,
            addralign: align,
            entsize,
        },
        data,
    )
}

fn make_script_dynamic_object(
    object_index: usize,
    mut export_candidates: Vec<String>,
    version: &linker_common::VersionScript,
    soname: Option<&str>,
    base_name: &str,
    bsymbolic: bool,
    machine: ScriptMachine,
) -> (Object, ScriptDynamic) {
    // Every named version node is itself an absolute dynamic symbol. Linux's
    // 32-bit vDSO has LINUX_2.6 and LINUX_2.5; emitting only the first made the
    // latter's syscall symbols claim the wrong ABI even though they remained
    // findable by name.
    let mut named_nodes: Vec<(usize, &linker_common::VersionNode)> = version
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| !node.name.is_empty())
        .collect();
    if named_nodes.is_empty() && !version.version_name.is_empty() {
        // Compatibility with a hand-constructed/legacy VersionScript whose
        // first-node fields are populated but `nodes` is empty.
        named_nodes = Vec::new();
    }
    for (_, node) in &named_nodes {
        if !export_candidates.iter().any(|name| name == &node.name) {
            export_candidates.push(node.name.clone());
        }
    }
    if named_nodes.is_empty()
        && !version.version_name.is_empty()
        && !export_candidates
            .iter()
            .any(|name| name == &version.version_name)
    {
        export_candidates.push(version.version_name.clone());
    }

    let (gnu_hash, exports) = build_gnu_hash(export_candidates, machine);
    let mut dynstr = vec![0u8];
    let mut name_offsets = Vec::with_capacity(exports.len());
    let mut dynstr_offsets: FxHashMap<String, u32> = FxHashMap::default();
    for name in &exports {
        let off = dynstr.len() as u32;
        name_offsets.push(off);
        dynstr_offsets.insert(name.clone(), off);
        dynstr.extend_from_slice(name.as_bytes());
        dynstr.push(0);
    }
    let soname_offset = soname.map(|name| {
        let off = dynstr.len() as u32;
        dynstr.extend_from_slice(name.as_bytes());
        dynstr.push(0);
        off
    });
    let base_off = if soname == Some(base_name) {
        soname_offset.expect("SONAME was inserted in dynstr")
    } else {
        let off = dynstr.len() as u32;
        dynstr.extend_from_slice(base_name.as_bytes());
        dynstr.push(0);
        off
    };

    let sysv_hash = build_sysv_hash(&exports);
    let dynsym_size = machine.sym_size();
    let dynsym = vec![0u8; (exports.len() + 1) * dynsym_size];

    // Map declaration-order nodes to consecutive ELF version indexes. Index 1
    // is the base object; named script nodes begin at 2. Anonymous nodes only
    // control visibility and do not receive a verdef index.
    let mut node_versions: FxHashMap<usize, u16> = FxHashMap::default();
    for (ordinal, (node_index, _)) in named_nodes.iter().enumerate() {
        node_versions.insert(*node_index, ordinal as u16 + 2);
    }
    let mut version_symbols: FxHashSet<String> = FxHashSet::default();
    for (_, node) in &named_nodes {
        version_symbols.insert(node.name.clone());
    }
    if named_nodes.is_empty() && !version.version_name.is_empty() {
        version_symbols.insert(version.version_name.clone());
    }

    let mut versym = Vec::with_capacity((exports.len() + 1) * 2);
    versym.extend_from_slice(&0u16.to_le_bytes());
    for name in &exports {
        let index = if let Some((ordinal, _)) = named_nodes
            .iter()
            .enumerate()
            .find(|(_, (_, node))| node.name == *name)
        {
            ordinal as u16 + 2
        } else if let Some(node_index) = version.version_node_for(name) {
            node_versions.get(&node_index).copied().unwrap_or(1)
        } else if version_symbols.contains(name) {
            2
        } else {
            1
        };
        versym.extend_from_slice(&index.to_le_bytes());
    }

    let mut verdef = Vec::new();
    let verdef_count = if named_nodes.is_empty() && version.version_name.is_empty() {
        0u64
    } else {
        let node_count = named_nodes.len().max(1);
        // VER_FLG_BASE = 1. The base entry always has one Verdaux.
        push_verdef(
            &mut verdef,
            1,
            1,
            base_name,
            base_off,
            None,
            if node_count > 0 { 28 } else { 0 },
        );
        if named_nodes.is_empty() {
            let off = *dynstr_offsets
                .get(&version.version_name)
                .expect("legacy version symbol was inserted");
            push_verdef(&mut verdef, 2, 0, &version.version_name, off, None, 0);
        } else {
            for (ordinal, (_, node)) in named_nodes.iter().enumerate() {
                let name_off = *dynstr_offsets
                    .get(&node.name)
                    .expect("version symbol was inserted");
                let parent_off = node
                    .parent
                    .as_ref()
                    .and_then(|parent| dynstr_offsets.get(parent).copied());
                let entry_len = if parent_off.is_some() { 36 } else { 28 };
                let next = if ordinal + 1 == named_nodes.len() {
                    0
                } else {
                    entry_len
                };
                push_verdef(
                    &mut verdef,
                    ordinal as u16 + 2,
                    0,
                    &node.name,
                    name_off,
                    parent_off,
                    next,
                );
            }
        }
        1 + node_count as u64
    };

    // Fixed tags plus optional SONAME and symbolic binding. Values that depend
    // on layout are patched after sections receive final virtual addresses.
    let dynamic_entries = 10usize + usize::from(soname.is_some()) + if bsymbolic { 2 } else { 0 };
    let dynamic = vec![
        0u8;
        dynamic_entries
            * if machine == ScriptMachine::X86_64 {
                16
            } else {
                8
            }
    ];
    let word_align = if machine == ScriptMachine::X86_64 {
        8
    } else {
        4
    };

    let mut sections = Vec::new();
    let mut section_data = Vec::new();
    let pairs = [
        synthetic_section("", SHT_NULL_, 0, 0, 0, Vec::new()),
        synthetic_section(".hash", 5, SHF_ALLOC_, 4, 4, sysv_hash),
        synthetic_section(
            ".gnu.hash",
            0x6fff_fff6,
            SHF_ALLOC_,
            word_align,
            0,
            gnu_hash,
        ),
        synthetic_section(
            ".dynsym",
            11,
            SHF_ALLOC_,
            word_align,
            dynsym_size as u64,
            dynsym,
        ),
        synthetic_section(".dynstr", SHT_STRTAB_, SHF_ALLOC_, 1, 0, dynstr),
        synthetic_section(".gnu.version", 0x6fff_ffff, SHF_ALLOC_, 2, 2, versym),
        synthetic_section(".gnu.version_d", 0x6fff_fffd, SHF_ALLOC_, 4, 0, verdef),
        synthetic_section(
            ".dynamic",
            6,
            SHF_ALLOC_ | SHF_WRITE_,
            word_align,
            if machine == ScriptMachine::X86_64 {
                16
            } else {
                8
            },
            dynamic,
        ),
    ];
    for (section, data) in pairs {
        sections.push(section);
        section_data.push(linker_common::SectionData::owned(data));
    }
    let section_count = sections.len();
    let object = Object {
        sections,
        symbols: Vec::new(),
        section_data,
        relocations: vec![Vec::new(); section_count],
        source_name: "<script-dynamic>".into(),
    };
    let plan = ScriptDynamic {
        object_index,
        exports,
        name_offsets,
        version_symbols,
        soname_offset,
        verdef_count,
    };
    (object, plan)
}

/// One placed input section.
struct Placed {
    obj_idx: usize,
    sec_idx: usize,
    vaddr: u64,
    size: u64,
}

/// One output section after layout.
struct OutSec {
    name: String,
    vaddr: u64,
    lma: u64,
    size: u64,
    align: u64,
    sh_type: u32,
    flags: u64,
    file_offset: u64,
    is_alloc: bool,
    nobits: bool,
    phdrs: Vec<String>,
    fill: Option<u64>,
    placed: Vec<Placed>,
    /// Linker-script BYTE/SHORT/LONG/QUAD commands.  Expressions are retained
    /// until the complete layout is known, while `offset` reserves their bytes
    /// during the first layout pass.
    data_commands: Vec<(u64, u8, linker_script::Expr)>,
}

pub fn link_with_script(
    objects: &[Object],
    script_src: &str,
    output_path: &str,
    emit_symtab: bool,
    is_pie: bool,
    emit_relocs: bool,
    gc_sections: bool,
    soname: Option<&str>,
    bsymbolic: bool,
    max_page_size: u64,
) -> Result<(), String> {
    link_with_script_machine(
        objects,
        script_src,
        output_path,
        emit_symtab,
        is_pie,
        emit_relocs,
        gc_sections,
        soname,
        bsymbolic,
        max_page_size,
        ScriptMachine::X86_64,
    )
}

/// ELF32/i386 counterpart used by Linux's real-mode `setup.elf`.  Input
/// records have already been normalised by the i686 parser, but output field
/// widths, REL relocation semantics and machine/class headers remain distinct.
pub fn link_with_script_i386(
    objects: &[Object],
    script_src: &str,
    output_path: &str,
    emit_symtab: bool,
    is_pie: bool,
    emit_relocs: bool,
    gc_sections: bool,
    soname: Option<&str>,
    bsymbolic: bool,
    max_page_size: u64,
) -> Result<(), String> {
    link_with_script_machine(
        objects,
        script_src,
        output_path,
        emit_symtab,
        is_pie,
        emit_relocs,
        gc_sections,
        soname,
        bsymbolic,
        max_page_size,
        ScriptMachine::I386,
    )
}

#[allow(clippy::too_many_arguments)]
fn link_with_script_machine(
    objects: &[Object],
    script_src: &str,
    output_path: &str,
    emit_symtab: bool,
    is_pie: bool,
    // `--emit-relocs`: retain the (already applied) relocations in `.rela.*`
    // for ELF64 or `.rel.*` for ELF32.
    emit_relocs: bool,
    gc_sections: bool,
    soname: Option<&str>,
    bsymbolic: bool,
    max_page_size: u64,
    machine: ScriptMachine,
) -> Result<(), String> {
    let script: LinkerScript = linker_script::parse_linker_script(script_src)?;

    // `--gc-sections` for script links uses the same relocation-reachability
    // engine as ordinary links, with two script-specific root classes:
    // ENTRY(symbol) and every input section matched by KEEP(...).  Merely
    // assigning `start = .; *(registry); stop = .` does not root a registry in
    // GNU ld; the script author must say KEEP, which is why the distinction is
    // retained in the parsed AST.
    let dead_sections: FxHashSet<(usize, usize)> = if gc_sections {
        let mut kept: FxHashSet<(usize, usize)> = FxHashSet::default();
        for item in &script.sections {
            let SectionsItem::Output(def) = item else {
                continue;
            };
            for body_item in &def.items {
                let SecItem::Input(spec) = body_item else {
                    continue;
                };
                if !spec.keep {
                    continue;
                }
                for (oi, obj) in objects.iter().enumerate() {
                    for (si, sec) in obj.sections.iter().enumerate() {
                        if spec.patterns.iter().any(|pattern| glob_match(pattern, &sec.name)) {
                            kept.insert((oi, si));
                        }
                    }
                }
            }
        }
        let roots: Vec<String> = script.entry.iter().cloned().collect();
        linker_common::gc_collect_sections_elf64_roots_and_sections(objects, &roots, &kept)
    } else {
        FxHashSet::default()
    };

    // ── Global symbol table from objects (defined globals + weak) ──
    // name -> (obj_idx, sec_idx(SHN), value, size, info)
    let mut def_syms: FxHashMap<String, (usize, u16, u64, u64, u8)> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_local() {
                continue;
            }
            if dead_sections.contains(&(oi, sym.shndx as usize)) {
                continue;
            }
            if sym.is_undefined() || sym.shndx == SHN_COMMON {
                continue;
            }
            let replace = match def_syms.get(sym.name.as_str()) {
                None => true,
                Some(&(_, _, _, _, info)) => (info >> 4) == STB_WEAK && sym.is_global(),
            };
            if replace {
                def_syms.insert(
                    sym.name.to_string(),
                    (oi, sym.shndx, sym.value, sym.size, sym.info),
                );
            }
        }
    }

    // Symbols the *inputs* declared STV_HIDDEN/STV_INTERNAL. Visibility is an
    // ABI property decided by the compiler; the linker must honour it before
    // any version-script globbing is applied.
    let mut hidden_object_syms: FxHashSet<String> = FxHashSet::default();
    for obj in objects.iter() {
        for sym in &obj.symbols {
            if sym.name.is_empty() || sym.is_undefined() {
                continue;
            }
            // STV_HIDDEN = 2, STV_INTERNAL = 1, STV_PROTECTED = 3 (still exported).
            let vis = sym.visibility();
            if vis == 1 || vis == 2 {
                hidden_object_syms.insert(sym.name.to_string());
            }
        }
    }

    // ET_DYN script links (notably Linux's vDSO) require linker-created
    // dynamic metadata. Materialise it as a synthetic input object so the
    // script's ordinary `*(.dynsym)` / `*(.dynamic)` rules decide placement,
    // exactly as they do in GNU ld.
    let embedded_version = linker_common::VersionScript::parse_text(script_src);
    let wants_dynamic = is_pie
        && script
            .sections
            .iter()
            .any(|item| matches!(item, SectionsItem::Output(def) if def.name == ".dynamic"));
    let mut owned_objects: Option<Vec<Object>> = None;
    let mut script_dynamic: Option<ScriptDynamic> = None;
    if wants_dynamic {
        if let Some(version) = embedded_version.as_ref() {
            // STV_HIDDEN / STV_INTERNAL symbols are deliberately not part of
            // the ABI: they must stay out of .dynsym even when the version
            // script says `global: *`. GNU ld applies visibility first and the
            // version script second; doing it the other way round exports the
            // vDSO's internal helpers and every -fvisibility=hidden symbol in
            // a script-linked shared object.
            let mut exports: Vec<String> = def_syms
                .iter()
                .filter(|(name, _)| version.matches_global(name))
                .filter(|(name, _)| !hidden_object_syms.contains(name.as_str()))
                .map(|(name, _)| name.clone())
                .collect();
            exports.sort();
            let mut all = objects.to_vec();
            let object_index = all.len();
            let output_base_name = soname.map(str::to_owned).unwrap_or_else(|| {
                std::path::Path::new(output_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("a.out")
                    .to_owned()
            });
            let (dynamic_object, plan) = make_script_dynamic_object(
                object_index,
                exports,
                version,
                soname,
                &output_base_name,
                bsymbolic,
                machine,
            );
            all.push(dynamic_object);
            owned_objects = Some(all);
            script_dynamic = Some(plan);
        }
    }
    let objects: &[Object] = owned_objects.as_deref().unwrap_or(objects);

    // ── Assign input sections to output sections ──
    // Collect allocatable + INFO-listed input sections.
    let mut unassigned: Vec<(usize, usize)> = Vec::new();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if dead_sections.contains(&(oi, si)) {
                continue;
            }
            // Ordinary object string/symbol/relocation tables are linker
            // metadata rather than input sections.  An allocated string table
            // is different: .dynstr must participate in script matching.
            if matches!(
                sec.sh_type,
                SHT_NULL_ | SHT_SYMTAB_ | SHT_RELA_ | SHT_REL_ | SHT_GROUP_
            ) || (sec.sh_type == SHT_STRTAB_ && sec.flags & SHF_ALLOC_ == 0)
            {
                continue;
            }
            if sec.size == 0 && sec.name.is_empty() {
                continue;
            }
            unassigned.push((oi, si));
        }
    }

    // Output section definitions in script order.
    let out_defs: Vec<&OutputSecDef> = script
        .sections
        .iter()
        .filter_map(|it| match it {
            SectionsItem::Output(o) => Some(o),
            _ => None,
        })
        .collect();

    // For each (output def, item order), record which input sections match.
    // First-match-wins across the whole script.
    let mut assigned: FxHashMap<(usize, usize), usize> = FxHashMap::default(); // input -> out_def index
    let mut assigned_item: FxHashMap<(usize, usize), usize> = FxHashMap::default(); // input -> item index
    let mut discard: FxHashSet<(usize, usize)> = FxHashSet::default();

    for (di, def) in out_defs.iter().enumerate() {
        for (ii, item) in def.items.iter().enumerate() {
            let SecItem::Input(spec) = item else { continue };
            for &(oi, si) in unassigned.iter() {
                if assigned.contains_key(&(oi, si)) || discard.contains(&(oi, si)) {
                    continue;
                }
                let name = &objects[oi].sections[si].name;
                if spec.patterns.iter().any(|p| glob_match(p, name)) {
                    if def.name == "/DISCARD/" {
                        discard.insert((oi, si));
                    } else {
                        assigned.insert((oi, si), di);
                        assigned_item.insert((oi, si), ii);
                    }
                }
            }
        }
    }

    // Orphan sections: allocatable sections not matched by any pattern get
    // appended after the last section (GNU --orphan-handling=place). For the
    // kernel script every real section is matched; orphans are mostly
    // .comment/.note.* metadata. Non-alloc orphans are dropped from the image
    // (we still emit .symtab/.strtab/.shstrtab ourselves).
    let mut orphans: Vec<(usize, usize)> = Vec::new();
    for &(oi, si) in unassigned.iter() {
        if assigned.contains_key(&(oi, si)) || discard.contains(&(oi, si)) {
            continue;
        }
        let sec = &objects[oi].sections[si];
        if sec.flags & SHF_ALLOC_ != 0 && sec.size > 0 {
            orphans.push((oi, si));
        }
    }

    // ── Layout: walk the script, maintaining dot ──
    let mut symbols: FxHashMap<String, u64> = FxHashMap::default();
    // The parser represents SIZEOF_HEADERS as this reserved symbol because the
    // value depends on PHDRS and therefore is not a context-free constant.
    // Seed it before the first assignment is evaluated: Linux's vDSO script
    // starts with `. = SIZEOF_HEADERS`, before any output section exists.
    // A synthesised PT_TLS adds a program header, so it must be counted here
    // too or SIZEOF_HEADERS under-reserves and the first section overlaps the
    // header table. Predict it from the inputs, since output sections do not
    // exist yet: any allocated SHF_TLS input section will produce one unless
    // the script already declares PT_TLS.
    let will_add_tls_phdr = objects.iter().any(|o| {
        o.sections
            .iter()
            .any(|sec| (sec.flags & SHF_TLS_) != 0 && (sec.flags & SHF_ALLOC_) != 0 && sec.size > 0)
    }) && !script.phdrs.iter().any(|d| d.ptype == PT_TLS_);
    symbols.insert(
        "__SIZEOF_HEADERS".into(),
        script_header_size_with(&script, usize::from(will_add_tls_phdr), machine),
    );
    // Script symbols carry expression relocatability, not merely a numeric
    // value or syntactic assignment scope.  In particular, a SECTIONS-scope
    // `end = .` remains relative to the current output section, while section
    // differences and explicit ABSOLUTE expressions are SHN_ABS.
    let mut sym_home: FxHashMap<String, String> = FxHashMap::default();
    let mut absolute_script_syms: FxHashSet<String> = FxHashSet::default();
    absolute_script_syms.insert("__SIZEOF_HEADERS".into());
    // Symbols defined by HIDDEN()/PROVIDE_HIDDEN(): emitted STV_HIDDEN and
    // kept out of .dynsym. Losing this bit exports internal markers from every
    // shared object built with a custom script.
    let mut hidden_syms: FxHashSet<String> = FxHashSet::default();
    let mut sections_meta: FxHashMap<String, (u64, u64, u64, u64)> = FxHashMap::default();
    let mut out_secs: Vec<OutSec> = Vec::new();
    let mut dot: u64 = 0;
    // Output section to which the SECTIONS-scope location counter currently
    // belongs.  An absolute `. = value` clears it until the next section.
    let mut dot_home: Option<String> = None;

    // MEMORY regions: base/limit for the overflow check, plus a per-region
    // allocation cursor so `> region` sections pack independently of `dot`.
    let mut region_bounds: FxHashMap<String, (u64, u64)> = FxHashMap::default();
    let mut region_cursor: FxHashMap<String, u64> = FxHashMap::default();
    for r in &script.memory {
        let empty_syms: FxHashMap<String, u64> = FxHashMap::default();
        let empty_secs: FxHashMap<String, (u64, u64, u64, u64)> = FxHashMap::default();
        let ctx = linker_script::EvalCtx {
            dot: 0,
            symbols: &empty_syms,
            sections: &empty_secs,
            segment_starts: None,
        };
        let origin = linker_script::eval_expr(&r.origin, &ctx)
            .map_err(|_| format!("MEMORY region '{}': ORIGIN is not a constant", r.name))?;
        let length = linker_script::eval_expr(&r.length, &ctx)
            .map_err(|_| format!("MEMORY region '{}': LENGTH is not a constant", r.name))?;
        region_bounds.insert(r.name.clone(), (origin, origin.saturating_add(length)));
        region_cursor.insert(r.name.clone(), origin);
    }

    // Deferred assignments that referenced not-yet-defined symbols; re-run to
    // fixed point after layout.
    let mut deferred: Vec<(Assignment, u64, Option<String>, bool)> = Vec::new();
    // (assignment, numeric dot, dot's output-section home, inside output body)

    // Symbol values that live inside output sections get resolved lazily:
    // we need input-section vaddrs first. We do a single forward pass, which
    // works because GNU scripts define symbols before use except for
    // cross-references handled via deferral.
    //
    // Pre-pass: seed `symbols` with per-object section-symbol placeholders? No —
    // symbol lookups during layout come from assignments only. References to
    // object symbols (e.g. `phys_startup_64 = ABSOLUTE(startup_64 - X)`) are
    // resolved through def_syms once their section has been placed.

    // Helper to resolve an object symbol to its final vaddr if its section is
    // already placed. Uses a by-(obj,sec) lookup of placed sections.
    let mut placed_map: FxHashMap<(usize, usize), u64> = FxHashMap::default();
    // Owning output-section index for each placed input section. Used for
    // relocation application: an address-based lookup is ambiguous because
    // empty marker sections (.init.begin) share addresses with real ones.
    let mut placed_owner: FxHashMap<(usize, usize), usize> = FxHashMap::default();

    // Resolve symbol via assignments or object definitions.
    fn lookup_sym(
        name: &str,
        symbols: &FxHashMap<String, u64>,
        def_syms: &FxHashMap<String, (usize, u16, u64, u64, u8)>,
        placed_map: &FxHashMap<(usize, usize), u64>,
    ) -> Option<u64> {
        if let Some(&v) = symbols.get(name) {
            return Some(v);
        }
        if let Some(&(oi, shndx, value, _, _)) = def_syms.get(name) {
            if shndx == SHN_ABS {
                return Some(value);
            }
            if let Some(&base) = placed_map.get(&(oi, shndx as usize)) {
                return Some(base + value);
            }
        }
        None
    }

    // Evaluate with object-symbol fallback; returns Err(name) on undefined.
    fn eval_full(
        e: &linker_script::Expr,
        dot: u64,
        symbols: &FxHashMap<String, u64>,
        sections_meta: &FxHashMap<String, (u64, u64, u64, u64)>,
        def_syms: &FxHashMap<String, (usize, u16, u64, u64, u8)>,
        placed_map: &FxHashMap<(usize, usize), u64>,
    ) -> Result<u64, String> {
        // Fast path: try direct eval; on undefined symbol, augment.
        let ctx = EvalCtx {
            dot,
            symbols,
            sections: sections_meta,
            segment_starts: None,
        };
        match eval_expr(e, &ctx) {
            Ok(v) => Ok(v),
            Err(EvalError::UndefinedSymbol(n)) => {
                if let Some(v) = lookup_sym(&n, symbols, def_syms, placed_map) {
                    // clone-augment (rare path)
                    let mut aug = symbols.clone();
                    aug.insert(n, v);
                    // may still hit more undefined symbols; recurse via loop
                    let mut aug2 = aug;
                    loop {
                        let ctx2 = EvalCtx {
                            dot,
                            symbols: &aug2,
                            sections: sections_meta,
                            segment_starts: None,
                        };
                        match eval_expr(e, &ctx2) {
                            Ok(v) => return Ok(v),
                            Err(EvalError::UndefinedSymbol(n2)) => {
                                match lookup_sym(&n2, &aug2, def_syms, placed_map) {
                                    Some(v2) => {
                                        aug2.insert(n2, v2);
                                    }
                                    None => return Err(n2),
                                }
                            }
                            Err(EvalError::UnknownSection(s)) => {
                                return Err(format!("section {}", s))
                            }
                            Err(EvalError::AssertFailed(m)) => {
                                return Err(format!("ASSERT failed: {}", m))
                            }
                        }
                    }
                } else {
                    Err(n)
                }
            }
            Err(EvalError::UnknownSection(s)) => Err(format!("section {}", s)),
            Err(EvalError::AssertFailed(m)) => Err(format!("ASSERT failed: {}", m)),
        }
    }

    for item in &script.sections {
        match item {
            SectionsItem::Assign(a) => {
                let expr_home = script_expr_home(
                    &a.expr,
                    dot_home.as_deref(),
                    &sym_home,
                    &absolute_script_syms,
                );
                match eval_full(
                    &a.expr,
                    dot,
                    &symbols,
                    &sections_meta,
                    &def_syms,
                    &placed_map,
                ) {
                    Ok(v) => {
                        if a.symbol == "." {
                            dot = if a.op == AssignOp::Add {
                                dot.wrapping_add(v)
                            } else {
                                v
                            };
                            if a.op == AssignOp::Set {
                                dot_home = match expr_home {
                                    ScriptExprHome::Section(home) => Some(home),
                                    ScriptExprHome::Absolute | ScriptExprHome::Unresolved => None,
                                };
                            }
                        } else {
                            let nv = if a.op == AssignOp::Add {
                                symbols.get(&a.symbol).copied().unwrap_or(0).wrapping_add(v)
                            } else {
                                v
                            };
                            if !(a.provide
                                && (symbols.contains_key(&a.symbol)
                                    || def_syms.contains_key(&a.symbol)))
                            {
                                symbols.insert(a.symbol.clone(), nv);
                                sym_home.remove(&a.symbol);
                                absolute_script_syms.remove(&a.symbol);
                                match expr_home {
                                    ScriptExprHome::Section(home) => {
                                        sym_home.insert(a.symbol.clone(), home);
                                    }
                                    ScriptExprHome::Absolute => {
                                        absolute_script_syms.insert(a.symbol.clone());
                                    }
                                    ScriptExprHome::Unresolved => {}
                                }
                                if a.hidden {
                                    hidden_syms.insert(a.symbol.clone());
                                }
                            }
                        }
                    }
                    Err(_) => deferred.push((a.clone(), dot, dot_home.clone(), false)),
                }
            }
            SectionsItem::Assert(e, msg) => {
                // Defer asserts until after layout (sections sized).
                deferred.push((
                    Assignment {
                        symbol: "__assert__".into(),
                        op: AssignOp::Set,
                        expr: linker_script::Expr::Assert(Box::new(e.clone()), msg.clone()),
                        provide: false,
                        hidden: false,
                    },
                    dot,
                    dot_home.clone(),
                    false,
                ));
            }
            SectionsItem::Output(def) => {
                if def.name == "/DISCARD/" {
                    continue;
                }
                let di = out_defs.iter().position(|d| std::ptr::eq(*d, def)).unwrap();

                // Non-alloc convention: explicit address 0 (debug sections) or (INFO).
                let explicit_zero = matches!(def.address, Some(linker_script::Expr::Num(0)));
                let is_alloc = !def.info && !explicit_zero;

                // Section start: explicit address, region base, or dot.
                //
                // `> region` sets the VMA from the region's allocation cursor,
                // so consecutive sections assigned to a region pack into it
                // independently of the global dot. An explicit address still
                // wins, matching GNU ld.
                let mut sec_start = if let Some(ref ae) = def.address {
                    eval_full(ae, dot, &symbols, &sections_meta, &def_syms, &placed_map)
                        .map_err(|e| format!("cannot evaluate address of {}: {}", def.name, e))?
                } else if let Some(ref rname) = def.region {
                    *region_cursor.get(rname.as_str()).ok_or_else(|| {
                        format!(
                            "linker script: section {} assigned to undefined MEMORY region '{}'",
                            def.name, rname
                        )
                    })?
                } else {
                    dot
                };
                if let Some(ref al) = def.align {
                    let a = eval_full(
                        al,
                        sec_start,
                        &symbols,
                        &sections_meta,
                        &def_syms,
                        &placed_map,
                    )
                    .unwrap_or(1)
                    .max(1);
                    sec_start = (sec_start + a - 1) & !(a - 1);
                } else if def.address.is_none() {
                    // GNU ld aligns the OUTPUT section start to the largest
                    // alignment among its (first-item) input sections, not
                    // just each input within the section. Without this,
                    // .text can start at an unaligned dot (observed as a
                    // 0x35-byte skew vs GNU ld on the kernel decompressor).
                    let di_self = di;
                    let max_in_align = unassigned
                        .iter()
                        .filter(|k| assigned.get(k) == Some(&di_self))
                        .map(|&(oi, si)| objects[oi].sections[si].addralign.max(1))
                        .max()
                        .unwrap_or(1);
                    if max_in_align > 1 {
                        sec_start = (sec_start + max_in_align - 1) & !(max_in_align - 1);
                    }
                }

                let mut cur = sec_start;
                let mut placed: Vec<Placed> = Vec::new();
                let mut max_align: u64 = 1;
                let mut sh_type = SHT_PROGBITS_;
                let mut flags: u64 = 0;
                let mut any_progbits = false;
                let mut data_commands: Vec<(u64, u8, linker_script::Expr)> = Vec::new();

                // Gather matched inputs per item, in item order.
                for (ii, sitem) in def.items.iter().enumerate() {
                    match sitem {
                        SecItem::Assign(a) => {
                            // In-section assignments evaluate against the walking dot.
                            // SIZEOF(this output section) is a forward reference
                            // until the section body has been laid out.
                            if expr_references_sizeof(&a.expr, &def.name) {
                                deferred.push((a.clone(), cur, Some(def.name.clone()), true));
                                continue;
                            }
                            match eval_full(
                                &a.expr,
                                cur,
                                &symbols,
                                &sections_meta,
                                &def_syms,
                                &placed_map,
                            ) {
                                Ok(v) => {
                                    if a.symbol == "." {
                                        let nv = if a.op == AssignOp::Add {
                                            cur.wrapping_add(v)
                                        } else {
                                            v
                                        };
                                        if nv < cur && a.op == AssignOp::Set {
                                            // moving dot backwards is an error in ld; clamp
                                            cur = nv.max(sec_start);
                                        } else {
                                            cur = nv;
                                        }
                                    } else {
                                        let v = if !expr_contains_absolute(&a.expr)
                                            && script_value_form(
                                                &a.expr,
                                                &sym_home,
                                                &absolute_script_syms,
                                            ) == ScriptValueForm::Offset
                                        {
                                            sec_start.wrapping_add(v)
                                        } else {
                                            v
                                        };
                                        let nv = if a.op == AssignOp::Add {
                                            symbols
                                                .get(&a.symbol)
                                                .copied()
                                                .unwrap_or(0)
                                                .wrapping_add(v)
                                        } else {
                                            v
                                        };
                                        if !(a.provide
                                            && (symbols.contains_key(&a.symbol)
                                                || def_syms.contains_key(&a.symbol)))
                                        {
                                            symbols.insert(a.symbol.clone(), nv);
                                            sym_home.remove(&a.symbol);
                                            absolute_script_syms.remove(&a.symbol);
                                            if expr_contains_absolute(&a.expr) {
                                                absolute_script_syms.insert(a.symbol.clone());
                                            } else {
                                                sym_home.insert(a.symbol.clone(), def.name.clone());
                                            }
                                            if a.hidden {
                                                hidden_syms.insert(a.symbol.clone());
                                            }
                                        }
                                    }
                                }
                                Err(_) => {
                                    deferred.push((a.clone(), cur, Some(def.name.clone()), true))
                                }
                            }
                        }
                        SecItem::Assert(e, msg) => {
                            deferred.push((
                                Assignment {
                                    symbol: "__assert__".into(),
                                    op: AssignOp::Set,
                                    expr: linker_script::Expr::Assert(
                                        Box::new(e.clone()),
                                        msg.clone(),
                                    ),
                                    provide: false,
                                    hidden: false,
                                },
                                cur,
                                Some(def.name.clone()),
                                true,
                            ));
                        }
                        SecItem::Data { width, expr } => {
                            // Data directives are byte-packed: unlike input
                            // sections they impose no implicit alignment.  The
                            // expression may refer to symbols defined later,
                            // so reserve its width now and evaluate it after
                            // the layout/deferred-assignment fixed point.
                            data_commands.push((cur - sec_start, *width, expr.clone()));
                            cur = cur.checked_add(*width as u64).ok_or_else(|| {
                                format!(
                                    "linker script: data directive in {} overflows address space",
                                    def.name
                                )
                            })?;
                            any_progbits = true;
                            if is_alloc {
                                flags |= SHF_ALLOC_;
                            }
                        }
                        SecItem::Constructors => { /* ELF: no-op */ }
                        SecItem::Input(spec) => {
                            // Collect matching inputs assigned to (di, ii).
                            let mut ins: Vec<(usize, usize)> = unassigned
                                .iter()
                                .filter(|k| {
                                    assigned.get(k) == Some(&di)
                                        && assigned_item.get(k) == Some(&ii)
                                })
                                .copied()
                                .collect();
                            match spec.sort {
                                SortKind::ByName | SortKind::ByInitPriority => {
                                    ins.sort_by(|a, b| {
                                        objects[a.0].sections[a.1]
                                            .name
                                            .cmp(&objects[b.0].sections[b.1].name)
                                            .then(a.cmp(b))
                                    });
                                }
                                SortKind::ByAlignment => {
                                    ins.sort_by(|a, b| {
                                        objects[b.0].sections[b.1]
                                            .addralign
                                            .cmp(&objects[a.0].sections[a.1].addralign)
                                            .then(a.cmp(b))
                                    });
                                }
                                SortKind::None => {
                                    // GNU ld: command-line (object) order, section order
                                    ins.sort();
                                }
                            }
                            for (oi, si) in ins {
                                let sec = &objects[oi].sections[si];
                                let a = sec.addralign.max(1);
                                cur = (cur + a - 1) & !(a - 1);
                                placed_map.insert((oi, si), cur);
                                placed_owner.insert((oi, si), out_secs.len());
                                placed.push(Placed {
                                    obj_idx: oi,
                                    sec_idx: si,
                                    vaddr: cur,
                                    size: sec.size,
                                });
                                cur += sec.size;
                                if a > max_align {
                                    max_align = a;
                                }
                                if sec.sh_type == SHT_PROGBITS_ {
                                    any_progbits = true;
                                }
                                if sec.sh_type != SHT_NOBITS_
                                    && sec.sh_type != SHT_PROGBITS_
                                    && sh_type == SHT_PROGBITS_
                                    && !any_progbits
                                {
                                    sh_type = sec.sh_type;
                                }
                                flags |= sec.flags
                                    & (SHF_WRITE_ | SHF_ALLOC_ | SHF_EXECINSTR_ | SHF_TLS_);
                            }
                        }
                    }
                }

                if std::env::var("LCCC_LD_MAP").is_ok() {
                    for p in &placed {
                        eprintln!(
                            "MAP {} 0x{:x} 0x{:x} {}({})",
                            def.name,
                            p.vaddr,
                            p.size,
                            objects[p.obj_idx].source_name,
                            objects[p.obj_idx].sections[p.sec_idx].name
                        );
                    }
                }

                let size = cur.saturating_sub(sec_start);
                // NOBITS only if ALL inputs are NOBITS and there was no ". += N" gap
                // requiring file content. Kernel .bss/.brk are file-backed zeros in
                // GNU ld only when fill is demanded; standard behavior: keep NOBITS.
                let nobits = placed
                    .iter()
                    .all(|p| objects[p.obj_idx].sections[p.sec_idx].sh_type == SHT_NOBITS_)
                    && def.fill.is_none()
                    && (def.name.starts_with(".bss")
                        || def.name.starts_with(".brk")
                        || placed.iter().len() > 0
                            && placed.iter().all(|p| {
                                objects[p.obj_idx].sections[p.sec_idx].sh_type == SHT_NOBITS_
                            })
                            && !placed.is_empty());
                let nobits = nobits && !placed.is_empty()
                    || (placed.is_empty()
                        && (def.name.starts_with(".bss") || def.name.starts_with(".brk"))
                        && size > 0);

                if is_alloc {
                    flags |= SHF_ALLOC_;
                }

                let lma = if let Some(ref at) = def.at_lma {
                    // AT() may reference ADDR(this-section): publish addr first.
                    sections_meta.insert(def.name.clone(), (sec_start, size, max_align, sec_start));
                    eval_full(
                        at,
                        sec_start,
                        &symbols,
                        &sections_meta,
                        &def_syms,
                        &placed_map,
                    )
                    .map_err(|e| format!("cannot evaluate AT() of {}: {}", def.name, e))?
                } else {
                    sec_start
                };
                sections_meta.insert(def.name.clone(), (sec_start, size, max_align, lma));

                // MEMORY region accounting. Overrunning a region is the single
                // most common linker-script mistake, and an unchecked overrun
                // produces an image whose sections silently overlap. Diagnose
                // it by name, with the overflow amount, like GNU ld does.
                if let Some(rname) = def.region.as_deref() {
                    let end = sec_start.saturating_add(size);
                    if let Some(&(origin, limit)) = region_bounds.get(rname) {
                        if end > limit {
                            return Err(format!(
                                "linker script: section {} overflows MEMORY region '{}' by \
                                 {} bytes (region {:#x}..{:#x}, section {:#x}..{:#x})",
                                def.name,
                                rname,
                                end - limit,
                                origin,
                                limit,
                                sec_start,
                                end
                            ));
                        }
                    }
                    region_cursor.insert(rname.to_string(), end);
                }
                if let Some(rname) = def.lma_region.as_deref() {
                    // AT> region: the load image must fit too.
                    if let Some(&(origin, limit)) = region_bounds.get(rname) {
                        let lend = lma.saturating_add(size);
                        if lend > limit {
                            return Err(format!(
                                "linker script: load image of {} overflows MEMORY region \
                                 '{}' by {} bytes (region {:#x}..{:#x})",
                                def.name,
                                rname,
                                lend - limit,
                                origin,
                                limit
                            ));
                        }
                    }
                }

                out_secs.push(OutSec {
                    name: def.name.clone(),
                    vaddr: sec_start,
                    lma,
                    size,
                    align: max_align,
                    sh_type: if nobits { SHT_NOBITS_ } else { sh_type },
                    flags,
                    file_offset: 0,
                    is_alloc,
                    nobits,
                    phdrs: def.phdrs.clone(),
                    fill: def.fill,
                    placed,
                    data_commands,
                });

                if is_alloc {
                    dot = cur;
                    dot_home = Some(def.name.clone());
                }
            }
        }
    }

    // Append orphan alloc sections at the end (page-aligned).
    if !orphans.is_empty() {
        for (oi, si) in orphans {
            let sec = &objects[oi].sections[si];
            let a = sec.addralign.max(1);
            dot = (dot + a - 1) & !(a - 1);
            placed_map.insert((oi, si), dot);
            placed_owner.insert((oi, si), out_secs.len());
            let start = dot;
            dot += sec.size;
            dot_home = Some(sec.name.clone());
            sections_meta.insert(sec.name.clone(), (start, sec.size, a, start));
            out_secs.push(OutSec {
                name: sec.name.clone(),
                vaddr: start,
                lma: start,
                size: sec.size,
                align: a,
                sh_type: sec.sh_type,
                flags: sec.flags & !SHF_GROUP_,
                file_offset: 0,
                is_alloc: true,
                nobits: sec.sh_type == SHT_NOBITS_,
                phdrs: Vec::new(),
                fill: None,
                placed: vec![Placed {
                    obj_idx: oi,
                    sec_idx: si,
                    vaddr: start,
                    size: sec.size,
                }],
                data_commands: Vec::new(),
            });
        }
    }

    // ── Re-run deferred assignments to fixed point ──
    let mut made_progress = true;
    let mut pending = deferred;
    while made_progress && !pending.is_empty() {
        made_progress = false;
        let mut still: Vec<(Assignment, u64, Option<String>, bool)> = Vec::new();
        for (a, adot, adot_home, in_output_body) in pending.into_iter() {
            match eval_full(
                &a.expr,
                adot,
                &symbols,
                &sections_meta,
                &def_syms,
                &placed_map,
            ) {
                Ok(v) => {
                    made_progress = true;
                    if a.symbol == "__assert__" || a.symbol == "." {
                        continue;
                    }
                    if a.provide
                        && (symbols.contains_key(&a.symbol) || def_syms.contains_key(&a.symbol))
                    {
                        continue;
                    }
                    let v = if in_output_body
                        && !expr_contains_absolute(&a.expr)
                        && script_value_form(&a.expr, &sym_home, &absolute_script_syms)
                            == ScriptValueForm::Offset
                    {
                        adot_home
                            .as_ref()
                            .and_then(|home| sections_meta.get(home))
                            .map(|section| section.0.wrapping_add(v))
                            .unwrap_or(v)
                    } else {
                        v
                    };
                    let nv = if a.op == AssignOp::Add {
                        symbols.get(&a.symbol).copied().unwrap_or(0).wrapping_add(v)
                    } else {
                        v
                    };
                    symbols.insert(a.symbol.clone(), nv);
                    sym_home.remove(&a.symbol);
                    absolute_script_syms.remove(&a.symbol);
                    let home = if in_output_body && !expr_contains_absolute(&a.expr) {
                        adot_home
                            .clone()
                            .map(ScriptExprHome::Section)
                            .unwrap_or(ScriptExprHome::Unresolved)
                    } else {
                        script_expr_home(
                            &a.expr,
                            adot_home.as_deref(),
                            &sym_home,
                            &absolute_script_syms,
                        )
                    };
                    match home {
                        ScriptExprHome::Section(section) => {
                            sym_home.insert(a.symbol.clone(), section);
                        }
                        ScriptExprHome::Absolute => {
                            absolute_script_syms.insert(a.symbol.clone());
                        }
                        ScriptExprHome::Unresolved => {}
                    }
                    if a.hidden {
                        hidden_syms.insert(a.symbol.clone());
                    }
                }
                Err(_) => still.push((a, adot, adot_home, in_output_body)),
            }
        }
        pending = still;
    }
    // Evaluate top-level assigns/asserts (PROVIDE etc.) the same way.
    for a in &script.top_assigns {
        if let Ok(v) = eval_full(
            &a.expr,
            dot,
            &symbols,
            &sections_meta,
            &def_syms,
            &placed_map,
        ) {
            if a.symbol == "." {
                continue;
            }
            if a.provide && (symbols.contains_key(&a.symbol) || def_syms.contains_key(&a.symbol)) {
                continue;
            }
            symbols.insert(a.symbol.clone(), v);
            sym_home.remove(&a.symbol);
            absolute_script_syms.remove(&a.symbol);
            match script_expr_home(
                &a.expr,
                dot_home.as_deref(),
                &sym_home,
                &absolute_script_syms,
            ) {
                ScriptExprHome::Section(section) => {
                    sym_home.insert(a.symbol.clone(), section);
                }
                ScriptExprHome::Absolute => {
                    absolute_script_syms.insert(a.symbol.clone());
                }
                ScriptExprHome::Unresolved => {}
            }
            if a.hidden {
                hidden_syms.insert(a.symbol.clone());
            }
        }
    }
    for (e, msg) in &script.top_asserts {
        if let Err(err) = eval_full(e, dot, &symbols, &sections_meta, &def_syms, &placed_map) {
            return Err(format!("linker script assert: {} ({})", msg, err));
        }
    }
    for (a, adot, _, _) in &pending {
        // Remaining failures: asserts get reported, plain assigns error out.
        match eval_full(
            &a.expr,
            *adot,
            &symbols,
            &sections_meta,
            &def_syms,
            &placed_map,
        ) {
            Ok(_) => {}
            Err(e) => {
                if a.symbol == "__assert__" {
                    return Err(format!("linker script ASSERT failed: {}", e));
                }
                // PROVIDE of an unresolvable symbol that nothing references is OK.
                if !a.provide {
                    return Err(format!(
                        "linker script: cannot resolve assignment {} = ... ({})",
                        a.symbol, e
                    ));
                }
            }
        }
    }

    // i386 PIC sequences address symbols relative to the conventional GOT
    // base. A vDSO has no GOT entries; GNU ld defines the base immediately
    // after `.dynamic`, and R_386_GOTPC/GOTOFF still use it. Define the same
    // absolute address before relocation resolution.
    if machine == ScriptMachine::I386 && !symbols.contains_key("_GLOBAL_OFFSET_TABLE_") {
        if let Some(&(addr, size, _, _)) = sections_meta.get(".dynamic") {
            symbols.insert("_GLOBAL_OFFSET_TABLE_".into(), addr + size);
            sym_home.insert("_GLOBAL_OFFSET_TABLE_".into(), ".dynamic".into());
        }
    }

    // ── Resolve every symbol needed by relocations ──
    // resolve(obj, sym) -> Option<vaddr>
    let resolve = |oi: usize, sym: &linker_common::Elf64Symbol| -> Option<u64> {
        if sym.sym_type() == STT_SECTION {
            return placed_map.get(&(oi, sym.shndx as usize)).copied();
        }
        if !sym.name.is_empty() {
            if sym.is_local() {
                if sym.shndx == SHN_ABS {
                    return Some(sym.value);
                }
                return placed_map
                    .get(&(oi, sym.shndx as usize))
                    .map(|b| b + sym.value);
            }
            if let Some(&v) = symbols.get(sym.name.as_str()) {
                return Some(v);
            }
            if let Some(&(doi, shndx, value, _, _)) = def_syms.get(sym.name.as_str()) {
                if shndx == SHN_ABS {
                    return Some(value);
                }
                return placed_map.get(&(doi, shndx as usize)).map(|b| b + value);
            }
            if sym.is_weak() {
                return Some(0);
            }
            return None;
        }
        if sym.shndx == SHN_ABS {
            return Some(sym.value);
        }
        placed_map
            .get(&(oi, sym.shndx as usize))
            .map(|b| b + sym.value)
    };

    // ── File layout ──
    // Compute program headers from PHDRS + section->phdr assignments.
    // Sections inherit the previous section's phdr list when unspecified (GNU rule).
    let mut cur_phdrs: Vec<String> = Vec::new();
    let mut sec_phdrs: Vec<Vec<String>> = Vec::with_capacity(out_secs.len());
    for os in &out_secs {
        if !os.phdrs.is_empty() {
            cur_phdrs = os.phdrs.clone();
        }
        if os.is_alloc {
            sec_phdrs.push(cur_phdrs.clone());
        } else {
            sec_phdrs.push(Vec::new());
        }
    }

    // ELF header + phdrs at file start.
    let declared_phdrs: Vec<&linker_script::PhdrDecl> = script.phdrs.iter().collect();
    // A script that places SHF_TLS sections but declares no PT_TLS gets one
    // synthesised. Without it the loader never allocates the thread block, so
    // every %fs-relative access reads whatever happens to precede the TCB --
    // the relocations are correct and the program still misbehaves.
    //
    // This must agree with `will_add_tls_phdr` above, which fed SIZEOF_HEADERS.
    let needs_tls_phdr = out_secs
        .iter()
        .any(|o| (o.flags & SHF_TLS_) != 0 && o.is_alloc)
        && !script.phdrs.iter().any(|d| d.ptype == PT_TLS_);
    debug_assert_eq!(
        needs_tls_phdr, will_add_tls_phdr,
        "PT_TLS prediction disagreed with the final layout; SIZEOF_HEADERS would be wrong"
    );
    let n_phdrs = declared_phdrs.len().max(1) + usize::from(needs_tls_phdr);
    let mut file_off = script_header_size_with(&script, usize::from(needs_tls_phdr), machine);

    // Assign file offsets: alloc PROGBITS sections in vaddr order get offsets
    // congruent to their LMA modulo the requested maximum page size. The
    // kernel proper uses 2 MiB; its vDSO explicitly requests 4 KiB.
    let page = if max_page_size.is_power_of_two() {
        max_page_size
    } else {
        0x200000
    };
    let mut order: Vec<usize> = (0..out_secs.len()).collect();
    order.sort_by_key(|&i| (!out_secs[i].is_alloc, out_secs[i].lma));

    if declared_phdrs.is_empty() {
        // The implicit single PT_LOAD requires one constant p_vaddr-p_offset
        // delta across every file-backed section.  Merely packing sections at
        // congruent page offsets is insufficient: a VMA gap larger than one
        // page then maps later bytes at the wrong address (and objcopy loses
        // the gap).  Preserve the complete LMA delta for the synthetic LOAD.
        let min_lma = out_secs
            .iter()
            .filter(|os| os.is_alloc && os.size > 0)
            .map(|os| os.lma)
            .min()
            .unwrap_or(0);
        let want = min_lma % page;
        let have = file_off % page;
        if want != have {
            file_off += (want + page - have) % page;
        }
        let load_file_base = file_off;
        for &i in &order {
            let os = &mut out_secs[i];
            if !os.is_alloc || os.nobits || os.size == 0 {
                continue;
            }
            os.file_offset = load_file_base + os.lma.saturating_sub(min_lma);
            file_off = file_off.max(os.file_offset + os.size);
        }
    } else {
        for &i in &order {
            let os = &mut out_secs[i];
            if !os.is_alloc || os.nobits || os.size == 0 {
                continue;
            }
            // congruence: file_off ≡ lma (mod page)
            let want = os.lma % page;
            let have = file_off % page;
            if want != have {
                file_off += (want + page - have) % page;
            }
            os.file_offset = file_off;
            file_off += os.size;
        }
    }
    // Non-allocated PROGBITS still occupy the file. DWARF sections are often
    // hundreds of MiB in a kernel build; assigning all of them the same final
    // alloc offset while writing none of their bytes produced section headers
    // extending past EOF and made pahole/nm reject the image. Pack them after
    // the load image at their own alignment, without page congruence.
    for os in out_secs
        .iter_mut()
        .filter(|os| !os.is_alloc && !os.nobits && os.size > 0)
    {
        let align = os.align.max(1);
        file_off = file_off
            .checked_add(align - 1)
            .ok_or("non-allocated section file offset overflows")?
            & !(align - 1);
        os.file_offset = file_off;
        file_off = file_off
            .checked_add(os.size)
            .ok_or("non-allocated section size overflows output")?;
    }
    // NOBITS and empty sections point at the current file offset (no data).
    for os in out_secs.iter_mut() {
        if os.nobits || os.size == 0 {
            os.file_offset = file_off;
        }
    }

    // ── Build output image ──
    let mut out = vec![0u8; file_off as usize];

    // Fill patterns + section data
    for os in &out_secs {
        if os.nobits || os.size == 0 {
            continue;
        }
        let base = os.file_offset as usize;
        if let Some(fill) = os.fill {
            // GNU ld uses the fill value as a (big-endian) byte pattern; the
            // kernel's 0xcccccccc means "fill gaps with int3".
            let b = [
                (fill >> 24) as u8,
                (fill >> 16) as u8,
                (fill >> 8) as u8,
                fill as u8,
            ];
            let pat: &[u8] = if fill <= 0xff { &b[3..] } else { &b[..] };
            for k in 0..os.size as usize {
                out[base + k] = pat[k % pat.len()];
            }
        }
        for p in &os.placed {
            let data = &objects[p.obj_idx].section_data[p.sec_idx];
            if data.is_empty() {
                continue;
            }
            let off = base + (p.vaddr - os.vaddr) as usize;
            let end = off + data.len();
            if end <= out.len() {
                out[off..end].copy_from_slice(data);
            }
        }
        for (offset, width, expr) in &os.data_commands {
            let command_vaddr = os.vaddr + *offset;
            let value = eval_full(
                expr,
                command_vaddr,
                &symbols,
                &sections_meta,
                &def_syms,
                &placed_map,
            )
            .map_err(|e| {
                format!(
                    "cannot evaluate {}-byte data directive in {} at {:#x}: {}",
                    width, os.name, command_vaddr, e
                )
            })?;
            let off = base.checked_add(*offset as usize).ok_or_else(|| {
                format!(
                    "linker script: data directive offset overflows in {}",
                    os.name
                )
            })?;
            let end = off.checked_add(*width as usize).ok_or_else(|| {
                format!(
                    "linker script: data directive width overflows in {}",
                    os.name
                )
            })?;
            if end > out.len() {
                return Err(format!(
                    "linker script: data directive in {} falls outside output",
                    os.name
                ));
            }
            let bytes = value.to_le_bytes();
            out[off..end].copy_from_slice(&bytes[..*width as usize]);
        }
    }

    /// Relax a GOT-relative reference into direct addressing.
    ///
    /// A linker-script link has no GOT: every address is known, so the
    /// *indirection through a GOT slot* that the compiler emitted must be
    /// removed rather than satisfied. The x86-64 psABI ("GOTPCRELX
    /// relaxations") specifies the legal rewrites; this implements the forms
    /// GCC and Clang actually emit for `-fPIC` code:
    ///
    /// | before                              | after                        |
    /// |-------------------------------------|------------------------------|
    /// | `mov  sym@GOTPCREL(%rip), %reg` 8b  | `lea sym(%rip), %reg`    8d  |
    /// | `cmp  %reg, sym@GOTPCREL(%rip)` 3b  | `cmp $sym, %reg`   81 /7 imm |
    /// | `call *sym@GOTPCREL(%rip)`   ff /2  | `call sym` (e8) + nop        |
    /// | `jmp  *sym@GOTPCREL(%rip)`   ff /4  | `jmp  sym` (e9) + nop        |
    ///
    /// `mov` -> `lea` keeps the operand PC-relative, so it is position
    /// independent and needs no range beyond the usual +/-2 GiB. The `cmp`
    /// and indirect-branch forms become absolute/direct and are therefore
    /// only valid when the target fits the encoding; that is checked.
    ///
    /// Returns `Err(<opcode bytes>)` when the form is not one of the above,
    /// so the caller can produce a precise diagnostic instead of emitting a
    /// silently corrupt image.
    fn relax_gotpcrel(out: &mut [u8], fp: usize, s: u64, a: i64, p: u64) -> Result<(), String> {
        // Displacement for a rewritten PC-relative operand. The instruction
        // length is unchanged by every rewrite below, so `p` still names the
        // end of the 4-byte field.
        let pcrel = s as i64 + a - p as i64;
        let describe = |o: &[u8]| {
            o.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<_>>()
                .join(" ")
        };

        if fp < 2 || fp + 4 > out.len() {
            return Err("<truncated>".to_string());
        }
        let modrm = out[fp - 1];
        let op = out[fp - 2];

        // mov m64, r64  ->  lea m64, r64   (opcode 8b -> 8d, ModRM unchanged)
        if op == 0x8b {
            out[fp - 2] = 0x8d;
            w32(out, fp, pcrel as u32);
            return Ok(());
        }

        // ALU r64, m64  ->  ALU r64, imm32.
        //
        // The x86-64 psABI relaxes the whole `op r64, sym@GOTPCREL(%rip)`
        // family to the group-1 immediate form `81 /ext id`, where `ext`
        // selects the operation. Handling only `cmp` here meant an ordinary
        // `sub`/`add` against a GOT-relative address -- which GCC emits for
        // pointer arithmetic on an external symbol -- failed the link with an
        // "instruction form this linker cannot relax" error while GNU ld
        // accepted it.
        //
        //   opcode  operation   group-1 /ext
        //     03      add            0
        //     0b      or             1
        //     13      adc            2
        //     1b      sbb            3
        //     23      and            4
        //     2b      sub            5
        //     33      xor            6
        //     3b      cmp            7
        let alu_ext: Option<u8> = match op {
            0x03 => Some(0),
            0x0b => Some(1),
            0x13 => Some(2),
            0x1b => Some(3),
            0x23 => Some(4),
            0x2b => Some(5),
            0x33 => Some(6),
            0x3b => Some(7),
            _ => None,
        };
        if let Some(ext) = alu_ext {
            // The addend is -4 for a rip-relative operand; undo it to recover
            // the absolute address the immediate must carry.
            let abs = s as i64 + a + 4;
            if (i32::MIN as i64..=i32::MAX as i64).contains(&abs) {
                let reg = (modrm >> 3) & 7;
                out[fp - 2] = 0x81;
                out[fp - 1] = 0xc0 | (ext << 3) | reg; // mod=11, /ext, rm=reg
                w32(out, fp, abs as u32);
                return Ok(());
            }
            return Err(describe(&out[fp - 2..fp]));
        }

        // call/jmp *m64 -> direct call/jmp. ff /2 = call, ff /4 = jmp.
        // The indirect form is 6 bytes (ff /r + disp32) and the direct form is
        // 5 (e8/e9 + rel32), so a leading nop keeps the length identical --
        // exactly what GNU ld does.
        if op == 0xff && fp >= 2 {
            let ext = (modrm >> 3) & 7;
            if ext == 2 || ext == 4 {
                // The rel32 is measured from the end of the 5-byte direct
                // instruction, which now starts one byte later.
                let rel = s as i64 + a + 1 - p as i64;
                if !(i32::MIN as i64..=i32::MAX as i64).contains(&rel) {
                    return Err(describe(&out[fp - 2..fp]));
                }
                out[fp - 2] = 0x90; // nop
                out[fp - 1] = if ext == 2 { 0xe8 } else { 0xe9 };
                w32(out, fp, rel as u32);
                return Ok(());
            }
        }

        Err(describe(&out[fp - 2..fp]))
    }

    // ── TLS segment bounds ──
    //
    // Initial-Exec TLS offsets are measured from the *end* of the TLS block,
    // because %fs:0 points just past it on x86-64. The block spans every
    // SHF_TLS output section; .tbss contributes to the memory size but not to
    // the file image, exactly as in a PT_TLS program header.
    let (tls_addr, tls_mem_size) = {
        let mut lo = u64::MAX;
        let mut hi = 0u64;
        for os in out_secs
            .iter()
            .filter(|o| (o.flags & SHF_TLS_) != 0 && o.is_alloc)
        {
            lo = lo.min(os.vaddr);
            hi = hi.max(os.vaddr + os.size);
        }
        if lo == u64::MAX {
            (0u64, 0u64)
        } else {
            (lo, hi - lo)
        }
    };

    // ── Relocation helpers ──
    //
    // A 32-bit displacement that does not fit is the classic way a large image
    // gets silently corrupted: the truncated value points somewhere plausible
    // and the failure surfaces much later as a wild jump. Diagnose instead.
    fn reloc_range_err(kind: &str, v: i64, sym: &str, src: &str) -> String {
        format!(
            "script link: {} against '{}' in {} does not fit: value {:#x} \
                 is out of range (image too large or wrong load address?)",
            kind, sym, src, v
        )
    }
    fn check_pcrel32(v: i64, sym: &str, src: &str) -> Result<(), String> {
        if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
            return Err(reloc_range_err("32-bit PC-relative reference", v, sym, src));
        }
        Ok(())
    }

    // ── Apply relocations ──
    //
    // TLS GD/LD relaxation rewrites a multi-instruction sequence, and the
    // `__tls_get_addr` call inside it carries its OWN PLT32/GOTPCRELX
    // relocation. Applying that one afterwards would patch a displacement into
    // bytes that are no longer a call. Record the consumed byte ranges and
    // skip any relocation landing inside them.
    let mut tls_consumed: Vec<(usize, usize)> = Vec::new();
    let mut undefined: Vec<String> = Vec::new();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, relas) in obj.relocations.iter().enumerate() {
            if relas.is_empty() {
                continue;
            }
            let Some(&sec_vaddr) = placed_map.get(&(oi, si)) else {
                continue;
            };
            let Some(&owner) = placed_owner.get(&(oi, si)) else {
                continue;
            };
            let os = &out_secs[owner];
            if os.nobits {
                continue;
            }
            let sec_foff = os.file_offset + (sec_vaddr - os.vaddr);

            for rela in relas {
                let sidx = rela.sym_idx as usize;
                if sidx >= obj.symbols.len() {
                    continue;
                }
                {
                    let fp_chk = (sec_foff + rela.offset) as usize;
                    if tls_consumed.iter().any(|&(a, b)| fp_chk >= a && fp_chk < b) {
                        continue;
                    }
                }
                let sym = &obj.symbols[sidx];
                let p = sec_vaddr + rela.offset;
                let fp = (sec_foff + rela.offset) as usize;
                let a = rela.addend;
                let s = match resolve(oi, sym) {
                    Some(v) => v,
                    // DWARF commonly retains references to compiler metadata
                    // sections that the final script deliberately discards
                    // (`.discard.addressable` in Linux). GNU ld resolves those
                    // debug-only addresses to zero. Runtime references from an
                    // allocated section remain hard errors.
                    None if !os.is_alloc && {
                        if sym.is_local() || sym.sym_type() == STT_SECTION {
                            discard.contains(&(oi, sym.shndx as usize))
                                || dead_sections.contains(&(oi, sym.shndx as usize))
                        } else {
                            def_syms
                                .get(sym.name.as_str())
                                .is_some_and(|&(doi, shndx, _, _, _)| {
                                    discard.contains(&(doi, shndx as usize))
                                        || dead_sections.contains(&(doi, shndx as usize))
                                })
                        }
                    } =>
                    {
                        0
                    }
                    None => {
                        let unresolved = if sym.name.is_empty() {
                            let section = obj
                                .sections
                                .get(sym.shndx as usize)
                                .map(|sec| sec.name.as_str())
                                .unwrap_or("<invalid>");
                            format!(
                                "<section {} '{}' in {}>",
                                sym.shndx, section, obj.source_name
                            )
                        } else {
                            sym.name.to_string()
                        };
                        if undefined.len() < 20 && !undefined.iter().any(|u| u == &unresolved) {
                            undefined.push(unresolved);
                        }
                        continue;
                    }
                };
                if machine == ScriptMachine::I386 {
                    let absolute = s as i64 + a;
                    let relative = absolute - p as i64;
                    match rela.rela_type {
                        R_386_NONE => {}
                        R_386_32 => {
                            if !(i32::MIN as i64..=u32::MAX as i64).contains(&absolute) {
                                return Err(reloc_range_err(
                                    "R_386_32",
                                    absolute,
                                    &sym.name,
                                    &obj.source_name,
                                ));
                            }
                            w32(&mut out, fp, absolute as u32);
                        }
                        R_386_PC32 | R_386_PLT32 => {
                            check_pcrel32(relative, &sym.name, &obj.source_name)?;
                            w32(&mut out, fp, relative as u32);
                        }
                        R_386_GOTPC => {
                            let got = symbols
                                .get("_GLOBAL_OFFSET_TABLE_")
                                .copied()
                                .ok_or("R_386_GOTPC requires _GLOBAL_OFFSET_TABLE_")?;
                            let value = got as i64 + a - p as i64;
                            check_pcrel32(value, &sym.name, &obj.source_name)?;
                            w32(&mut out, fp, value as u32);
                        }
                        R_386_GOTOFF => {
                            let got = symbols
                                .get("_GLOBAL_OFFSET_TABLE_")
                                .copied()
                                .ok_or("R_386_GOTOFF requires _GLOBAL_OFFSET_TABLE_")?;
                            w32(&mut out, fp, (s as i64 + a - got as i64) as u32);
                        }
                        R_386_16 => {
                            if !(0..=u16::MAX as i64).contains(&absolute) {
                                return Err(reloc_range_err(
                                    "R_386_16",
                                    absolute,
                                    &sym.name,
                                    &obj.source_name,
                                ));
                            }
                            w16(&mut out, fp, absolute as u16);
                        }
                        R_386_PC16 => {
                            // ELF i386 defines this relocation modulo 2^16.
                            // In Linux real-mode setup code a branch near the
                            // top of a 64 KiB segment can legitimately target
                            // its low half: the mathematical S+A-P is then
                            // below -32768, but its wrapped 16-bit displacement
                            // is exactly what the CPU consumes.  BFD accepts
                            // that case; a signed-i16 range check rejects it.
                            w16(&mut out, fp, relative as u16);
                        }
                        R_386_8 => {
                            if !(0..=u8::MAX as i64).contains(&absolute) {
                                return Err(reloc_range_err(
                                    "R_386_8",
                                    absolute,
                                    &sym.name,
                                    &obj.source_name,
                                ));
                            }
                            out[fp] = absolute as u8;
                        }
                        R_386_PC8 => {
                            if !(i8::MIN as i64..=i8::MAX as i64).contains(&relative) {
                                return Err(reloc_range_err(
                                    "R_386_PC8",
                                    relative,
                                    &sym.name,
                                    &obj.source_name,
                                ));
                            }
                            out[fp] = relative as u8;
                        }
                        R_386_SIZE32 => w32(&mut out, fp, (sym.size as i64 + a) as u32),
                        other => {
                            return Err(format!(
                                "script link: unsupported i386 relocation type {} for '{}' in {}",
                                other, sym.name, obj.source_name
                            ));
                        }
                    }
                    continue;
                }
                match rela.rela_type {
                    R_X86_64_64 => w64(&mut out, fp, (s as i64 + a) as u64),
                    R_X86_64_PC32 | R_X86_64_PLT32 => {
                        // A -T link resolves every call directly; there is no
                        // PLT, so PLT32 degenerates to PC32.
                        let v = s as i64 + a - p as i64;
                        check_pcrel32(v, &sym.name, &obj.source_name)?;
                        w32(&mut out, fp, v as u32)
                    }
                    R_X86_64_32 => {
                        let v = s as i64 + a;
                        if !(0..=u32::MAX as i64).contains(&v) {
                            return Err(reloc_range_err(
                                "R_X86_64_32",
                                v,
                                &sym.name,
                                &obj.source_name,
                            ));
                        }
                        w32(&mut out, fp, v as u32)
                    }
                    R_X86_64_32S => {
                        let v = s as i64 + a;
                        if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                            return Err(reloc_range_err(
                                "R_X86_64_32S",
                                v,
                                &sym.name,
                                &obj.source_name,
                            ));
                        }
                        w32(&mut out, fp, v as u32)
                    }
                    R_X86_64_16 => w16(&mut out, fp, (s as i64 + a) as u16),
                    R_X86_64_PC16 => w16(&mut out, fp, (s as i64 + a - p as i64) as u16),
                    R_X86_64_8 => {
                        if fp < out.len() {
                            out[fp] = (s as i64 + a) as u8;
                        }
                    }
                    R_X86_64_PC8 => {
                        if fp < out.len() {
                            out[fp] = (s as i64 + a - p as i64) as u8;
                        }
                    }
                    R_X86_64_PC64 => w64(&mut out, fp, (s as i64 + a - p as i64) as u64),
                    // Symbol size, not address: used by some hand-written asm
                    // and by __builtin_object_size lowering.
                    R_X86_64_SIZE32 => w32(&mut out, fp, (sym.size as i64 + a) as u32),
                    R_X86_64_SIZE64 => w64(&mut out, fp, (sym.size as i64 + a) as u64),

                    // ── GOT-relative forms ──
                    //
                    // A linker-script link produces a fully-resolved image with
                    // no dynamic loader and no GOT of its own, so every GOT
                    // reference must be relaxed into direct addressing. This is
                    // exactly what GNU ld does for -no-pie/static links, and it
                    // is what the kernel's vDSO and early-boot objects rely on:
                    // they are compiled -fPIC but linked to fixed addresses.
                    R_X86_64_GOTPCREL | R_X86_64_GOTPCRELX | R_X86_64_REX_GOTPCRELX => {
                        // A -T link produces a fully-resolved image with no
                        // dynamic loader and no GOT, so every GOT reference
                        // must be relaxed into direct addressing. The x86-64
                        // psABI defines exactly which instruction forms may be
                        // rewritten; anything else is a hard error, because
                        // pointing a LOAD at the symbol would read the bytes
                        // stored there and treat them as a pointer.
                        match relax_gotpcrel(&mut out, fp, s, a, p) {
                            Ok(()) => {}
                            Err(op) => {
                                return Err(format!(
                                    "script link: GOTPCREL against '{}' in {} uses an \
                                     instruction form this linker cannot relax \
                                     (opcode bytes {}); a -T link has no GOT, and \
                                     pointing the load at the symbol would read its \
                                     bytes instead of its address",
                                    sym.name, obj.source_name, op
                                ));
                            }
                        }
                    }
                    // GOTPC32: distance from the reference to the GOT origin.
                    // With no GOT, GNU ld still defines _GLOBAL_OFFSET_TABLE_;
                    // treat its address as the value so `lea _GLOBAL_OFFSET_TABLE_(%rip)`
                    // sequences stay self-consistent.
                    R_X86_64_GOTPC32 => {
                        let got_base = symbols.get("_GLOBAL_OFFSET_TABLE_").copied().unwrap_or(p);
                        w32(&mut out, fp, (got_base as i64 + a - p as i64) as u32)
                    }
                    R_X86_64_GOTOFF64 => {
                        let got_base = symbols.get("_GLOBAL_OFFSET_TABLE_").copied().unwrap_or(0);
                        w64(&mut out, fp, (s as i64 + a - got_base as i64) as u64)
                    }
                    // ── Thread-local storage ──
                    //
                    // A -T link is always static with a fixed TLS block, so
                    // the Initial-Exec / Local-Exec forms resolve directly and
                    // no GOT slot or __tls_get_addr call is needed.
                    // ── General-Dynamic / Local-Dynamic TLS ──
                    //
                    // A -T link is static with a fixed TLS block, so there is
                    // no dynamic module and __tls_get_addr can never be
                    // reached. GNU ld therefore RELAXES these sequences to
                    // Local-Exec rather than resolving them. Rejecting them,
                    // which is what happened before, made any -fPIC C++ or
                    // pthread-using object unlinkable through a script.
                    R_X86_64_TLSGD => {
                        if tls_mem_size == 0 {
                            return Err(format!(
                                "script link: TLS relocation against '{}' in {} but the \
                                 script places no SHF_TLS section (add .tdata/.tbss)",
                                sym.name, obj.source_name
                            ));
                        }
                        // Canonical GD sequence, 16 bytes spanning [fp-4, fp+12):
                        //   66 48 8d 3d <disp32>   data16 lea sym@tlsgd(%rip),%rdi
                        //   66 66 48 e8 <disp32>   data16 data16 rex.W call __tls_get_addr
                        let st = fp.checked_sub(4).filter(|&st| {
                            st + 16 <= out.len()
                                && out[st] == 0x66
                                && out[st + 1] == 0x48
                                && out[st + 2] == 0x8d
                                && out[st + 3] == 0x3d
                                && out[fp + 4] == 0x66
                                && out[fp + 5] == 0x66
                                && out[fp + 6] == 0x48
                                && out[fp + 7] == 0xe8
                        });
                        let Some(st) = st else {
                            return Err(format!(
                                "script link: TLSGD relaxation failed for '{}' in {}: \
                                 unrecognised code sequence",
                                sym.name, obj.source_name
                            ));
                        };
                        // GD -> LE:
                        //   64 48 8b 04 25 00000000   mov %fs:0,%rax
                        //   48 8d 80 <tpoff32>        lea tpoff(%rax),%rax
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        if !(i32::MIN as i64..=i32::MAX as i64).contains(&tpoff) {
                            return Err(reloc_range_err(
                                "TLSGD->LE offset",
                                tpoff,
                                &sym.name,
                                &obj.source_name,
                            ));
                        }
                        out[st..st + 9]
                            .copy_from_slice(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
                        out[st + 9] = 0x48;
                        out[st + 10] = 0x8d;
                        out[st + 11] = 0x80;
                        w32(&mut out, st + 12, tpoff as u32);
                        tls_consumed.push((fp + 4, fp + 12));
                    }
                    R_X86_64_TLSLD => {
                        // LD sequence, 12 bytes spanning [fp-3, fp+9):
                        //   48 8d 3d <disp32>   lea sym@tlsld(%rip),%rdi
                        //   e8 <disp32>         call __tls_get_addr
                        // becomes a 12-byte `mov %fs:0,%rax`, after which the
                        // accompanying DTPOFF32 values are TP-relative.
                        let st = fp.checked_sub(3).filter(|&st| {
                            st + 12 <= out.len()
                                && out[st] == 0x48
                                && out[st + 1] == 0x8d
                                && out[st + 2] == 0x3d
                                && out[fp + 4] == 0xe8
                        });
                        let Some(st) = st else {
                            return Err(format!(
                                "script link: TLSLD relaxation failed in {}: \
                                 unrecognised code sequence",
                                obj.source_name
                            ));
                        };
                        out[st..st + 12].copy_from_slice(&[
                            0x66, 0x66, 0x66, 0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0,
                        ]);
                        tls_consumed.push((fp + 4, fp + 9));
                    }
                    // TLSDESC: `lea sym@tlsdesc(%rip),%rax` -> `mov $tpoff,%rax`,
                    // and the indirect call through the descriptor becomes a nop.
                    R_X86_64_GOTPC32_TLSDESC => {
                        if fp >= 3
                            && out[fp - 3] == 0x48
                            && out[fp - 2] == 0x8d
                            && out[fp - 1] == 0x05
                        {
                            let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                            out[fp - 2] = 0xc7;
                            out[fp - 1] = 0xc0;
                            w32(&mut out, fp, tpoff as u32);
                        } else {
                            return Err(format!(
                                "script link: TLSDESC relaxation failed for '{}' in {}: \
                                 unrecognised code sequence",
                                sym.name, obj.source_name
                            ));
                        }
                    }
                    R_X86_64_TLSDESC_CALL => {
                        // call *(%rax) [ff 10] -> xchg %ax,%ax [66 90]
                        if fp + 2 <= out.len() && out[fp] == 0xff && out[fp + 1] == 0x10 {
                            out[fp] = 0x66;
                            out[fp + 1] = 0x90;
                        }
                    }
                    R_X86_64_GOTTPOFF => {
                        // Initial-Exec through a GOT slot. With no GOT, relax
                        //   mov sym@gottpoff(%rip),%reg   48 8b ..
                        // into
                        //   mov $tpoff,%reg               48 c7 c0|..
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        if fp >= 3 && out[fp - 2] == 0x8b {
                            let modrm = out[fp - 1];
                            let reg = (modrm >> 3) & 7;
                            out[fp - 2] = 0xc7;
                            out[fp - 1] = 0xc0 | reg;
                            w32(&mut out, fp, tpoff as u32);
                        } else {
                            return Err(format!(
                                "script link: GOTTPOFF relaxation failed for '{}' in {}: \
                                 unrecognised code sequence",
                                sym.name, obj.source_name
                            ));
                        }
                    }
                    R_X86_64_TPOFF32 => {
                        if tls_mem_size == 0 {
                            return Err(format!(
                                "script link: TLS relocation against '{}' in {} but the \
                                 script places no SHF_TLS section (add .tdata/.tbss)",
                                sym.name, obj.source_name
                            ));
                        }
                        // %fs:0 is the end of the block: offsets are negative.
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        let v = tpoff + a;
                        if !(i32::MIN as i64..=i32::MAX as i64).contains(&v) {
                            return Err(reloc_range_err(
                                "R_X86_64_TPOFF32",
                                v,
                                &sym.name,
                                &obj.source_name,
                            ));
                        }
                        w32(&mut out, fp, v as u32);
                    }
                    R_X86_64_TPOFF64 => {
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w64(&mut out, fp, (tpoff + a) as u64);
                    }
                    // DTPOFF accompanies a Local-Dynamic sequence. Because that
                    // sequence has just been relaxed to Local-Exec -- %rax now
                    // holds the thread pointer rather than the module base --
                    // the offset must be TP-relative, i.e. the same formula as
                    // TPOFF32. Using the block-relative value instead yields
                    // `cmpl $0xb,0x4(%rax)` where GNU ld emits
                    // `cmpl $0xb,-0x4(%rax)`: every Local-Dynamic access reads
                    // the wrong side of the thread pointer.
                    R_X86_64_DTPOFF32 => {
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w32(&mut out, fp, (tpoff + a) as u32)
                    }
                    R_X86_64_DTPOFF64 => {
                        let tpoff = (s as i64 - tls_addr as i64) - tls_mem_size as i64;
                        w64(&mut out, fp, (tpoff + a) as u64)
                    }
                    R_X86_64_NONE => {}
                    other => {
                        return Err(format!(
                            "script link: unsupported relocation type {} for '{}' in {}",
                            other, sym.name, obj.source_name
                        ));
                    }
                }
            }
        }
    }
    if !undefined.is_empty() {
        undefined.sort();
        return Err(format!("undefined symbols: {}", undefined.join(", ")));
    }

    // ── ELF + program headers ──
    let entry = script
        .entry
        .as_ref()
        .and_then(|e| {
            symbols.get(e).copied().or_else(|| {
                def_syms.get(e).and_then(|&(oi, shndx, value, _, _)| {
                    if shndx == SHN_ABS {
                        Some(value)
                    } else {
                        placed_map.get(&(oi, shndx as usize)).map(|b| b + value)
                    }
                })
            })
        })
        // A shared object without an explicit ENTRY has no process entry
        // point.  Choosing its first allocated section would incorrectly make
        // the ELF hash table look executable (and differs from GNU ld).
        .unwrap_or_else(|| {
            if is_pie {
                0
            } else {
                out_secs
                    .iter()
                    .find(|o| o.is_alloc)
                    .map(|o| o.vaddr)
                    .unwrap_or(0)
            }
        });

    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[5] = ELFDATA2LSB;
    out[6] = 1;
    w16(&mut out, 16, if is_pie { ET_DYN } else { ET_EXEC });
    w32(&mut out, 20, 1);
    match machine {
        ScriptMachine::X86_64 => {
            out[4] = ELFCLASS64;
            w16(&mut out, 18, EM_X86_64);
            w64(&mut out, 24, entry);
            w64(&mut out, 32, machine.ehdr_size());
            w32(&mut out, 48, 0);
            w16(&mut out, 52, machine.ehdr_size() as u16);
            w16(&mut out, 54, machine.phdr_size() as u16);
            w16(&mut out, 56, n_phdrs as u16);
            w16(&mut out, 58, machine.shdr_size() as u16);
        }
        ScriptMachine::I386 => {
            if entry > u32::MAX as u64 {
                return Err(format!(
                    "ELF32 entry point {entry:#x} does not fit in 32 bits"
                ));
            }
            out[4] = crate::backend::elf::ELFCLASS32;
            w16(&mut out, 18, crate::backend::elf::EM_386);
            w32(&mut out, 24, entry as u32);
            w32(&mut out, 28, machine.ehdr_size() as u32);
            w32(&mut out, 36, 0);
            w16(&mut out, 40, machine.ehdr_size() as u16);
            w16(&mut out, 42, machine.phdr_size() as u16);
            w16(&mut out, 44, n_phdrs as u16);
            w16(&mut out, 46, machine.shdr_size() as u16);
        }
    }

    // Program headers: for each declared phdr, span the sections assigned to it.
    let mut ph_off = machine.ehdr_size() as usize;
    for decl in &declared_phdrs {
        let mut min_v = u64::MAX;
        let mut max_v = 0u64;
        let mut min_f = u64::MAX;
        let mut max_f = 0u64;
        let mut max_mem = 0u64;
        let mut min_lma = u64::MAX;
        let mut segment_align = 1u64;
        for (i, os) in out_secs.iter().enumerate() {
            if !os.is_alloc || os.size == 0 {
                continue;
            }
            if !sec_phdrs[i].contains(&decl.name) {
                continue;
            }
            segment_align = segment_align.max(os.align);
            min_v = min_v.min(os.vaddr);
            max_mem = max_mem.max(os.vaddr + os.size);
            min_lma = min_lma.min(os.lma);
            if !os.nobits {
                min_f = min_f.min(os.file_offset);
                max_f = max_f.max(os.file_offset + os.size);
                max_v = max_v.max(os.vaddr + os.size);
            }
        }
        if min_v == u64::MAX {
            // Empty phdr
            write_script_phdr(
                machine,
                &mut out,
                ph_off,
                decl.ptype,
                decl.flags.unwrap_or(0) as u32,
                0,
                0,
                0,
                0,
                0,
                8,
            )?;
            ph_off += machine.phdr_size() as usize;
            continue;
        }
        if min_f == u64::MAX {
            min_f = 0;
            max_f = 0;
            max_v = min_v;
        }
        // FILEHDR/PHDRS make the ELF/program headers part of this segment.
        // Linux's vDSO verifier requires its sole PT_LOAD to begin exactly at
        // file offset and virtual address zero.
        if decl.has_filehdr {
            // Preserve the VMA-to-file-offset delta while extending the
            // segment back over every byte preceding its first section.
            min_v = min_v.saturating_sub(min_f);
            min_lma = min_lma.saturating_sub(min_f);
            min_f = 0;
        } else if decl.has_phdrs {
            let prefix = min_f.saturating_sub(machine.ehdr_size());
            min_v = min_v.saturating_sub(prefix);
            min_lma = min_lma.saturating_sub(prefix);
            min_f = machine.ehdr_size();
        }
        let filesz = max_f.saturating_sub(min_f);
        let memsz = max_mem - min_v;
        let flags = decl.flags.unwrap_or(match decl.ptype {
            PT_LOAD_ => 5,
            _ => 4,
        }) as u32;
        let align = if decl.ptype == PT_LOAD_ {
            page
        } else {
            segment_align
        };
        // p_paddr = LMA
        write_script_phdr(
            machine, &mut out, ph_off, decl.ptype, flags, min_f, min_v, min_lma, filesz, memsz,
            align,
        )?;
        ph_off += machine.phdr_size() as usize;
        let _ = max_v;
    }
    if declared_phdrs.is_empty() {
        // Single LOAD spanning everything.
        //
        // The span is taken over LOAD addresses, not virtual ones. With
        // overlays (or any AT()) several sections share a VMA while occupying
        // distinct LMAs, so a VMA-derived memsz can come out smaller than the
        // file image and produce `filesz > memsz`, which readelf rejects and
        // loaders treat as corrupt.
        let alloc = |o: &&OutSec| o.is_alloc && o.size > 0;
        let min_v = out_secs
            .iter()
            .filter(alloc)
            .map(|o| o.lma)
            .min()
            .unwrap_or(0);
        let max_m = out_secs
            .iter()
            .filter(alloc)
            .map(|o| o.lma + o.size)
            .max()
            .unwrap_or(0);
        let min_f = out_secs
            .iter()
            .filter(|o| alloc(o) && !o.nobits)
            .map(|o| o.file_offset)
            .min()
            .unwrap_or(0);
        let max_f = out_secs
            .iter()
            .filter(|o| alloc(o) && !o.nobits)
            .map(|o| o.file_offset + o.size)
            .max()
            .unwrap_or(0);
        let filesz = max_f.saturating_sub(min_f);
        let memsz = (max_m.saturating_sub(min_v)).max(filesz);
        write_script_phdr(
            machine,
            &mut out,
            machine.ehdr_size() as usize,
            PT_LOAD_,
            7,
            min_f,
            min_v,
            min_v,
            filesz,
            memsz,
            page,
        )?;
        ph_off = (machine.ehdr_size() + machine.phdr_size()) as usize;
    }
    if needs_tls_phdr {
        // p_filesz covers .tdata only (.tbss is NOBITS); p_memsz covers both.
        let tls: Vec<&OutSec> = out_secs
            .iter()
            .filter(|o| (o.flags & SHF_TLS_) != 0 && o.is_alloc)
            .collect();
        let vlo = tls.iter().map(|o| o.vaddr).min().unwrap_or(0);
        let vhi = tls.iter().map(|o| o.vaddr + o.size).max().unwrap_or(0);
        let flo = tls
            .iter()
            .filter(|o| !o.nobits)
            .map(|o| o.file_offset)
            .min()
            .unwrap_or(0);
        let fhi = tls
            .iter()
            .filter(|o| !o.nobits)
            .map(|o| o.file_offset + o.size)
            .max()
            .unwrap_or(flo);
        let talign = tls.iter().map(|o| o.align).max().unwrap_or(1).max(1);
        write_script_phdr(
            machine,
            &mut out,
            ph_off,
            PT_TLS_,
            4, /* PF_R */
            flo,
            vlo,
            vlo,
            fhi - flo,
            vhi - vlo,
            talign,
        )?;
    }

    // ── Section headers (+ optional symtab) ──
    let mut shstrtab: Vec<u8> = vec![0];
    let mut shname = |t: &mut Vec<u8>, n: &str| -> u32 {
        let off = t.len() as u32;
        t.extend_from_slice(n.as_bytes());
        t.push(0);
        off
    };

    // symtab: linker-script symbols + object globals + locals (for kallsyms/System.map)
    let mut symtab: Vec<[u8; 24]> = vec![[0u8; 24]];
    let mut strtab: Vec<u8> = vec![0];
    let n_local_syms;

    // Map vaddr -> output section header index for st_shndx.
    // Header order: NULL + emitted sections (alloc first as laid out) + symtab/strtab/shstrtab
    let mut hdr_secs: Vec<usize> = Vec::new(); // indices into out_secs
    for (i, os) in out_secs.iter().enumerate() {
        // GNU ld drops zero-size output sections from the final image;
        // keeping them (e.g. the kernel's ASSERT-guard .got/.plt/.rela.dyn)
        // confuses objcopy and inflates the header table.
        if os.size > 0 {
            hdr_secs.push(i);
        }
    }
    let find_shndx = |vaddr: u64| -> u16 {
        // Two passes: prefer strict containment, then inclusive end.
        // End-of-section symbols like `_end` (== .brk end) must be attributed
        // to the PRECEDING section, not a follow-on section that happens to
        // start at the same address (the kernel strips .modinfo from the
        // final image; a symbol attributed to it would be stripped too).
        for (h, &i) in hdr_secs.iter().enumerate() {
            let os = &out_secs[i];
            if os.is_alloc && vaddr >= os.vaddr && vaddr < os.vaddr + os.size.max(1) {
                return (h + 1) as u16;
            }
        }
        for (h, &i) in hdr_secs.iter().enumerate() {
            let os = &out_secs[i];
            if os.is_alloc && vaddr >= os.vaddr && vaddr <= os.vaddr + os.size {
                return (h + 1) as u16;
            }
        }
        SHN_ABS
    };

    // Finish linker-created ET_DYN metadata now that every exported symbol and
    // synthetic section has a final address.
    if let Some(plan) = script_dynamic.as_ref() {
        let synthetic_file_offset = |section_index: usize| -> Option<usize> {
            let &vaddr = placed_map.get(&(plan.object_index, section_index))?;
            let &owner = placed_owner.get(&(plan.object_index, section_index))?;
            let os = &out_secs[owner];
            Some((os.file_offset + vaddr - os.vaddr) as usize)
        };

        // .dynsym: null entry followed by the GNU-hash bucket order.
        if let Some(base) = synthetic_file_offset(3) {
            for (i, name) in plan.exports.iter().enumerate() {
                let (value, size, info, shndx) =
                    if let Some(&(oi, shndx, value, size, info)) = def_syms.get(name) {
                        let value = if shndx == SHN_ABS {
                            value
                        } else {
                            placed_map.get(&(oi, shndx as usize)).copied().unwrap_or(0) + value
                        };
                        (value, size, info, find_shndx(value))
                    } else if plan.version_symbols.contains(name) {
                        (0, 0, (STB_GLOBAL << 4) | STT_OBJECT, SHN_ABS)
                    } else {
                        continue;
                    };
                let off = base + (i + 1) * machine.sym_size();
                match machine {
                    ScriptMachine::X86_64 => {
                        w32(&mut out, off, plan.name_offsets[i]);
                        out[off + 4] = info;
                        out[off + 5] = 0;
                        w16(&mut out, off + 6, shndx);
                        w64(&mut out, off + 8, value);
                        w64(&mut out, off + 16, size);
                    }
                    ScriptMachine::I386 => {
                        if value > u32::MAX as u64 || size > u32::MAX as u64 {
                            return Err(format!("ELF32 dynamic symbol '{}' overflows", name));
                        }
                        w32(&mut out, off, plan.name_offsets[i]);
                        w32(&mut out, off + 4, value as u32);
                        w32(&mut out, off + 8, size as u32);
                        out[off + 12] = info;
                        out[off + 13] = 0;
                        w16(&mut out, off + 14, shndx);
                    }
                }
            }
        }

        let section_addr =
            |name: &str| -> u64 { sections_meta.get(name).map(|meta| meta.0).unwrap_or(0) };
        if let Some(mut off) = synthetic_file_offset(7) {
            let mut emit = |tag: i64, value: u64| -> Result<(), String> {
                match machine {
                    ScriptMachine::X86_64 => {
                        w64(&mut out, off, tag as u64);
                        w64(&mut out, off + 8, value);
                        off += 16;
                    }
                    ScriptMachine::I386 => {
                        if value > u32::MAX as u64 {
                            return Err(format!("ELF32 dynamic value for tag {} overflows", tag));
                        }
                        w32(&mut out, off, tag as u32);
                        w32(&mut out, off + 4, value as u32);
                        off += 8;
                    }
                }
                Ok(())
            };
            if let Some(soname_offset) = plan.soname_offset {
                emit(DT_SONAME, soname_offset as u64)?;
            }
            if bsymbolic {
                emit(16, 0)?; // DT_SYMBOLIC
                emit(DT_FLAGS, 0x2)?; // DF_SYMBOLIC
            }
            emit(DT_HASH, section_addr(".hash"))?;
            emit(DT_GNU_HASH, section_addr(".gnu.hash"))?;
            emit(DT_STRTAB, section_addr(".dynstr"))?;
            emit(DT_SYMTAB, section_addr(".dynsym"))?;
            emit(
                DT_STRSZ,
                out_secs
                    .iter()
                    .find(|s| s.name == ".dynstr")
                    .map(|s| s.size)
                    .unwrap_or(0),
            )?;
            emit(DT_SYMENT, machine.sym_size() as u64)?;
            emit(DT_VERSYM, section_addr(".gnu.version"))?;
            emit(DT_VERDEF, section_addr(".gnu.version_d"))?;
            emit(DT_VERDEFNUM, plan.verdef_count)?;
            emit(DT_NULL, 0)?;
        }
    }

    // --emit-relocs bookkeeping: input symbol -> output .symtab index.
    let mut local_sym_index: FxHashMap<(usize, usize), u32> = FxHashMap::default();
    let mut global_sym_index: FxHashMap<String, u32> = FxHashMap::default();
    // out_secs index -> index of that section's STT_SECTION symbol in .symtab
    let mut out_sec_sym_index: FxHashMap<usize, u32> = FxHashMap::default();

    // STV_* visibility lives in st_other in both ELF classes.
    fn add_sym_vis(
        machine: ScriptMachine,
        name: &str,
        value: u64,
        size: u64,
        info: u8,
        other: u8,
        shndx: u16,
        symtab: &mut Vec<[u8; 24]>,
        strtab: &mut Vec<u8>,
    ) -> Result<(), String> {
        let noff = push_strtab_name(strtab, name.as_bytes());
        symtab.push(script_sym_entry(
            machine, noff, info, other, shndx, value, size,
        )?);
        Ok(())
    }

    if emit_symtab {
        let add_sym = |name: &str,
                       value: u64,
                       size: u64,
                       info: u8,
                       shndx: u16,
                       symtab: &mut Vec<[u8; 24]>,
                       strtab: &mut Vec<u8>| {
            add_sym_vis(machine, name, value, size, info, 0, shndx, symtab, strtab)
        };

        // --emit-relocs: one STT_SECTION symbol per output section, first in
        // the local range. GNU ld emits relocations against section symbols
        // (`.text + 0x9`) rather than folding the address into the addend,
        // and the kernel's relocs pass and objtool both expect that form.
        // Without these the records still describe the right bytes, but they
        // lose the "which section" information a consumer needs.
        if emit_relocs {
            for (h, &i) in hdr_secs.iter().enumerate() {
                out_sec_sym_index.insert(i, symtab.len() as u32);
                add_sym(
                    "",
                    out_secs[i].vaddr,
                    0,
                    // STB_LOCAL = 0, so the binding nibble is zero.
                    STT_SECTION,
                    (h + 1) as u16,
                    &mut symtab,
                    &mut strtab,
                )?;
            }
        }

        // Locals first (ELF requirement).
        for (oi, obj) in objects.iter().enumerate() {
            for (sidx, sym) in obj.symbols.iter().enumerate() {
                if !sym.is_local() || sym.name.is_empty() {
                    continue;
                }
                if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE {
                    continue;
                }
                if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON {
                    continue;
                }
                let Some(&base) = placed_map.get(&(oi, sym.shndx as usize)) else {
                    continue;
                };
                let v = base + sym.value;
                // --emit-relocs: remember where this input symbol landed in the
                // output .symtab so retained relocations can point at it.
                local_sym_index.insert((oi, sidx), symtab.len() as u32);
                add_sym(
                    &sym.name,
                    v,
                    sym.size,
                    sym.info,
                    find_shndx(v),
                    &mut symtab,
                    &mut strtab,
                )?;
            }
        }
        n_local_syms = symtab.len();
        // Object globals
        let mut names: Vec<&String> = def_syms.keys().collect();
        names.sort();
        for name in names {
            let &(oi, shndx, value, size, info) = def_syms.get(name).unwrap();
            let v = if shndx == SHN_ABS {
                value
            } else {
                match placed_map.get(&(oi, shndx as usize)) {
                    Some(&b) => b + value,
                    None => continue,
                }
            };
            let sx = if shndx == SHN_ABS {
                SHN_ABS
            } else {
                find_shndx(v)
            };
            global_sym_index.insert(name.clone(), symtab.len() as u32);
            add_sym(name, v, size, info, sx, &mut symtab, &mut strtab)?;
        }
        // Script-defined symbols
        let mut snames: Vec<&String> = symbols.keys().collect();
        snames.sort();
        // hdr index by section name for home attribution
        let hdr_by_name: FxHashMap<&str, u16> = hdr_secs
            .iter()
            .enumerate()
            .map(|(h, &i)| (out_secs[i].name.as_str(), (h + 1) as u16))
            .collect();
        for name in snames {
            if def_syms.contains_key(name) {
                continue;
            }
            let v = symbols[name];
            let sx = if absolute_script_syms.contains(name.as_str()) {
                SHN_ABS
            } else {
                sym_home
                    .get(name)
                    .and_then(|h| hdr_by_name.get(h.as_str()).copied())
                    .unwrap_or_else(|| find_shndx(v))
            };
            // STV_HIDDEN = 2. A hidden symbol keeps GLOBAL binding in .symtab
            // (so debuggers still see it) but is excluded from .dynsym.
            let vis = if hidden_syms.contains(name.as_str()) {
                2u8
            } else {
                0u8
            };
            add_sym_vis(
                machine,
                name,
                v,
                0,
                (STB_GLOBAL << 4) | STT_NOTYPE_,
                vis,
                sx,
                &mut symtab,
                &mut strtab,
            )?;
        }
    } else {
        n_local_syms = 1;
    }

    // ── --emit-relocs: retain relocations as .rela.<section> ───────────────
    //
    // The Linux kernel's arch/x86/tools/relocs pass reads these from a fully
    // linked vmlinux to build the table that lets the boot code relocate the
    // image at runtime (CONFIG_RELOCATABLE / CONFIG_RANDOMIZE_BASE, i.e.
    // KASLR). Without them the kernel links "successfully" and then fails to
    // boot, which is why accepting-and-ignoring the flag was worse than
    // rejecting it.
    //
    // Semantics (matching GNU ld):
    //   * relocations are still APPLIED to the section contents above; this
    //     only preserves a record of them;
    //   * r_offset is rebased from section-relative to the final VIRTUAL
    //     ADDRESS (bfd emits absolute addresses here for a linked image);
    //   * r_info's symbol index is remapped from the input object's symtab to
    //     the merged output .symtab;
    //   * the addend is carried through unchanged;
    //   * one .rela.NAME section per output section that has any, with
    //     sh_info = target section header index and sh_link = .symtab index.
    //
    // Relocations against symbols that did not make it into the output symtab
    // are emitted with symbol index 0 (STN_UNDEF) and the resolved value
    // folded into the addend, so the record stays self-consistent rather than
    // dangling — the same trick bfd uses for section-relative references.
    struct RetainedRelocs {
        /// index into out_secs of the section these apply to
        target: usize,
        /// raw Elf64_Rela entries, already in output form
        entries: Vec<[u8; 24]>,
    }
    let mut retained: Vec<RetainedRelocs> = Vec::new();
    if emit_relocs {
        // Group by target output section, preserving output-section order so
        // the .rela.* headers appear in a deterministic sequence.
        let mut by_target: FxHashMap<usize, Vec<[u8; 24]>> = FxHashMap::default();
        for (oi, obj) in objects.iter().enumerate() {
            for (si, relas) in obj.relocations.iter().enumerate() {
                if relas.is_empty() {
                    continue;
                }
                let Some(&sec_vaddr) = placed_map.get(&(oi, si)) else {
                    continue;
                };
                let Some(&owner) = placed_owner.get(&(oi, si)) else {
                    continue;
                };
                if !out_secs[owner].is_alloc {
                    continue;
                }
                for rela in relas {
                    if rela.rela_type == R_X86_64_NONE {
                        continue;
                    }
                    let sidx = rela.sym_idx as usize;
                    let Some(sym) = obj.symbols.get(sidx) else {
                        continue;
                    };

                    // Map the input symbol to its output .symtab index.
                    let (out_sym, extra_addend) = if sym.is_local() {
                        match local_sym_index.get(&(oi, sidx)) {
                            Some(&ix) => (ix, 0i64),
                            None => {
                                // A reference through a section symbol (the
                                // common case for .eh_frame and switch tables).
                                // Point at the OUTPUT section's symbol and
                                // rebase the addend by how far this input
                                // section sits inside it — exactly what GNU ld
                                // emits, e.g. `.text + 0x9`.
                                let secsym = out_sec_sym_index.get(&owner).copied();
                                match secsym {
                                    Some(ix) => {
                                        let delta =
                                            sec_vaddr.wrapping_sub(out_secs[owner].vaddr) as i64;
                                        (ix, delta)
                                    }
                                    // No section symbol available (symtab
                                    // suppressed): keep the record
                                    // self-consistent by folding the resolved
                                    // address into the addend.
                                    None => (0u32, resolve(oi, sym).unwrap_or(0) as i64),
                                }
                            }
                        }
                    } else {
                        match global_sym_index.get(sym.name.as_str()) {
                            Some(&ix) => (ix, 0i64),
                            None => (0u32, resolve(oi, sym).unwrap_or(0) as i64),
                        }
                    };

                    let r_offset = sec_vaddr + rela.offset;
                    let mut e = [0u8; 24];
                    match machine {
                        ScriptMachine::X86_64 => {
                            let r_info = ((out_sym as u64) << 32) | (rela.rela_type as u64);
                            let r_addend = rela.addend + extra_addend;
                            e[0..8].copy_from_slice(&r_offset.to_le_bytes());
                            e[8..16].copy_from_slice(&r_info.to_le_bytes());
                            e[16..24].copy_from_slice(&r_addend.to_le_bytes());
                        }
                        ScriptMachine::I386 => {
                            if r_offset > u32::MAX as u64
                                || out_sym > 0x00ff_ffff
                                || rela.rela_type > 0xff
                            {
                                return Err("ELF32 retained relocation fields overflow".into());
                            }
                            let r_info = (out_sym << 8) | rela.rela_type;
                            e[0..4].copy_from_slice(&(r_offset as u32).to_le_bytes());
                            e[4..8].copy_from_slice(&r_info.to_le_bytes());
                            let _ = extra_addend; // Elf32_Rel carries its addend in section data.
                        }
                    }
                    by_target.entry(owner).or_default().push(e);
                }
            }
        }
        // Deterministic order: follow the output-section header order, and
        // sort each group by r_offset (bfd emits ascending offsets, and the
        // kernel's relocs tool is easier to diff against when we match).
        for (h, &i) in hdr_secs.iter().enumerate() {
            let _ = h;
            if let Some(mut entries) = by_target.remove(&i) {
                entries.sort_by_key(|e| match machine {
                    ScriptMachine::X86_64 => u64::from_le_bytes(e[0..8].try_into().unwrap()),
                    ScriptMachine::I386 => u32::from_le_bytes(e[0..4].try_into().unwrap()) as u64,
                });
                retained.push(RetainedRelocs { target: i, entries });
            }
        }
    }

    // Append symtab/strtab/shstrtab data at the class-specific natural width.
    let table_align = if machine == ScriptMachine::X86_64 {
        8
    } else {
        4
    };
    while out.len() % table_align != 0 {
        out.push(0);
    }
    let symtab_off = out.len() as u64;
    for e in &symtab {
        out.extend_from_slice(&e[..machine.sym_size()]);
    }
    let strtab_off = out.len() as u64;
    out.extend_from_slice(&strtab);

    let mut rela_offsets: Vec<u64> = Vec::with_capacity(retained.len());
    for rr in &retained {
        while out.len() % table_align != 0 {
            out.push(0);
        }
        rela_offsets.push(out.len() as u64);
        for e in &rr.entries {
            out.extend_from_slice(&e[..machine.rel_size()]);
        }
    }

    // section header name offsets
    let mut name_offs: Vec<u32> = Vec::new();
    for &i in &hdr_secs {
        let n = out_secs[i].name.clone();
        name_offs.push(shname(&mut shstrtab, &n));
    }
    // ELF64 retains RELA; i386 retains the architecture's native REL form.
    let rela_names: Vec<u32> = retained
        .iter()
        .map(|rr| {
            let prefix = if machine == ScriptMachine::X86_64 {
                ".rela"
            } else {
                ".rel"
            };
            let n = format!("{}{}", prefix, out_secs[rr.target].name);
            shname(&mut shstrtab, &n)
        })
        .collect();
    let symtab_name = shname(&mut shstrtab, ".symtab");
    let strtab_name = shname(&mut shstrtab, ".strtab");
    let shstrtab_name = shname(&mut shstrtab, ".shstrtab");

    while out.len() % table_align != 0 {
        out.push(0);
    }
    let shstrtab_off = out.len() as u64;
    out.extend_from_slice(&shstrtab);

    while out.len() % table_align != 0 {
        out.push(0);
    }
    let shoff = out.len() as u64;

    // NULL
    write_script_shdr(machine, &mut out, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)?;
    let dynamic_header_index: FxHashMap<&str, u32> = hdr_secs
        .iter()
        .enumerate()
        .map(|(h, &i)| (out_secs[i].name.as_str(), (h + 1) as u32))
        .collect();
    for (h, &i) in hdr_secs.iter().enumerate() {
        let os = &out_secs[i];
        let link = match os.name.as_str() {
            ".hash" | ".gnu.hash" | ".gnu.version" => {
                dynamic_header_index.get(".dynsym").copied().unwrap_or(0)
            }
            ".dynsym" | ".dynamic" | ".gnu.version_d" => {
                dynamic_header_index.get(".dynstr").copied().unwrap_or(0)
            }
            _ => 0,
        };
        let info = if os.name == ".gnu.version_d" {
            script_dynamic
                .as_ref()
                .map(|p| p.verdef_count as u32)
                .unwrap_or(0)
        } else if os.name == ".dynsym" {
            1
        } else {
            0
        };
        let entsize = match os.name.as_str() {
            ".hash" => 4,
            ".dynsym" => machine.sym_size() as u64,
            ".gnu.version" => 2,
            ".dynamic" => {
                if machine == ScriptMachine::X86_64 {
                    16
                } else {
                    8
                }
            }
            _ => 0,
        };
        write_script_shdr(
            machine,
            &mut out,
            name_offs[h],
            os.sh_type,
            os.flags,
            os.vaddr,
            os.file_offset,
            os.size,
            link,
            info,
            os.align.max(1),
            entsize,
        )?;
    }
    let n_hdrs = 1 + hdr_secs.len();
    // Header index layout from here on:
    //   [0]              NULL
    //   [1 .. n_hdrs)    output sections (hdr_secs order)
    //   [n_hdrs ..]      .rela.* (one per retained group), if --emit-relocs
    //   then             .symtab, .strtab, .shstrtab
    let symtab_idx = n_hdrs + retained.len();
    let strtab_idx = symtab_idx + 1;

    // Map an out_secs index to its section-header index.
    let hdr_index_of = |target: usize| -> u32 {
        hdr_secs
            .iter()
            .position(|&i| i == target)
            .map(|p| p as u32 + 1)
            .unwrap_or(0)
    };
    for (k, rr) in retained.iter().enumerate() {
        // sh_link = .symtab, sh_info = section these relocations apply to.
        let rel_type = if machine == ScriptMachine::X86_64 {
            SHT_RELA_
        } else {
            SHT_REL_
        };
        write_script_shdr(
            machine,
            &mut out,
            rela_names[k],
            rel_type,
            0,
            0,
            rela_offsets[k],
            (rr.entries.len() * machine.rel_size()) as u64,
            symtab_idx as u32,
            hdr_index_of(rr.target),
            table_align as u64,
            machine.rel_size() as u64,
        )?;
    }

    write_script_shdr(
        machine,
        &mut out,
        symtab_name,
        SHT_SYMTAB_,
        0,
        0,
        symtab_off,
        (symtab.len() * machine.sym_size()) as u64,
        strtab_idx as u32,
        n_local_syms as u32,
        table_align as u64,
        machine.sym_size() as u64,
    )?;
    write_script_shdr(
        machine,
        &mut out,
        strtab_name,
        SHT_STRTAB_,
        0,
        0,
        strtab_off,
        strtab.len() as u64,
        0,
        0,
        1,
        0,
    )?;
    write_script_shdr(
        machine,
        &mut out,
        shstrtab_name,
        SHT_STRTAB_,
        0,
        0,
        shstrtab_off,
        shstrtab.len() as u64,
        0,
        0,
        1,
        0,
    )?;

    let sh_count = (n_hdrs + retained.len() + 3) as u16;
    match machine {
        ScriptMachine::X86_64 => {
            w64(&mut out, 40, shoff);
            w16(&mut out, 60, sh_count);
            w16(&mut out, 62, sh_count - 1);
        }
        ScriptMachine::I386 => {
            if shoff > u32::MAX as u64 {
                return Err("ELF32 section-header offset does not fit in 32 bits".into());
            }
            w32(&mut out, 32, shoff as u32);
            w16(&mut out, 48, sh_count);
            w16(&mut out, 50, sh_count - 1);
        }
    }

    // --build-id=sha1: the synthetic "<build-id>" object carries a
    // .note.gnu.build-id section that the script's *(.note.*) pattern
    // placed like any input. Fill the header now and hash the COMPLETE
    // image with a zeroed descriptor, then patch the digest in.
    {
        let bid = objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.source_name == "<build-id>")
            .flat_map(|(oi, o)| {
                o.sections
                    .iter()
                    .enumerate()
                    .filter(|(_, sec)| sec.name == ".note.gnu.build-id")
                    .map(move |(si, _)| (oi, si))
            })
            .next();
        if let Some((oi, si)) = bid {
            if let (Some(&vaddr), Some(&owner)) =
                (placed_map.get(&(oi, si)), placed_owner.get(&(oi, si)))
            {
                let os = &out_secs[owner];
                if !os.nobits && os.is_alloc {
                    let foff = (os.file_offset + (vaddr - os.vaddr)) as usize;
                    if foff + linker_common::build_id::BUILD_ID_NOTE_SIZE as usize <= out.len() {
                        linker_common::build_id::write_build_id_skeleton(&mut out, foff);
                        linker_common::build_id::patch_build_id(&mut out, foff);
                    }
                }
            }
        }
    }

    std::fs::write(output_path, &out)
        .map_err(|e| format!("failed to write '{}': {}", output_path, e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(output_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_script_phdr(
    machine: ScriptMachine,
    out: &mut [u8],
    off: usize,
    ptype: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
) -> Result<(), String> {
    match machine {
        ScriptMachine::X86_64 => {
            w32(out, off, ptype);
            w32(out, off + 4, flags);
            w64(out, off + 8, offset);
            w64(out, off + 16, vaddr);
            w64(out, off + 24, paddr);
            w64(out, off + 32, filesz);
            w64(out, off + 40, memsz);
            w64(out, off + 48, align);
        }
        ScriptMachine::I386 => {
            let values = [offset, vaddr, paddr, filesz, memsz, align];
            if values.iter().any(|&value| value > u32::MAX as u64) {
                return Err(format!(
                    "ELF32 program header field does not fit in 32 bits: {values:x?}"
                ));
            }
            w32(out, off, ptype);
            w32(out, off + 4, offset as u32);
            w32(out, off + 8, vaddr as u32);
            w32(out, off + 12, paddr as u32);
            w32(out, off + 16, filesz as u32);
            w32(out, off + 20, memsz as u32);
            w32(out, off + 24, flags);
            w32(out, off + 28, align as u32);
        }
    }
    Ok(())
}

fn script_sym_entry(
    machine: ScriptMachine,
    name: u32,
    info: u8,
    other: u8,
    shndx: u16,
    value: u64,
    size: u64,
) -> Result<[u8; 24], String> {
    if machine == ScriptMachine::X86_64 {
        return Ok(elf64_sym_entry(name, info, other, shndx, value, size));
    }
    // ELF32 linker-script arithmetic is modulo 2^32. Absolute expressions
    // such as `. - 6 * PAGE_SIZE` intentionally produce 0xffffa000 while the
    // layout counter is still zero; in the u64 evaluator that is represented
    // as 0xffff_ffff_ffff_a000 and must wrap, not be rejected.
    if size > u32::MAX as u64 {
        return Err(format!("ELF32 symbol size does not fit: {size:#x}"));
    }
    let mut entry = [0u8; 24];
    entry[0..4].copy_from_slice(&name.to_le_bytes());
    entry[4..8].copy_from_slice(&(value as u32).to_le_bytes());
    entry[8..12].copy_from_slice(&(size as u32).to_le_bytes());
    entry[12] = info;
    entry[13] = other;
    entry[14..16].copy_from_slice(&shndx.to_le_bytes());
    Ok(entry)
}

#[allow(clippy::too_many_arguments)]
fn write_script_shdr(
    machine: ScriptMachine,
    out: &mut Vec<u8>,
    name: u32,
    sh_type: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    addralign: u64,
    entsize: u64,
) -> Result<(), String> {
    if machine == ScriptMachine::X86_64 {
        linker_common::write_elf64_shdr(
            out, name, sh_type, flags, addr, offset, size, link, info, addralign, entsize,
        );
        return Ok(());
    }
    let values = [flags, addr, offset, size, addralign, entsize];
    if values.iter().any(|&value| value > u32::MAX as u64) {
        return Err(format!(
            "ELF32 section header field does not fit in 32 bits: {values:x?}"
        ));
    }
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&sh_type.to_le_bytes());
    out.extend_from_slice(&(flags as u32).to_le_bytes());
    out.extend_from_slice(&(addr as u32).to_le_bytes());
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&link.to_le_bytes());
    out.extend_from_slice(&info.to_le_bytes());
    out.extend_from_slice(&(addralign as u32).to_le_bytes());
    out.extend_from_slice(&(entsize as u32).to_le_bytes());
    Ok(())
}

// Local aliases for constants used with underscore suffix to avoid clashes
const SHT_NULL_: u32 = 0;
const SHT_PROGBITS_: u32 = 1;
const SHT_SYMTAB_: u32 = 2;
const SHT_STRTAB_: u32 = 3;
const SHT_RELA_: u32 = 4;
const SHT_NOBITS_: u32 = 8;
const SHT_REL_: u32 = 9;
const SHT_GROUP_: u32 = 17;
const SHF_WRITE_: u64 = 0x1;
const SHF_ALLOC_: u64 = 0x2;
const SHF_EXECINSTR_: u64 = 0x4;
const SHF_MERGE_: u64 = 0x10;
const SHF_GROUP_: u64 = 0x200;
const SHF_TLS_: u64 = 0x400;
const STT_NOTYPE_: u8 = 0;
const STT_FILE: u8 = 4;

// i386 psABI relocation numbers used by ELF32 script links. Elf32_Rel carries
// the addend in the relocated field; the i686 parser normalises it to i64 for
// the shared layout engine before these formulas are applied.
const R_386_NONE: u32 = 0;
const R_386_32: u32 = 1;
const R_386_PC32: u32 = 2;
const R_386_PLT32: u32 = 4;
const R_386_GOTOFF: u32 = 9;
const R_386_GOTPC: u32 = 10;
const R_386_16: u32 = 20;
const R_386_PC16: u32 = 21;
const R_386_8: u32 = 22;
const R_386_PC8: u32 = 23;
const R_386_SIZE32: u32 = 38;

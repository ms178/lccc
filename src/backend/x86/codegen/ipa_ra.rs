//! Interprocedural register allocation (IPA-RA) for x86-64.
//!
//! The SysV ABI lets every callee destroy `%rax %rcx %rdx %rsi %rdi %r8-%r11`,
//! so a value live across a call needs a callee-saved home (a `push`/`pop`
//! pair) or a stack slot. When the callee is code this translation unit
//! emitted, binds locally, and provably never writes some of those
//! registers, the caller may keep values in them across the call for free.
//! GCC does this by default at `-O2` (`-fipa-ra`); `hot_web_loop` keeps its
//! four accumulators in `%rcx/%rsi/%r8/%r9` across `helper()` there, while
//! LCCC saturated `%rbx/%r12-%r15/%rbp` and spilled the cold values.
//!
//! # Contract
//!
//! 1. **Summaries.** After a function's final emission its text is scanned
//!    ([`text_clobbers`]): every general-purpose register the text mentions
//!    counts as written (reads are over-approximated as writes: a callee
//!    reads without writing only its argument registers, which the call
//!    site's staging writes anyway), plus the exact implicit writes of the
//!    instructions that have them (string ops, `div`, `cqto`, `cpuid`, ...).
//!    A call or tail jump to a summarized function contributes its summary.
//!    Anything the scanner cannot classify exactly -- an unlisted mnemonic,
//!    an indirect call or jump, a call to an unsummarized symbol, bytes
//!    emitted into a text section, section stacking -- yields no summary.
//!    Only functions that bind locally and contain no inline assembly are
//!    summarized ([`summarizable`]); recursion yields no summary because a
//!    function's own summary does not exist while it is scanned.
//! 2. **Keep set.** For a caller whose every call point is a direct call of
//!    the restricted shape ([`call_staging`]: register-only scalar arguments,
//!    no stack arguments, not variadic, no sret, no static chain) to a
//!    summarized callee, the keep set is
//!    `pool ∩ {%rsi,%rdi,%r8,%r9} \ ⋃(staged argument registers ∪ callee summary)`
//!    ([`IpaRaState::keep_set`]). Those registers survive every call the
//!    function makes, so the allocator treats them as callee-saved registers
//!    that cost no save/restore. The candidates are exactly the argument
//!    registers outside the fixed-scratch set (`%rax/%rcx/%rdx`) and the
//!    staging-scratch set (`%r10` indirect target and static chain, `%r11`
//!    stack-argument and realignment scratch): every hand-written call model
//!    in the late peephole already treats them as READ at every call, which
//!    is the conservative reading of "preserved".
//! 3. **Markers.** Each call of such a function carries
//!    `# LCCC_IPA_KEEP <mask>`; the peephole oracles model the call as
//!    reading (and, via the ABI clobber set, writing) the kept registers
//!    (`peephole::passes::liveness::call_markers`).
//! 4. **Verification.** The summaries are predicted from the pre-peephole
//!    text, so after the whole-module peephole and TLS lowering
//!    [`verify`] recomputes every summary from the FINAL text and re-checks
//!    every caller's calls. A callee whose final clobbers exceed its
//!    prediction, or a caller with a call the record does not cover, is
//!    reported; the driver regenerates the module with those functions
//!    excluded, and finally with IPA-RA off, so a violated assumption can
//!    never reach the output.
//!
//! Kill switch: `CCC_DISABLE_PASSES=ipa_ra`; `CCC_DEBUG_RA` traces the
//! verify rounds. Disabled under `-pg` / `-finstrument-
//! functions` / patchable function entries (runtime-patched code, as GCC).

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::{CallInfo, Instruction, IrFunction};

// Canonical GP family bits (the peephole numbering, `REG_NAMES`).
const RAX: u16 = 1 << 0;
const RCX: u16 = 1 << 1;
const RDX: u16 = 1 << 2;
const RBX: u16 = 1 << 3;
const RSP: u16 = 1 << 4;
const RBP: u16 = 1 << 5;
const RSI: u16 = 1 << 6;
const RDI: u16 = 1 << 7;
const R8: u16 = 1 << 8;
const R9: u16 = 1 << 9;
const R10: u16 = 1 << 10;
const R11: u16 = 1 << 11;
/// SysV AMD64 caller-saved general-purpose families.
pub(crate) const SYSV_CALLER_SAVED: u16 = RAX | RCX | RDX | RSI | RDI | R8 | R9 | R10 | R11;
/// Families a caller may keep live across a call (see the module contract).
const KEEP_CANDIDATES: u16 = RSI | RDI | R8 | R9;
/// SysV GP argument registers in order.
const ARG_ORDER: [u16; 6] = [RDI, RSI, RDX, RCX, R8, R9];
/// The keep candidates with their allocator `PhysReg` ids (x86-64 numbering:
/// 12=r8, 13=r9, 14=rdi, 15=rsi).
pub(crate) const KEEP_PHYS: [(u16, u8); 4] = [(R8, 12), (R9, 13), (RDI, 14), (RSI, 15)];

/// Allocator pool (`PhysReg` ids) to family bits, keep candidates only.
pub(crate) fn pool_families(pool: &[crate::backend::regalloc::PhysReg]) -> u16 {
    KEEP_PHYS
        .iter()
        .filter(|(_, id)| pool.iter().any(|r| r.0 == *id))
        .fold(0, |m, (f, _)| m | f)
}

/// Module-scoped IPA-RA state, owned by the x86-64 code generator.
#[derive(Default)]
pub(crate) struct IpaRaState {
    enabled: bool,
    /// Emitted symbol -> predicted caller-saved clobber families.
    summaries: FxHashMap<String, u16>,
    /// Summarized symbols in emission order (callees precede their callers).
    order: Vec<String>,
    /// Functions whose summaries a previous verification rejected.
    poisoned_callees: FxHashSet<String>,
    /// Functions whose keep sets a previous verification rejected.
    poisoned_callers: FxHashSet<String>,
    /// Caller -> (keep families, callee symbols its calls name).
    records: FxHashMap<String, (u16, Vec<String>)>,
}

impl IpaRaState {
    pub(crate) fn new(
        enabled: bool,
        poisoned_callees: FxHashSet<String>,
        poisoned_callers: FxHashSet<String>,
    ) -> Self {
        Self {
            enabled,
            poisoned_callees,
            poisoned_callers,
            ..Self::default()
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// Record the summary of a function whose final emission is `text`.
    pub(crate) fn note_function_emitted(
        &mut self,
        func: &IrFunction,
        binds_locally: bool,
        text: &str,
    ) {
        if !self.enabled
            || !summarizable(func, binds_locally)
            || self.poisoned_callees.contains(&func.name)
        {
            return;
        }
        let summaries = &self.summaries;
        if let Some(mask) = text_clobbers(text, &|s| summaries.get(s).copied()) {
            if self.summaries.insert(func.name.clone(), mask).is_none() {
                self.order.push(func.name.clone());
            }
        }
    }

    /// The keep set of `func` (family bits) given the keep candidates its
    /// caller-saved pool holds, and the callees its calls name; `(0, [])`
    /// when any call point disqualifies the function.
    pub(crate) fn keep_set(&self, func: &IrFunction, pool_fams: u16) -> (u16, Vec<String>) {
        if !self.enabled || self.poisoned_callers.contains(&func.name) {
            return (0, Vec::new());
        }
        let mut keep = KEEP_CANDIDATES & pool_fams;
        let mut callees: Vec<String> = Vec::new();
        for inst in func.blocks.iter().flat_map(|b| &b.instructions) {
            if keep == 0 {
                return (0, Vec::new());
            }
            if !crate::backend::liveness::instruction_is_call_point(inst) {
                continue;
            }
            let Instruction::Call { func: name, info } = inst else {
                return (0, Vec::new());
            };
            // Inline-expanded memcpy/memset are call points with their own
            // register traffic (`rep movsb`, staging), not calls.
            if crate::backend::generation::inline_memcpy_len(name, info).is_some()
                || crate::backend::generation::x86_inline_memset_len(name, info).is_some()
            {
                return (0, Vec::new());
            }
            let (Some(staging), Some(clobbers)) = (call_staging(info), self.summaries.get(name))
            else {
                return (0, Vec::new());
            };
            keep &= !(staging | clobbers);
            if !callees.contains(name) {
                callees.push(name.clone());
            }
        }
        if callees.is_empty() || keep == 0 {
            return (0, Vec::new());
        }
        (keep, callees)
    }

    /// Remember the keep set the final emission of `caller` relied on.
    pub(crate) fn record(&mut self, caller: &str, keep: u16, callees: Vec<String>) {
        if keep == 0 {
            self.records.remove(caller);
        } else {
            self.records.insert(caller.to_string(), (keep, callees));
        }
    }
}

/// A function whose emitted code is the code every direct call reaches:
/// defined here, not interposable/replaceable, and free of inline assembly
/// (opaque text, possibly runtime-patched).
pub(crate) fn summarizable(func: &IrFunction, binds_locally: bool) -> bool {
    binds_locally
        && !func.is_declaration
        && !func.is_weak
        && !func.is_naked
        && !func.is_gnu_inline_def
        && !func
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .any(|i| matches!(i, Instruction::InlineAsm { .. }))
}

/// GP argument registers a call of the restricted shape stages (family
/// bits), or `None` when the call's lowering may touch more registers than
/// its leading argument registers: stack arguments (`%r11` relay, dynamic
/// realignment), by-value aggregates (copy loops), sret, variadic (`%al`),
/// fastcall/regparm, and non-scalar values.
pub(crate) fn call_staging(info: &CallInfo) -> Option<u16> {
    if info.is_variadic
        || info.is_sret
        || info.is_fastcall
        || info.regparm.is_some()
        || info.ret_is_f128_sse
        || info.arg_types.len() != info.args.len()
        || info.struct_arg_sizes.iter().any(Option::is_some)
    {
        return None;
    }
    let scalar_gp = |t: &IrType| {
        matches!(
            t,
            IrType::I8
                | IrType::I16
                | IrType::I32
                | IrType::I64
                | IrType::U8
                | IrType::U16
                | IrType::U32
                | IrType::U64
                | IrType::Ptr
        )
    };
    let mut gp = 0usize;
    let mut fp = 0usize;
    for t in &info.arg_types {
        if scalar_gp(t) {
            gp += 1;
        } else if matches!(t, IrType::F32 | IrType::F64) {
            fp += 1;
        } else {
            return None;
        }
    }
    let ret_ok = info.return_type == IrType::Void
        || scalar_gp(&info.return_type)
        || matches!(info.return_type, IrType::F32 | IrType::F64);
    if gp > 6 || fp > 8 || !ret_ok {
        return None;
    }
    Some(ARG_ORDER[..gp].iter().fold(0, |m, r| m | r))
}

/// GP family of a `%name` register spelling. `Ok(None)` for registers
/// outside the general-purpose file (vector, mask, x87, segment, `%rip`,
/// APX `%r16`-`%r31`, which no keep set contains); `Err` for an unknown
/// spelling (fail closed).
fn family_of(name: &str) -> Result<Option<u16>, ()> {
    let f = match name {
        "rax" | "eax" | "ax" | "al" | "ah" => RAX,
        "rcx" | "ecx" | "cx" | "cl" | "ch" => RCX,
        "rdx" | "edx" | "dx" | "dl" | "dh" => RDX,
        "rbx" | "ebx" | "bx" | "bl" | "bh" => RBX,
        "rsp" | "esp" | "sp" | "spl" => RSP,
        "rbp" | "ebp" | "bp" | "bpl" => RBP,
        "rsi" | "esi" | "si" | "sil" => RSI,
        "rdi" | "edi" | "di" | "dil" => RDI,
        "rip" | "st" | "fs" | "gs" | "cs" | "ds" | "es" | "ss" => return Ok(None),
        _ => {
            let vector = ["xmm", "ymm", "zmm", "mm"].iter().any(|p| {
                name.strip_prefix(p)
                    .is_some_and(|d| d.parse::<u8>().is_ok())
            });
            let mask =
                name.len() == 2 && name.starts_with('k') && name.as_bytes()[1].is_ascii_digit();
            if vector || mask {
                return Ok(None);
            }
            let Some(rest) = name.strip_prefix('r') else {
                return Err(());
            };
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let suffix = &rest[digits.len()..];
            let Ok(n) = digits.parse::<u8>() else {
                return Err(());
            };
            if !matches!(suffix, "" | "d" | "w" | "b" | "l") {
                return Err(());
            }
            return match n {
                8..=15 => Ok(Some(1 << n)),
                16..=31 => Ok(None),
                _ => Err(()),
            };
        }
    };
    Ok(Some(f))
}

/// Families every `%register` in `ops` names, or `None` for an unknown
/// spelling.
fn mentioned_families(ops: &str) -> Option<u16> {
    let mut mask = 0u16;
    let mut rest = ops;
    while let Some(p) = rest.find('%') {
        let after = &rest[p + 1..];
        let len = after
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        if let Some(f) = family_of(&after[..len]).ok()? {
            mask |= f;
        }
        rest = &after[len..];
    }
    Some(mask)
}

/// `true` when `mnem` is one of the string instructions (`movs`, `stos`,
/// `lods`, `scas`, `cmps`, optional size suffix) in its operand-less form.
fn is_string_op(mnem: &str, ops: &str) -> bool {
    let (stem, suffix) = if mnem.len() == 5 {
        mnem.split_at(4)
    } else {
        (mnem, "")
    };
    matches!(stem, "movs" | "stos" | "lods" | "scas" | "cmps")
        && matches!(suffix, "" | "b" | "w" | "l" | "q")
        && ops.is_empty()
}

/// Implicit GP writes of `mnem` (registers it writes without naming them),
/// `Some(0)` for an instruction known to have none, `None` for anything not
/// classified (fail closed).
fn implicit_writes(mnem: &str, ops: &str) -> Option<u16> {
    if is_string_op(mnem, ops) {
        return Some(RAX | RCX | RSI | RDI);
    }
    let implicit = match mnem {
        "cqto" | "cqo" | "cltd" | "cdq" | "cwtd" | "cwd" => RDX,
        "cltq" | "cdqe" | "cwtl" | "cwde" | "cbtw" | "cbw" => RAX,
        "div" | "divb" | "divw" | "divl" | "divq" | "idiv" | "idivb" | "idivw" | "idivl"
        | "idivq" | "mul" | "mulb" | "mulw" | "mull" | "mulq" => RAX | RDX,
        "imul" | "imulb" | "imulw" | "imull" | "imulq" if !ops.contains(',') => RAX | RDX,
        "cmpxchg" | "cmpxchgb" | "cmpxchgw" | "cmpxchgl" | "cmpxchgq" => RAX,
        "cmpxchg8b" | "cmpxchg16b" => RAX | RDX,
        "cpuid" => RAX | RBX | RCX | RDX,
        "rdtsc" | "xgetbv" => RAX | RDX,
        "rdtscp" => RAX | RCX | RDX,
        "syscall" => RAX | RCX | R11,
        "lahf" => RAX,
        "fnstsw" | "fstsw" if ops.is_empty() => RAX,
        "leave" | "leaveq" => RBP | RSP,
        "pcmpestri" | "pcmpistri" | "vpcmpestri" | "vpcmpistri" => RCX,
        "xbegin" => RAX,
        _ => {
            return no_implicit_gp_write(mnem).then_some(0);
        }
    };
    Some(implicit)
}

/// Mnemonics that write no general-purpose register they do not name. The
/// implicit-write table in [`implicit_writes`] is consulted first, so a
/// prefix here never admits one of its entries (`mul` vs `mulq`, `cmp` vs
/// `cmpxchg`, `lea` vs `leave`, `mov` vs the string form `movsq`).
fn no_implicit_gp_write(m: &str) -> bool {
    const PREFIXES: &[&str] = &[
        // integer ALU, moves, bit manipulation
        "mov",
        "lea",
        "add",
        "adc",
        "sub",
        "sbb",
        "and",
        "or",
        "xor",
        "not",
        "neg",
        "inc",
        "dec",
        "cmp",
        "test",
        "shl",
        "shr",
        "sal",
        "sar",
        "rol",
        "ror",
        "rcl",
        "rcr",
        "bt",
        "bsf",
        "bsr",
        "bswap",
        "set",
        "cmov",
        "push",
        "pop",
        "xchg",
        "xadd",
        "imul",
        "lzcnt",
        "tzcnt",
        "bextr",
        "blsi",
        "blsmsk",
        "blsr",
        "bzhi",
        "pdep",
        "pext",
        "rorx",
        "sarx",
        "shlx",
        "shrx",
        "mulx",
        "crc32",
        "nop",
        "endbr",
        "ud2",
        "int3",
        "pause",
        "lfence",
        "mfence",
        "sfence",
        "prefetch",
        "clflush",
        "clwb",
        "cld",
        "std",
        "clc",
        "stc",
        "cmc",
        // SSE / AVX arithmetic, conversion and data movement
        "mul",
        "div",
        "min",
        "max",
        "sqrt",
        "rcp",
        "rsqrt",
        "ucomi",
        "comi",
        "cvt",
        "unpck",
        "shuf",
        "blend",
        "round",
        "dp",
        "insertps",
        "extractps",
        "hadd",
        "hsub",
        "addsub",
        "lddqu",
        "aes",
        "sha",
        "ldmxcsr",
        "stmxcsr",
        "emms",
        // explicit-destination reads of hardware state
        "rdrand",
        "rdseed",
        "rdfsbase",
        "rdgsbase",
    ];
    // AVX (`v...`), SSE integer (`p...`) and x87 (`f...`) families name every
    // GP register they write, except the VMX/SEV/TDX system instructions
    // that share the prefixes (and the `pcmp*stri` forms tabled above).
    const DENY: &[&str] = &[
        "vmcall",
        "vmclear",
        "vmfunc",
        "vmlaunch",
        "vmptrld",
        "vmptrst",
        "vmread",
        "vmresume",
        "vmwrite",
        "vmxoff",
        "vmxon",
        "vmrun",
        "vmload",
        "vmsave",
        "vmmcall",
        "vmgexit",
        "pconfig",
        "pvalidate",
        "psmash",
        "pcmpestrm",
        "pcmpistrm",
    ];
    if DENY.contains(&m) {
        return false;
    }
    matches!(m.as_bytes().first(), Some(b'v' | b'p' | b'f'))
        || PREFIXES.iter().any(|p| m.starts_with(p))
}

/// The clobber contribution of a call/jump target: a local label is this
/// function's own code (scanned in place); a summarized symbol contributes
/// its summary; `__stack_chk_fail` never returns; anything else is unknown.
fn target_clobbers(target: &str, lookup: &dyn Fn(&str) -> Option<u16>) -> Option<u16> {
    if target.starts_with('.') {
        return Some(0);
    }
    let sym = target.strip_suffix("@PLT").unwrap_or(target);
    if sym == "__stack_chk_fail" {
        return Some(0);
    }
    lookup(sym)
}

/// GP families one instruction writes, or `None` when it is not classified.
fn insn_writes(t: &str, lookup: &dyn Fn(&str) -> Option<u16>) -> Option<u16> {
    let mut s = t.trim();
    for p in [
        "lock ", "rep ", "repe ", "repz ", "repne ", "repnz ", "notrack ",
    ] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.trim_start();
        }
    }
    let (mnem, ops) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
    let ops = ops.trim();
    let named = mentioned_families(ops)?;
    match mnem {
        "call" | "callq" => {
            // TLS descriptor call: the TLSDESC ABI preserves every register
            // except %rax.
            if ops.starts_with('*') {
                return (ops.contains("@TLSCALL(%rax)")).then_some(RAX);
            }
            target_clobbers(ops, lookup)
        }
        "jmp" | "jmpq" => {
            if ops.starts_with('*') {
                return None; // jump table or indirect tail call
            }
            target_clobbers(ops, lookup)
        }
        "ret" | "retq" => Some(0),
        _ if mnem.starts_with('j') => {
            // Conditional branch: a local label, or a conditional tail call.
            if ops.starts_with('*') {
                return None;
            }
            target_clobbers(ops, lookup)
        }
        _ => Some(named | implicit_writes(mnem, ops)?),
    }
}

/// `true` for a `.section` operand list naming executable code.
fn section_is_text(args: &str) -> bool {
    let name = args.split(',').next().unwrap_or("").trim();
    name.starts_with(".text") || args.split(',').nth(1).is_some_and(|f| f.contains('x'))
}

/// Caller-saved GP families the function text `text` may write (including
/// through the functions it calls), or `None` when the text contains
/// anything the scanner does not classify exactly. `lookup` answers the
/// summary of a called symbol.
pub(crate) fn text_clobbers(text: &str, lookup: &dyn Fn(&str) -> Option<u16>) -> Option<u16> {
    let mut acc = 0u16;
    let mut in_text = true;
    for raw in text.lines() {
        // Comments (the LCCC_* markers among them) carry no instructions.
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.ends_with(':') && !line.contains(char::is_whitespace) {
            continue; // label
        }
        if let Some(dir) = line.strip_prefix('.') {
            let (d, args) = dir.split_once(char::is_whitespace).unwrap_or((dir, ""));
            match d {
                "text" => in_text = true,
                "data" | "bss" => in_text = false,
                "section" => in_text = section_is_text(args),
                "pushsection" | "popsection" | "previous" | "subsection" => return None,
                "byte" | "short" | "value" | "word" | "long" | "int" | "quad" | "2byte"
                | "4byte" | "8byte" | "octa" | "inst" | "ascii" | "asciz" | "string" | "zero"
                | "skip" | "space" | "fill" | "incbin"
                    if in_text =>
                {
                    return None;
                }
                _ => {}
            }
            continue;
        }
        if !in_text {
            continue;
        }
        for insn in line.split(';') {
            if !insn.trim().is_empty() {
                acc |= insn_writes(insn, lookup)?;
            }
        }
    }
    Some(acc & SYSV_CALLER_SAVED)
}

/// Functions whose IPA-RA assumptions the final text violates.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct IpaViolations {
    /// Summarized functions whose final text clobbers more than predicted.
    pub(crate) callees: Vec<String>,
    /// Callers whose final text calls something its record does not cover.
    pub(crate) callers: Vec<String>,
}

impl IpaViolations {
    pub(crate) fn is_empty(&self) -> bool {
        self.callees.is_empty() && self.callers.is_empty()
    }
}

/// The body lines of `name` in the final assembly: from the `name:` label to
/// its `.size name, ...` directive.
fn function_body<'a>(
    lines: &'a [&'a str],
    index: &FxHashMap<&str, usize>,
    name: &str,
) -> Option<&'a [&'a str]> {
    let start = *index.get(name)?;
    let size = format!(".size {name},");
    let end = lines[start..]
        .iter()
        .position(|l| l.trim_start().starts_with(&size))?;
    Some(&lines[start + 1..start + end])
}

/// Re-derive every summary from the final assembly `asm` and re-check every
/// caller record against it (contract step 4).
pub(crate) fn verify(asm: &str, state: &IpaRaState) -> IpaViolations {
    let mut out = IpaViolations::default();
    if state.records.is_empty() {
        return out;
    }
    let lines: Vec<&str> = asm.lines().collect();
    let mut index: FxHashMap<&str, usize> = FxHashMap::default();
    // A name defined twice makes `function_body` ambiguous: a summary could
    // be derived from the wrong definition and the keep-set proof built on
    // it would be unverifiable. Such names fail closed (every record naming
    // them is a violation) instead of picking a definition.
    let mut duplicated: FxHashSet<&str> = FxHashSet::default();
    for (i, l) in lines.iter().enumerate() {
        if let Some(label) = l.strip_suffix(':') {
            if !label.contains(char::is_whitespace) && !label.starts_with('.') {
                if index.insert(label, i).is_some() {
                    duplicated.insert(label);
                }
            }
        }
    }
    // Final summaries in emission order: every callee a summary depends on
    // was summarized (and is re-derived here) before it.
    let mut actual: FxHashMap<&str, u16> = FxHashMap::default();
    for name in &state.order {
        let predicted = state.summaries[name];
        let final_mask = (if duplicated.contains(name.as_str()) {
            None
        } else {
            function_body(&lines, &index, name)
        })
        .and_then(|body| text_clobbers(&body.join("\n"), &|s| actual.get(s).copied()));
        match final_mask {
            Some(m) if m & !predicted == 0 => {
                actual.insert(name.as_str(), m);
            }
            _ => out.callees.push(name.clone()),
        }
    }
    let mut callers: Vec<&String> = state.records.keys().collect();
    callers.sort();
    for caller in callers {
        let (keep, callees) = &state.records[caller];
        let calls_ok = function_body(&lines, &index, caller).is_some_and(|body| {
            body.iter().all(|l| {
                let line = l.split('#').next().unwrap_or("").trim();
                let Some(ops) = line
                    .strip_prefix("call ")
                    .or_else(|| line.strip_prefix("callq "))
                else {
                    return true;
                };
                let ops = ops.trim();
                if ops.starts_with('.') || ops == "__stack_chk_fail" {
                    return true;
                }
                if ops.starts_with('*') {
                    // TLSDESC preserves all but %rax, which no keep set holds.
                    return ops.contains("@TLSCALL(%rax)");
                }
                let sym = ops.strip_suffix("@PLT").unwrap_or(ops);
                // A twice-defined target has no trustworthy summary.
                !duplicated.contains(sym)
                    && callees.iter().any(|c| c == sym)
                    && actual.get(sym).is_some_and(|m| m & keep == 0)
            })
        });
        if !calls_ok {
            out.callers.push(caller.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<u16> {
        None
    }

    #[test]
    fn helper_body_clobbers_exactly_what_it_names() {
        let helper = "helper:\n.cfi_startproc\n    # LCCC_RET_RAX 1\n    xorq salt(%rip), %rdi\n\
                      \x20   movabsq $-2960836687051489901, %rax\n    imulq %rax, %rdi\n\
                      \x20   movq %rdi, %rax\n    ret\n.cfi_endproc\n";
        assert_eq!(text_clobbers(helper, &none), Some(RAX | RDI));
    }

    #[test]
    fn implicit_writers_are_counted() {
        let cases: &[(&str, u16)] = &[
            ("    cqto\n    idivq %rcx\n", RAX | RCX | RDX),
            ("    rep movsb\n", RAX | RCX | RSI | RDI),
            ("    rep stosq\n", RAX | RCX | RSI | RDI),
            ("    cltq\n", RAX),
            ("    mulq %r8\n", RAX | RDX | R8),
            ("    imulq %r8, %r9\n", R8 | R9),
            ("    imulq %r8\n", RAX | RDX | R8),
            ("    lock cmpxchgq %rcx, (%r8)\n", RAX | RCX | R8),
            ("    cmpxchg16b (%rdi)\n", RAX | RDX | RDI),
            ("    cpuid\n", RAX | RCX | RDX),
            ("    rdtscp\n", RAX | RCX | RDX),
            ("    syscall\n", RAX | RCX | R11),
            ("    pcmpistri $12, %xmm1, %xmm0\n", RCX),
            ("    movsbl %r9b, %r10d\n", R9 | R10),
            ("    movsd %xmm0, 8(%rsi)\n", RSI),
            ("    vmovdqu (%r8), %ymm0\n    vzeroupper\n", R8),
            ("    sete %sil\n    movzbl %sil, %esi\n", RSI),
            ("    nop;nop;nop\n", 0),
        ];
        for (text, want) in cases {
            assert_eq!(text_clobbers(text, &none), Some(*want), "{text}");
        }
        // Callee-saved writes are the callee's own obligation.
        assert_eq!(
            text_clobbers(
                "    pushq %rbx\n    movq %rdi, %rbx\n    popq %rbx\n",
                &none
            ),
            Some(RDI)
        );
    }

    #[test]
    fn unclassified_text_has_no_summary() {
        for text in [
            "    call printf\n",
            "    call *%rax\n",
            "    jmp *%rcx\n",
            "    jmp other\n",
            "    rdmsr\n",
            "    vmcall\n",
            "    int $128\n",
            "    loop .L1\n",
            "    movq %cr3, %rax\n",
            ".pushsection .data\n.popsection\n",
            "    .byte 0x0f, 0x0b\n",
            "    movq %r99, %rax\n",
        ] {
            assert_eq!(text_clobbers(text, &none), None, "{text}");
        }
    }

    #[test]
    fn calls_compose_summaries() {
        let lookup = |s: &str| (s == "leaf").then_some(RAX | R8);
        assert_eq!(
            text_clobbers("    call leaf\n    ret\n", &lookup),
            Some(RAX | R8)
        );
        assert_eq!(
            text_clobbers("    call leaf@PLT\n", &lookup),
            Some(RAX | R8)
        );
        assert_eq!(text_clobbers("    jmp leaf\n", &lookup), Some(RAX | R8));
        assert_eq!(text_clobbers("    jne leaf\n", &lookup), Some(RAX | R8));
        assert_eq!(text_clobbers("    call .Lrpl_set\n", &lookup), Some(0));
        assert_eq!(
            text_clobbers("    call __stack_chk_fail\n", &lookup),
            Some(0)
        );
        assert_eq!(
            text_clobbers(
                "    leaq x@TLSDESC(%rip), %rax\n    call *x@TLSCALL(%rax)\n",
                &lookup
            ),
            Some(RAX)
        );
    }

    #[test]
    fn data_sections_are_skipped_and_text_bytes_refused() {
        let jt =
            "    movq %rdi, %r8\n.section .rodata\n.LJT:\n    .long .L1-.LJT\n.text\n    ret\n";
        assert_eq!(text_clobbers(jt, &none), Some(R8 | RDI));
        assert_eq!(
            text_clobbers(".section .text.hot,\"ax\",@progbits\n    .byte 1\n", &none),
            None
        );
    }

    fn info(types: &[IrType], ret: IrType) -> CallInfo {
        CallInfo {
            args: types
                .iter()
                .map(|_| {
                    crate::ir::reexports::Operand::Const(crate::ir::reexports::IrConst::I64(0))
                })
                .collect(),
            arg_types: types.to_vec(),
            return_type: ret,
            struct_arg_sizes: vec![None; types.len()],
            ..CallInfo::default()
        }
    }

    #[test]
    fn call_shape_and_staging() {
        assert_eq!(call_staging(&info(&[IrType::I64], IrType::I64)), Some(RDI));
        assert_eq!(
            call_staging(&info(
                &[IrType::Ptr, IrType::F64, IrType::U32],
                IrType::Void
            )),
            Some(RDI | RSI)
        );
        let six = [IrType::I64; 6];
        assert_eq!(
            call_staging(&info(&six, IrType::F32)),
            Some(RDI | RSI | RDX | RCX | R8 | R9)
        );
        assert_eq!(
            call_staging(&info(&[IrType::I64; 7], IrType::I64)),
            None,
            "stack argument"
        );
        assert_eq!(call_staging(&info(&[IrType::I128], IrType::I64)), None);
        assert_eq!(call_staging(&info(&[IrType::F128], IrType::I64)), None);
        assert_eq!(call_staging(&info(&[IrType::I64], IrType::I128)), None);
        let mut v = info(&[IrType::I64], IrType::I64);
        v.is_variadic = true;
        assert_eq!(call_staging(&v), None);
        let mut s = info(&[IrType::Ptr], IrType::Void);
        s.struct_arg_sizes = vec![Some(24)];
        assert_eq!(call_staging(&s), None);
    }

    #[test]
    fn verify_accepts_exact_and_rejects_grown_or_uncovered() {
        let mut st = IpaRaState::new(true, FxHashSet::default(), FxHashSet::default());
        st.summaries.insert("helper".into(), RAX | RDI);
        st.order.push("helper".into());
        st.record("main", R8 | R9 | RSI, vec!["helper".into()]);
        let good = "helper:\n    movq %rdi, %rax\n    ret\n.size helper, .-helper\n\
                    main:\n    call helper\n    # LCCC_IPA_KEEP 0x0340\n    ret\n.size main, .-main\n";
        assert!(verify(good, &st).is_empty());
        // The peephole introduced an %r8 write into the callee.
        let grown = good.replace(
            "    movq %rdi, %rax\n",
            "    movq %rdi, %r8\n    movq %r8, %rax\n",
        );
        assert_eq!(verify(&grown, &st).callees, vec!["helper".to_string()]);
        assert_eq!(verify(&grown, &st).callers, vec!["main".to_string()]);
        // A call the record does not cover (a late libcall).
        let extra = good.replace(
            "    ret\n.size main",
            "    call memcpy\n    ret\n.size main",
        );
        let v = verify(&extra, &st);
        assert!(v.callees.is_empty());
        assert_eq!(v.callers, vec!["main".to_string()]);
        // A missing body is a violation, never an assumption.
        assert_eq!(
            verify("main:\n    ret\n.size main, .-main\n", &st).callees,
            vec!["helper".to_string()]
        );
    }

    /// A twice-defined name is ambiguous, not first-wins: whichever body
    /// `function_body` would pick, the summary cannot be trusted, so both
    /// the callee summary and every caller record naming it fail closed.
    #[test]
    fn verify_fails_closed_on_duplicated_label() {
        let mut st = IpaRaState::new(true, FxHashSet::default(), FxHashSet::default());
        // `movq %rdi, %rax` names both families, so the predicted summary
        // carries both.
        st.summaries.insert("helper".into(), RAX | RDI);
        st.order.push("helper".into());
        st.record("main", R8 | R9, vec!["helper".into()]);
        // Both definitions are individually clean against the predicted
        // summary: a first-wins index would hide the second one and let the
        // verification pass.
        let dup = "helper:\n    movq %rdi, %rax\n    ret\n.size helper, .-helper\n\
                   helper:\n    xorq %r9, %r9\n    ret\n.size helper, .-helper\n\
                   main:\n    call helper\n    ret\n.size main, .-main\n";
        let v = verify(dup, &st);
        assert_eq!(v.callees, vec!["helper".to_string()]);
        assert_eq!(v.callers, vec!["main".to_string()]);
        // One definition: clean.
        let single = "helper:\n    movq %rdi, %rax\n    ret\n.size helper, .-helper\n\
              main:\n    call helper\n    ret\n.size main, .-main\n";
        assert!(verify(single, &st).is_empty());
    }
}

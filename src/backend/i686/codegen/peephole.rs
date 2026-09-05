//! i686 peephole optimizer for assembly text.
//!
//! Operates on generated assembly text to eliminate redundant patterns from the
//! stack-based codegen. Adapted from the x86-64 peephole optimizer for 32-bit
//! i686 assembly (uses %ebp instead of %rbp, %eax instead of %rax, etc.).
//!
//! ## Pass structure
//!
//! 1. **Local passes** (iterative, up to 8 rounds): adjacent store/load elimination,
//!    self-move elimination, redundant jump elimination, branch inversion, reverse
//!    move elimination.
//!
//! 2. **Global passes** (once): dead register move elimination, dead store elimination,
//!    compare+branch fusion, memory operand folding.
//!
//! 3. **Local cleanup** (up to 4 rounds): re-run local and global passes to clean up
//!    opportunities exposed by the first round.
//!
//! 4. **Never-read store elimination**: global analysis to remove stores to
//!    stack slots that are never read anywhere in the function.

// ── Constants ────────────────────────────────────────────────────────────────

const MAX_LOCAL_PASS_ITERATIONS: usize = 8;
const MAX_POST_GLOBAL_ITERATIONS: usize = 4;

// Register IDs (i686 has fewer registers)
type RegId = u8;
const REG_NONE: RegId = 255;
const REG_EAX: RegId = 0;
const REG_ECX: RegId = 1;
const REG_EDX: RegId = 2;
const REG_EBX: RegId = 3;
const REG_ESP: RegId = 4;
const REG_EBP: RegId = 5;
const REG_ESI: RegId = 6;
const REG_EDI: RegId = 7;
const REG_GP_MAX: RegId = 7;

/// Sentinel value for ebp_offset meaning "no %ebp reference" or "complex reference".
const EBP_OFFSET_NONE: i32 = i32::MIN;

// ── Line classification ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Nop,
    Empty,
    StoreEbp {
        reg: RegId,
        offset: i32,
        size: MoveSize,
    },
    LoadEbp {
        reg: RegId,
        offset: i32,
        size: MoveSize,
    },
    Move {
        dst: RegId,
        src: RegId,
    },
    SelfMove,
    Label,
    Jmp,
    JmpIndirect,
    CondJmp,
    Call,
    Ret,
    Push {
        reg: RegId,
    },
    Pop {
        reg: RegId,
    },
    SetCC {
        reg: RegId,
    },
    Cmp,
    Directive,
    Other {
        dest_reg: RegId,
    },
    /// A line inside a `#APP`/`#NO_APP` inline-asm region.  User-authored
    /// assembly is OPAQUE: it may hold deliberate no-op length templates,
    /// runtime-patched sites, or any side effect.  No pass may remove,
    /// rewrite, or reason about these lines; every conservative fallback
    /// treats them as clobbering every register and all memory
    /// (has_indirect_mem = true, ebp_offset = NONE, and they barrier every
    /// forwarding window).
    InlineAsm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveSize {
    L, // movl (32-bit)
    W, // movw (16-bit)
    B, // movb (8-bit)
}

impl MoveSize {
    fn mnemonic(self) -> &'static str {
        match self {
            MoveSize::L => "movl",
            MoveSize::W => "movw",
            MoveSize::B => "movb",
        }
    }
    fn byte_size(self) -> i32 {
        match self {
            MoveSize::L => 4,
            MoveSize::W => 2,
            MoveSize::B => 1,
        }
    }
}

/// Check if two byte ranges `[a, a+a_size)` and `[b, b+b_size)` overlap.
#[inline]
fn ranges_overlap(a_off: i32, a_size: i32, b_off: i32, b_size: i32) -> bool {
    a_off < b_off + b_size && b_off < a_off + a_size
}

#[derive(Clone, Copy)]
struct LineInfo {
    kind: LineKind,
    trim_start: u16,
    has_indirect_mem: bool,
    ebp_offset: i32,
}

impl LineInfo {
    #[inline]
    fn is_nop(self) -> bool {
        self.kind == LineKind::Nop
    }
    #[inline]
    fn is_barrier(self) -> bool {
        matches!(
            self.kind,
            LineKind::Label | LineKind::Call | LineKind::Jmp | LineKind::JmpIndirect |
            LineKind::CondJmp | LineKind::Ret | LineKind::Directive | LineKind::InlineAsm |
            // ESP changes renumber every (%esp) slot: push/pop and explicit
            // %esp arithmetic fence all slot windows now that ESP-relative
            // slots participate in the peepholes. Pushes/pops only appear
            // in prologue/epilogue and around calls, so the cost is nil.
            LineKind::Push { .. } | LineKind::Pop { .. }
        ) || matches!(self.kind, LineKind::Other { dest_reg } if dest_reg == REG_ESP)
    }
}

#[inline]
fn line_info(kind: LineKind, ts: u16) -> LineInfo {
    LineInfo {
        kind,
        trim_start: ts,
        has_indirect_mem: false,
        ebp_offset: EBP_OFFSET_NONE,
    }
}

/// Classify a `cmpX`/`testX` line.
///
/// A compare/test with a base-only frame memory operand READS that slot
/// (`cmpl $0, 44(%esp)` reads bytes 44..48). The offset must be recorded in
/// `ebp_offset` so every slot-liveness guard sees the reader: the
/// compare-with-memory fold (`movl SLOT,%R; testl %R,%R` → `cmpl $0, SLOT`)
/// produces exactly this shape, and with `ebp_offset` left at NONE both the
/// windowed `eliminate_dead_stores` and the global
/// `eliminate_never_read_stores_range` deleted the still-live store feeding
/// the folded read — a union type-pun then read uninitialized stack
/// (gcc.c-torture/execute cbrt.c: `n0 = (ut.pt[0] == 0)` saw garbage, every
/// call at -O1+ returned -nan; the store survived only when a previous call
/// happened to leave zeros on the same stack bytes).
///
/// Mirrors the x86-64 backend's `cmp_line_info`, which has parsed the frame
/// offset for compare lines for the same reason. Indexed forms
/// (`12(%esp,%ebx,4)`) are runtime-computed addresses: `has_indirect_mem`
/// marks them and the offset stays NONE (same rule as the generic arm).
fn cmp_line_info(ts: u16, s: &str) -> LineInfo {
    let has_indirect = has_indirect_memory_access(s);
    let ebp_offset = if has_indirect {
        EBP_OFFSET_NONE
    } else {
        parse_ebp_offset_in_line(s)
    };
    LineInfo {
        kind: LineKind::Cmp,
        trim_start: ts,
        has_indirect_mem: has_indirect,
        ebp_offset,
    }
}

// ── Register parsing ─────────────────────────────────────────────────────────

/// Map i686 register name to family ID.
fn register_family(name: &str) -> RegId {
    let name = name.trim_start_matches('%');
    match name {
        "eax" | "ax" | "al" | "ah" => REG_EAX,
        "ecx" | "cx" | "cl" | "ch" => REG_ECX,
        "edx" | "dx" | "dl" | "dh" => REG_EDX,
        "ebx" | "bx" | "bl" | "bh" => REG_EBX,
        "esp" | "sp" => REG_ESP,
        "ebp" | "bp" => REG_EBP,
        "esi" | "si" => REG_ESI,
        "edi" | "di" => REG_EDI,
        _ => REG_NONE,
    }
}

/// Get the 32-bit register name for a family ID.
fn reg32_name(id: RegId) -> &'static str {
    match id {
        REG_EAX => "%eax",
        REG_ECX => "%ecx",
        REG_EDX => "%edx",
        REG_EBX => "%ebx",
        REG_ESP => "%esp",
        REG_EBP => "%ebp",
        REG_ESI => "%esi",
        REG_EDI => "%edi",
        _ => "%???",
    }
}

/// Byte/word register name for a family ID, matching the width of `size`
/// (B => 8-bit low register, W => 16-bit register).
///
/// `None` when the encoding does not exist on this target: in 32-bit mode
/// only %al/%cl/%dl/%bl have 8-bit forms — %spl/%bpl/%sil/%dil require a
/// REX prefix and are 64-bit-only, so a caller asking for them must refuse
/// the rewrite instead of emitting text the assembler would reject.
fn narrow_reg_name(id: RegId, size: MoveSize) -> Option<&'static str> {
    match (id, size) {
        (REG_EAX, MoveSize::B) => Some("%al"),
        (REG_ECX, MoveSize::B) => Some("%cl"),
        (REG_EDX, MoveSize::B) => Some("%dl"),
        (REG_EBX, MoveSize::B) => Some("%bl"),
        (REG_EAX, MoveSize::W) => Some("%ax"),
        (REG_ECX, MoveSize::W) => Some("%cx"),
        (REG_EDX, MoveSize::W) => Some("%dx"),
        (REG_EBX, MoveSize::W) => Some("%bx"),
        (REG_ESP, MoveSize::W) => Some("%sp"),
        (REG_EBP, MoveSize::W) => Some("%bp"),
        (REG_ESI, MoveSize::W) => Some("%si"),
        (REG_EDI, MoveSize::W) => Some("%di"),
        _ => None,
    }
}

/// Parse a zero-extension into a 32-bit register: `movzbl %al, %eax` /
/// `movzwl %ax, %eax`.  Returns (source width, source operand text, dest
/// family).  The source TEXT is returned verbatim because `movzbl %ah` is
/// a different byte than `movzbl %al` — callers must compare it against the
/// expected low sub-register, not just the family.
fn parse_zext32(s: &str) -> Option<(MoveSize, &str, RegId)> {
    let (rest, w) = if let Some(r) = s.strip_prefix("movzbl ") {
        (r, MoveSize::B)
    } else if let Some(r) = s.strip_prefix("movzwl ") {
        (r, MoveSize::W)
    } else {
        return None;
    };
    let (a, b) = rest.split_once(',')?;
    let a = a.trim();
    let dst = register_family(b.trim());
    if a.starts_with('%') && dst <= REG_GP_MAX {
        Some((w, a, dst))
    } else {
        None
    }
}

/// Check if a register is caller-saved (clobbered by calls).
fn is_caller_saved(reg: RegId) -> bool {
    matches!(reg, REG_EAX | REG_ECX | REG_EDX)
}

// ── Store/Load parsing ───────────────────────────────────────────────────────

/// Parse `movX %reg, offset(%ebp)` → (reg_name, offset_str, MoveSize)
fn parse_store_to_ebp(s: &str) -> Option<(&str, &str, MoveSize)> {
    let (rest, size) = if let Some(r) = s.strip_prefix("movl ") {
        (r, MoveSize::L)
    } else if let Some(r) = s.strip_prefix("movw ") {
        (r, MoveSize::W)
    } else if let Some(r) = s.strip_prefix("movb ") {
        (r, MoveSize::B)
    } else {
        return None;
    };
    // rest = "%eax, -8(%ebp)"
    let rest = rest.trim();
    if !rest.starts_with('%') {
        return None;
    }
    let comma = rest.find(',')?;
    let reg = &rest[..comma];
    let mem = rest[comma + 1..].trim();
    // Accept BOTH frame-pointer and stack-pointer slots. With
    // -fomit-frame-pointer (implied by -Os and the kernel's realmode
    // Makefile) every local lives at N(%esp); matching only (%ebp) made
    // all slot peepholes dead letters -- the real-mode printf.o carried
    // ~100 store/reload round-trips GCC folds away, overflowing the
    // 64-sector setup limit. ESP-relative offsets are stable between ESP
    // adjustments; push/pop/%esp-writes are barriers (see is_barrier), so
    // a slot tracked within a window can never be renumbered mid-window.
    let is_ebp = mem.ends_with("(%ebp)");
    let is_esp = mem.ends_with("(%esp)");
    if !is_ebp && !is_esp {
        return None;
    }
    // Reject indexed forms like 4(%esp,%eax,4): only plain offset(base).
    let paren = mem.find('(').unwrap_or(0);
    if mem[paren..].contains(',') {
        return None;
    }
    let offset_str = &mem[..mem.len() - 6]; // strip "(%ebp)"/"(%esp)"
    Some((reg.trim(), offset_str, size))
}

/// Parse `movX offset(%ebp), %reg` → (offset_str, reg_name, MoveSize)
fn parse_load_from_ebp(s: &str) -> Option<(&str, &str, MoveSize)> {
    let (rest, size) = if let Some(r) = s.strip_prefix("movl ") {
        (r, MoveSize::L)
    } else if let Some(r) = s.strip_prefix("movw ") {
        (r, MoveSize::W)
    } else if let Some(r) = s.strip_prefix("movb ") {
        (r, MoveSize::B)
    } else {
        return None;
    };
    let rest = rest.trim();
    // Accept both frame-pointer and stack-pointer slots (see
    // parse_store_to_ebp for the -fomit-frame-pointer rationale).
    let paren_start = match rest.find("(%ebp)") {
        Some(p) => p,
        None => rest.find("(%esp)")?,
    };
    let offset_str = &rest[..paren_start];
    // Reject scaled/indexed forms like 4(%esp,%eax,4).
    if offset_str.contains('(') {
        return None;
    }
    let after = rest[paren_start + 6..].trim();
    if !after.starts_with(',') {
        return None;
    }
    let reg = after[1..].trim();
    if !reg.starts_with('%') {
        return None;
    }
    Some((offset_str, reg, size))
}

/// Parse `movl %src, %dst` (register-to-register move).
fn parse_reg_to_reg_move(s: &str) -> Option<(RegId, RegId)> {
    let rest = s.strip_prefix("movl ")?.trim();
    if !rest.starts_with('%') {
        return None;
    }
    let comma = rest.find(',')?;
    let src_name = rest[..comma].trim();
    let dst_name = rest[comma + 1..].trim();
    if !dst_name.starts_with('%') {
        return None;
    }
    // Must not be memory operands
    if src_name.contains('(') || dst_name.contains('(') {
        return None;
    }
    let src = register_family(src_name);
    let dst = register_family(dst_name);
    if src <= REG_GP_MAX && dst <= REG_GP_MAX {
        Some((src, dst))
    } else {
        None
    }
}

/// Parse integer offset from string.
fn parse_offset(s: &str) -> i32 {
    if s.is_empty() {
        return 0;
    }
    s.parse::<i32>().unwrap_or(EBP_OFFSET_NONE)
}

/// Check if a line has indirect memory access (pointer dereference through a register).
fn has_indirect_memory_access(s: &str) -> bool {
    // Pattern: offset(%eXX) where XX is not bp or sp
    // or (%eXX) where XX is not bp or sp
    // or (%eXX, %eYY, N)
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'(' && i + 4 < bytes.len() && bytes[i + 1] == b'%' {
            // Check if it's (%ebp) or (%esp) - those are stack accesses, not
            // indirect POINTER aliasing ...
            if &bytes[i + 1..i + 5] == b"%ebp" || &bytes[i + 1..i + 5] == b"%esp" {
                // ... but ONLY for BASE-ONLY forms! An INDEXED form like
                // `12(%esp, %ebx)` computes its effective address at RUNTIME:
                // no text-level displacement can describe the slot it writes,
                // so it must be treated as an indirect memory access exactly
                // like any other indexed operand. A missed marker here let
                // store-forwarding windows sail straight past byte-granular
                // indexed stores that alias the forwarded slot (union
                // byte[reg]=0 vs word reload: gcc.c-torture/execute
                // 990531-1 abort at -O2 once frame-SIB indexed addressing
                // reached the i686 emitters). Mirrors x86-64's identical
                // post-base-whitelist rule.
                let after_base = &s[i + 5..];
                if !after_base.starts_with(')') {
                    return true;
                }
                continue;
            }
            return true;
        }
    }
    false
}

/// Parse the %ebp offset from a line, or return EBP_OFFSET_NONE.
/// Namespace bias for %esp-relative slots: keeps them disjoint from
/// %ebp-relative offsets inside the same LineKind (an (%esp) slot and an
/// (%ebp) slot with equal displacement are DIFFERENT memory).
const ESP_SLOT_BIAS: i32 = 1 << 24;

#[inline]
fn slot_bias(s: &str) -> i32 {
    if s.contains("(%esp)") {
        ESP_SLOT_BIAS
    } else {
        0
    }
}

fn parse_ebp_offset_in_line(s: &str) -> i32 {
    let (pos, bias) = if let Some(p) = s.find("(%ebp)") {
        (p, 0)
    } else if let Some(p) = s.find("(%esp)") {
        (p, ESP_SLOT_BIAS)
    } else {
        return EBP_OFFSET_NONE;
    };
    let before = &s[..pos];
    // Find the start of the offset number
    let offset_start = before
        .rfind(|c: char| !c.is_ascii_digit() && c != '-')
        .map(|p| p + 1)
        .unwrap_or(0);
    let offset_str = &before[offset_start..];
    if offset_str.is_empty() {
        bias
    } else {
        match offset_str.parse::<i32>() {
            Ok(v) => v + bias,
            Err(_) => EBP_OFFSET_NONE,
        }
    }
}

/// Parse the destination register of a generic instruction.
/// For two-operand instructions (AT&T syntax), the destination is the last operand.
fn parse_dest_reg(s: &str) -> RegId {
    // Find the last %reg
    if let Some(comma) = s.rfind(',') {
        let after = s[comma + 1..].trim();
        if after.starts_with('%') && !after.contains('(') {
            return register_family(after);
        }
    }
    REG_NONE
}

/// Explicit-operand mention test: does the TEXT name any spelling of the
/// family?  This is [`line_references_reg`] WITHOUT the implicit-operand
/// union — consumers that must distinguish "the instruction names the
/// register" from "the architecture touches it" (a purely-architectural
/// redefinition proof must not be vetoed by the oracle's own mentions).
fn line_references_reg_explicit(s: &str, reg: RegId) -> bool {
    let names: &[&str] = match reg {
        REG_EAX => &["%eax", "%ax", "%al", "%ah"],
        REG_ECX => &["%ecx", "%cx", "%cl", "%ch"],
        REG_EDX => &["%edx", "%dx", "%dl", "%dh"],
        REG_EBX => &["%ebx", "%bx", "%bl", "%bh"],
        REG_ESP => &["%esp", "%sp"],
        REG_EBP => &["%ebp", "%bp"],
        REG_ESI => &["%esi", "%si"],
        REG_EDI => &["%edi", "%di"],
        _ => return false,
    };
    names.iter().any(|name| s.contains(name))
}

/// Check if a line references a specific register family.
/// This includes both explicit register operands and the architectural
/// implicit operands ([`classify_implicit_operands`]) of instructions like
/// cltd, idivl, rep movsb, rdtsc or syscall.
fn line_references_reg(s: &str, reg: RegId) -> bool {
    if line_references_reg_explicit(s, reg) {
        return true;
    }
    // Check implicit register uses by specific instructions
    if implicit_reg_use(s, reg) {
        return true;
    }
    false
}

/// Check if an instruction implicitly uses a register (not mentioned in
/// text): the union half of the central implicit-operand oracle.
fn implicit_reg_use(s: &str, reg: RegId) -> bool {
    if !(0..=REG_GP_MAX).contains(&reg) {
        return false;
    }
    let (reads, writes) = classify_implicit_operands(s);
    (reads | writes) & (1 << reg) != 0
}

/// Families an instruction implicitly READS without naming them in text.
fn line_implicitly_reads(s: &str, reg: RegId) -> bool {
    if !(0..=REG_GP_MAX).contains(&reg) {
        return false;
    }
    classify_implicit_operands(s).0 & (1 << reg) != 0
}

// ── Implicit register operands (central oracle) ─────────────────────────────
//
// Single source of truth for every register an instruction touches WITHOUT
// naming it in its operand text.  Four consumers used to keep four separate
// hand-rolled tables in sync (`implicit_reg_use` mentions,
// `line_writes_reg_implicitly` writes, `implicit_reads` reads, and the
// `line_reg_use_def` use/def table); each had drifted differently — none
// knew `leave`/`syscall`/`cpuid`/bare string ops, the write table missed
// every word/byte div-mul form except divl/idivl/mull, and `cmpxchg`'s
// implicit %eax rewrite (and `lock xadd`'s register RMW) was invisible to
// copy-propagation invalidation.
//
// The table records the implicit READS and the implicit WRITES separately:
// the union (a set bit only makes a pass more conservative) serves the
// mention-style checks, while the exact halves serve redefinition checks
// (writes only) and observation checks (reads only) that must not conflate
// the two — `cltd` reads %eax and writes %edx, and confusing those halves
// is exactly how a staging-copy fold deletes a live definition.
//
// Prefixes (`lock`, `rep`, `repe`, `repz`, `repne`, `repnz`) are skipped so
// the mnemonic that follows is classified; the `rep` family additionally
// contributes %ecx (count) — and `rep ret` / `rep nop` (pause) contribute
// nothing.  An UNRECOGNIZED prefixed mnemonic keeps the legacy fail-closed
// posture ("assume all registers used") — the emitter never produces one,
// but an unknown rep form must never look clean.
//
// Mnemonics are matched EXACTLY, never by prefix: `divsd`/`mulsd` are SSE
// instructions with fully explicit operands and must not inherit the
// integer div/mul accumulator pair.  `movsl` is the 32-bit string move;
// `movsd` is left out of the table entirely (SSE owns that spelling).
//
// The emitter today only produces a subset of the table (cltd, idivl/divl,
// one-operand imull, rep movsb, rdtsc/rdtscp, lock cmpxchg8b, lock
// cmpxchg*, lock xadd), but the oracle is written against the ISA, so the
// next new emitter site is covered for free.
fn classify_implicit_operands(s: &str) -> (u8, u8) {
    const EAX: u8 = 1 << REG_EAX;
    const ECX: u8 = 1 << REG_ECX;
    const EDX: u8 = 1 << REG_EDX;
    const EBX: u8 = 1 << REG_EBX;
    const ESP: u8 = 1 << REG_ESP;
    const EBP: u8 = 1 << REG_EBP;
    const ESI: u8 = 1 << REG_ESI;
    const EDI: u8 = 1 << REG_EDI;
    const ALL: u8 = 0xFF;

    // Callers mostly pass `trimmed(...)` slices, but defensive trimming
    // keeps every direct call site (and test) correct — the x86-64 oracle
    // documents the same contract.
    let s = s.trim_start();
    fn mnemonic(s: &str) -> (&str, &str) {
        match s.split_once(' ').or_else(|| s.split_once('\t')) {
            Some((m, rest)) => (m, rest.trim_start()),
            None => (s, ""),
        }
    }

    if s.is_empty() {
        return (0, 0);
    }
    let (mut m, mut rest) = mnemonic(s);
    if m.is_empty() || m.starts_with('.') || m.ends_with(':') || m.starts_with('#') {
        // Labels, directives and comments name no instruction.
        return (0, 0);
    }
    let mut reads: u8 = 0;
    let mut writes: u8 = 0;
    // Prefixes.
    loop {
        match m {
            "lock" => {}
            "rep" | "repe" | "repz" | "repne" | "repnz" => {
                // `rep ret` / `rep nop` (pause) are branch-hint idioms, not
                // counted loops: they touch no register at all.
                let (m2, _) = mnemonic(rest);
                if m2 == "ret" || m2 == "nop" {
                    return (0, 0);
                }
                reads |= ECX; // count tested for zero
                writes |= ECX; // count decremented
                if m2.is_empty() || !is_known_string_mnemonic(m2) {
                    // Unknown counted form: fail closed on every family.
                    return (ALL, ALL);
                }
            }
            _ => break,
        }
        let (m2, r2) = mnemonic(rest);
        if m2.is_empty() {
            return (reads, writes);
        }
        m = m2;
        rest = r2;
    }
    let has_comma = rest.contains(',');
    let table: (u8, u8) = match m {
        // Sign-extension of the accumulator in place.  GAS's AT&T spellings
        // (cbtw/cwtl) and Intel's (cbw/cwde) name the same encodings.
        "cbw" | "cbtw" | "cwde" | "cwtl" | "cdqe" => (EAX, EAX),
        // Sign-extension into the data register: %eax is the INPUT.
        "cwd" | "cwtd" | "cltd" | "cdq" | "cqo" => (EAX, EDX),
        // Byte multiply/divide: the accumulator pair is AX (quotient AL,
        // remainder AH, product AX) — the %edx family is never touched.
        "divb" | "idivb" | "mulb" | "imulb" => (EAX, EAX),
        // Word/long single-operand multiply/divide: edx:eax implicit.
        "div" | "divw" | "divl" | "idiv" | "idivw" | "idivl" | "mul" | "mulw"
        | "mull" => (EAX | EDX, EAX | EDX),
        // imul with a single operand is the widening form (edx:eax); the
        // two/three-operand forms name every register they touch.
        "imul" | "imulw" | "imull" if !has_comma => (EAX, EAX | EDX),
        // BMI2 mulx reads %edx implicitly; both destinations are explicit.
        "mulx" | "mulxl" | "mulxq" => (EDX, 0),
        // String instructions: %esi source, %edi destination, %eax data.  The
        // pointer registers are post-incremented, i.e. also written.
        "movsb" | "movsw" | "movsl" | "cmpsb" | "cmpsw" | "cmpsl" => (ESI | EDI, ESI | EDI),
        "stosb" | "stosw" | "stosl" | "scasb" | "scasw" | "scasl" => (EAX | EDI, EDI),
        "lodsb" | "lodsw" | "lodsl" => (ESI, EAX | ESI),
        "insb" | "insw" | "insl" | "insd" => (EDX, EDI),
        "outsb" | "outsw" | "outsl" | "outsd" => (EDX | ESI, ESI),
        "in" | "inb" | "inw" | "inl" | "ind" => (EDX, EAX),
        "out" | "outb" | "outw" | "outl" | "outd" => (EAX | EDX, 0),
        // Serialising / system instructions with fixed register interfaces.
        "cpuid" => (EAX, EAX | EBX | ECX | EDX),
        "rdtsc" => (0, EAX | EDX),
        // rdtscp additionally returns the TSC_AUX in %ecx; rdtsc does not.
        "rdtscp" => (0, EAX | ECX | EDX),
        "rdpmc" | "rdmsr" | "xgetbv" => (ECX, EAX | EDX),
        "wrmsr" | "xsetbv" => (EAX | EDX | ECX, 0),
        "monitor" | "mwait" => (EAX | ECX | EDX, 0),
        // The xsave family passes its feature mask in %edx:%eax; state goes
        // to/from memory, never to a GP register.
        "xsave" | "xsaveopt" | "xsavec" | "xsaves" | "xrstor" | "xrstors" => (EAX | EDX, 0),
        // ia32 syscall: number/return in %eax, arguments in
        // ebx/ecx/edx/esi/edi/ebp; the kernel destroys %ecx (return EIP) and
        // rewrites %eax.  Fail closed on the argument side.
        "syscall" | "sysenter" => (EAX | EBX | ECX | EDX | ESI | EDI | EBP, EAX | ECX),
        "sysexit" => (EAX | ECX | EDX, EAX | ECX | EDX),
        // Software interrupt: the handler's register contract is unknown
        // (i386 `int $0x80` passes ebx/ecx/edx/esi/edi/ebp and returns eax).
        "int" => (EAX | EBX | ECX | EDX | ESI | EDI | EBP, EAX | EBX | ECX | EDX | ESI | EDI | EBP),
        // Atomics with an implicit accumulator: %eax holds the compare value
        // and is reloaded when the exchange fails.
        "cmpxchg" | "cmpxchgb" | "cmpxchgw" | "cmpxchgl" => (EAX, EAX),
        // CMPXCHG8B compares %edx:%eax (reloaded on failure) against
        // memory; %ecx:%ebx are the store payload, read-only.
        "cmpxchg8b" | "cmpxchg16b" => (EAX | EBX | ECX | EDX, EAX | EDX),
        // XADD/ADC-style register RMW is explicit-text; only the memory form
        // has no implicit register half, so nothing is added here.
        // Misc legacy: table lookup, flags <-> %ah, counted loops.
        "xlat" | "xlatb" => (EBX, EAX),
        "lahf" => (0, EAX),
        "sahf" => (EAX, 0),
        "loop" | "loope" | "loopz" | "loopne" | "loopnz" => (ECX, ECX),
        "jecxz" => (ECX, 0),
        // Frame instructions.
        "enter" => (0, ESP | EBP),
        // `leave` copies %ebp into %esp (pure write of %esp) and pops %ebp
        // (read-modify-write of %ebp, and a read of %esp's memory).
        "leave" => (EBP, ESP | EBP),
        // pusha/pushad push all eight GP registers (including %esp's current
        // value) and decrement %esp; popa/popad pop everything except %esp.
        "pusha" | "pushad" => (ALL, ESP),
        "popa" | "popad" => (ESP, ALL & !ESP),
        _ => (0, 0),
    };
    (reads | table.0, writes | table.1)
}

/// The string primitives the `rep` prefix can legally drive (used to decide
/// whether an unknown rep form must fail closed).
fn is_known_string_mnemonic(m: &str) -> bool {
    matches!(
        m,
        "movsb"
            | "movsw"
            | "movsl"
            | "movsd"
            | "movsq"
            | "stosb"
            | "stosw"
            | "stosl"
            | "stosd"
            | "stosq"
            | "lodsb"
            | "lodsw"
            | "lodsl"
            | "lodsd"
            | "lodsq"
            | "scasb"
            | "scasw"
            | "scasl"
            | "scasd"
            | "scasq"
            | "cmpsb"
            | "cmpsw"
            | "cmpsl"
            | "cmpsd"
            | "cmpsq"
            | "insb"
            | "insw"
            | "insl"
            | "insd"
            | "outsb"
            | "outsw"
            | "outsl"
            | "outsd"
    )
}

// ── Line classifier ──────────────────────────────────────────────────────────

fn classify_line(raw: &str) -> LineInfo {
    let trim_start = raw.len() - raw.trim_start().len();
    let s = &raw[trim_start..];

    if s.is_empty() {
        return line_info(LineKind::Empty, trim_start as u16);
    }

    let bytes = s.as_bytes();
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    let ts = trim_start as u16;

    // Label
    if last == b':' {
        return line_info(LineKind::Label, ts);
    }

    // Directive
    if first == b'.' {
        return line_info(LineKind::Directive, ts);
    }

    // Comment
    if first == b'#' {
        return line_info(LineKind::Directive, ts);
    }

    // mov instructions - check store/load/self-move/reg-reg
    if first == b'm' && bytes.len() >= 4 && bytes[1] == b'o' && bytes[2] == b'v' {
        if let Some((reg_str, offset_str, size)) = parse_store_to_ebp(s) {
            let reg = register_family(reg_str);
            if reg <= REG_GP_MAX {
                let offset = parse_offset(offset_str) + slot_bias(s);
                return line_info(LineKind::StoreEbp { reg, offset, size }, ts);
            }
        }
        if let Some((offset_str, reg_str, size)) = parse_load_from_ebp(s) {
            let reg = register_family(reg_str);
            if reg <= REG_GP_MAX {
                let offset = parse_offset(offset_str) + slot_bias(s);
                return line_info(LineKind::LoadEbp { reg, offset, size }, ts);
            }
        }
        if let Some((src, dst)) = parse_reg_to_reg_move(s) {
            if src == dst {
                return line_info(LineKind::SelfMove, ts);
            }
            return line_info(LineKind::Move { dst, src }, ts);
        }
    }

    // Control flow
    if first == b'j' {
        if bytes.len() >= 4 && bytes[1] == b'm' && bytes[2] == b'p' {
            if bytes.len() > 4 && bytes[4] == b'*' {
                return line_info(LineKind::JmpIndirect, ts);
            }
            if bytes[3] == b' ' {
                if s.contains("indirect_thunk") || s.contains("*%") {
                    return line_info(LineKind::JmpIndirect, ts);
                }
                return line_info(LineKind::Jmp, ts);
            }
        }
        if is_conditional_jump(s) {
            return line_info(LineKind::CondJmp, ts);
        }
    }

    if first == b'c' {
        if bytes.len() >= 4 && bytes[1] == b'a' && bytes[2] == b'l' && bytes[3] == b'l' {
            return line_info(LineKind::Call, ts);
        }
        if bytes.len() >= 4 && bytes[1] == b'm' && bytes[2] == b'p' {
            return cmp_line_info(ts, s);
        }
    }

    if first == b'r' && s == "ret" {
        return line_info(LineKind::Ret, ts);
    }

    // test instructions
    if first == b't' && bytes.len() >= 5 && bytes[1] == b'e' && bytes[2] == b's' && bytes[3] == b't'
    {
        return cmp_line_info(ts, s);
    }

    // push/pop
    if first == b'p' {
        if let Some(rest) = s.strip_prefix("pushl ") {
            let reg = register_family(rest.trim());
            return line_info(LineKind::Push { reg }, ts);
        }
        if let Some(rest) = s.strip_prefix("popl ") {
            let reg = register_family(rest.trim());
            return line_info(LineKind::Pop { reg }, ts);
        }
    }

    // setCC
    if first == b's'
        && bytes.len() >= 4
        && bytes[1] == b'e'
        && bytes[2] == b't'
        && parse_setcc(s).is_some()
    {
        let setcc_reg = if let Some(space_pos) = s.rfind(' ') {
            register_family(s[space_pos + 1..].trim())
        } else {
            REG_EAX
        };
        return line_info(LineKind::SetCC { reg: setcc_reg }, ts);
    }

    // Other instruction
    let mut dest_reg = parse_dest_reg(s);
    // Single-operand unary RMWs (`incl %eax`, `notl %ebx`, ...) write their
    // only register operand, but `parse_dest_reg` finds no comma and returns
    // REG_NONE.  Without the dest, copy propagation lets a `movl %edx,%eax`
    // alias SURVIVE `notl %eax` and rewrites the result's consumer back to
    // the pre-not register — silently discarding the operation (v0 = ~v0
    // returned the old v0; slot_rmw_differential seed 2).  Memory operands
    // (`incl -8(%ebp)`) keep REG_NONE: they write memory, not a register.
    if dest_reg == REG_NONE {
        if let Some((mnem, operand)) = s.split_once(char::is_whitespace) {
            let op = operand.trim();
            let is_unary_rmw = matches!(
                mnem,
                "incl" | "incw" | "incb" | "decl" | "decw" | "decb"
                    | "notl" | "notw" | "notb"
                    | "negl" | "negw" | "negb"
                    // bswap writes its single operand (Agent-B audit found it
                    // missing from the original list); single-operand
                    // shifts/rotates mean shift-by-1 and write the operand
                    // (defensive: the emitter always prints an explicit $1,
                    // but hand-written asm and foreign emitters may not).
                    | "bswapl" | "bswapw"
                    | "sall" | "salw" | "salb" | "shll" | "shlw" | "shlb"
                    | "shrl" | "shrw" | "shrb" | "sarl" | "sarw" | "sarb"
                    | "roll" | "rolw" | "rolb" | "rorl" | "rorw" | "rorb"
                    | "rcll" | "rclw" | "rclb" | "rcrl" | "rcrw" | "rcrb"
            );
            if is_unary_rmw && op.starts_with('%') && !op.contains('(') {
                dest_reg = register_family(op);
            }
        }
    }
    let has_indirect = has_indirect_memory_access(s);
    let ebp_off = if has_indirect {
        EBP_OFFSET_NONE
    } else {
        parse_ebp_offset_in_line(s)
    };
    LineInfo {
        kind: LineKind::Other { dest_reg },
        trim_start: ts,
        has_indirect_mem: has_indirect,
        ebp_offset: ebp_off,
    }
}

// ── Conditional jump helpers ─────────────────────────────────────────────────

fn is_conditional_jump(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 3 || b[0] != b'j' {
        return false;
    }
    // jCC where CC is one of: e, ne, l, le, g, ge, b, be, a, ae, s, ns, o, no, p, np, z, nz
    matches!(
        &s[1..2],
        "e" | "a" | "b" | "g" | "l" | "s" | "o" | "p" | "z" | "n"
    ) && s.contains(' ')
}

/// Invert a condition code.
fn invert_cc(cc: &str) -> Option<&'static str> {
    match cc {
        "e" | "z" => Some("ne"),
        "ne" | "nz" => Some("e"),
        "l" => Some("ge"),
        "ge" => Some("l"),
        "le" => Some("g"),
        "g" => Some("le"),
        "b" => Some("ae"),
        "ae" => Some("b"),
        "be" => Some("a"),
        "a" => Some("be"),
        "s" => Some("ns"),
        "ns" => Some("s"),
        "o" => Some("no"),
        "no" => Some("o"),
        "p" => Some("np"),
        "np" => Some("p"),
        _ => None,
    }
}

/// Extract condition code and target from a conditional jump.
fn parse_condjmp(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('j') {
        return None;
    }
    let space = s.find(' ')?;
    let cc = &s[1..space];
    let target = s[space + 1..].trim();
    Some((cc, target))
}

/// Parse setCC instruction → condition code.
fn parse_setcc(s: &str) -> Option<&str> {
    if !s.starts_with("set") {
        return None;
    }
    let rest = &s[3..];
    let space = rest.find(' ')?;
    let cc = &rest[..space];
    // Validate it's a real condition code
    match cc {
        "e" | "ne" | "z" | "nz" | "l" | "le" | "g" | "ge" | "b" | "be" | "a" | "ae" | "s"
        | "ns" | "o" | "no" | "p" | "np" => Some(cc),
        _ => None,
    }
}

/// Extract the jump target from a jmp instruction.
fn parse_jmp_target(s: &str) -> Option<&str> {
    s.strip_prefix("jmp ")
}

// ── Line store ───────────────────────────────────────────────────────────────

/// Efficient line storage that avoids reallocating strings.
/// Lines are stored as byte offsets into the original assembly string.
/// Replaced lines are stored in a side buffer.
// Re-export the shared LineStore from peephole_common.
// See backend/peephole_common.rs for the implementation.
use crate::backend::peephole_common::LineStore;

// ── Trimmed line helper ──────────────────────────────────────────────────────

#[inline]
fn trimmed<'a>(store: &'a LineStore, info: &LineInfo, idx: usize) -> &'a str {
    &store.get(idx)[info.trim_start as usize..]
}

/// Source-location directives carry no machine state and may be crossed by
/// local data-flow proofs.  Other directives remain barriers: section, symbol,
/// alignment and CFI changes delimit regions even though most do not emit an
/// instruction.  Keeping `.loc` in the output preserves debug line tables;
/// treating it as transparent here merely prevents `-g` from disabling the
/// same peepholes that run without debug information.
#[inline]
fn is_debug_location(store: &LineStore, infos: &[LineInfo], idx: usize) -> bool {
    infos[idx].kind == LineKind::Directive && trimmed(store, &infos[idx], idx).starts_with(".loc ")
}

/// Check if the next instruction reads the carry flag (CF).
/// Instructions like `adcl`, `sbbl`, `rcl`, `rcr` depend on CF.
/// `incl`/`decl` do NOT set CF (unlike `addl`/`subl`), so converting
/// `addl $1` → `incl` or `subl $1` → `decl` is invalid when the next
/// instruction reads CF.
fn next_reads_carry_flag(store: &LineStore, infos: &[LineInfo], start: usize) -> bool {
    let len = infos.len();
    for j in (start + 1)..len {
        let s = store.get(j).trim();
        if s.is_empty() || s.starts_with('#') || s.starts_with("//") || s.ends_with(':') {
            continue;
        }
        // Check if the instruction reads CF
        return s.starts_with("adcl ")
            || s.starts_with("adcb ")
            || s.starts_with("adcw ")
            || s.starts_with("sbbl ")
            || s.starts_with("sbbb ")
            || s.starts_with("sbbw ")
            || s.starts_with("rcl ")
            || s.starts_with("rcr ")
            || s.starts_with("setc ")
            || s.starts_with("setb ")
            || s.starts_with("jc ")
            || s.starts_with("jb ")
            || s.starts_with("jnc ")
            || s.starts_with("jnb ")
            || s.starts_with("jae ")
            || s.starts_with("cmc");
    }
    false
}

// ── Pass 1: Local patterns ───────────────────────────────────────────────────

/// Combined local pass: scan once, apply multiple patterns.
fn combined_local_pass(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Pattern 1: Self-move elimination
        if infos[i].kind == LineKind::SelfMove {
            infos[i].kind = LineKind::Nop;
            changed = true;
            i += 1;
            continue;
        }

        // Pattern 1b: Strength reduction for code size
        // - addl $1, %reg → incl %reg (saves 2 bytes, critical for 16-bit boot code)
        // - subl $1, %reg → decl %reg (saves 2 bytes)
        // - movl $0, %reg → xorl %reg, %reg (saves 3 bytes)
        // - addl $-1, %reg → decl %reg (saves 2 bytes)
        // - subl $-1, %reg → incl %reg (saves 2 bytes)
        if let LineKind::Other { dest_reg } = infos[i].kind {
            if dest_reg != REG_NONE
                && dest_reg <= REG_GP_MAX
                && dest_reg != REG_ESP
                && dest_reg != REG_EBP
            {
                let s = trimmed(store, &infos[i], i);
                let rn = reg32_name(dest_reg);
                // addl $1, %reg → incl %reg
                // SAFETY: incl does NOT set the carry flag (CF), so this
                // conversion is invalid if the next instruction reads CF
                // (e.g., adcl used in 64-bit add-with-carry chains).
                if s.starts_with("addl $1, ")
                    && s.ends_with(rn)
                    && !next_reads_carry_flag(store, infos, i)
                {
                    store.replace(i, format!("    incl {}", rn));
                    infos[i] = LineInfo {
                        kind: LineKind::Other { dest_reg },
                        trim_start: 4,
                        has_indirect_mem: false,
                        ebp_offset: EBP_OFFSET_NONE,
                    };
                    changed = true;
                    i += 1;
                    continue;
                }
                // subl $1, %reg → decl %reg
                // SAFETY: decl does NOT set CF, skip if next reads CF.
                if s.starts_with("subl $1, ")
                    && s.ends_with(rn)
                    && !next_reads_carry_flag(store, infos, i)
                {
                    store.replace(i, format!("    decl {}", rn));
                    infos[i] = LineInfo {
                        kind: LineKind::Other { dest_reg },
                        trim_start: 4,
                        has_indirect_mem: false,
                        ebp_offset: EBP_OFFSET_NONE,
                    };
                    changed = true;
                    i += 1;
                    continue;
                }
                // addl $-1, %reg → decl %reg
                // SAFETY: decl does NOT set CF, skip if next reads CF.
                if s.starts_with("addl $-1, ")
                    && s.ends_with(rn)
                    && !next_reads_carry_flag(store, infos, i)
                {
                    store.replace(i, format!("    decl {}", rn));
                    infos[i] = LineInfo {
                        kind: LineKind::Other { dest_reg },
                        trim_start: 4,
                        has_indirect_mem: false,
                        ebp_offset: EBP_OFFSET_NONE,
                    };
                    changed = true;
                    i += 1;
                    continue;
                }
                // subl $-1, %reg → incl %reg
                // SAFETY: incl does NOT set CF, skip if next reads CF.
                if s.starts_with("subl $-1, ")
                    && s.ends_with(rn)
                    && !next_reads_carry_flag(store, infos, i)
                {
                    store.replace(i, format!("    incl {}", rn));
                    infos[i] = LineInfo {
                        kind: LineKind::Other { dest_reg },
                        trim_start: 4,
                        has_indirect_mem: false,
                        ebp_offset: EBP_OFFSET_NONE,
                    };
                    changed = true;
                    i += 1;
                    continue;
                }
            }
        }
        // movl $0, %reg → xorl %reg, %reg (saves 3 bytes, clears flags)
        if let LineKind::Other { dest_reg } = infos[i].kind {
            if dest_reg != REG_NONE
                && dest_reg <= REG_GP_MAX
                && dest_reg != REG_ESP
                && dest_reg != REG_EBP
            {
                let s = trimmed(store, &infos[i], i);
                let rn = reg32_name(dest_reg);
                if s == format!("movl $0, {}", rn) {
                    store.replace(i, format!("    xorl {}, {}", rn, rn));
                    infos[i] = LineInfo {
                        kind: LineKind::Other { dest_reg },
                        trim_start: 4,
                        has_indirect_mem: false,
                        ebp_offset: EBP_OFFSET_NONE,
                    };
                    changed = true;
                    i += 1;
                    continue;
                }
            }
        }

        // Find next non-nop line
        let mut j = i + 1;
        while j < len && (infos[j].is_nop() || is_debug_location(store, infos, j)) {
            j += 1;
        }
        if j >= len {
            i += 1;
            continue;
        }

        // Pattern 2w: immediate-to-scratch feeding a comparison --
        //   movl $IMM, %C ; cmpl %C, %R  ->  cmpl $IMM, %R
        // AT&T semantics match exactly: `cmpl %C, %R` computes R - C and
        // `cmpl $IMM, %R` computes R - IMM. Fires only when %C is dead
        // after the cmp (bounded written-before-read scan). GCC always
        // emits the immediate form; the scratch turnaround costs 5 bytes
        // and a register per comparison.
        if let LineKind::Other { dest_reg: c_reg } = infos[i].kind {
            if c_reg <= REG_GP_MAX
                && c_reg != REG_ESP
                && c_reg != REG_EBP
                && infos[j].kind == LineKind::Cmp
            {
                let s0 = trimmed(store, &infos[i], i).to_string();
                if s0.starts_with("movl $") && !s0.contains('(') {
                    let cn = reg32_name(c_reg);
                    if s0.ends_with(cn) {
                        let imm = s0[5..].split(',').next().unwrap_or("").trim().to_string();
                        let s1 = trimmed(store, &infos[j], j).to_string();
                        let pref = format!("cmpl {}, ", cn);
                        if let Some(rhs) = s1.strip_prefix(&pref) {
                            let rhs = rhs.trim();
                            if rhs.starts_with('%')
                                && !rhs.contains('(')
                                && register_family(rhs) != c_reg
                            {
                                // deadness of %C after the cmp
                                let mut k = j + 1;
                                let mut cnt = 0;
                                let mut dead = false;
                                while k < len && cnt < 24 {
                                    if infos[k].is_nop() || is_debug_location(store, infos, k) {
                                        k += 1;
                                        continue;
                                    }
                                    if infos[k].is_barrier() {
                                        break;
                                    }
                                    let s2 = trimmed(store, &infos[k], k);
                                    let written = match infos[k].kind {
                                        LineKind::Other { dest_reg } => dest_reg == c_reg,
                                        LineKind::Move { dst, .. } => dst == c_reg,
                                        LineKind::LoadEbp { reg, .. } => reg == c_reg,
                                        LineKind::SetCC { reg } => reg == c_reg,
                                        _ => false,
                                    };
                                    if written && !line_reads_dest_source(s2, c_reg) {
                                        dead = true;
                                        break;
                                    }
                                    if line_references_reg(s2, c_reg)
                                        || line_writes_reg_implicitly(s2, c_reg)
                                    {
                                        break;
                                    }
                                    k += 1;
                                    cnt += 1;
                                }
                                if dead {
                                    store.replace(j, format!("    cmpl {}, {}", imm, rhs));
                                    infos[j] = classify_line(store.get(j));
                                    infos[i].kind = LineKind::Nop;
                                    changed = true;
                                    i += 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pattern 2z: Copy propagation through a scratch register --
        //   `movl %SRC, %T; <op reading %T once>` where %T is dead after.
        // The i686 codegen funnels almost everything through %eax:
        //   movl %esi, %eax; movl %eax, 108(%esp)  ->  movl %esi, 108(%esp)
        //   movl %esi, %eax; movl %eax, %edi       ->  movl %esi, %edi
        //   movl %esi, %eax; decl %eax; movl %eax, %edi
        //        -> (first pair fuses when %eax dead after the store/move)
        // GCC/ICX -Os never materialize the scratch copy. Soundness: %T must
        // be WRITTEN before any other read after the consuming instruction
        // (bounded scan, conservative at barriers), and the consumer must
        // read %T exactly as a whole-register source.
        if let LineKind::Move {
            dst: t_reg,
            src: s_reg,
        } = infos[i].kind
        {
            if t_reg <= REG_GP_MAX
                && s_reg <= REG_GP_MAX
                && t_reg != s_reg
                && t_reg != REG_ESP
                && t_reg != REG_EBP
            {
                // The consumer is the next non-nop instruction j.
                let consumed = match infos[j].kind {
                    // movl %T, X(%esp)  (store)
                    LineKind::StoreEbp { reg, .. } if reg == t_reg => Some(1u8),
                    // movl %T, %R
                    LineKind::Move { src, dst } if src == t_reg && dst != t_reg => Some(2u8),
                    // testl %T, %T / cmpl $i, %T -- flag-setting reads.
                    // `movl %R,%eax; testl %eax,%eax; jcc` is the single
                    // most common surviving triple; GCC emits
                    // `testl %R,%R` directly.
                    LineKind::Cmp => {
                        let s2 = trimmed(store, &infos[j], j);
                        let rn = reg32_name(t_reg);
                        let both = format!("testl {}, {}", rn, rn);
                        let is_test_self = s2 == both;
                        let is_cmp_imm = s2.starts_with("cmpl $")
                            && s2.ends_with(rn)
                            && !s2[..s2.len() - rn.len()].contains('%');
                        if is_test_self || is_cmp_imm {
                            Some(3u8)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(kind2) = consumed {
                    // %T must be dead after j: written before any read.
                    let mut k = j + 1;
                    let mut cnt = 0;
                    let mut dead = false;
                    while k < len && cnt < 24 {
                        if infos[k].is_nop() || is_debug_location(store, infos, k) {
                            k += 1;
                            continue;
                        }
                        if infos[k].is_barrier() {
                            break;
                        }
                        let s2 = trimmed(store, &infos[k], k);
                        let written = match infos[k].kind {
                            LineKind::Other { dest_reg } => dest_reg == t_reg,
                            LineKind::Move { dst, .. } => dst == t_reg,
                            LineKind::LoadEbp { reg, .. } => reg == t_reg,
                            LineKind::SetCC { reg } => reg == t_reg,
                            _ => false,
                        };
                        if written && !line_reads_dest_source(s2, t_reg) {
                            dead = true;
                            break;
                        }
                        if line_references_reg(s2, t_reg) || line_writes_reg_implicitly(s2, t_reg) {
                            break;
                        }
                        k += 1;
                        cnt += 1;
                    }
                    if dead {
                        // Also: %SRC must not be MODIFIED between i and j
                        // (j is the immediate next instruction, so no gap).
                        // Rewrite the consumer to read %SRC directly and
                        // drop the copy.
                        let consumer = trimmed(store, &infos[j], j).to_string();
                        let old_reg = reg32_name(t_reg);
                        let new_reg = reg32_name(s_reg);
                        // Only whole-register 32-bit forms are rewritten;
                        // the classifier guarantees the consumer is movl.
                        if kind2 == 1 || kind2 == 2 {
                            let rewritten = consumer.replacen(old_reg, new_reg, 1);
                            if rewritten != consumer {
                                store.replace(j, format!("    {}", rewritten));
                                infos[j] = classify_line(store.get(j));
                                infos[i].kind = LineKind::Nop;
                                changed = true;
                                i += 1;
                                continue;
                            }
                        } else if kind2 == 3 {
                            // test/cmp: replace every %T occurrence.
                            let rewritten = consumer.replace(old_reg, new_reg);
                            if rewritten != consumer {
                                store.replace(j, format!("    {}", rewritten));
                                infos[j] = classify_line(store.get(j));
                                infos[i].kind = LineKind::Nop;
                                changed = true;
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Pattern 2a: Load+move fusion -- `movl SLOT, %eax; movl %eax, %REG`
        // becomes `movl SLOT, %REG` when %eax is provably dead after the
        // move (written before any read, checked by a bounded forward scan
        // that stops conservatively at barriers). The i686 codegen emits
        // this shape for nearly every slot reload (everything goes through
        // %eax first), so this is the single highest-value size peephole:
        // ICX/GCC -Os never materialize the intermediate register.
        if let LineKind::LoadEbp {
            reg: lreg,
            offset: _loff,
            size: _lsize,
        } = infos[i].kind
        {
            if let LineKind::Move {
                dst: mdst,
                src: msrc,
            } = infos[j].kind
            {
                if msrc == lreg && mdst != lreg && mdst <= REG_GP_MAX {
                    // Is lreg written before any read after j?
                    let mut k = j + 1;
                    let mut cnt = 0;
                    let mut dead = false;
                    while k < len && cnt < 24 {
                        if infos[k].is_nop() || is_debug_location(store, infos, k) {
                            k += 1;
                            continue;
                        }
                        if infos[k].is_barrier() {
                            break;
                        }
                        let s = trimmed(store, &infos[k], k);
                        let written = match infos[k].kind {
                            LineKind::Other { dest_reg } => dest_reg == lreg,
                            LineKind::Move { dst, .. } => dst == lreg,
                            LineKind::LoadEbp { reg, .. } => reg == lreg,
                            LineKind::SetCC { reg } => reg == lreg,
                            _ => false,
                        };
                        if written && !line_reads_dest_source(s, lreg) {
                            dead = true;
                            break;
                        }
                        if line_references_reg(s, lreg) || line_writes_reg_implicitly(s, lreg) {
                            break;
                        }
                        k += 1;
                        cnt += 1;
                    }
                    if dead {
                        // Rebuild using the ORIGINAL memory operand text.
                        let orig = trimmed(store, &infos[i], i).to_string();
                        if let Some(comma) = orig.find(',') {
                            let memop = &orig[..comma]; // e.g. "movl 84(%esp)"
                            let fused = format!("    {}, {}", memop, reg32_name(mdst));
                            store.replace(i, fused);
                            infos[i] = classify_line(store.get(i));
                            infos[j].kind = LineKind::Nop;
                            changed = true;
                            i += 1;
                            continue;
                        }
                    }
                }
            }
        }

        // Pattern 2b: Immediate/symbol/lea + move fusion --
        // `movl $X, %eax; movl %eax, %REG` becomes `movl $X, %REG`, and
        // `leal M(%r), %eax; movl %eax, %REG` becomes `leal M(%r), %REG`,
        // under the same deadness scan as Pattern 2a. The codegen emits
        // these shapes for every constant/address that lands in a phase-2
        // register (%ecx/%edx), e.g. `leal 68(%esp), %eax; movl %eax, %edx`
        // before every regparm intcall. GCC materializes into the final
        // register directly. The lea form requires the source ADDRESS not
        // to use %eax as base/index (it would change meaning when retargeted;
        // dest-only retarget is always safe since lea doesn't read its dest).
        if let LineKind::Other { dest_reg } = infos[i].kind {
            if dest_reg == REG_EAX {
                let src_line = trimmed(store, &infos[i], i);
                let is_imm = src_line.starts_with("movl $")
                    && src_line.ends_with("%eax")
                    && !src_line.contains('(');
                let is_lea = src_line.starts_with("leal ") && src_line.ends_with("%eax")
                    && !src_line.contains("%eax,") // %eax not a base/index
                    && !src_line.contains("(%eax");
                if is_imm {
                    // Pattern 2b-mem: immediate + slot store --
                    // `movl $X, %eax; mov{l,w,b} %eax|%ax|%al, SLOT` becomes a
                    // direct immediate store when %eax is dead afterwards.
                    // Encodings: movl $i32,disp8(%esp) is 8B vs 5+4=9B (-1),
                    // movw is 7B vs 5+5=10B (-3), movb is 5B vs 5+4=9B (-4);
                    // one instruction and the accumulator round-trip gone.
                    // W/B forms need a NUMERIC immediate to truncate; the L
                    // form may carry a symbol ($sym) unchanged.
                    if let LineKind::StoreEbp {
                        reg: sreg,
                        offset,
                        size,
                    } = infos[j].kind
                    {
                        if sreg == REG_EAX {
                            let comma = src_line.rfind(',').unwrap();
                            let imm_txt = src_line[5..comma].trim(); // "$X"
                            let imm_val: Option<i64> = imm_txt[1..].parse().ok();
                            let imm_out = match (size, imm_val) {
                                (MoveSize::L, _) => Some(imm_txt.to_string()),
                                (MoveSize::W, Some(v)) => Some(format!("${}", v as u16)),
                                (MoveSize::B, Some(v)) => Some(format!("${}", v as u8)),
                                _ => None,
                            };
                            if let Some(imm_out) = imm_out {
                                // Same bounded deadness scan as Pattern 2a,
                                // PLUS: a re-read of the same slot in the
                                // window blocks the fusion. In the unfused
                                // shape `forward_slot_loads` deletes that
                                // reload outright (%eax still holds the
                                // value); fusing here would trade a 4-byte
                                // reload for a 1-3 byte immediate saving —
                                // a net size loss.
                                let mut k = j + 1;
                                let mut cnt = 0;
                                let mut dead = false;
                                while k < len && cnt < 24 {
                                    if infos[k].is_nop() || is_debug_location(store, infos, k) {
                                        k += 1;
                                        continue;
                                    }
                                    if infos[k].is_barrier() {
                                        break;
                                    }
                                    let s = trimmed(store, &infos[k], k);
                                    if matches!(infos[k].kind,
                                        LineKind::LoadEbp { offset: o, size: ls, .. }
                                            if ranges_overlap(offset, size.byte_size(), o, ls.byte_size()))
                                        || (infos[k].ebp_offset != EBP_OFFSET_NONE
                                            && ranges_overlap(
                                                offset,
                                                size.byte_size(),
                                                infos[k].ebp_offset,
                                                16,
                                            ))
                                    {
                                        break;
                                    }
                                    // Zero idioms KILL the register; they are
                                    // not data-dependent on the old value.
                                    if s == "xorl %eax, %eax" || s == "subl %eax, %eax" {
                                        dead = true;
                                        break;
                                    }
                                    let written = match infos[k].kind {
                                        LineKind::Other { dest_reg } => dest_reg == REG_EAX,
                                        LineKind::Move { dst, .. } => dst == REG_EAX,
                                        LineKind::LoadEbp { reg, .. } => reg == REG_EAX,
                                        LineKind::SetCC { reg } => reg == REG_EAX,
                                        _ => false,
                                    };
                                    if written && !line_reads_dest_source(s, REG_EAX) {
                                        dead = true;
                                        break;
                                    }
                                    if line_references_reg(s, REG_EAX)
                                        || line_writes_reg_implicitly(s, REG_EAX)
                                    {
                                        break;
                                    }
                                    k += 1;
                                    cnt += 1;
                                }
                                if dead {
                                    let (disp, base) = if offset >= ESP_SLOT_BIAS / 2 {
                                        (offset - ESP_SLOT_BIAS, "%esp")
                                    } else {
                                        (offset, "%ebp")
                                    };
                                    let fused = format!(
                                        "    {} {}, {}({})",
                                        size.mnemonic(),
                                        imm_out,
                                        disp,
                                        base
                                    );
                                    store.replace(j, fused);
                                    infos[j] = classify_line(store.get(j));
                                    infos[i].kind = LineKind::Nop;
                                    changed = true;
                                    i += 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
                if is_imm || is_lea {
                    if let LineKind::Move {
                        dst: mdst,
                        src: msrc,
                    } = infos[j].kind
                    {
                        if msrc == REG_EAX && mdst != REG_EAX && mdst <= REG_GP_MAX {
                            // Same bounded deadness scan as Pattern 2a.
                            let mut k = j + 1;
                            let mut cnt = 0;
                            let mut dead = false;
                            while k < len && cnt < 24 {
                                if infos[k].is_nop() || is_debug_location(store, infos, k) {
                                    k += 1;
                                    continue;
                                }
                                if infos[k].is_barrier() {
                                    break;
                                }
                                let s = trimmed(store, &infos[k], k);
                                let written = match infos[k].kind {
                                    LineKind::Other { dest_reg } => dest_reg == REG_EAX,
                                    LineKind::Move { dst, .. } => dst == REG_EAX,
                                    LineKind::LoadEbp { reg, .. } => reg == REG_EAX,
                                    LineKind::SetCC { reg } => reg == REG_EAX,
                                    _ => false,
                                };
                                if written && !line_reads_dest_source(s, REG_EAX) {
                                    dead = true;
                                    break;
                                }
                                if line_references_reg(s, REG_EAX)
                                    || line_writes_reg_implicitly(s, REG_EAX)
                                {
                                    break;
                                }
                                k += 1;
                                cnt += 1;
                            }
                            if dead {
                                let comma = src_line.rfind(',').unwrap();
                                let head = &src_line[..comma]; // "movl $X" or "leal M(%r)"
                                let fused = format!("    {}, {}", head, reg32_name(mdst));
                                store.replace(i, fused);
                                infos[i] = classify_line(store.get(i));
                                infos[j].kind = LineKind::Nop;
                                changed = true;
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Pattern 2c: Zero-idiom + move fusion --
        // `xorl %eax, %eax; movl %eax, %REG` becomes `xorl %REG, %REG` under
        // the Pattern-2a deadness scan. Flags: both forms leave the same
        // arithmetic flag state (xor sets ZF/PF, clears CF/OF; the dropped
        // mov never touched flags), so even a flag-consuming successor is
        // unaffected. Codegen emits this pair for every zero landing in a
        // phase-2 register (%ecx/%edx).
        if let LineKind::Other { dest_reg } = infos[i].kind {
            if dest_reg == REG_EAX {
                let src_line = trimmed(store, &infos[i], i);
                if src_line == "xorl %eax, %eax" {
                    if let LineKind::Move {
                        dst: mdst,
                        src: msrc,
                    } = infos[j].kind
                    {
                        if msrc == REG_EAX && mdst != REG_EAX && mdst <= REG_GP_MAX {
                            let mut k = j + 1;
                            let mut cnt = 0;
                            let mut dead = false;
                            while k < len && cnt < 24 {
                                if infos[k].is_nop() || is_debug_location(store, infos, k) {
                                    k += 1;
                                    continue;
                                }
                                if infos[k].is_barrier() {
                                    break;
                                }
                                let s = trimmed(store, &infos[k], k);
                                let written = match infos[k].kind {
                                    LineKind::Other { dest_reg } => dest_reg == REG_EAX,
                                    LineKind::Move { dst, .. } => dst == REG_EAX,
                                    LineKind::LoadEbp { reg, .. } => reg == REG_EAX,
                                    LineKind::SetCC { reg } => reg == REG_EAX,
                                    _ => false,
                                };
                                if written && !line_reads_dest_source(s, REG_EAX) {
                                    dead = true;
                                    break;
                                }
                                if line_references_reg(s, REG_EAX)
                                    || line_writes_reg_implicitly(s, REG_EAX)
                                {
                                    break;
                                }
                                k += 1;
                                cnt += 1;
                            }
                            if dead {
                                let rn = reg32_name(mdst);
                                store.replace(i, format!("    xorl {}, {}", rn, rn));
                                infos[i] = classify_line(store.get(i));
                                infos[j].kind = LineKind::Nop;
                                changed = true;
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Pattern 2d: Copy-then-test collapse --
        // `movl %S, %eax; testl %S, %eax` (or testl %eax, %S) computes S&S;
        // rewrite the test to `testl %S, %S`. The mov is kept only if %eax
        // is read before being rewritten (deadness scan); the common
        // emitted shape has the branch consume the flags and reload %eax
        // immediately after, so the mov almost always dies too.
        if let LineKind::Move {
            dst: mdst,
            src: msrc,
        } = infos[i].kind
        {
            if mdst == REG_EAX && msrc != REG_EAX && msrc <= REG_GP_MAX {
                if infos[j].kind == LineKind::Cmp {
                    let t = trimmed(store, &infos[j], j);
                    let rn = reg32_name(msrc);
                    let f1 = format!("testl {}, %eax", rn);
                    let f2 = format!("testl %eax, {}", rn);
                    if t == f1 || t == f2 {
                        store.replace(j, format!("    testl {}, {}", rn, rn));
                        infos[j] = classify_line(store.get(j));
                        // Deadness scan for the mov: skip the (rewritten) test.
                        let mut k = j + 1;
                        let mut cnt = 0;
                        let mut dead = false;
                        while k < len && cnt < 24 {
                            if infos[k].is_nop() || is_debug_location(store, infos, k) {
                                k += 1;
                                continue;
                            }
                            if infos[k].is_barrier() {
                                break;
                            }
                            let s2 = trimmed(store, &infos[k], k);
                            let written = match infos[k].kind {
                                LineKind::Other { dest_reg } => dest_reg == REG_EAX,
                                LineKind::Move { dst, .. } => dst == REG_EAX,
                                LineKind::LoadEbp { reg, .. } => reg == REG_EAX,
                                LineKind::SetCC { reg } => reg == REG_EAX,
                                _ => false,
                            };
                            if written && !line_reads_dest_source(s2, REG_EAX) {
                                dead = true;
                                break;
                            }
                            if line_references_reg(s2, REG_EAX)
                                || line_writes_reg_implicitly(s2, REG_EAX)
                            {
                                break;
                            }
                            k += 1;
                            cnt += 1;
                        }
                        if dead {
                            infos[i].kind = LineKind::Nop;
                        }
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Pattern 2e: Memory read-modify-write collapse --
        // `movl SLOT, %eax; op $imm, %eax; movl %eax, SLOT` becomes
        // `op $imm, SLOT` when %eax is dead afterwards (Pattern-2a scan).
        // The store/load pair around a single immediate ALU op is codegen's
        // shape for `local op= const` on slot-resident locals; GCC emits
        // the memory-destination form directly. Also covers incl/decl.
        if let LineKind::LoadEbp {
            reg: lreg,
            offset: loff,
            size: MoveSize::L,
        } = infos[i].kind
        {
            if lreg == REG_EAX {
                // find op line j, then store line at k
                if let LineKind::Other { dest_reg: op_dst } = infos[j].kind {
                    if op_dst == REG_EAX {
                        let op_s = trimmed(store, &infos[j], j).to_string();
                        let is_imm_alu = (op_s.starts_with("addl $")
                            || op_s.starts_with("subl $")
                            || op_s.starts_with("andl $")
                            || op_s.starts_with("orl $")
                            || op_s.starts_with("xorl $")
                            || op_s.starts_with("shll $")
                            || op_s.starts_with("sarl $")
                            || op_s.starts_with("shrl $"))
                            && op_s.ends_with(", %eax");
                        let is_incdec = op_s == "incl %eax" || op_s == "decl %eax";
                        if is_imm_alu || is_incdec {
                            let mut k = j + 1;
                            while k < len
                                && (infos[k].is_nop() || is_debug_location(store, infos, k))
                            {
                                k += 1;
                            }
                            if k < len {
                                if let LineKind::StoreEbp {
                                    reg: sreg,
                                    offset: soff,
                                    size: MoveSize::L,
                                } = infos[k].kind
                                {
                                    if sreg == REG_EAX && soff == loff {
                                        // %eax deadness after the store.
                                        let mut m = k + 1;
                                        let mut cnt = 0;
                                        let mut dead = false;
                                        while m < len && cnt < 24 {
                                            if infos[m].is_nop()
                                                || is_debug_location(store, infos, m)
                                            {
                                                m += 1;
                                                continue;
                                            }
                                            if infos[m].is_barrier() {
                                                break;
                                            }
                                            let s2 = trimmed(store, &infos[m], m);
                                            let written = match infos[m].kind {
                                                LineKind::Other { dest_reg } => dest_reg == REG_EAX,
                                                LineKind::Move { dst, .. } => dst == REG_EAX,
                                                LineKind::LoadEbp { reg, .. } => reg == REG_EAX,
                                                LineKind::SetCC { reg } => reg == REG_EAX,
                                                _ => false,
                                            };
                                            if written && !line_reads_dest_source(s2, REG_EAX) {
                                                dead = true;
                                                break;
                                            }
                                            if line_references_reg(s2, REG_EAX)
                                                || line_writes_reg_implicitly(s2, REG_EAX)
                                            {
                                                break;
                                            }
                                            m += 1;
                                            cnt += 1;
                                        }
                                        if dead {
                                            // Rebuild the memory operand exactly as the
                                            // load line spelled it.
                                            let load_s = trimmed(store, &infos[i], i);
                                            let mem = load_s
                                                .strip_prefix("movl ")
                                                .and_then(|r| r.split(',').next())
                                                .unwrap_or("")
                                                .trim()
                                                .to_string();
                                            if !mem.is_empty() {
                                                let new_line = if is_incdec {
                                                    format!("    {} {}", &op_s[..4], mem)
                                                } else {
                                                    // "addl $N, %eax" -> "addl $N, MEM"
                                                    let head = &op_s[..op_s.len() - ", %eax".len()];
                                                    format!("    {}, {}", head, mem)
                                                };
                                                store.replace(j, new_line);
                                                infos[j] = classify_line(store.get(j));
                                                infos[i].kind = LineKind::Nop;
                                                infos[k].kind = LineKind::Nop;
                                                changed = true;
                                                i += 1;
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pattern 2: Adjacent store/load with same offset
        if let LineKind::StoreEbp {
            reg: store_reg,
            offset: store_off,
            size: store_size,
        } = infos[i].kind
        {
            if let LineKind::LoadEbp {
                reg: load_reg,
                offset: load_off,
                size: load_size,
            } = infos[j].kind
            {
                if store_off == load_off && store_size == load_size {
                    if store_reg == load_reg {
                        // movl %eax, -8(%ebp); movl -8(%ebp), %eax → keep store only
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                        i += 1;
                        continue;
                    } else {
                        // movl %eax, -8(%ebp); movl -8(%ebp), %ecx → movl %eax, -8(%ebp); movl %eax, %ecx
                        let new_line = format!(
                            "    {} {}, {}",
                            store_size.mnemonic(),
                            reg32_name(store_reg),
                            reg32_name(load_reg)
                        );
                        store.replace(j, new_line);
                        infos[j] = LineInfo {
                            kind: LineKind::Move {
                                dst: load_reg,
                                src: store_reg,
                            },
                            trim_start: 4,
                            has_indirect_mem: false,
                            ebp_offset: EBP_OFFSET_NONE,
                        };
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Pattern 3: Redundant jump to next label
        if infos[i].kind == LineKind::Jmp && infos[j].kind == LineKind::Label {
            let jmp_s = trimmed(store, &infos[i], i);
            let label_s = trimmed(store, &infos[j], j);
            if let Some(target) = parse_jmp_target(jmp_s) {
                if let Some(label_name) = label_s.strip_suffix(':') {
                    if target.trim() == label_name {
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Pattern 4: Branch inversion: jCC .L1; jmp .L2; .L1: → j!CC .L2; .L1:
        if infos[i].kind == LineKind::CondJmp {
            let mut k = j + 1;
            while k < len && (infos[k].is_nop() || is_debug_location(store, infos, k)) {
                k += 1;
            }
            if k < len && infos[j].kind == LineKind::Jmp && infos[k].kind == LineKind::Label {
                let cond_s = trimmed(store, &infos[i], i);
                let jmp_s = trimmed(store, &infos[j], j);
                let label_s = trimmed(store, &infos[k], k);
                if let (Some((cc, cond_target)), Some(jmp_target)) =
                    (parse_condjmp(cond_s), parse_jmp_target(jmp_s))
                {
                    if let Some(label_name) = label_s.strip_suffix(':') {
                        if cond_target == label_name {
                            if let Some(inv_cc) = invert_cc(cc) {
                                let new_line = format!("    j{} {}", inv_cc, jmp_target.trim());
                                store.replace(i, new_line);
                                infos[i].kind = LineKind::CondJmp;
                                infos[j].kind = LineKind::Nop;
                                changed = true;
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Pattern 5b: Redundant movsbl %al, %eax after movsbl (...), %eax
        // The first sign-extension already produces a properly sign-extended 32-bit result,
        // so the second `movsbl %al, %eax` is a no-op.
        if let LineKind::Other { dest_reg: REG_EAX } = infos[i].kind {
            let si = trimmed(store, &infos[i], i);
            if si.starts_with("movsbl ") && si.ends_with(", %eax") {
                if let LineKind::Other { dest_reg: REG_EAX } = infos[j].kind {
                    let sj = trimmed(store, &infos[j], j);
                    if sj == "movsbl %al, %eax" {
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Pattern 5: Reverse move elimination: movl %A, %B; movl %B, %A → keep first only
        if let LineKind::Move {
            dst: dst1,
            src: src1,
        } = infos[i].kind
        {
            if let LineKind::Move {
                dst: dst2,
                src: src2,
            } = infos[j].kind
            {
                if dst1 == src2 && src1 == dst2 {
                    infos[j].kind = LineKind::Nop;
                    changed = true;
                    i += 1;
                    continue;
                }
            }
        }

        i += 1;
    }

    changed
}

// ── Pass 2: Global store forwarding ──────────────────────────────────────────

/// Track which register value is stored at each stack slot.
/// When we see `movl %eax, -8(%ebp)`, record that slot -8 contains eax.
/// When we see `movl -8(%ebp), %ecx`, forward to `movl %eax, %ecx` or eliminate if same reg.
// TODO: Disabled - causes 21 regressions in FP computation tests (matrix/FP operations
// produce wrong numerical results). Needs investigation into FP load/store forwarding patterns.
#[allow(dead_code)]
fn global_store_forwarding(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    // Mapping: offset → (reg, line_idx)
    // Small flat array for common offsets (-256..0)
    const SLOT_COUNT: usize = 256;
    let mut slots: [(RegId, MoveSize); SLOT_COUNT] = [(REG_NONE, MoveSize::L); SLOT_COUNT];

    // Collect jump targets so we can invalidate at them
    let mut jump_targets = crate::common::fx_hash::FxHashSet::default();
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let s = trimmed(store, &infos[i], i);
        match infos[i].kind {
            LineKind::Jmp | LineKind::JmpIndirect => {
                if let Some(target) = parse_jmp_target(s) {
                    jump_targets.insert(target.trim().to_string());
                }
            }
            LineKind::CondJmp => {
                if let Some((_, target)) = parse_condjmp(s) {
                    jump_targets.insert(target.to_string());
                }
            }
            _ => {}
        }
    }

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }

        match infos[i].kind {
            LineKind::Label => {
                // Check if this label is a jump target (invalidate all)
                let s = trimmed(store, &infos[i], i);
                if let Some(name) = s.strip_suffix(':') {
                    if jump_targets.contains(name) {
                        // This label is a jump target - invalidate all mappings
                        slots = [(REG_NONE, MoveSize::L); SLOT_COUNT];
                    }
                    // If it's just a fallthrough label, keep mappings
                }
            }
            LineKind::StoreEbp { reg, offset, size } => {
                // Record that this slot now contains this register's value
                if offset < 0 && (-offset as usize) <= SLOT_COUNT {
                    slots[(-offset - 1) as usize] = (reg, size);
                }
            }
            LineKind::LoadEbp {
                reg: load_reg,
                offset,
                size: load_size,
            } => {
                // Check if we know what register value is in this slot
                let mut forwarded = false;
                if offset < 0 && (-offset as usize) <= SLOT_COUNT {
                    let (stored_reg, stored_size) = slots[(-offset - 1) as usize];
                    if stored_reg != REG_NONE && stored_size == load_size {
                        if stored_reg == load_reg {
                            // Same register - just eliminate the load
                            infos[i].kind = LineKind::Nop;
                            changed = true;
                            forwarded = true;
                        } else {
                            // Different register - forward as reg-reg move
                            let new_line = format!(
                                "    {} {}, {}",
                                load_size.mnemonic(),
                                reg32_name(stored_reg),
                                reg32_name(load_reg)
                            );
                            store.replace(i, new_line);
                            infos[i] = LineInfo {
                                kind: LineKind::Move {
                                    dst: load_reg,
                                    src: stored_reg,
                                },
                                trim_start: 4,
                                has_indirect_mem: false,
                                ebp_offset: EBP_OFFSET_NONE,
                            };
                            changed = true;
                            forwarded = true;
                        }
                    }
                }
                // The load writes to load_reg, so invalidate any slot
                // that maps to load_reg (its value has changed).
                // This must happen even if we forwarded, because the
                // destination register now has a new value.
                for slot in slots.iter_mut() {
                    if slot.0 == load_reg {
                        *slot = (REG_NONE, MoveSize::L);
                    }
                }
                if forwarded {
                    continue;
                }
            }
            LineKind::Call => {
                // Calls clobber caller-saved registers (eax, ecx, edx)
                // Invalidate all mappings involving these registers
                for slot in slots.iter_mut() {
                    if is_caller_saved(slot.0) {
                        *slot = (REG_NONE, MoveSize::L);
                    }
                }
            }
            LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret => {
                // Control flow change - invalidate all
                slots = [(REG_NONE, MoveSize::L); SLOT_COUNT];
            }
            LineKind::Move { dst, .. } => {
                // Invalidate any slot that was mapped to the overwritten register
                for slot in slots.iter_mut() {
                    if slot.0 == dst {
                        *slot = (REG_NONE, MoveSize::L);
                    }
                }
            }
            LineKind::SetCC { reg } => {
                // setCC modifies a byte register, invalidate its family
                for slot in slots.iter_mut() {
                    if slot.0 == reg {
                        *slot = (REG_NONE, MoveSize::L);
                    }
                }
            }
            LineKind::Other { dest_reg } => {
                // Invalidate any slot mapped to the destination register
                if dest_reg != REG_NONE {
                    for slot in slots.iter_mut() {
                        if slot.0 == dest_reg {
                            *slot = (REG_NONE, MoveSize::L);
                        }
                    }
                }
                // If line has indirect memory access or might clobber stack,
                // invalidate all (conservative)
                let s = trimmed(store, &infos[i], i);
                if infos[i].has_indirect_mem || s.contains("(%ebp)") {
                    // Only invalidate the specific slot if we can parse it
                    let off = infos[i].ebp_offset;
                    if off != EBP_OFFSET_NONE && off < 0 && (-off as usize) <= SLOT_COUNT {
                        slots[(-off - 1) as usize] = (REG_NONE, MoveSize::L);
                    } else if infos[i].has_indirect_mem {
                        // Indirect memory - could write anywhere, invalidate all
                        slots = [(REG_NONE, MoveSize::L); SLOT_COUNT];
                    }
                }
                // Check for inline asm or instructions that clobber multiple regs
                if s.contains(';')
                    || s.starts_with("rdmsr")
                    || s.starts_with("cpuid")
                    || s.starts_with("syscall")
                    || s.starts_with("int ")
                    || s.starts_with("int$")
                    || s.starts_with("rep")
                    || s.starts_with("cld")
                {
                    slots = [(REG_NONE, MoveSize::L); SLOT_COUNT];
                }
            }
            LineKind::Push { .. } | LineKind::Pop { .. } => {
                // Push/pop modify esp but don't affect ebp-relative slots
                if let LineKind::Pop { reg } = infos[i].kind {
                    // Pop writes to a register, invalidate mappings
                    for slot in slots.iter_mut() {
                        if slot.0 == reg {
                            *slot = (REG_NONE, MoveSize::L);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    changed
}

// ── Pass: Dead store elimination ─────────────────────────────────────────────

/// Remove stores to stack slots that are immediately overwritten.
// ── Pass: immediate store-to-load forwarding ────────────────────────────────

/// Parse `movX $imm, N(%esp|%ebp)` into (immediate value, biased slot, size).
/// These immediate homes are emitted by the accumulator backend when it stages
/// a constant argument or local initializer into a stack slot; unlike the
/// register case (`StoreEbp`) they classify as plain `Other`, so the register
/// forwarding pass never sees them.
fn parse_immediate_store(s: &str) -> Option<(i64, i32, MoveSize)> {
    let (rest, size) = if let Some(r) = s.strip_prefix("movl ") {
        (r, MoveSize::L)
    } else if let Some(r) = s.strip_prefix("movw ") {
        (r, MoveSize::W)
    } else if let Some(r) = s.strip_prefix("movb ") {
        (r, MoveSize::B)
    } else {
        return None;
    };
    let rest = rest.trim();
    if !rest.starts_with('$') {
        return None;
    }
    let comma = rest.find(',')?;
    let imm = rest[1..comma].trim();
    let mem = rest[comma + 1..].trim();
    // Plain offset(base) only — no indexed/SIB forms.
    let paren = mem.find('(')?;
    let base = &mem[paren..];
    if base.contains(',') {
        return None;
    }
    if !base.ends_with("(%ebp)") && !base.ends_with("(%esp)") {
        return None;
    }
    let off_str = &mem[..paren];
    let off = parse_offset(off_str);
    if off == EBP_OFFSET_NONE {
        return None;
    }
    let val: i64 = imm.parse().ok()?;
    Some((val, off + slot_bias(mem), size))
}

/// Forward an immediate store to a stack slot into its later reads:
/// `movl $3, slot` … `movzbl slot, %eax` becomes a direct materialization
/// (`movl $3, %eax` / `movzbl`-free), mirroring what GCC emits for a constant
/// that never needed a home. The register forwarding pass cannot do this:
/// `movl $imm, slot` is not a `StoreEbp` (its source is an immediate, not a
/// register), so it is invisible there.
///
/// Soundness is the same store-forwarding-window proof as
/// [`forward_slot_loads`]: scan forward over instructions that cannot
/// overwrite the slot or the destination register until the slot is overwritten
/// or a barrier (call / label / indirect memory / control flow) is hit, and
/// rewrite every plain reload of the slot in that window.
fn forward_immediate_slot_loads(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    const WINDOW: usize = 48;

    for i in 0..len {
        if infos[i].is_nop() || !matches!(infos[i].kind, LineKind::Other { .. }) {
            continue;
        }
        let line = trimmed(store, &infos[i], i);
        let Some((val, slot, store_size)) = parse_immediate_store(line) else {
            continue;
        };

        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() || is_debug_location(store, infos, j) {
                j += 1;
                continue;
            }
            if infos[j].is_barrier() {
                break;
            }

            // Another write (register OR immediate) to overlapping slot bytes
            // ends the constant's validity.
            let mut killed = false;
            match infos[j].kind {
                LineKind::StoreEbp { offset, size, .. } => {
                    if ranges_overlap(slot, store_size.byte_size(), offset, size.byte_size()) {
                        killed = true;
                    }
                }
                LineKind::Other { .. } => {
                    let t = trimmed(store, &infos[j], j);
                    if let Some((_, s2, sz2)) = parse_immediate_store(t) {
                        if ranges_overlap(slot, store_size.byte_size(), s2, sz2.byte_size()) {
                            killed = true;
                        }
                    } else if infos[j].has_indirect_mem || infos[j].ebp_offset != EBP_OFFSET_NONE {
                        // An indexed/aliased slot access or an unparsed memory
                        // operand that might touch the slot: stop.
                        if infos[j].has_indirect_mem {
                            killed = true;
                        } else if infos[j].ebp_offset != EBP_OFFSET_NONE
                            && ranges_overlap(slot, store_size.byte_size(), infos[j].ebp_offset, 16)
                        {
                            killed = true;
                        }
                    }
                }
                LineKind::LoadEbp { offset, size, .. } => {
                    // Only a SAME-WIDTH reload of exactly the stored bytes can
                    // become the constant.  A wider `movl` reload reads slot
                    // bytes the immediate store never wrote (adjacent locals
                    // or raw stack garbage — this pass runs over real-mode
                    // boot code where the stack is arbitrary); it is not a
                    // zero-extension, so materializing the masked constant
                    // would fabricate zero bits.  A narrower or partial-
                    // overlap read is left alone and stops tracking.
                    if ranges_overlap(slot, store_size.byte_size(), offset, size.byte_size())
                        && offset == slot
                        && size == store_size
                    {
                        // Rewrite as a direct immediate materialization into
                        // the load's destination register, keeping the load's
                        // exact width (a `movw slot,%ax` reload writes only
                        // %ax — materializing through %eax would clobber the
                        // high half).
                        if let LineKind::LoadEbp { reg: dst, .. } = infos[j].kind {
                            if dst <= REG_GP_MAX {
                                let mn = match size {
                                    MoveSize::L => "movl",
                                    MoveSize::W => "movw",
                                    MoveSize::B => "movb",
                                };
                                let dstname = match size {
                                    MoveSize::L => Some(reg32_name(dst)),
                                    MoveSize::W => narrow_reg_name(dst, MoveSize::W),
                                    MoveSize::B => narrow_reg_name(dst, MoveSize::B),
                                };
                                if let Some(dstname) = dstname {
                                    // Mask the immediate to the stored width
                                    // so a negative/wide constant stays
                                    // correct at this width.
                                    let mask: i64 = match store_size {
                                        MoveSize::L => 0xFFFF_FFFF,
                                        MoveSize::W => 0xFFFF,
                                        MoveSize::B => 0xFF,
                                    };
                                    let v = val & mask;
                                    store.replace(j, format!("    {mn} ${v}, {dstname}"));
                                    infos[j] = classify_line(store.get(j));
                                    changed = true;
                                    j += 1;
                                    count += 1;
                                    continue;
                                }
                            }
                        }
                        killed = true;
                    } else if offset == slot
                        && size == MoveSize::L
                        && store_size != MoveSize::L
                        && ranges_overlap(slot, store_size.byte_size(), offset, size.byte_size())
                    {
                        // A wider reload is materializable ONLY when a
                        // zero-extension of exactly the stored width
                        // immediately consumes it — the bytes above the
                        // store are then dead on arrival:
                        //   movb $7,S; movl S,%eax; movzbl %al,%eax
                        //     -> movl $7,%eax
                        // Without the zext the reload reads unwritten slot
                        // bytes (adjacent locals or raw stack garbage) and
                        // must stay a slot read.
                        let mut fused = false;
                        if let LineKind::LoadEbp { reg: dst, .. } = infos[j].kind {
                            if dst <= REG_GP_MAX {
                                let mut k = j + 1;
                                while k < len && infos[k].is_nop() {
                                    k += 1;
                                }
                                if let Some((zw, zsrc, zdst)) = (k < len)
                                    .then(|| parse_zext32(trimmed(store, &infos[k], k)))
                                    .flatten()
                                {
                                    if zw == store_size
                                        && zdst == dst
                                        && Some(zsrc) == narrow_reg_name(dst, store_size)
                                    {
                                        let mask: i64 = match store_size {
                                            MoveSize::W => 0xFFFF,
                                            MoveSize::B => 0xFF,
                                            MoveSize::L => unreachable!(),
                                        };
                                        let v = val & mask;
                                        store.replace(
                                            j,
                                            format!("    movl ${v}, {}", reg32_name(dst)),
                                        );
                                        infos[j] = classify_line(store.get(j));
                                        infos[k].kind = LineKind::Nop;
                                        changed = true;
                                        j = k + 1;
                                        count += 1;
                                        fused = true;
                                    }
                                }
                            }
                        }
                        if fused {
                            continue;
                        }
                        killed = true;
                    } else {
                        killed = true;
                    }
                }
                _ => {}
            }
            if killed {
                break;
            }
            // Any generic indirect access we did not classify above breaks the
            // window (it could write the slot through a pointer).
            if infos[j].has_indirect_mem {
                break;
            }
            j += 1;
            count += 1;
        }
    }
    changed
}

/// Store-to-load forwarding for stack slots: after `movl %R, N(%esp)`, a
/// later `movl N(%esp), %R2` in the same window (slot and %R untouched)
/// becomes `movl %R, %R2` -- 2 bytes instead of 4-7 -- and once every load
/// of a slot is forwarded, eliminate_never_read_stores deletes the store.
/// This is the pass that shrinks -fomit-frame-pointer code: the i686
/// codegen round-trips most IR values through slots, and the real-mode
/// setup.elf overflowed its 64-sector limit on exactly that bloat
/// (GCC -Os: 312 bytes for number(); lccc before: 1048).
///
/// Soundness: the scan stops at barriers (labels/branches/calls/push/pop/
/// %esp-writes -- see is_barrier), at any write to %R, at any overlapping
/// store to the slot, and at any indirect memory access (a taken slot
/// address could alias). Forwarding into %R itself just NOPs the reload.
fn forward_slot_loads(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    const WINDOW: usize = 48;

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let LineKind::StoreEbp {
            reg: src_reg,
            offset: slot,
            size,
        } = infos[i].kind
        else {
            continue;
        };
        if src_reg > REG_GP_MAX {
            continue;
        }
        let store_bytes = size.byte_size();

        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() || is_debug_location(store, infos, j) {
                j += 1;
                continue;
            }
            if infos[j].is_barrier() {
                break;
            }

            match infos[j].kind {
                LineKind::LoadEbp {
                    reg: dst_reg,
                    offset,
                    size: lsize,
                } if offset == slot && lsize == size => {
                    if dst_reg == src_reg {
                        // Exact reload into the same register: pure no-op.
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                        j += 1;
                        count += 1;
                        continue;
                    } else if dst_reg <= REG_GP_MAX {
                        // Width-preserving rewrite.  A word/byte reload
                        // writes ONLY the low 16/8 bits of its destination
                        // and the store only proved the low 16/8 bits of the
                        // source; emitting a 32-bit `movl %src32, %dst32`
                        // here would both clobber the destination's high
                        // half and copy source bits that never went through
                        // the slot.
                        let text = match size {
                            MoveSize::L => Some(format!(
                                "    movl {}, {}",
                                reg32_name(src_reg),
                                reg32_name(dst_reg)
                            )),
                            MoveSize::W => {
                                match (
                                    narrow_reg_name(src_reg, MoveSize::W),
                                    narrow_reg_name(dst_reg, MoveSize::W),
                                ) {
                                    (Some(s), Some(d)) => Some(format!("    movw {s}, {d}")),
                                    _ => None,
                                }
                            }
                            MoveSize::B => {
                                match (
                                    narrow_reg_name(src_reg, MoveSize::B),
                                    narrow_reg_name(dst_reg, MoveSize::B),
                                ) {
                                    (Some(s), Some(d)) => Some(format!("    movb {s}, {d}")),
                                    _ => None,
                                }
                            }
                        };
                        if let Some(text) = text {
                            store.replace(j, text);
                            infos[j] = classify_line(store.get(j));
                            changed = true;
                            j += 1;
                            count += 1;
                            continue;
                        }
                        break;
                    } else {
                        break;
                    }
                }
                // Store->load value forwarding across a slot round-trip.
                // The accumulator backend homes arguments and asm results in
                // stack slots and immediately reads them back, even though
                // the stored register still holds the value. Forward the
                // value register-to-register and let the dead-store pass drop
                // the home slot:
                //
                //  * SAME-WIDTH store/load (`movl %edx,slot` / `movl slot,%eax`,
                //    or `movw %dx,slot`/`movzwl slot,%eax`, …): the reload is
                //    a plain copy; emit movl (or movzwl for a word load, which
                //    is exactly what the load already implied).
                //  * NARROWER store feeding a WIDER unsigned reload
                //    (`movb %al,slot` / `movl slot,%eax`): emit the matching
                //    movzbl/movzwl so the zero-extension the slot round-trip
                //    implied is explicit; the redundant-zext pass then deletes
                //    it when the source is already narrow-canonical.
                //
                // The window proof below guarantees src_reg still holds the
                // stored value and the slot bytes are untouched. Byte stores
                // feeding WORD loads are not forwarded (the high byte of the
                // target word is a DIFFERENT slot byte, not zero-extension).
                LineKind::LoadEbp {
                    reg: dst_reg,
                    offset,
                    size: lsize,
                } if offset == slot && dst_reg <= REG_GP_MAX => {
                    // Only width-MISMATCHED pairs reach this arm (the
                    // same-width case is handled by the arm above).  Both
                    // mismatch directions are refused at the text level:
                    //  * narrower store, WIDER `movl` reload: the reload reads
                    //    slot bytes the store never wrote (adjacent variables
                    //    or raw stack garbage — this pass runs over real-mode
                    //    boot code), it is NOT a zero-extension, so rewriting
                    //    to movzbl/movzwl fabricates zero bits;
                    //  * wider store, narrower reload: the register is not
                    //    proven to hold the low bytes canonically.
                    // The sole sound exception is a zero-extension of exactly
                    // the stored width immediately consuming the reload —
                    // handled below.
                    // Width-mismatched (B/W store, 32-bit reload) pairs are
                    // foldable ONLY when a zero-extension of exactly the
                    // stored width immediately consumes the reload — then
                    // the slot bytes above the store are dead on arrival and
                    // the triple collapses without fabricating any bits:
                    //   movb %al,S; movl S,%eax; movzbl %al,%eax
                    //     -> movzbl %al,%eax
                    if lsize == MoveSize::L && size != MoveSize::L {
                        let mut k = j + 1;
                        while k < len && infos[k].is_nop() {
                            k += 1;
                        }
                        if let Some((zw, zsrc, zdst)) = (k < len)
                            .then(|| parse_zext32(trimmed(store, &infos[k], k)))
                            .flatten()
                        {
                            if zw == size
                                && zdst == dst_reg
                                && Some(zsrc) == narrow_reg_name(dst_reg, size)
                            {
                                if let Some(srcname) = narrow_reg_name(src_reg, size) {
                                    let mn = if size == MoveSize::B {
                                        "movzbl"
                                    } else {
                                        "movzwl"
                                    };
                                    store.replace(
                                        j,
                                        format!("    {mn} {srcname}, {}", reg32_name(dst_reg)),
                                    );
                                    infos[j] = classify_line(store.get(j));
                                    infos[k].kind = LineKind::Nop;
                                    changed = true;
                                    j = k + 1;
                                    count += 1;
                                    continue;
                                }
                            }
                        }
                    }
                    break;
                }
                LineKind::LoadEbp {
                    offset,
                    size: lsize,
                    ..
                } if ranges_overlap(slot, store_bytes, offset, lsize.byte_size()) => {
                    // Partial-overlap read: store live, no forwarding.
                    break;
                }
                LineKind::StoreEbp {
                    offset,
                    size: ssize,
                    ..
                } if ranges_overlap(slot, store_bytes, offset, ssize.byte_size()) => break,
                LineKind::Other { dest_reg } if dest_reg == src_reg => break,
                LineKind::Move { dst, .. } if dst == src_reg => break,
                LineKind::SetCC { reg } if reg == src_reg => break,
                LineKind::LoadEbp { reg, .. } if reg == src_reg => break,
                // Two-operand 32-bit ALU op reading the stored slot as a
                // full-width SOURCE operand: `op DISP(%esp), %dst` becomes
                // `op %src_reg, %dst`. Same proof obligations as the reload
                // forwarding above — the window guarantees %src_reg still
                // holds the stored value (any explicit or implicit write
                // broke the window before this line) and nothing else has
                // touched the slot bytes. This is the `define in %eax ->
                // store to home slot -> use as memory operand` idiom the
                // accumulator lowering emits constantly at -Os; forwarding
                // turns a 6-7 byte memory-operand form into a 2-3 byte
                // register form and lets eliminate_dead_stores drop the
                // store itself once no reader remains.
                LineKind::Other { dest_reg: dst_reg } if dst_reg <= REG_GP_MAX => {
                    if let Some(new_line) =
                        rewrite_alu_slot_source(trimmed(store, &infos[j], j), slot, src_reg)
                    {
                        store.replace(j, new_line);
                        infos[j] = classify_line(store.get(j));
                        changed = true;
                        j += 1;
                        count += 1;
                        continue;
                    }
                }
                _ => {}
            }
            if infos[j].has_indirect_mem {
                break;
            }
            // A folded-operand access near the slot: reads keep the store
            // live; a WRITE through a folded operand is indistinguishable
            // here, so stop conservatively either way. The access must be
            // treated as a RANGE, not a point: `fstpl 20(%esp)` writes
            // 20..28 and killed a forwarded `movl %edx, 24(%esp)` in
            // spectral_norm (sum's high word replaced by v[j]'s stale high
            // word at -m32 -O2). 16 bytes covers every x86 access (x87
            // fstpt = 10, SSE movdqu = 16).
            if infos[j].ebp_offset != EBP_OFFSET_NONE
                && ranges_overlap(slot, store_bytes, infos[j].ebp_offset, 16)
            {
                break;
            }
            // Implicit register writes the generic parser misses.
            {
                let s = trimmed(store, &infos[j], j);
                if line_writes_reg_implicitly(s, src_reg) {
                    break;
                }
            }
            j += 1;
            count += 1;
        }
    }
    changed
}

/// Canonical operand text for an ebp-space slot offset: esp-relative slots
/// are encoded as ESP_SLOT_BIAS + disp, plain frame slots as %ebp offsets.
fn slot_operand_text(slot: i32) -> String {
    if slot >= ESP_SLOT_BIAS {
        format!("{}(%esp)", slot - ESP_SLOT_BIAS)
    } else {
        format!("{}(%ebp)", slot)
    }
}

/// Rewrite `OP DISP(%esp|%ebp), %dst` — a two-operand 32-bit ALU op whose
/// FIRST operand is exactly the stored slot and whose SECOND operand is a
/// plain general-purpose register — into `OP %src, %dst`.
///
/// Returns None for every other shape: immediate-first forms (`imull $4,
/// slot` writes the slot), address takes (`leal slot, %r`), x87/SSE memory
/// ops, scaled-index accesses, and non-ALU mnemonics. Callers must
/// independently prove %src still holds the stored value at this line.
fn rewrite_alu_slot_source(s: &str, slot: i32, src_reg: RegId) -> Option<String> {
    const ALU_OPS: &[&str] = &[
        "addl", "subl", "andl", "orl", "xorl", "cmpl", "testl", "imull",
    ];
    let (mn, rest) = s.split_once(' ')?;
    if !ALU_OPS.contains(&mn) {
        return None;
    }
    let (op1, op2) = rest.split_once(',')?;
    let op1 = op1.trim();
    let op2 = op2.trim();
    if op1 != slot_operand_text(slot) {
        return None;
    }
    if !op2.starts_with('%')
        || op2.contains('(')
        || op2.starts_with('$')
        || register_family(op2) > REG_GP_MAX
    {
        return None;
    }
    Some(format!("    {} {}, {}", mn, reg32_name(src_reg), op2))
}

/// Instructions with IMPLICIT register writes that parse_dest_reg misses
/// (one-operand mul/div families, cltd, string ops, xchg).
/// Does this line READ `reg` as a source rather than only writing it?
/// `movl X, %reg` reads reg only if X mentions it; `addl $1, %reg` is a
/// read-modify-write. Used by the load+move fusion deadness check.
fn line_reads_dest_source(s: &str, reg: RegId) -> bool {
    let mn = s.split_whitespace().next().unwrap_or("");
    if mn.starts_with("mov") || mn.starts_with("lea") || mn.starts_with("set") {
        if let Some(comma) = s.rfind(',') {
            return line_references_reg(&s[..comma], reg);
        }
        // A comma-less mov-prefixed line is a string primitive (`movsl`),
        // not a pure write: fall through to the conservative answer
        // instead of claiming "reads nothing".
    }
    // Everything else: conservatively a read.
    line_references_reg(s, reg)
}

fn line_writes_reg_implicitly(s: &str, reg: RegId) -> bool {
    // Architectural implicit writes first (central oracle): `cltd`→%edx,
    // `idivl %ecx`→%eax:%edx, `rep stosb`→%edi:%ecx, `cmpxchg`'s implicit
    // %eax reload, `rdtscp`→%ecx, `leave`→%esp:%ebp, ...  Exact halves —
    // see `classify_implicit_operands`.
    if (0..=REG_GP_MAX).contains(&reg) && classify_implicit_operands(s).1 & (1 << reg) != 0 {
        return true;
    }
    let mn = s.split_whitespace().next().unwrap_or("");
    // The explicit-second-destination arms below must see through the
    // `lock` prefix: `lock xaddl %ebx, (%eax)` returns the old value in
    // %ebx — a WRITE the AT&T destination parse never sees (the last
    // operand is memory).  Missing it let copy propagation rename %ebx
    // readers across the fetch-add onto a stale pre-add value.
    let unlocked = match mn {
        "lock" | "rep" | "repe" | "repz" | "repne" | "repnz" => {
            s.split_ascii_whitespace().nth(1).unwrap_or("")
        }
        _ => mn,
    };
    match unlocked {
        // XCHG writes BOTH named operands; XADD writes the register
        // operand (it receives the old value).  classify_line only sees
        // the last operand as the destination.
        "xchgl" | "xchgw" | "xchgb" | "xaddl" | "xaddw" | "xaddb" => {
            line_references_reg(s, reg)
        }
        // Single-operand read-modify-write forms (`negl %eax`, `incl %ecx`,
        // `bswapl %eax`): classify_line derives the destination from the
        // operand AFTER the last comma — these have none, so they classify
        // as Other{dest_reg: REG_NONE} and their overwrite was invisible.
        // Missing them let `movl %edi,%eax`-aliases survive `negl %eax` and
        // rewrite later %eax readers to the un-negated %edi (alias-fuzz
        // scenario 7, struct-copy loops): lccc computed 0x10 where gcc
        // computed 0x04.
        "negl" | "negw" | "negb" | "notl" | "notw" | "notb" | "incl" | "incw" | "incb" | "decl"
        | "decw" | "decb" | "bswapl" | "bswapw"
            if mn == unlocked =>
        {
            let rest = s[mn.len()..].trim();
            !rest.contains(',') && rest.starts_with('%') && register_family(rest) == reg
        }
        _ => false,
    }
}

fn eliminate_dead_stores(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    const WINDOW: usize = 16;

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        if let LineKind::StoreEbp {
            offset: store_off,
            size: store_size,
            ..
        } = infos[i].kind
        {
            // Look ahead for another store to the same slot (meaning this one is dead)
            // or a load from the same slot (meaning this one is alive)
            let mut j = i + 1;
            let mut count = 0;
            while j < len && count < WINDOW {
                if infos[j].is_nop() || is_debug_location(store, infos, j) {
                    j += 1;
                    continue;
                }

                let store_bytes = store_size.byte_size();
                match infos[j].kind {
                    LineKind::StoreEbp { offset, size, .. }
                        if offset == store_off && size == store_size =>
                    {
                        // Another store to the exact same slot - this store is dead
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        break;
                    }
                    LineKind::StoreEbp { offset, size, .. }
                        if ranges_overlap(store_off, store_bytes, offset, size.byte_size()) =>
                    {
                        // Overlapping store but not identical - conservatively keep alive
                        break;
                    }
                    LineKind::LoadEbp { offset, size, .. }
                        if ranges_overlap(store_off, store_bytes, offset, size.byte_size()) =>
                    {
                        // Load overlaps this store's byte range - this store is alive
                        break;
                    }
                    _ => {}
                }

                // Stop at barriers
                if infos[j].is_barrier() {
                    break;
                }
                // NOTE: the stored REGISTER being modified is irrelevant
                // here. This pass proves the MEMORY slot is overwritten
                // before any read — deadness of the store — and the fate of
                // the source register cannot change that. (A register-fate
                // break belongs to store-FORWARDING, which reuses the value;
                // forward_slot_loads has one.) The old break here hid ~40%
                // of the same-block dead stores in the kernel setup corpus
                // (spill, ALU on the register, respill to the same slot).
                // Stop at indirect memory access (could read the slot)
                let s = trimmed(store, &infos[j], j);
                if infos[j].has_indirect_mem {
                    break;
                }
                // A folded-operand access overlapping the slot keeps the
                // store alive. Same range-not-point rule as the forwarding
                // pass: `fldl 16(%esp)` names offset 16 but reads 16..24,
                // so equality alone missed reads of the high dword.
                if infos[j].ebp_offset != EBP_OFFSET_NONE
                    && ranges_overlap(store_off, store_bytes, infos[j].ebp_offset, 16)
                {
                    break;
                }
                // leaq N(%ebp) takes address of slot
                if s.contains("(%ebp)")
                    && !matches!(
                        infos[j].kind,
                        LineKind::StoreEbp { .. } | LineKind::LoadEbp { .. }
                    )
                {
                    break;
                }

                j += 1;
                count += 1;
            }
        }
    }

    changed
}

// ── Pass: Dead register move elimination ─────────────────────────────────────

/// Remove register moves where the destination is overwritten before being read.
/// Substitute `old_reg` -> `new_reg` in the SOURCE operands of an AT&T
/// instruction, leaving the destination untouched.
///
/// AT&T puts the destination LAST (`orl %ecx, %eax` writes %eax), which is the
/// opposite of Intel syntax. `peephole_common::replace_source_reg_in_instruction`
/// rewrites everything after the first comma -- correct for Intel, and exactly
/// wrong here: it rewrote the destination and turned `orl %ecx,%eax` into
/// `orl %ecx,%edi`, silently computing into the wrong register.
///
/// Rules implemented:
///   * 2+ operands: every operand EXCEPT the last is a source.
///   * 1 operand: it is read-modify-write (`incl %eax`, `negl %eax`) or a
///     jump/call target -- never a pure source, so nothing is rewritten.
///   * Memory operands among the sources are rewritten too: the registers in
///     `disp(%base,%index,scale)` are read, not written.
///   * A destination that is a MEMORY operand (`movl %eax, (%ebx)`) still
///     only reads %ebx, so its registers may be substituted as well.
fn replace_att_source_reg(line: &str, old_reg: &str, new_reg: &str) -> Option<String> {
    replace_att_operand_reg(line, old_reg, new_reg, false)
}

/// Like `replace_att_source_reg`, but with `allow_dest_reads` the LAST
/// operand's bare register may also be substituted when the caller proved it
/// is only READ by this instruction (cmpl/testl write flags exclusively, so
/// their register operands are pure reads).  With `allow_dest_reads == false`
/// the behaviour is bit-for-bit the historical one: a bare register
/// destination is never touched, because substituting it would redirect a
/// WRITE.
fn replace_att_operand_reg(
    line: &str,
    old_reg: &str,
    new_reg: &str,
    allow_dest_reads: bool,
) -> Option<String> {
    use crate::backend::peephole_common::{has_whole_word, replace_whole_word};

    let leading = line.len() - line.trim_start().len();
    let (ws, body) = line.split_at(leading);
    let body = body.trim_end();
    if body.is_empty() {
        return None;
    }
    // Split mnemonic from operand list.
    let sp = body.find(|c: char| c.is_whitespace())?;
    let (mnemonic, rest) = body.split_at(sp);
    let operands = rest.trim_start();
    if operands.is_empty() {
        return None;
    }

    // Split the operand list on TOP-LEVEL commas: a memory operand contains
    // commas of its own (`4(%eax,%ebx,2)`).
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in operands.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);

    // A single operand is read-modify-write (incl/neg/not/bswap…), a branch
    // target, or a stack slot — EXCEPT for the single-operand forms whose one
    // register operand is a pure SOURCE: `divl %reg` / `idivl %reg` (the
    // divisor is read; the implicit results land in eax/edx), `push %reg`,
    // and `jmp/call *%reg`. Rewriting the source of those is exactly as safe
    // as rewriting a two-operand source. Everything else keeps refusing.
    if parts.len() < 2 {
        let p0 = parts[0].trim();
        if !(p0.starts_with('%')
            && !p0.contains('(')
            && (mnemonic.starts_with("div")
                || mnemonic.starts_with("idiv")
                || mnemonic.starts_with("push")
                || ((mnemonic == "jmp" || mnemonic == "call") && p0.starts_with('*'))))
        {
            return None;
        }
        if !has_whole_word(p0, old_reg) {
            return None;
        }
        let replaced = replace_whole_word(p0, old_reg, new_reg);
        return Some(format!("{}{}{}{}", ws, mnemonic, " ", replaced));
    }

    let last = parts.len() - 1;
    let mut hit = false;
    for (idx, part) in parts.iter_mut().enumerate() {
        let is_dest = idx == last;
        // The destination may still be a MEMORY reference, whose registers are
        // read. A bare register destination must not be touched.
        let dest_is_memory = is_dest && part.contains('(');
        if is_dest && !dest_is_memory && !allow_dest_reads {
            continue;
        }
        if has_whole_word(part, old_reg) {
            *part = replace_whole_word(part, old_reg, new_reg);
            hit = true;
        }
    }
    if !hit {
        return None;
    }
    Some(format!("{}{} {}", ws, mnemonic, parts.join(",")))
}

/// Copy propagation over register-to-register moves.
///
/// `movl %eax, %ebx` makes %ebx an alias of %eax. Every later reader of %ebx
/// can read %eax instead, as long as NEITHER register has been redefined in
/// between. Rewriting the readers usually makes the copy itself dead, and
/// `eliminate_dead_reg_moves` (which runs after this) then deletes it.
///
/// Why this matters so much on i686: the register allocator hands out a fresh
/// register for every SSA value, so a simple expression turns into a chain
///     movl 32(%esp),%eax ; movl %eax,%ebx ; movl %ebx,%ecx ; movzbl (%ecx),%eax
/// Four registers for one address. That is what forced `read_word` to save
/// ebx/esi/edi/ebp and cost 73 bytes where GCC needs 11. Breaking the chains
/// removes both the moves AND the register pressure that produced the
/// callee-save prologue -- which is the dominant cost in the 32 KiB kernel
/// setup binary.
///
/// SOUNDNESS. The rewrite is applied only while both registers are provably
/// unchanged, and the scan stops at anything that could invalidate that:
///   * any write to src or dst (including implicit ones via `Other.dest_reg`,
///     `SetCC`, `Pop`, `LoadEbp`, `Call` clobbers);
///   * any barrier -- label, jump, call, ret -- because this is a LOCAL
///     analysis with no CFG;
///   * `%esp`/`%ebp`, which addressing depends on;
///   * a partial-register read of dst (`%bl`, `%bx`) -- `line_references_reg`
///     matches those spellings, and substituting a 32-bit name for them would
///     change the operand width.
/// The substitution itself only touches SOURCE operands
/// (`replace_source_reg_in_instruction` refuses to rewrite a destination), so
/// a reader that also writes dst is never corrupted.
/// Whole-word register substitution for CONSTANT-alias propagation: the
/// alias value is an immediate/symbol text (`$.Lstr1`, `$-1`), so every
/// rewritten use must become that text.  A use of the register inside a
/// memory operand cannot express an immediate base and refuses the whole
/// line (the alias simply survives there).  With `allow_dest_reads`, a bare
/// register in last-operand position is also substituted (cmpl/testl read
/// their register operands).
fn replace_att_reg_with_text(
    line: &str,
    reg_name: &str,
    value_text: &str,
    allow_dest_reads: bool,
) -> Option<String> {
    use crate::backend::peephole_common::{has_whole_word, replace_whole_word};

    let leading = line.len() - line.trim_start().len();
    let (ws, body) = line.split_at(leading);
    let body = body.trim_end();
    if body.is_empty() {
        return None;
    }
    let sp = body.find(|c: char| c.is_whitespace())?;
    let (mnemonic, rest) = body.split_at(sp);
    let operands = rest.trim_start();
    if operands.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in operands.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);

    let last = parts.len() - 1;
    let mut hit = false;
    for (idx, part) in parts.iter_mut().enumerate() {
        let is_dest = idx == last;
        let dest_is_memory = is_dest && part.contains('(');
        if part.contains('(') && has_whole_word(part, reg_name) {
            // Register used as base/index: an immediate cannot live there.
            return None;
        }
        if is_dest && !dest_is_memory && !allow_dest_reads {
            continue;
        }
        if has_whole_word(part, reg_name) {
            *part = replace_whole_word(part, reg_name, value_text);
            hit = true;
        }
    }
    if !hit {
        return None;
    }
    Some(format!("{}{} {}", ws, mnemonic, parts.join(",")))
}

fn propagate_reg_copies(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    // Bounded like the other local passes: copies live only a few
    // instructions in practice, and this keeps the pass linear.
    const WINDOW: usize = 24;

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let (dst_reg, src_reg) = match infos[i].kind {
            LineKind::Move { dst, src } => (dst, src),
            _ => continue,
        };
        // Never rewrite through the stack or frame pointer.
        if dst_reg == REG_ESP || dst_reg == REG_EBP || src_reg == REG_ESP || src_reg == REG_EBP {
            continue;
        }
        if dst_reg == src_reg {
            continue;
        }

        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() || is_debug_location(store, infos, j) {
                j += 1;
                continue;
            }
            // No CFG here: anything that can transfer control ends the region
            // in which the alias is known to hold — EXCEPT conditional
            // branches.  A CondJmp writes no register and reads no register;
            // the lines up to the next label are its FALLTHROUGH region,
            // i.e. a direct continuation of the path that executed the
            // copy, where the alias provably holds.  Only its jump target
            // (a Label, also a barrier) starts a region with unknown entry
            // state.  This is what lets the dispatcher idiom
            // `movl %A,%B; testl %A,%A; je L; cmpl $1,%B; ...` drop the
            // staging copy: every compare in the fallthrough chain is
            // rewritten to %A and the move dies.
            if infos[j].kind != LineKind::CondJmp && infos[j].is_barrier() {
                break;
            }

            let line = trimmed(store, &infos[j], j);

            // A write to the SOURCE breaks the alias immediately -- readers
            // after this point must keep using dst.
            //
            // IMPLICIT writes matter as much as explicit ones: `idivl %ecx`
            // has no comma, so classify_line gives Other{dest_reg: REG_NONE},
            // yet it clobbers %eax AND %edx. Without the implicit check the
            // alias from `movl %ebx,%eax` survived the division and the
            // quotient consumer `movl %eax,%edi` was rewritten to
            // `movl %ebx,%edi` -- silently replacing a/b with a. Same story
            // for mull, cltd, xchg and the string ops.
            let writes_src = match infos[j].kind {
                LineKind::Move { dst, .. } => dst == src_reg,
                LineKind::LoadEbp { reg, .. } => reg == src_reg,
                LineKind::Pop { reg } => reg == src_reg,
                LineKind::SetCC { reg } => reg == src_reg,
                LineKind::Other { dest_reg } => dest_reg == src_reg,
                _ => false,
            } || line_writes_reg_implicitly(line, src_reg);
            if writes_src {
                // A write-only load/LEA reads its address before replacing the
                // destination.  The alias is therefore valid for this final
                // instruction even when the destination is `src` itself:
                //
                //   movl %eax,%ecx; movsbl (%ecx),%eax
                //       ->             movsbl (%eax),%eax
                //
                // This relay is pervasive in pointer-chasing C.  Stop after
                // the one rewrite because the source value is gone afterwards.
                let mnemonic = line.split_ascii_whitespace().next().unwrap_or("");
                if line_references_reg(line, dst_reg)
                    && (mnemonic.starts_with("mov") || mnemonic.starts_with("lea"))
                {
                    let dst_name = reg32_name(dst_reg);
                    let src_name = reg32_name(src_reg);
                    if let Some(new_line) = replace_att_source_reg(store.get(j), dst_name, src_name)
                    {
                        store.replace(j, new_line);
                        let re = classify_line(store.get(j));
                        infos[j] = LineInfo {
                            trim_start: infos[j].trim_start,
                            ..re
                        };
                        changed = true;
                    }
                }
                break;
            }

            // Rewrite this reader to use the source register.
            if line_references_reg(line, dst_reg) {
                // Only the plain 32-bit spelling is safe to substitute: a
                // partial read (`%bl`/`%bx`) would need the matching sub-name
                // of src, and getting that wrong silently changes the width.
                // cmpl/testl write ONLY flags, so a bare register in their
                // last-operand position is a pure read and may be
                // substituted like any source — this is what lets the
                // dispatcher chain `cmpl $1,%eax` (alias of %edi) become
                // `cmpl $1,%edi` so the staging move can die.
                //
                // NEVER substitute through an XCHG: its source operand is
                // simultaneously a destination (the swap writes both).
                // Rewriting `xchgl %eax,%ecx` to `xchgl %esi,%ecx` changes
                // WHICH register receives the swapped value — %eax keeps
                // the byte instead of the crc — and every later reader of
                // the pair runs on the wrong operands (simd_crc_adler:
                // crc32b staged (byte, byte) instead of (crc, byte) and the
                // checksum collapsed to 0).
                let dst_name = reg32_name(dst_reg);
                let src_name = reg32_name(src_reg);
                let is_flag_only_cmp = infos[j].kind == LineKind::Cmp;
                let is_xchg = line
                    .split_ascii_whitespace()
                    .next()
                    .is_some_and(|mn| mn.starts_with("xchg"));
                let new_line = if is_xchg {
                    None
                } else if is_flag_only_cmp {
                    replace_att_operand_reg(store.get(j), dst_name, src_name, true)
                } else {
                    replace_att_source_reg(store.get(j), dst_name, src_name)
                };
                if let Some(new_line) = new_line {
                    store.replace(j, new_line);
                    let re = classify_line(store.get(j));
                    infos[j] = LineInfo {
                        trim_start: infos[j].trim_start,
                        ..re
                    };
                    changed = true;
                }
            }

            // A write to the DESTINATION ends the alias (after any rewrite of
            // this same instruction's sources, which is why this comes last).
            let writes_dst = match infos[j].kind {
                LineKind::Move { dst, .. } => dst == dst_reg,
                LineKind::LoadEbp { reg, .. } => reg == dst_reg,
                LineKind::Pop { reg } => reg == dst_reg,
                LineKind::SetCC { reg } => reg == dst_reg,
                LineKind::Other { dest_reg } => dest_reg == dst_reg,
                _ => false,
            } || line_writes_reg_implicitly(trimmed(store, &infos[j], j), dst_reg);
            if writes_dst {
                break;
            }

            j += 1;
            count += 1;
        }
    }

    changed
}

/// Registers read and definitely written by one assembly line.  This is
/// deliberately conservative: unknown instructions are treated as reading
/// every explicitly named register and writing none.  Under-approximating
/// writes only misses dead moves; it can never delete a live one.
fn line_reg_use_def(store: &LineStore, infos: &[LineInfo], idx: usize) -> (u16, u16) {
    let info = infos[idx];
    let line = trimmed(store, &info, idx);
    let bit = |reg: RegId| -> u16 {
        if reg <= REG_GP_MAX {
            1 << reg
        } else {
            0
        }
    };
    let mut uses = 0u16;
    for reg in 0..=REG_GP_MAX {
        if line_references_reg(line, reg) {
            uses |= bit(reg);
        }
    }
    let mut defs = 0u16;
    match info.kind {
        LineKind::Move { dst, src } => {
            uses = bit(src);
            defs = bit(dst);
        }
        LineKind::LoadEbp { reg, .. } => {
            uses = bit(REG_EBP);
            defs = bit(reg);
        }
        LineKind::SetCC { reg } => {
            uses &= !bit(reg);
            defs = bit(reg);
        }
        LineKind::Pop { reg } => {
            uses &= !bit(reg);
            uses |= bit(REG_ESP);
            defs = bit(reg) | bit(REG_ESP);
        }
        LineKind::Push { .. } => {
            uses |= bit(REG_ESP);
            defs = bit(REG_ESP);
        }
        LineKind::Call => {
            // A regparm callee may consume all three incoming registers; the
            // call then clobbers the same caller-saved set.
            let caller = bit(REG_EAX) | bit(REG_ECX) | bit(REG_EDX);
            uses |= caller;
            defs = caller;
        }
        LineKind::Ret => {
            // EAX is every scalar return.  EDX liveness for wide integer
            // returns is added from the private function metadata by the CFG
            // driver below.
            uses |= bit(REG_EAX) | bit(REG_ESP);
        }
        LineKind::JmpIndirect => {
            // Computed branches and inline jump tables may select any GPR.
            uses = (1u16 << (REG_GP_MAX + 1)) - 1;
        }
        LineKind::Other { dest_reg } if dest_reg <= REG_GP_MAX => {
            let mnemonic = line.split_ascii_whitespace().next().unwrap_or("");
            let comma_count = line.bytes().filter(|&b| b == b',').count();
            let zero_idiom = mnemonic.starts_with("xor")
                && line.split_once(',').is_some_and(|(lhs, rhs)| {
                    let lhs = lhs.split_ascii_whitespace().last().unwrap_or("");
                    lhs == rhs.trim()
                });
            let write_only = mnemonic.starts_with("mov")
                || mnemonic.starts_with("lea")
                || (mnemonic.starts_with("imul") && comma_count >= 2)
                || zero_idiom;
            let read_modify_write = mnemonic.starts_with("add")
                || mnemonic.starts_with("sub")
                || mnemonic.starts_with("adc")
                || mnemonic.starts_with("sbb")
                || mnemonic.starts_with("and")
                || mnemonic.starts_with("or")
                || mnemonic.starts_with("xor")
                || mnemonic.starts_with("inc")
                || mnemonic.starts_with("dec")
                || mnemonic.starts_with("neg")
                || mnemonic.starts_with("not")
                || mnemonic.starts_with("shl")
                || mnemonic.starts_with("shr")
                || mnemonic.starts_with("sar")
                || mnemonic.starts_with("sal")
                || (mnemonic.starts_with("imul") && comma_count < 2);
            if write_only || read_modify_write {
                defs |= bit(dest_reg);
            }
            if zero_idiom {
                // `xorl %eax,%eax` has no data dependency on the old value,
                // despite spelling the destination as both operands.
                uses &= !bit(dest_reg);
            } else if write_only {
                // The destination can still occur in a source address, as in
                // `movl (%eax), %eax`; inspect the source text separately.
                let source = line.rsplit_once(',').map(|(s, _)| s).unwrap_or("");
                if !line_references_reg(source, dest_reg) {
                    uses &= !bit(dest_reg);
                }
            }
        }
        _ => {}
    }

    // Account for architectural operands omitted from AT&T syntax.  Missing a
    // definition is merely pessimistic, but missing a use can make CFG DCE
    // delete a live dividend or string pointer.  The central oracle supplies
    // the exact halves (see `classify_implicit_operands`); the XCHG arm below
    // covers the explicit second destination the oracle knows nothing about.
    let (imp_reads, imp_writes) = classify_implicit_operands(line);
    uses |= u16::from(imp_reads);
    defs |= u16::from(imp_writes);
    let mnemonic = line.split_ascii_whitespace().next().unwrap_or("");
    if matches!(mnemonic, "xchgl" | "xchgw" | "xchgb") {
        for reg in 0..=REG_GP_MAX {
            if line_references_reg(line, reg) {
                uses |= bit(reg);
                defs |= bit(reg);
            }
        }
    }
    (uses, defs)
}

fn direct_jump_target(line: &str) -> Option<&str> {
    let target = line.split_ascii_whitespace().last()?;
    if target.starts_with('.')
        || target
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic())
    {
        Some(target.trim_end_matches(','))
    } else {
        None
    }
}

/// Whole-function GPR liveness oracle shared by every peephole that must
/// prove a register DEAD before deleting or retargeting a definition.
///
/// The bounded forward scans (`fold_memory_operands` and friends) used to
/// stop at the first control-flow barrier and *assume* deadness there. That
/// assumption is only valid for accumulator scratch loads whose value the
/// emitter re-materialises after every label; it is unsound for
/// register-allocated values (`%ebx/%esi/%edi/%ebp`, and `%eax/%ecx/%edx`
/// when the allocator hands them out), which are live across labels,
/// conditional jumps and loop back-edges. `maxi(a,b){return a>b?a:b;}`
/// miscompiled exactly this way: the param load `movl 20(%esp),%esi` was
/// folded into `cmpl 20(%esp),%ebx`, and the select's fall-through arm then
/// read a never-written `%esi` (reproduced on 4927a774: the emitted `maxi`
/// returned caller garbage for `a <= b`).
///
/// Two further holes of the same barrier-scan class are closed as a side
/// effect, because the oracle models the architectural reads the scan never
/// saw: a regparm `call` READS `%eax/%ecx/%edx` (a load feeding such an
/// argument survived the fold only by luck of the barrier), and `ret` READS
/// `%eax` (+ `%edx` via the `lccc-i686-return-uses-edx` marker), so a load
/// feeding the return value is retained.
///
/// This is the same eight-register backward dataflow used by
/// `optimize_cfg_register_liveness`, factored out so every pass can ask the
/// precise question "is `reg` dead after line `i`?" instead of guessing.
/// Deleting a definition whose value is provably dead never invalidates the
/// cached answer for any *other* register, and can only make the deleted
/// register's cached liveness conservative (live-where-dead); folds rewrite
/// lines in place / nop them (never Vec::remove), so line indices — and the
/// cached vectors — stay aligned for a whole sweep. One computation per pass
/// invocation is therefore sound.
struct GprLiveness {
    /// Live-out bitmask per line (`bit(reg)`), indexed by absolute line.
    live_out: Vec<u16>,
    /// Lines outside any recognised function body are conservatively
    /// treated as "everything live".
    covered: Vec<bool>,
}

impl GprLiveness {
    const ALL: u16 = (1u16 << (REG_GP_MAX + 1)) - 1;

    /// Compute liveness for every function in the line store.
    fn compute(store: &LineStore, infos: &[LineInfo]) -> Self {
        let len = infos.len();
        let mut lv = GprLiveness {
            live_out: vec![Self::ALL; len],
            covered: vec![false; len],
        };
        let extents = function_extents(store, infos);
        for &(start, end) in &extents {
            let return_uses_edx = (start..end).any(|i| {
                !infos[i].is_nop()
                    && trimmed(store, &infos[i], i) == "# lccc-i686-return-uses-edx"
            });
            let lo = compute_gpr_live_out(store, infos, start, end, return_uses_edx);
            for local in 0..(end - start) {
                lv.live_out[start + local] = lo[local];
                lv.covered[start + local] = true;
            }
        }
        // Bare-fragment fallback: line stores without any global label (only
        // unit-test fragments in practice — the real pipeline always feeds
        // whole files whose functions carry `name:` labels). Dataflow over
        // the fragment is exact for its own content; there is no external
        // continuation that could keep a register live past the fragment end,
        // so an empty live-out at EOF is sound here. Without this fallback
        // every fold would be refused on uncovered lines (conservative ALL).
        if extents.is_empty() && len > 0 {
            let return_uses_edx = (0..len).any(|i| {
                !infos[i].is_nop()
                    && trimmed(store, &infos[i], i) == "# lccc-i686-return-uses-edx"
            });
            let lo = compute_gpr_live_out(store, infos, 0, len, return_uses_edx);
            for i in 0..len {
                lv.live_out[i] = lo[i];
                lv.covered[i] = true;
            }
        }
        lv
    }

    /// True when `reg`'s value is provably dead after line `i` executes.
    #[inline]
    fn dead_after(&self, i: usize, reg: RegId) -> bool {
        reg <= REG_GP_MAX && self.covered[i] && self.live_out[i] & (1u16 << reg) == 0
    }
}

/// `[start, end)` line ranges of every function body: from the global label
/// (`name:` not starting with `.`) up to its `.cfi_endproc`/`.size`.
/// Identical to the per-function walk in `optimize_cfg_register_liveness`.
fn function_extents(store: &LineStore, infos: &[LineInfo]) -> Vec<(usize, usize)> {
    let len = infos.len();
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < len {
        while start < len {
            if infos[start].kind == LineKind::Label {
                let line = trimmed(store, &infos[start], start);
                if line.ends_with(':') && !line.starts_with('.') {
                    break;
                }
            }
            start += 1;
        }
        if start >= len {
            break;
        }
        let mut end = start + 1;
        while end < len {
            let line = trimmed(store, &infos[end], end);
            if line == ".cfi_endproc" || line.starts_with(".size ") {
                break;
            }
            end += 1;
        }
        out.push((start, end));
        start = end;
    }
    out
}

/// Backward GPR liveness over `[start, end)`; returns the live-out bitmask
/// per line, indexed by `line - start`. Direct jumps to unknown labels are
/// treated as "all registers live" (tail calls — their ABI arguments may
/// occupy any regparm register). Indirect jumps get an ALL live-out as well:
/// `line_reg_use_def` already makes one read every GPR (jump-table index and
/// tail-call arguments), so pre-dispatch queries are fully conservative, and
/// giving the dispatch line itself an ALL live-out keeps even a query placed
/// *at* the dispatch on the sound side of the jump-table edge.
fn compute_gpr_live_out(
    store: &LineStore,
    infos: &[LineInfo],
    start: usize,
    end: usize,
    return_uses_edx: bool,
) -> Vec<u16> {
    let n = end - start;
    let mut labels = std::collections::HashMap::<&str, usize>::new();
    for i in start..end {
        if infos[i].kind == LineKind::Label {
            labels.insert(
                trimmed(store, &infos[i], i).trim_end_matches(':'),
                i - start,
            );
        }
    }
    let mut live_in = vec![0u16; n];
    let mut live_out = vec![0u16; n];
    let mut progress = true;
    while progress {
        progress = false;
        for local in (0..n).rev() {
            let i = start + local;
            if infos[i].is_nop() {
                let out = if local + 1 < n { live_in[local + 1] } else { 0 };
                if out != live_out[local] || out != live_in[local] {
                    live_out[local] = out;
                    live_in[local] = out;
                    progress = true;
                }
                continue;
            }
            let fallthrough = (local + 1 < n).then_some(local + 1);
            let target = direct_jump_target(trimmed(store, &infos[i], i))
                .and_then(|name| labels.get(name).copied());
            let mut out = 0u16;
            match infos[i].kind {
                LineKind::Ret => {}
                LineKind::JmpIndirect => {
                    // See above: jump tables / tail calls — stay conservative.
                    out = GprLiveness::ALL;
                }
                LineKind::Jmp => {
                    if let Some(target) = target {
                        out |= live_in[target];
                    } else {
                        // External direct jumps are tail calls.  Their ABI
                        // arguments may occupy any regparm register.
                        out = GprLiveness::ALL;
                    }
                }
                LineKind::CondJmp => {
                    if let Some(target) = target {
                        out |= live_in[target];
                    } else {
                        out = GprLiveness::ALL;
                    }
                    if let Some(next) = fallthrough {
                        out |= live_in[next];
                    }
                }
                _ => {
                    if let Some(next) = fallthrough {
                        out |= live_in[next];
                    }
                }
            }
            let (mut uses, defs) = line_reg_use_def(store, infos, i);
            if return_uses_edx && infos[i].kind == LineKind::Ret {
                uses |= 1u16 << REG_EDX;
            }
            let input = uses | (out & !defs);
            if out != live_out[local] || input != live_in[local] {
                live_out[local] = out;
                live_in[local] = input;
                progress = true;
            }
        }
    }
    live_out
}

/// Use whole-function GPR liveness to remove dead moves and retarget safe
/// accumulator update/copy-back chains.  The earlier bounded scans intentionally
/// stop at branches, so they cannot reason about loop backedges or both sides of
/// a conditional.  This tiny eight-register backward dataflow can.
fn optimize_cfg_register_liveness(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    consume_return_marker: bool,
) -> bool {
    let len = infos.len();
    let mut changed = false;
    let mut start = 0usize;
    while start < len {
        while start < len {
            if infos[start].kind == LineKind::Label {
                let line = trimmed(store, &infos[start], start);
                if line.ends_with(':') && !line.starts_with('.') {
                    break;
                }
            }
            start += 1;
        }
        if start >= len {
            break;
        }
        let mut end = start + 1;
        while end < len {
            let line = trimmed(store, &infos[end], end);
            if line == ".cfi_endproc" || line.starts_with(".size ") {
                break;
            }
            end += 1;
        }
        let n = end - start;
        let return_uses_edx = (start..end).any(|i| {
            !infos[i].is_nop() && trimmed(store, &infos[i], i) == "# lccc-i686-return-uses-edx"
        });
        let live_out = compute_gpr_live_out(store, infos, start, end, return_uses_edx);
        // Retarget accumulator update/copy-back chains when EAX is dead after
        // the copy.  The accumulator backend naturally emits
        //
        //   movl %ebx,%eax; incl %eax; movl %eax,%ebx
        //
        // even though x86 can update EBX directly.  CFG liveness is essential
        // for loop backedges: the following jump is a local-analysis barrier,
        // but its target commonly overwrites EAX before any read.
        let mut local = 0usize;
        while local < n {
            let i = start + local;
            let (reg, first_ok) = match infos[i].kind {
                LineKind::Move { dst: REG_EAX, src } if src <= REG_GP_MAX => (src, true),
                _ => (REG_NONE, false),
            };
            if !first_ok {
                local += 1;
                continue;
            }
            let Some(mid_local) = (local + 1..n).find(|&p| !infos[start + p].is_nop()) else {
                break;
            };
            let Some(last_local) = (mid_local + 1..n).find(|&p| !infos[start + p].is_nop()) else {
                break;
            };
            let mid = start + mid_local;
            let last = start + last_local;
            let copy_back = matches!(
                infos[last].kind,
                LineKind::Move { dst, src: REG_EAX } if dst == reg
            );
            let line = trimmed(store, &infos[mid], mid);
            let mnemonic = line.split_ascii_whitespace().next().unwrap_or("");
            let safe_update = matches!(infos[mid].kind, LineKind::Other { .. })
                && line.trim_end().ends_with("%eax")
                && (matches!(
                    mnemonic,
                    "incl"
                        | "decl"
                        | "negl"
                        | "notl"
                        | "addl"
                        | "subl"
                        | "andl"
                        | "orl"
                        | "xorl"
                        | "shll"
                        | "shrl"
                        | "sarl"
                        | "sall"
                ) || (mnemonic == "imull" && line.contains(',')));
            if copy_back && safe_update && live_out[last_local] & (1u16 << REG_EAX) == 0 {
                // EAX may also occur explicitly in a source operand, e.g.
                // `addl %eax,%eax` or `addl (%eax),%eax`.  It held `reg` at
                // this point, so every whole-register occurrence must follow
                // the retargeted value after the setup copy is removed.
                let replacement = store.get(mid).replace("%eax", reg32_name(reg));
                store.replace(mid, replacement);
                let re = classify_line(store.get(mid));
                infos[mid] = LineInfo {
                    trim_start: infos[mid].trim_start,
                    ..re
                };
                infos[i].kind = LineKind::Nop;
                infos[last].kind = LineKind::Nop;
                changed = true;
                local = last_local + 1;
            } else {
                local += 1;
            }
        }

        // Load/materialize retarget: `movl SRC, %eax; movl %eax, %REG` with
        // %eax dead after the copy becomes `movl SRC, %REG`. The windowed
        // Pattern 2a/2b twins stop at barriers, so the ubiquitous
        // `load; copy; jmp .LBBn` shape (phi-move before a branch edge)
        // never fused there — CFG liveness proves %eax dead ACROSS the
        // edge. Only the trailing ", %eax" destination is rewritten; a SRC
        // that mentions any %eax alias is skipped so the rewrite never has
        // to reason about reads of the old accumulator value.
        let mut local = 0usize;
        while local < n {
            let i = start + local;
            let retarget_ok = match infos[i].kind {
                LineKind::LoadEbp { reg: REG_EAX, .. } => true,
                LineKind::Other { dest_reg } if dest_reg == REG_EAX => {
                    let s = trimmed(store, &infos[i], i);
                    let mn = s.split_ascii_whitespace().next().unwrap_or("");
                    matches!(
                        mn,
                        "movl" | "leal" | "movzbl" | "movzwl" | "movsbl" | "movswl"
                    ) && s.ends_with(", %eax")
                }
                _ => false,
            };
            if !retarget_ok {
                local += 1;
                continue;
            }
            let s = trimmed(store, &infos[i], i);
            let Some(src_part) = s.strip_suffix(", %eax") else {
                local += 1;
                continue;
            };
            if src_part.contains("%eax")
                || src_part.contains("%ax")
                || src_part.contains("%al")
                || src_part.contains("%ah")
            {
                local += 1;
                continue;
            }
            let Some(j_local) = (local + 1..n).find(|&p| !infos[start + p].is_nop()) else {
                break;
            };
            let j = start + j_local;
            let LineKind::Move { dst, src: REG_EAX } = infos[j].kind else {
                local += 1;
                continue;
            };
            if dst == REG_EAX || dst > REG_GP_MAX || dst == REG_ESP {
                local += 1;
                continue;
            }
            if live_out[j_local] & (1u16 << REG_EAX) != 0 {
                local += 1;
                continue;
            }
            let new_line = format!("    {}, {}", src_part, reg32_name(dst));
            store.replace(i, new_line);
            let re = classify_line(store.get(i));
            infos[i] = LineInfo {
                trim_start: infos[i].trim_start,
                ..re
            };
            infos[j].kind = LineKind::Nop;
            changed = true;
            local = j_local + 1;
        }

        // Copy-then-store retarget: `movl %REG, %eax; movl %eax, SLOT`
        // with %eax dead after the store becomes `movl %REG, SLOT`. Same
        // CFG-liveness need as the load retarget above (a label or branch
        // usually follows the spill of a phi value).
        let mut local = 0usize;
        while local < n {
            let i = start + local;
            let LineKind::Move { dst: REG_EAX, src } = infos[i].kind else {
                local += 1;
                continue;
            };
            if src > REG_GP_MAX || src == REG_EAX {
                local += 1;
                continue;
            }
            let Some(j_local) = (local + 1..n).find(|&p| !infos[start + p].is_nop()) else {
                break;
            };
            let j = start + j_local;
            let LineKind::StoreEbp {
                reg: REG_EAX,
                offset,
                size: MoveSize::L,
            } = infos[j].kind
            else {
                local += 1;
                continue;
            };
            if live_out[j_local] & (1u16 << REG_EAX) != 0 {
                local += 1;
                continue;
            }
            let (disp, base) = if offset >= ESP_SLOT_BIAS / 2 {
                (offset - ESP_SLOT_BIAS, "%esp")
            } else {
                (offset, "%ebp")
            };
            store.replace(
                j,
                format!("    movl {}, {}({})", reg32_name(src), disp, base),
            );
            let re = classify_line(store.get(j));
            infos[j] = LineInfo {
                trim_start: infos[j].trim_start,
                ..re
            };
            infos[i].kind = LineKind::Nop;
            changed = true;
            local = j_local + 1;
        }

        // Compare-with-memory fold: `movl SLOT, %R; testl %R, %R` (or
        // `cmpl $imm, %R`) with %R dead after the flags op becomes
        // `cmpl $0|$imm, SLOT`. Only the FLAGS survive; the load is gone
        // and no register is consumed. testl: 4+2 -> 5 bytes (disp8);
        // cmpl $imm8: 4+3 -> 5. CFG liveness is required because the
        // consumer is almost always a jcc into another block; the
        // windowed passes cannot see across it.
        let mut local = 0usize;
        while local < n {
            let i = start + local;
            let LineKind::LoadEbp {
                reg,
                offset,
                size: MoveSize::L,
            } = infos[i].kind
            else {
                local += 1;
                continue;
            };
            if reg > REG_GP_MAX {
                local += 1;
                continue;
            }
            // Only full-width loads: a movzbl/movsbl/movzwl/movswl reads
            // 1-2 bytes, but `cmpl $imm, SLOT` would read 4.
            if !is_full_width_load(trimmed(store, &infos[i], i)) {
                local += 1;
                continue;
            }
            let Some(j_local) = (local + 1..n).find(|&p| !infos[start + p].is_nop()) else {
                break;
            };
            let j = start + j_local;
            if infos[j].kind != LineKind::Cmp {
                local += 1;
                continue;
            }
            let s = trimmed(store, &infos[j], j);
            let rn = reg32_name(reg);
            let imm: Option<&str> = if s == format!("testl {rn}, {rn}") {
                Some("$0")
            } else {
                s.strip_prefix("cmpl ")
                    .and_then(|rest| rest.strip_suffix(&format!(", {rn}")))
                    .filter(|lhs| lhs.starts_with('$') && !lhs.contains(rn))
            };
            let Some(imm) = imm else {
                local += 1;
                continue;
            };
            if live_out[j_local] & (1u16 << reg) != 0 {
                local += 1;
                continue;
            }
            let (disp, base) = if offset >= ESP_SLOT_BIAS / 2 {
                (offset - ESP_SLOT_BIAS, "%esp")
            } else {
                (offset, "%ebp")
            };
            let fused = format!("    cmpl {}, {}({})", imm, disp, base);
            store.replace(j, fused);
            let re = classify_line(store.get(j));
            infos[j] = LineInfo {
                trim_start: infos[j].trim_start,
                ..re
            };
            infos[i].kind = LineKind::Nop;
            changed = true;
            local = j_local + 1;
        }

        for local in 0..n {
            let i = start + local;
            if let LineKind::Move { dst, .. } = infos[i].kind {
                if dst <= REG_GP_MAX && live_out[local] & (1u16 << dst) == 0 {
                    infos[i].kind = LineKind::Nop;
                    changed = true;
                }
            }
            if consume_return_marker
                && !infos[i].is_nop()
                && trimmed(store, &infos[i], i) == "# lccc-i686-return-uses-edx"
            {
                infos[i].kind = LineKind::Nop;
                changed = true;
            }
        }
        start = end.saturating_add(1);
    }
    changed
}

/// Is `reg` callee-saved in the i386 SysV ABI?
///
/// %ebx, %esi, %edi and %ebp are preserved across calls, which means a write
/// to one of them that is never read before `ret` is dead: the epilogue's
/// `pop` overwrites it, and no caller can observe the value. %eax/%edx are
/// return-value registers and %esp/%ecx are not callee-saved.
fn is_callee_saved_reg(reg: RegId) -> bool {
    matches!(reg, REG_EBX | REG_ESI | REG_EDI | REG_EBP)
}

fn eliminate_dead_reg_moves(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    const WINDOW: usize = 16;

    // Detect a real frame pointer: `movl %esp, %ebp` in the prologue, or any
    // %ebp-relative addressing. Either means %ebp is load-bearing and must not
    // be treated as a dataflow register. Scanning once per call keeps this
    // O(n) and cannot be fooled by a function that only *pushes* %ebp as a
    // callee-saved register without establishing a frame.
    let frame_pointer_in_use = (0..len).any(|k| {
        if infos[k].is_nop() || is_debug_location(store, infos, k) {
            return false;
        }
        let l = trimmed(store, &infos[k], k);
        l.contains("(%ebp)") || l == "movl %esp, %ebp" || l.starts_with("movl %esp, %ebp")
    });

    // Function boundaries for the census fallback below.
    let mut bounds: Vec<(usize, usize)> = Vec::new();
    {
        let mut s = 0usize;
        for idx in 0..=len {
            let at_end = idx == len;
            let is_boundary = at_end || {
                !infos[idx].is_nop() && {
                    let t = trimmed(store, &infos[idx], idx);
                    t == ".cfi_endproc" || t.starts_with(".size ")
                }
            };
            if is_boundary {
                bounds.push((s, idx));
                s = idx;
            }
        }
    }

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let dst_reg = match infos[i].kind {
            LineKind::Move { dst, .. } => dst,
            _ => continue,
        };
        // %esp is never a dataflow register. %ebp is only off-limits while it
        // serves as the FRAME POINTER; under -fomit-frame-pointer (which the
        // kernel's 32 KiB setup code is built with) it is an ordinary
        // allocatable register, and refusing to touch it kept every dead write
        // to it alive -- together with the push/pop pair the prologue emits
        // because the allocator claimed it.
        if dst_reg == REG_ESP {
            continue;
        }
        if dst_reg == REG_EBP && frame_pointer_in_use {
            continue;
        }

        // Look ahead: if dst is overwritten before being read, this move is dead
        let mut saw_alive = false;
        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() || is_debug_location(store, infos, j) {
                j += 1;
                continue;
            }
            // A `ret` is not an opaque barrier for a callee-saved register: by
            // the ABI the caller does not observe it, and the epilogue's `pop`
            // restores it anyway. Treating `ret` like a label kept every dead
            // write to %ebx/%esi/%edi/%ebp alive, and since the prologue saves
            // exactly the registers the allocator claims, each one cost a
            // push+pop pair on top of the dead move itself.
            //
            // %eax/%edx carry the return value, so they must stay live to the
            // `ret`; the pops between here and it are handled by the Pop arm
            // below, which sees the register being redefined.
            if matches!(infos[j].kind, LineKind::Ret) {
                if is_callee_saved_reg(dst_reg) {
                    infos[i].kind = LineKind::Nop;
                    changed = true;
                }
                break;
            }
            // Stack-pointer maintenance and the matching callee-save push/pop
            // are transparent while tracking a different data register.  The
            // Pop/Push arms below understand whether that register is read or
            // overwritten.  Treating them as generic barriers made those arms
            // unreachable and kept every final dead write alive across the
            // epilogue (for example `movl %eax,%edi; ...; popl %edi`).
            if infos[j].is_barrier()
                && !matches!(
                    infos[j].kind,
                    LineKind::Push { .. }
                        | LineKind::Pop { .. }
                        | LineKind::Other { dest_reg: REG_ESP }
                )
            {
                break;
            }

            // Check if dst is read by this instruction
            let s = trimmed(store, &infos[j], j);
            // A purely-architectural write redefines dst outright: `cltd`
            // overwrites %edx (and `rdtsc` %eax, `cpuid` %ebx, ...) without
            // naming it in the operand text, so a move into the family in
            // front of it is dead.  Never fired when the instruction also
            // READS the family (`idivl` consumes %eax:%edx and the string
            // pointers are post-incremented — those are read-modify-writes,
            // not redefinitions) or names it in explicit operand text.
            let (imp_reads, imp_writes) = classify_implicit_operands(s);
            let dst_bit = 1u8 << dst_reg;
            if imp_writes & dst_bit != 0
                && imp_reads & dst_bit == 0
                && !line_references_reg_explicit(s, dst_reg)
            {
                infos[i].kind = LineKind::Nop;
                changed = true;
                break;
            }
            match infos[j].kind {
                LineKind::StoreEbp { reg, .. } if reg == dst_reg => {
                    // dst is read (stored to stack) - move is alive
                    saw_alive = true;
                    break;
                }
                // `popl %reg` OVERWRITES the register -- it does not read it.
                // Without this arm the epilogue's pops fell into the catch-all
                // below, which sees the register mentioned and concludes it is
                // live. Every dead write to a callee-saved register therefore
                // survived, e.g. the final `movl %eax,%ebp` immediately before
                // `popl %ebp`.
                LineKind::Pop { reg } => {
                    if reg == dst_reg {
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        break;
                    }
                    // Restoring a different callee-saved register does not
                    // observe or clobber the value tracked here.
                }
                // `pushl %reg` READS it: the move that produced it is alive.
                LineKind::Push { reg } if reg == dst_reg => {
                    saw_alive = true;
                    break;
                }
                LineKind::Move { src, dst } => {
                    if src == dst_reg {
                        // dst is read - move is alive
                        saw_alive = true;
                        break;
                    }
                    if dst == dst_reg {
                        // dst is overwritten - this move is dead
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        break;
                    }
                }
                LineKind::Other { dest_reg } => {
                    if dest_reg == dst_reg && !line_references_reg(s, dst_reg) {
                        // Destination only writes to dst, doesn't read it: dead
                        // But we need to make sure it doesn't ALSO read it
                        // Actually, just check if the line references the reg at all
                        // For "movl $5, %eax", dest is eax and it doesn't read eax
                        // For "addl $5, %eax", dest is eax and it reads eax
                        // parse_dest_reg returns the last operand. If the instruction writes
                        // to dst_reg but the source doesn't reference it, then this move is dead.
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        break;
                    }
                    if line_references_reg(s, dst_reg) {
                        saw_alive = true;
                        break; // dst is read
                    }
                }
                _ => {
                    if line_references_reg(s, dst_reg) {
                        saw_alive = true;
                        break; // dst is read
                    }
                }
            }

            j += 1;
            count += 1;
        }

        // Census fallback: the linear walk ended without a verdict (label,
        // ret on a caller-saved reg, window end).  A function-wide census
        // makes the deletion CFG-independent: if NO line anywhere reads the
        // destination, the move is dead no matter how control flows.
        if !saw_alive && infos[i].kind != LineKind::Nop {
            if let Some(&(fs, fe)) = bounds.iter().find(|&&(s, e)| i >= s && i < e) {
                // %eax/%edx carry the return value: every `ret` observes
                // them on the caller's side, which no in-function census
                // can see.  Refuse the fallback for the return registers
                // unless the function has no `ret` at all (noreturn tail).
                let ret_observes = matches!(dst_reg, REG_EAX | REG_EDX)
                    && (fs..fe).any(|k| infos[k].kind == LineKind::Ret && !infos[k].is_nop());
                if !ret_observes && census_reg_reads(store, &infos, fs, fe, dst_reg, &[i]) == 0 {
                    infos[i].kind = LineKind::Nop;
                    changed = true;
                }
            }
        }
    }

    changed
}

// ── Pass: Compare and branch fusion ──────────────────────────────────────────

/// Maximum number of store/load offsets tracked during compare-and-branch fusion.
const MAX_TRACKED_STORE_LOAD_OFFSETS: usize = 4;

/// Size of the instruction lookahead window for compare-and-branch fusion.
const CMP_FUSION_LOOKAHEAD: usize = 8;

/// Collect up to N non-NOP line indices following `start_idx` (exclusive).
/// Returns the number of indices collected.
fn collect_non_nop_indices<const N: usize>(
    infos: &[LineInfo],
    start_idx: usize,
    len: usize,
    out: &mut [usize; N],
) -> usize {
    let mut count = 0;
    let mut j = start_idx + 1;
    while j < len && count < N {
        if !infos[j].is_nop() {
            out[count] = j;
            count += 1;
        }
        j += 1;
    }
    count
}

/// Fuse `cmpl/testl + setCC %al + movzbl %al, %eax + [store/load] + testl %eax, %eax + jne/je`
/// into a single `jCC`/`j!CC` directly.
///
/// This enhanced version can skip over store/load pairs between the movzbl and
/// testl, allowing fusion even when the boolean is temporarily spilled to the
/// stack. It tracks stored offsets and verifies each has a matching load nearby,
/// ensuring the stored boolean is only consumed locally.
fn fuse_compare_and_branch(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].kind != LineKind::Cmp {
            i += 1;
            continue;
        }

        // Collect next non-NOP lines: cmp itself + (CMP_FUSION_LOOKAHEAD-1) following
        let mut seq_indices = [0usize; CMP_FUSION_LOOKAHEAD];
        seq_indices[0] = i;
        let mut rest = [0usize; CMP_FUSION_LOOKAHEAD - 1];
        let rest_count =
            collect_non_nop_indices::<{ CMP_FUSION_LOOKAHEAD - 1 }>(infos, i, len, &mut rest);
        seq_indices[1..(rest_count + 1)].copy_from_slice(&rest[..rest_count]);
        let seq_count = 1 + rest_count;

        if seq_count < 4 {
            i += 1;
            continue;
        }

        // Second must be setCC %al
        let setcc_cc = if let LineKind::SetCC { reg: REG_EAX } = infos[seq_indices[1]].kind {
            let s = trimmed(store, &infos[seq_indices[1]], seq_indices[1]);
            parse_setcc(s)
        } else {
            None
        };
        if setcc_cc.is_none() {
            i += 1;
            continue;
        }
        let setcc_cc = setcc_cc.unwrap();

        // Scan for testl %eax, %eax pattern.
        // Track StoreEbp offsets so we can bail out if any store's slot is
        // potentially read by another basic block (no matching load nearby).
        let mut test_idx = None;
        let mut store_offsets: [i32; MAX_TRACKED_STORE_LOAD_OFFSETS] =
            [0; MAX_TRACKED_STORE_LOAD_OFFSETS];
        let mut store_count = 0usize;
        let mut scan = 2;
        while scan < seq_count {
            let si = seq_indices[scan];
            let line = trimmed(store, &infos[si], si);

            // Skip zero-extend of setcc result
            if line == "movzbl %al, %eax" {
                scan += 1;
                continue;
            }
            // Skip store/load to ebp (pre-parsed fast check).
            if let LineKind::StoreEbp { offset, .. } = infos[si].kind {
                if store_count < MAX_TRACKED_STORE_LOAD_OFFSETS {
                    store_offsets[store_count] = offset;
                    store_count += 1;
                } else {
                    store_count = usize::MAX;
                    break;
                }
                scan += 1;
                continue;
            }
            if matches!(infos[si].kind, LineKind::LoadEbp { .. }) {
                scan += 1;
                continue;
            }
            // Skip cwtl (sign-extend ax->eax, i686 equivalent of cltq)
            if line == "cwtl" || line.starts_with("movswl ") || line.starts_with("movsbl ") {
                scan += 1;
                continue;
            }
            // Check for test
            if line == "testl %eax, %eax" {
                test_idx = Some(scan);
                break;
            }
            break;
        }

        let test_scan = match test_idx {
            Some(t) => t,
            None => {
                i += 1;
                continue;
            }
        };

        // If there are stores in the sequence, verify each has a matching load nearby.
        if store_count == usize::MAX {
            i += 1;
            continue;
        }
        if store_count > 0 {
            let range_start = seq_indices[1];
            let range_end = seq_indices[test_scan];
            let mut load_offsets: [i32; MAX_TRACKED_STORE_LOAD_OFFSETS] =
                [0; MAX_TRACKED_STORE_LOAD_OFFSETS];
            let mut load_count = 0usize;
            for ri in range_start..=range_end {
                let off = match infos[ri].kind {
                    LineKind::LoadEbp { offset, .. } => Some(offset),
                    // Check NOP'd lines too - earlier passes (store/load forwarding)
                    // may have NOP'd a load that originally matched a store.
                    LineKind::Nop => {
                        let orig = classify_line(store.get(ri));
                        match orig.kind {
                            LineKind::LoadEbp { offset, .. } => Some(offset),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(o) = off {
                    if load_count < MAX_TRACKED_STORE_LOAD_OFFSETS {
                        load_offsets[load_count] = o;
                        load_count += 1;
                    }
                }
            }
            let has_unmatched_store = (0..store_count)
                .any(|si| !(0..load_count).any(|li| load_offsets[li] == store_offsets[si]));
            if has_unmatched_store {
                i += 1;
                continue;
            }
        }

        if test_scan + 1 >= seq_count {
            i += 1;
            continue;
        }

        // Find jne/je after test
        let jmp_line = trimmed(
            store,
            &infos[seq_indices[test_scan + 1]],
            seq_indices[test_scan + 1],
        );
        let (is_jne, branch_target) = if let Some(target) = jmp_line.strip_prefix("jne ") {
            (true, target.trim())
        } else if let Some(target) = jmp_line.strip_prefix("je ") {
            (false, target.trim())
        } else {
            i += 1;
            continue;
        };

        let fused_cc = if is_jne {
            setcc_cc
        } else {
            match invert_cc(setcc_cc) {
                Some(inv) => inv,
                None => {
                    i += 1;
                    continue;
                }
            }
        };

        let fused_jcc = format!("    j{} {}", fused_cc, branch_target);

        // NOP out everything from setCC through testl
        for s in 1..=test_scan {
            infos[seq_indices[s]].kind = LineKind::Nop;
        }
        // Replace the jne/je with the fused conditional jump
        let idx = seq_indices[test_scan + 1];
        store.replace(idx, fused_jcc);
        infos[idx] = LineInfo {
            kind: LineKind::CondJmp,
            trim_start: 4,
            has_indirect_mem: false,
            ebp_offset: EBP_OFFSET_NONE,
        };

        changed = true;
        i = idx + 1;
    }

    changed
}

// ── Pass: setCC-bool test/branch fusion ──────────────────────────────────────

/// Fuse `setCC %r8; movzbl %r8, %r32; [movl relays]; testl A, B; {je|jne}`:
/// the test is REDUNDANT — `movzbl` and `movl` preserve EFLAGS, so the
/// original producer's flags (the ones the setCC consumed) are still live at
/// the conditional jump, and `je`-after-test ⇔ (bool == 0) ⇔ CC false.
///
/// The i64 `res & mask != 0` gates (kstrtoull's overflow check) lower to
/// exactly this shape with a register relay: `orl; setne %al; movzbl;
/// movl %eax,%edx; testl %eax,%edx; je` — none of which the Cmp-headed
/// fuse_compare_and_branch or the and/or/xor-headed
/// eliminate_redundant_test_i686 can match (cross-register test + relay).
///
/// When the bool is dead after the branch (bounded written-before-read scan
/// for every carrier register, no unmatched slot stores — the window admits
/// none), the whole setCC..relay window is NOPed; otherwise only the test
/// is dropped. In both cases the jcc is rewritten: `je`→`j<inv(CC)>`,
/// `jne`→`j<CC>` on the surviving producer flags.
///
/// Soundness gates:
///  * the window (setCC..test) contains ONLY flag-preserving carrier moves
///    (movzbl same-family, movl reg-reg from a carrier) — any other line,
///    label, or branch bails;
///  * the tested operands are both carriers of the bool;
///  * after the jcc, a bounded scan requires a flags WRITER before any
///    flags READER — otherwise a later reader could observe the producer's
///    flags where the test's were intended (strictly stricter than the
///    existing fuse_compare_and_branch, which relies on the emitter's
///    always-produce-before-consume discipline alone).
fn fuse_setcc_branch(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    if std::env::var_os("CCC_NO_SETCC_BRANCH").is_some() {
        return false;
    }
    /// Instruction mnemonics that WRITE the flags register.
    fn is_flags_writer(s: &str) -> bool {
        let m = s.split_whitespace().next().unwrap_or("");
        matches!(
            m,
            "addl"
                | "addw"
                | "addb"
                | "subl"
                | "subw"
                | "subb"
                | "adcl"
                | "sbbl"
                | "andl"
                | "andw"
                | "andb"
                | "orl"
                | "orw"
                | "orb"
                | "xorl"
                | "xorw"
                | "xorb"
                | "cmpl"
                | "cmpw"
                | "cmpb"
                | "testl"
                | "testw"
                | "testb"
                | "shll"
                | "shrl"
                | "sarl"
                | "roll"
                | "rorl"
                | "rcll"
                | "rcrl"
                | "imull"
                | "mull"
                | "idivl"
                | "divl"
                | "incl"
                | "decl"
                | "negl"
                | "negw"
                | "negb"
        )
    }
    /// Lines whose EXECUTION READS the flags (jcc/setcc/cmov).
    ///
    /// The mnemonic check is authoritative, not the classifier: the
    /// LineKind-based arm misses synonyms the classifier folds into
    /// Other — `is_conditional_jump` keys on the second character, so
    /// `jc` classifies as Other while `jnc` classifies as CondJmp. A
    /// flags reader that slips through here would let the fusion hand
    /// it the producer's flags instead of the dropped test's (caught by
    /// setcc_fuse_refuses_jcc_reader_after_branch with a `jc`).
    fn is_flags_reader(kind: LineKind, s: &str) -> bool {
        matches!(kind, LineKind::CondJmp | LineKind::SetCC { .. })
            || is_cond_jcc(s)
            || s.split_whitespace()
                .next()
                .unwrap_or("")
                .starts_with("cmov")
    }

    let len = infos.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        let (cc, byte_reg) = match infos[i].kind {
            LineKind::SetCC { reg } => {
                let s = trimmed(store, &infos[i], i);
                match parse_setcc(s) {
                    Some(cc) if reg <= REG_GP_MAX => (cc, reg),
                    _ => {
                        i += 1;
                        continue;
                    }
                }
            }
            _ => {
                i += 1;
                continue;
            }
        };
        // Next non-nop: movz{b,w}l %r8, %r32 (same family) — the bool lands
        // in the full register.
        let mut j = i + 1;
        while j < len && (infos[j].is_nop() || is_debug_location(store, infos, j)) {
            j += 1;
        }
        if j >= len {
            break;
        }
        let mz = trimmed(store, &infos[j], j);
        let carrier = if let Some(rest) = mz.strip_prefix("movzbl ") {
            let mut it = rest.split(',');
            let src = it.next().unwrap_or("").trim();
            let dst = it.next().unwrap_or("").trim();
            if it.next().is_some() || !src.starts_with('%') || !dst.starts_with('%') {
                None
            } else {
                let sf = register_family(src);
                let df = register_family(dst);
                if sf == byte_reg && sf == df && df <= REG_GP_MAX {
                    Some(df)
                } else {
                    None
                }
            }
        // movzwl is deliberately NOT accepted here: setCC writes only the
        // low BYTE of its destination, so `movzwl %ax, %r32` would import
        // whatever the high byte of %ax held into the bool — the fused test
        // (and the branch rewritten onto the producer CC) would then observe
        // garbage bits, not the 0/1 bool. movzbl is the only zero-extend of
        // a setCC result that is exact by construction.
        } else {
            None
        };
        let Some(carrier) = carrier else {
            i += 1;
            continue;
        };
        // Window: movl relays then the test. No stores, labels, branches.
        let mut carriers = [REG_NONE; 4];
        carriers[0] = carrier;
        let mut carrier_count = 1usize;
        let mut window: Vec<usize> = Vec::new();
        let mut test_idx = None;
        let mut k = j + 1;
        let mut scanned = 0;
        while k < len && scanned < 6 {
            if infos[k].is_nop() || is_debug_location(store, infos, k) {
                k += 1;
                continue;
            }
            if infos[k].is_barrier() {
                break;
            }
            let s = trimmed(store, &infos[k], k);
            if let Some(rest) = s.strip_prefix("movl ") {
                let mut it = rest.split(',');
                let src = it.next().unwrap_or("").trim();
                let dst = it.next().unwrap_or("").trim();
                let sf = register_family(src);
                let df = register_family(dst);
                if it.next().is_some()
                    || !src.starts_with('%')
                    || !dst.starts_with('%')
                    || sf == REG_NONE
                    || sf > REG_GP_MAX
                    || df > REG_GP_MAX
                    || dst.contains('(')
                {
                    break; // slot store or odd form: bail (strict window)
                }
                if (0..carrier_count).any(|c| carriers[c] == sf) {
                    if df != sf && (0..carrier_count).all(|c| carriers[c] != df) {
                        if carrier_count < carriers.len() {
                            carriers[carrier_count] = df;
                            carrier_count += 1;
                            window.push(k);
                            k += 1;
                            scanned += 1;
                            continue;
                        }
                    } else if df == sf {
                        // movl %eax, %eax: no-op relay, skip
                        k += 1;
                        scanned += 1;
                        continue;
                    }
                }
                break;
            }
            if let Some(rest) = s.strip_prefix("testl ") {
                let mut it = rest.split(',');
                let a = it.next().unwrap_or("").trim();
                let b = it.next().unwrap_or("").trim();
                if it.next().is_some() || !a.starts_with('%') || !b.starts_with('%') {
                    break;
                }
                let af = register_family(a);
                let bf = register_family(b);
                let a_c = (0..carrier_count).any(|c| carriers[c] == af);
                let b_c = (0..carrier_count).any(|c| carriers[c] == bf);
                if a_c && b_c {
                    test_idx = Some(k);
                    k += 1;
                    break;
                }
            }
            break;
        }
        let Some(test_idx) = test_idx else {
            i += 1;
            continue;
        };
        // Next non-nop after the test: je/jne.
        let mut t = test_idx + 1;
        while t < len && (infos[t].is_nop() || is_debug_location(store, infos, t)) {
            t += 1;
        }
        if t >= len {
            i += 1;
            continue;
        }
        let jl = trimmed(store, &infos[t], t);
        let (is_jne, target) = if let Some(x) = jl.strip_prefix("jne ") {
            (true, x.trim())
        } else if let Some(x) = jl.strip_prefix("je ") {
            (false, x.trim())
        } else {
            i += 1;
            continue;
        };
        // Post-jcc guard: a flags writer must precede any flags reader on
        // the fallthrough path. ORDER MATTERS: the reader check runs BEFORE
        // the barrier check because a CondJmp is both a reader and a
        // barrier — after the rewrite it would consume the producer's
        // flags instead of the dropped test's. Labels are SKIPPED, not
        // barriers: the fallthrough physically continues into the next
        // block, so a reader there sees the changed flags too. The scan
        // still stops at true control/analysis boundaries (call destroys
        // the flags entirely; jmp/ret leave the path; push/pop/esp fences
        // are handled by their own barrier classes) and at any flag-
        // preserving instruction it keeps scanning.
        let mut ok_after = true;
        let mut m = t + 1;
        let mut cnt = 0;
        while m < len && cnt < 16 {
            if infos[m].is_nop() || is_debug_location(store, infos, m) {
                m += 1;
                continue;
            }
            let s2 = trimmed(store, &infos[m], m);
            if is_flags_writer(s2) {
                break;
            }
            if is_flags_reader(infos[m].kind, s2) {
                ok_after = false;
                break;
            }
            if infos[m].kind == LineKind::Label {
                m += 1;
                continue;
            }
            if infos[m].is_barrier() {
                break;
            }
            m += 1;
            cnt += 1;
        }
        if !ok_after {
            i += 1;
            continue;
        }
        // Bool deadness: every carrier written-before-read after the jcc.
        let mut bool_dead = true;
        for c in (0..carrier_count).map(|c| carriers[c]) {
            let mut k2 = t + 1;
            let mut cnt2 = 0;
            let mut dead = false;
            while k2 < len && cnt2 < 24 {
                if infos[k2].is_nop() || is_debug_location(store, infos, k2) {
                    k2 += 1;
                    continue;
                }
                if infos[k2].is_barrier() {
                    break;
                }
                let s3 = trimmed(store, &infos[k2], k2);
                // A conditional branch of ANY synonym spelling ends the
                // provable region: its target may rejoin and read the
                // carrier. (The classifier may fold rare spellings like
                // `jc` into Other — is_cond_jcc is the authoritative
                // mnemonic list.)
                if is_cond_jcc(s3) {
                    break;
                }
                let written = match infos[k2].kind {
                    LineKind::Other { dest_reg } => dest_reg == c,
                    LineKind::Move { dst, .. } => dst == c,
                    LineKind::LoadEbp { reg, .. } => reg == c,
                    LineKind::SetCC { reg } => reg == c,
                    _ => false,
                };
                if written && !line_reads_dest_source(s3, c) {
                    dead = true;
                    break;
                }
                if line_references_reg(s3, c) || line_writes_reg_implicitly(s3, c) {
                    break;
                }
                k2 += 1;
                cnt2 += 1;
            }
            if !dead {
                bool_dead = false;
                break;
            }
        }
        // Rewrite: drop the test; rewrite the jcc onto the producer flags.
        let fused_cc = if is_jne {
            cc.to_string()
        } else {
            match invert_cc(cc) {
                Some(inv) => inv.to_string(),
                None => {
                    i += 1;
                    continue;
                }
            }
        };
        infos[test_idx].kind = LineKind::Nop;
        if bool_dead {
            infos[i].kind = LineKind::Nop; // setCC
            infos[j].kind = LineKind::Nop; // movzbl
            for w in &window {
                infos[*w].kind = LineKind::Nop; // relays
            }
        }
        store.replace(t, format!("    j{} {}", fused_cc, target));
        infos[t] = LineInfo {
            kind: LineKind::CondJmp,
            trim_start: 4,
            has_indirect_mem: false,
            ebp_offset: EBP_OFFSET_NONE,
        };
        changed = true;
        i = t + 1;
    }
    changed
}

// ── Pass: redundant testl-after-logical elimination ─────────────────────────

/// `andl`/`orl` (any operand form) and the `xorl %R,%R` zero idiom set
/// ZF/SF/PF from the result and clear CF/OF — the EXACT flag state that a
/// following `testl %R, %R` would produce.  The test is therefore pure
/// overhead: delete it (2 bytes per site; 29 sites in the kernel boot
/// corpus, e.g. cpucheck's `andl $536870912,%eax; testl %eax,%eax; je`).
///
/// Only AND/OR/XOR-self qualify: shifts leak CF/OF from the operand, and
/// ADD/SUB/NEG set CF/OF from carry — neither matches TEST's flag contract.
/// The producer and the test must be adjacent (nothing may perturb flags
/// or the register between them), and the test's size must match.
fn eliminate_redundant_test_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    /// If `s` is an AND/OR with any source, or a same-register XOR zero
    /// idiom, writing a result of `size` to `dest` — i.e. an instruction
    /// whose flags are exactly `test dest,dest` over that result — return
    /// (size, dest).
    fn flag_equiv_producer(s: &str) -> Option<(MoveSize, RegId)> {
        let candidates: &[(&str, MoveSize)] = &[
            ("andl", MoveSize::L),
            ("orl", MoveSize::L),
            ("xorl", MoveSize::L),
            ("andw", MoveSize::W),
            ("orw", MoveSize::W),
            ("xorw", MoveSize::W),
            ("andb", MoveSize::B),
            ("orb", MoveSize::B),
        ];
        for (mnem, size) in candidates {
            if let Some(rest) = s.strip_prefix(mnem) {
                if !rest.starts_with(' ') && !rest.starts_with('\t') {
                    continue;
                }
                let dest = parse_dest_reg(s);
                if dest == REG_NONE || dest > REG_GP_MAX {
                    return None;
                }
                if mnem.starts_with("xor") {
                    // Only the zero idiom `xor R, R` writes a result whose
                    // flags equal test R, R; `xor A, B` with A != B reads B
                    // before writing, and the flag state still matches, but
                    // the DEST register held a different value beforehand —
                    // irrelevant for flags, yet keep the strict form: the
                    // corpus only ever contains the idiom.
                    let lhs = rest.trim_start().split(',').next().unwrap_or("").trim();
                    let rhs = s.rsplit(',').next().unwrap_or("").trim();
                    if lhs != rhs {
                        return None;
                    }
                }
                return Some((*size, dest));
            }
        }
        None
    }

    /// Parse `test{l,w,b} %R, %R` (same register twice) -> (size, R).
    fn self_test(s: &str) -> Option<(MoveSize, RegId)> {
        let (mnem, size) = if let Some(r) = s.strip_prefix("testl ") {
            (r, MoveSize::L)
        } else if let Some(r) = s.strip_prefix("testw ") {
            (r, MoveSize::W)
        } else if let Some(r) = s.strip_prefix("testb ") {
            (r, MoveSize::B)
        } else {
            return None;
        };
        let mut parts = mnem.split(',');
        let a = parts.next()?.trim();
        let b = parts.next()?.trim();
        if parts.next().is_some() || a != b || !a.starts_with('%') {
            return None;
        }
        let reg = register_family(a);
        if reg == REG_NONE || reg > REG_GP_MAX {
            return None;
        }
        Some((size, reg))
    }

    let mut changed = false;
    let len = infos.len();
    let mut i = 0;
    while i + 1 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let (psize, preg) = match flag_equiv_producer(trimmed(store, &infos[i], i)) {
            Some(p) => p,
            None => {
                i += 1;
                continue;
            }
        };
        let j = next_non_nop(infos, i + 1);
        if j >= len {
            break;
        }
        if let Some((tsize, treg)) = self_test(trimmed(store, &infos[j], j)) {
            if tsize == psize && treg == preg {
                infos[j].kind = LineKind::Nop;
                changed = true;
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    changed
}

// ── Pass: sign-extend + test fusion ──────────────────────────────────────────

/// Fuse `movs{b,w}l %L, %eR` + `testl %eR, %eR` into `test{b,w} %L, %L`.
///
/// Sign-extension preserves both the zero-ness and the sign of the value, so
/// `testl` over the extended register sets the same ZF and SF as `testb`/`testw`
/// over the narrow source; CF and OF are zero for both forms.  Every flag is
/// identical, so any consumer (jcc/setcc/cmov) is unaffected.
///
/// The extension's register result is destroyed by the rewrite, so the pass
/// is gated on a forward scan proving the destination register family is not
/// read before its next pure write (the same deadness proof
/// `collapse_slot_rmw_i686` uses; barriers stop the scan conservatively).
/// The narrow SOURCE register is untouched by both forms.
fn fuse_sext_testb(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    if std::env::var_os("CCC_NO_SEXT_TEST").is_some() {
        return false;
    }
    /// Parse `movsbl %al, %eax` / `movswl %ax, %eax` style same-family
    /// sign-extensions → (narrow test mnemonic, narrow reg, family, full name).
    fn same_family_sext(s: &str) -> Option<(&'static str, &'static str, RegId, &'static str)> {
        for (mnem, narrow_mnem, narrow, family, full) in [
            ("movsbl %al, %eax", "testb", "%al", REG_EAX, "%eax"),
            ("movsbl %cl, %ecx", "testb", "%cl", REG_ECX, "%ecx"),
            ("movsbl %dl, %edx", "testb", "%dl", REG_EDX, "%edx"),
            ("movsbl %bl, %ebx", "testb", "%bl", REG_EBX, "%ebx"),
            ("movswl %ax, %eax", "testw", "%ax", REG_EAX, "%eax"),
            ("movswl %cx, %ecx", "testw", "%cx", REG_ECX, "%ecx"),
            ("movswl %dx, %edx", "testw", "%dx", REG_EDX, "%edx"),
            ("movswl %bx, %ebx", "testw", "%bx", REG_EBX, "%ebx"),
        ] {
            if s == mnem {
                return Some((narrow_mnem, narrow, family, full));
            }
        }
        None
    }
    let len = infos.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        let Some((narrow_mnem, narrow, family, full)) =
            same_family_sext(trimmed(store, &infos[i], i))
        else {
            i += 1;
            continue;
        };
        // Next code line must be the matching full-width self-test.
        let mut j = i + 1;
        while j < len && (infos[j].is_nop() || infos[j].kind == LineKind::Directive) {
            j += 1;
        }
        if j >= len {
            break;
        }
        let expected_test = format!("testl {}, {}", full, full);
        if trimmed(store, &infos[j], j) != expected_test {
            i += 1;
            continue;
        }
        // Forward scan: the family must be dead (overwritten) before any read.
        let reg_bit: u16 = 1 << family;
        let mut k = j + 1;
        let mut reg_dead = false;
        while k < len {
            if infos[k].is_nop() || infos[k].kind == LineKind::Directive {
                k += 1;
                continue;
            }
            if let LineKind::Pop { reg: popped } = infos[k].kind {
                if popped == family {
                    reg_dead = true;
                    break;
                }
                k += 1;
                continue;
            }
            if infos[k].is_barrier() {
                break;
            }
            let (uses, defs) = line_reg_use_def(store, infos, k);
            if uses & reg_bit != 0 {
                break; // extended value would be read
            }
            if defs & reg_bit != 0 {
                reg_dead = true;
                break;
            }
            k += 1;
        }
        if !reg_dead {
            i = j + 1;
            continue;
        }
        store.replace(i, format!("    {} {}, {}", narrow_mnem, narrow, narrow));
        infos[i] = classify_line(store.get(i));
        infos[j].kind = LineKind::Nop;
        changed = true;
        i = j + 1;
    }
    changed
}

// ── Pass: div/rem pair fusion ────────────────────────────────────────────────

/// Fuse a recomputation of an identical division when the first division's
/// result registers are still live.
///
/// The accumulator backend lowers `a % b` and `a / b` (IR `URem`/`UDiv`,
/// `SRem`/`SDiv`) as two independent sequences with identical staging:
///
/// ```text
///     movl %SRC1, %eax      ─┐
///     movl %SRC2, %ecx       │ staging A
///     xorl %edx, %edx        │ (or cltd)
///     divl %ecx             ─┘  → %eax = quotient, %edx = remainder
///     movl %edx, 4(%esp)        (remainder consumer)
///     movl %SRC1, %eax      ─┐
///     movl %SRC2, %ecx       │ staging B (textually identical)
///     xorl %edx, %edx        │
///     divl %ecx             ─┘  → recomputes the same quotient
///     movl %eax, 8(%esp)        (quotient consumer)
/// ```
///
/// Because both divisions receive identical inputs, their outputs are equal,
/// so deleting staging B + the second `divl` leaves `%eax`/`%edx` holding
/// exactly the values the deleted division would have produced.  This is the
/// machine-level equivalent of GCC's divmod fusion (one `div` supplies both
/// `/` and `%`), which the kernel's `number()` in `arch/x86/boot/printf.c`
/// exercises in its digit loop.
///
/// Soundness window (strictly between the first `divl` and staging B):
/// * no label/branch/call/ret/push/pop/ESP write (straight line only);
/// * no write to `%eax` or `%edx` (the live quotient/remainder);
/// * no write to any register or stack slot READ by the staging block
///   (input equality — partial byte/word slot writes alias by range);
/// * any instruction with an unparsed destination, or an implicit
///   multi-register writer (`div*`, `mul*`, `imul*`, `xchg*`, `xadd`,
///   `cmpxchg*`, string ops, `rep`), or an indirect memory store aborts the
///   candidate (fail closed).
fn fuse_div_rem_pairs(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    if std::env::var_os("CCC_NO_DIV_FUSION").is_some() {
        return false;
    }
    const MAX_WINDOW: usize = 32;
    const MAX_SETUP: usize = 4;

    /// Size of a move in bytes by move width.
    fn size_bytes(size: MoveSize) -> u32 {
        match size {
            MoveSize::B => 1,
            MoveSize::W => 2,
            MoveSize::L => 4,
        }
    }

    /// Next code-line index treating `.loc` markers as transparent (a
    /// non-debug directive stops the scan: section/alignment/CFI changes
    /// delimit regions).
    fn next_code_skip_loc(store: &LineStore, infos: &[LineInfo], start: usize) -> usize {
        let mut k = start;
        while k < infos.len() {
            if infos[k].is_nop() || infos[k].kind == LineKind::Empty {
                k += 1;
                continue;
            }
            if infos[k].kind == LineKind::Directive {
                if is_debug_location(store, infos, k) {
                    k += 1;
                    continue;
                }
                return usize::MAX;
            }
            return k;
        }
        usize::MAX
    }

    /// Parse a `divl %R` / `idivl %R` line → register family.
    fn div_reg(s: &str) -> Option<RegId> {
        let rest = s
            .strip_prefix("divl ")
            .or_else(|| s.strip_prefix("idivl "))?;
        let reg = register_family(rest.trim());
        if reg == REG_NONE || reg > REG_GP_MAX {
            return None;
        }
        Some(reg)
    }

    /// Collect the staging block backward from `marker`: consecutive code
    /// lines whose destination is %eax or the divisor register.  Returns None
    /// when the block is longer than MAX_SETUP (fail closed) or a staging
    /// line has no parsed register destination/source shape we understand.
    fn collect_staging(
        store: &LineStore,
        infos: &[LineInfo],
        marker: usize,
        divisor: RegId,
    ) -> Option<(Vec<(usize, String)>, Vec<RegId>, Vec<(i32, u32)>)> {
        let mut staging = Vec::new();
        let mut read_regs = Vec::new();
        let mut read_slots = Vec::new();
        let mut k = marker;
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 64 {
                return None;
            }
            // Walk to the previous code line, treating .loc as transparent.
            let mut p = k;
            loop {
                if p == 0 {
                    return Some((staging, read_regs, read_slots));
                }
                p -= 1;
                if infos[p].is_nop() || infos[p].kind == LineKind::Empty {
                    continue;
                }
                if infos[p].kind == LineKind::Directive {
                    if is_debug_location(store, infos, p) {
                        continue;
                    }
                    // Non-debug directives end the staging block.
                    return Some((staging, read_regs, read_slots));
                }
                break;
            }
            let text = trimmed(store, &infos[p], p);
            // Stop at control flow / barriers.
            if infos[p].is_barrier() {
                return Some((staging, read_regs, read_slots));
            }
            // Only accept register-writing `movl`-family staging whose
            // destination is %eax or the divisor register; anything else
            // (including flags producers and unknown destinations) ends the
            // block.
            let dest = match &infos[p].kind {
                LineKind::Move { dst, src } => {
                    read_regs.push(*src);
                    *dst
                }
                LineKind::LoadEbp { reg, offset, size } => {
                    read_slots.push((*offset, size_bytes(*size)));
                    *reg
                }
                LineKind::Other { dest_reg } => {
                    // `movl %src, %dst` style: recover the source register.
                    let rest = text.strip_prefix("movl ").map(|r| r.trim()).unwrap_or("");
                    if let Some(comma) = rest.find(',') {
                        let src_name = rest[..comma].trim();
                        if src_name.starts_with('%') && !src_name.contains('(') {
                            let src = register_family(src_name);
                            if src <= REG_GP_MAX {
                                read_regs.push(src);
                            }
                        }
                    }
                    *dest_reg
                }
                _ => return Some((staging, read_regs, read_slots)),
            };
            if dest == REG_NONE || (dest != REG_EAX && dest != divisor) {
                return Some((staging, read_regs, read_slots));
            }
            staging.push((p, text.to_string()));
            if staging.len() > MAX_SETUP {
                return None;
            }
            k = p;
        }
    }

    /// Opaque multi-register writers and unknown-effect mnemonics that the
    /// kind-based analysis below cannot prove safe.
    fn opaque_multi_writer(text: &str) -> bool {
        text.starts_with("div")
            || text.starts_with("idiv")
            || text.starts_with("mul")
            || text.starts_with("imul")
            || text.starts_with("xchg")
            || text.starts_with("xadd")
            || text.starts_with("cmpxchg")
            || text.starts_with("lock ")
            || text.starts_with("rep")
            || text.starts_with("lods")
            || text.starts_with("stos")
            || text.starts_with("movs")
            || text.starts_with("cmps")
            || text.starts_with("scas")
            || text.starts_with("bswap")
            || text.starts_with("call")
            || text.starts_with("cltd")
            || text.starts_with("cdq")
    }

    /// Whether a window line may be crossed: verifies it writes neither the
    /// quotient/remainder registers nor anything the staging reads, and is
    /// not an opaque multi-writer.  Returns false when the candidate must be
    /// abandoned.
    fn window_line_ok(
        store: &LineStore,
        infos: &[LineInfo],
        idx: usize,
        read_regs: &[RegId],
        read_slots: &[(i32, u32)],
    ) -> bool {
        let text = trimmed(store, &infos[idx], idx);
        if opaque_multi_writer(text) {
            return false;
        }
        let dest = match &infos[idx].kind {
            LineKind::Move { dst, .. } => Some(*dst),
            LineKind::LoadEbp { reg, .. } => Some(*reg),
            LineKind::SetCC { reg } => Some(*reg),
            LineKind::StoreEbp { offset, size, .. } => {
                // Stack write: check range overlap with staging slot reads.
                let w_lo = *offset as i64;
                let w_hi = w_lo + size_bytes(*size) as i64;
                for (r_off, r_size) in read_slots {
                    let r_lo = *r_off as i64;
                    let r_hi = r_lo + *r_size as i64;
                    if w_lo < r_hi && r_lo < w_hi {
                        return false;
                    }
                }
                None
            }
            LineKind::Other { dest_reg } => {
                if *dest_reg == REG_NONE {
                    // Unparsed destination (includes indirect memory
                    // operands): conservative abort.
                    return false;
                }
                Some(*dest_reg)
            }
            LineKind::Cmp => None, // flags only
            LineKind::Nop | LineKind::Empty => None,
            _ => return false,
        };
        if let Some(dst) = dest {
            if dst == REG_EAX || dst == REG_EDX || read_regs.contains(&dst) {
                return false;
            }
        }
        // Indirect memory reads/writes with a register base are allowed only
        // if they cannot WRITE memory; a load through a register is a read.
        // Stores through a register indirect destination fail the parse above
        // (REG_NONE), so reaching here with memory operands means a load.
        true
    }

    let len = infos.len();

    // Collect every division site first: the forward scan must know where
    // site B's STAGING starts (its first eax/divisor-writing line), which
    // sits before B's marker and would otherwise be mistaken for a window
    // hazard.
    struct Site {
        marker: usize,
        div: usize,
        /// First staging line index (== marker when staging is empty).
        staging_first: usize,
        staging: Vec<(usize, String)>,
        read_regs: Vec<RegId>,
        read_slots: Vec<(i32, u32)>,
        div_text: String,
        divisor: RegId,
    }
    let mut sites: Vec<Site> = Vec::new();
    {
        let mut i = 0;
        while i < len {
            if infos[i].is_nop() {
                i += 1;
                continue;
            }
            let text_a = trimmed(store, &infos[i], i);
            let is_marker = text_a == "xorl %edx, %edx" || text_a == "cltd" || text_a == "cdq";
            if !is_marker {
                i += 1;
                continue;
            }
            let div_idx = next_code_skip_loc(store, infos, i + 1);
            if div_idx == usize::MAX || div_idx >= len {
                i += 1;
                continue;
            }
            let div_text = trimmed(store, &infos[div_idx], div_idx).to_string();
            let Some(divisor) = div_reg(&div_text) else {
                i = div_idx + 1;
                continue;
            };
            match collect_staging(store, infos, i, divisor) {
                Some((staging, read_regs, read_slots)) => {
                    let staging_first = staging.last().map(|(p, _)| *p).unwrap_or(i);
                    sites.push(Site {
                        marker: i,
                        div: div_idx,
                        staging_first,
                        staging,
                        read_regs,
                        read_slots,
                        div_text,
                        divisor,
                    });
                    i = div_idx + 1;
                }
                None => i = div_idx + 1,
            }
        }
    }

    let mut changed = false;
    // Fuse consecutive identical sites.  After fusing A-B, a further site C
    // may still fuse with A: the deleted B lines become NOPs (transparent),
    // and the window proof extends from A's division to C's staging start.
    let mut a = 0;
    while a < sites.len() {
        let site_a = &sites[a];
        if site_a.staging.is_empty() {
            a += 1;
            continue;
        }
        // walk candidates after a
        let mut b_idx = a + 1;
        while b_idx < sites.len() {
            let site_b = &sites[b_idx];
            // Any intervening site (before b) whose staging/marker/div lines
            // are still live would have written %eax: only NOP-ed (fused)
            // sites may be crossed.
            let mut crossed_live_site = false;
            for m in (a + 1)..b_idx {
                if !infos[sites[m].marker].is_nop() {
                    crossed_live_site = true;
                    break;
                }
            }
            if crossed_live_site {
                break;
            }
            let same = site_a.div_text == site_b.div_text
                && site_a.staging.len() == site_b.staging.len()
                && site_a
                    .staging
                    .iter()
                    .zip(site_b.staging.iter())
                    .all(|(x, y)| x.1 == y.1);
            if !same || site_b.staging.is_empty() {
                break;
            }
            // Verify the window strictly between A's div and B's staging
            // start: no barrier, no %eax/%edx write, no write to anything A
            // reads.
            let mut k = site_a.div;
            let mut ok = true;
            loop {
                let n = next_code_skip_loc(store, infos, k + 1);
                if n == usize::MAX {
                    // A non-debug directive (or end of stream) inside the
                    // window: the straight-line proof stops here, so the
                    // window is NOT verified.
                    ok = false;
                    break;
                }
                if n >= site_b.staging_first {
                    // Reached staging B: every line in between was checked.
                    break;
                }
                k = n;
                if !window_line_ok(store, infos, k, &site_a.read_regs, &site_a.read_slots) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
            // Delete staging B, marker B, and div B: %eax (and %edx) hold
            // exactly the results the deleted division would have produced.
            for (idx, _) in &site_b.staging {
                infos[*idx].kind = LineKind::Nop;
            }
            infos[site_b.marker].kind = LineKind::Nop;
            infos[site_b.div].kind = LineKind::Nop;
            changed = true;
            // Keep A as the anchor: eax still holds the quotient, so further
            // identical sites can fuse too.
            b_idx += 1;
        }
        a += 1;
    }
    changed
}

// ── Pass: slot read-modify-write collapse ────────────────────────────────────

/// Collapse `movl S,%eax; OP %eax; movl %eax,S` (S = N(%esp)) into a single
/// memory-destination `OP S`.  The register round-trip is pure staging: the
/// value lives in the slot, the op touches it once, and it goes straight back
/// (boot corpus examples: a20's retry counter `incl`, cpu's `addl $4`,
/// printf's width `decl`).  Savings 5-7 bytes per site.
///
/// Soundness: the memory form writes the same value and sets the IDENTICAL
/// flags (incl/decl leave CF alone in both forms).  The ONLY semantic
/// difference is that %eax no longer holds the result afterwards — the
/// collapse is therefore gated on a forward scan proving %eax is not read
/// before its next pure write.  Any barrier (label/branch/call/ret/push/
/// pop/esp-arithmetic/directive) stops the scan conservatively.
fn collapse_slot_rmw_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    if std::env::var("CCC_NO_SLOT_RMW").is_ok() {
        return false;
    }
    /// Next line index that is neither a NOP nor a (code-less) directive —
    /// `-g` interleaves `.loc` markers between instructions, and they must
    /// not break the load/op/store adjacency.
    fn next_code_line(infos: &[LineInfo], start: usize) -> usize {
        let mut k = start;
        while k < infos.len() && (infos[k].is_nop() || infos[k].kind == LineKind::Directive) {
            k += 1;
        }
        k
    }
    /// Parse an op line that reads and writes %REG and has a valid
    /// memory-destination form: returns (mnemonic, optional source operand).
    fn parse_rmw_op<'a>(s: &'a str, reg_name: &str) -> Option<(&'a str, Option<&'a str>)> {
        let (mnem, rest) = s.split_once(char::is_whitespace)?;
        match mnem {
            "incl" | "decl" | "negl" | "notl" => {
                if rest.trim() == reg_name {
                    return Some((mnem, None));
                }
                return None;
            }
            "addl" | "subl" | "andl" | "orl" | "xorl" => {}
            _ => return None,
        }
        // "SRC, %REG"
        let (src, dst) = rest.rsplit_once(',')?;
        if dst.trim() != reg_name {
            return None;
        }
        let src = src.trim();
        // Immediate or a different 32-bit register only: memory sources
        // cannot pair with a memory destination, and %esp is never data.
        if src.starts_with('$') {
            return Some((mnem, Some(src)));
        }
        const GPR32: &[&str] = &["%eax", "%ebx", "%ecx", "%edx", "%esi", "%edi", "%ebp"];
        if GPR32.contains(&src) && src != reg_name {
            return Some((mnem, Some(src)));
        }
        None
    }

    let mut changed = false;
    let len = infos.len();
    let mut i = 0;
    while i + 2 < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        // Line i: movl SLOT, %REG
        let (reg, load) = match infos[i].kind {
            LineKind::LoadEbp { reg, .. }
                if reg != REG_ESP && reg != REG_EBP && reg <= REG_GP_MAX =>
            {
                (reg, trimmed(store, &infos[i], i))
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let reg_name = reg32_name(reg);
        let load_suffix = format!(", {}", reg_name);
        let slot = load
            .strip_prefix("movl ")
            .and_then(|r| r.strip_suffix(load_suffix.as_str()))
            .map(str::trim);
        let slot = match slot {
            Some(s) if (s.ends_with("(%esp)") || s.ends_with("(%ebp)")) && !s.starts_with('(') => s,
            _ => {
                i += 1;
                continue;
            }
        };
        // Line i+1: the RMW op on %REG (directives between are skipped)
        let op_idx = next_code_line(infos, i + 1);
        if op_idx >= len {
            break;
        }
        let op = trimmed(store, &infos[op_idx], op_idx);
        let (mnem, src) = match parse_rmw_op(op, reg_name) {
            Some(p) => p,
            None => {
                i += 1;
                continue;
            }
        };
        // Line i+2: movl %REG, SAME slot
        let st_idx = next_code_line(infos, op_idx + 1);
        if st_idx >= len {
            break;
        }
        let store_line = match infos[st_idx].kind {
            LineKind::StoreEbp { reg: sr, .. } if sr == reg => {
                trimmed(store, &infos[st_idx], st_idx)
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let store_prefix = format!("movl {}, ", reg_name);
        let store_slot = store_line
            .strip_prefix(store_prefix.as_str())
            .map(str::trim);
        if store_slot != Some(slot) {
            i += 1;
            continue;
        }
        // Forward scan: %REG must not be read before its next pure write.
        let reg_bit: u16 = 1 << reg;
        let mut k = st_idx + 1;
        let mut reg_dead = false;
        while k < len {
            if infos[k].is_nop() || infos[k].kind == LineKind::Directive {
                k += 1;
                continue;
            }
            // `popl %REG` in the epilogue is a pure restore (def) of the
            // register — the stale collapsed value is overwritten, never
            // read.  Pops of OTHER registers leave %REG untouched (they do
            // shift %esp, but no slot reasoning happens after the collapse).
            if let LineKind::Pop { reg: popped } = infos[k].kind {
                if popped == reg {
                    reg_dead = true;
                    break;
                }
                k += 1;
                continue;
            }
            if infos[k].is_barrier() {
                break;
            }
            let (uses, defs) = line_reg_use_def(store, infos, k);
            if uses & reg_bit != 0 {
                break; // stale value would be read
            }
            if defs & reg_bit != 0 {
                reg_dead = true; // overwritten before any read
                break;
            }
            k += 1;
        }
        if !reg_dead {
            i += 1;
            continue;
        }
        // Collapse.
        let mem_op = match src {
            Some(src) => format!("    {} {}, {}", mnem, src, slot),
            None => format!("    {} {}", mnem, slot),
        };
        store.replace(i, mem_op);
        infos[i] = classify_line(store.get(i));
        infos[op_idx].kind = LineKind::Nop;
        infos[st_idx].kind = LineKind::Nop;
        changed = true;
        i = st_idx + 1;
    }
    changed
}

// ── Pass: Memory operand folding ─────────────────────────────────────────────

/// True when `s` is a genuine 32-bit `movl` load whose memory operand is a
/// full 4 bytes. `movzbl`/`movsbl`/`movzwl`/`movswl` read 1-2 bytes into a
/// 32-bit destination and are classified with the same `MoveSize::L` (their
/// destination register is 32-bit), so a size check alone is NOT enough to
/// distinguish them. Folding a sub-word load into a `cmpl`/`addl` memory
/// operand would read 3-4 bytes of a narrower slot — a miscompile:
/// `uu.b[0] != 0x04` became `cmpl $4, 176(%esp)`, comparing the whole union
/// word 0x01020304 instead of the byte 0x04.
fn is_full_width_load(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("movl ")
}

/// Fold `movl -N(%ebp), %ecx; addl %ecx, %eax` into `addl -N(%ebp), %eax`.
fn fold_memory_operands(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    // Computed lazily (first fold candidate) and reused for the whole sweep:
    // see `GprLiveness` for why one computation stays sound across folds.
    let mut liveness_cache: Option<GprLiveness> = None;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Look for load from stack slot
        if let LineKind::LoadEbp {
            reg: load_reg,
            offset,
            size,
        } = infos[i].kind
        {
            // Any GPR whose value dies after the consumer may fold. The
            // caller-saved-only filter left ebx/esi/edi reloads that the
            // deadness scan already proves unused (i686 -Os size). Never
            // fold through the stack/frame pointer.
            if load_reg > REG_GP_MAX || load_reg == REG_ESP || load_reg == REG_EBP {
                i += 1;
                continue;
            }
            // Only full-width loads: movzbl/movsbl/movzwl/movswl read fewer
            // bytes than the folded memory operand would.
            if !is_full_width_load(trimmed(store, &infos[i], i)) {
                i += 1;
                continue;
            }

            // Find next non-nop instruction
            let j = next_non_nop(infos, i + 1);
            if j >= len {
                i += 1;
                continue;
            }

            // Check if next instruction uses this register as a source operand
            // Pattern: load into %ecx, then `addl %ecx, %eax` etc.
            let s = trimmed(store, &infos[j], j);
            if let Some(folded) = try_fold_memory_operand(s, load_reg, offset, size) {
                // Soundness: the fold deletes the load, so the loaded register
                // must be DEAD after the folded consumer. Copy propagation can
                // extend a slot load's register across several consumers
                // (`movl SLOT,%eax; movl %eax,%edx; cmpl %edx,%edi; subl
                // %edx,%ebp` -> `...; cmpl %eax,%edi; subl %eax,%ebp`), and a
                // register-allocated param/phi home (`movl 20(%esp),%esi`) is
                // live across labels, conditional jumps and back-edges. The
                // whole-function liveness oracle (`GprLiveness`) answers
                // exactly; the former bounded scan stopped at the first
                // barrier (label/call/ret/jump) and *assumed* deadness there,
                // which miscompiled `a > b ? a : b` (the select's fall-through
                // arm read a never-written %esi) and, less visibly, let a fold
                // delete a load that a following regparm call read from
                // %eax/%ecx/%edx or a `ret` read from %eax (the return value).
                //
                // The oracle is computed lazily (first fold candidate) and
                // reused for the whole sweep: folds nop lines in place instead
                // of removing them, so indices stay aligned, and deleting a
                // dead definition only makes that register's cached answer
                // conservative — see `GprLiveness`.
                let liveness = liveness_cache.get_or_insert_with(|| GprLiveness::compute(store, infos));
                if !liveness.dead_after(j, load_reg) {
                    i += 1;
                    continue;
                }
                // Belt-and-braces: the consumer must not write load_reg in a
                // way the folded form cannot reproduce (`try_fold_memory_operand`
                // already refuses an explicit dest==load_reg; this catches an
                // architectural write such as `cltd` overwriting %edx). A pure
                // refusal here is always safe.
                if line_writes_reg_implicitly(s, load_reg) {
                    i += 1;
                    continue;
                }

                store.replace(j, format!("    {}", folded));
                let dest_reg = parse_dest_reg(&folded);
                infos[j] = LineInfo {
                    kind: LineKind::Other { dest_reg },
                    trim_start: 4,
                    has_indirect_mem: false,
                    ebp_offset: offset,
                };
                infos[i].kind = LineKind::Nop; // Remove the load
                changed = true;
            }
        }
        i += 1;
    }

    changed
}

/// Try to fold a stack slot into an ALU instruction.
/// Returns the folded instruction string if successful.
fn try_fold_memory_operand(
    s: &str,
    load_reg: RegId,
    offset: i32,
    _size: MoveSize,
) -> Option<String> {
    let reg_name = reg32_name(load_reg);

    // The offset carries the slot-namespace bias: decode it back into the
    // real displacement and its base register. Emitting the BIASED number
    // with a hard-coded (%ebp) produced silent wrong code once ESP slots
    // joined the peepholes (`cmpl 16777244(%ebp), %eax`).
    let (disp, base) = if offset >= ESP_SLOT_BIAS / 2 {
        (offset - ESP_SLOT_BIAS, "%esp")
    } else {
        (offset, "%ebp")
    };

    // Try patterns: `OPCODE %load_reg, %other_reg`
    for op in &[
        "addl", "subl", "andl", "orl", "xorl", "cmpl", "testl", "imull",
    ] {
        if let Some(rest) = s.strip_prefix(op) {
            let rest = rest.trim();
            // Pattern: `%load_reg, %dst` -> `OPCODE disp(base), %dst`
            if let Some(after) = rest.strip_prefix(reg_name) {
                let after = after.trim();
                if let Some(after_comma) = after.strip_prefix(',') {
                    let dst = after_comma.trim();
                    if dst.starts_with('%') && !dst.contains('(') {
                        // Don't fold if dst is the same as load_reg (would be read after free)
                        if register_family(dst) != load_reg {
                            return Some(format!("{} {}({}), {}", op, disp, base, dst));
                        }
                    }
                }
            }
        }
    }

    None
}

// ── Pass: indirect call through a spilled global function pointer ───────────

/// Does `s` reference the ESP-relative slot at raw displacement `disp`?
/// Textual scan with numeric comparison of the displacement digits (a naive
/// substring test for "0(%esp)" would also hit "40(%esp)").
fn references_esp_slot(s: &str, disp: i32) -> bool {
    let mut rest = s;
    while let Some(p) = rest.find("(%esp)") {
        let before = &rest[..p];
        let digits_start = before
            .rfind(|c: char| !c.is_ascii_digit() && c != '-')
            .map(|q| q + 1)
            .unwrap_or(0);
        let num = &before[digits_start..];
        let val = if num.is_empty() {
            0
        } else {
            num.parse::<i32>().unwrap_or(i32::MIN)
        };
        if val == disp {
            return true;
        }
        rest = &rest[p + 6..];
    }
    false
}

/// Fold `movl SYM, %reg; movl %reg, N(%esp); ...; call *N(%esp)` into
/// `...; call *SYM`.
///
/// The i686 emitter has no register home for a call target under regparm
/// (%eax/%ecx/%edx all carry arguments), so every `pio_ops.f(...)` call in
/// the kernel setup code stages the pointer through a stack slot — 11 bytes
/// of load+spill per site that GCC never emits (it calls `*pio_ops`
/// directly). x86-64 needs no port: its emitter stages indirect targets in
/// %r10/%r11, which no argument register overlaps.
///
/// Soundness:
///  * The SYM read moves FORWARD to the call. Every crossed line must be a
///    register/immediate-only data op (no memory operands of any kind), so
///    no crossed store can retarget SYM. For volatile pointers the read
///    count is preserved and only reordered against non-memory ops, which
///    volatile semantics do not constrain.
///  * Crossed lines cannot write the slot or change ESP (reg/imm-only), so
///    the textual `N(%esp)` in the call names the same slot the store
///    wrote.
///  * The spilled register must not be read on the crossed lines (it no
///    longer holds the pointer).
///  * The slot must have no OTHER reference anywhere in the function
///    (numeric displacement match): with the store and call gone, a stale
///    read elsewhere would see garbage; a same-name reference under a
///    shifted ESP is over-matched and merely blocks the fold.

/// Fold register-copy staging idioms that the accumulator codegen emits
/// around tests, zeroing, immediates and byte extension.  Every rewrite is
/// flag- and value-identical; the copies it orphans are reclaimed by the
/// existing dead-move/dead-store cleanup loops.
///
/// Patterns (S/D/X/Y/C/X2 = 32-bit GPRs, adjacent lines):
///   1. `movl %S,%D; testl %S,%D` | `testl %D,%S`  ->  `testl %S,%S`
///      (D is a copy of S, so S&D == S&S; same flags).  The kernel boot
///      path pays this relay on every `if (c)`-style test.
///   2. `xorl %X,%X; movl %X,%Y`  ->  `xorl %Y,%Y`
///      (zeroing any register sets identical flags; Y==X skipped).
///   3. `movl $imm32,%C; ALU %C,%X`  ->  `ALU $imm32,%X` for
///      ALU in {andl,orl,addl,subl,cmpl,testl}.  The mov stays if %C has a
///      later use; flags are unchanged because the immediate encodes
///      exactly the staged value.  `err_flags & 0xF9FFFFFF` style masks
///      stage through %ecx today and cost two extra bytes per site.
///   4. `movl N(%esp),%R32; movz{b,w}l %r8/%r16,%R32`  ->
///      `movz{b,w}l N(%esp),%R32`  (and movsb{l,w}).  The widened load
///      reads exactly the bytes the movz/movs consumes, so dropping the
///      full-width staging load cannot change the value.  16-bit low-byte
///      aliases on i386 are al/cl/dl/bl only (sil/dil/bpl need REX).

/// Alias of `register_family` for the staging-copy folds.
fn reg_id_from_name(name: &str) -> RegId {
    register_family(name)
}

/// Parse `movl %R, N(%esp)` returning (R, N).
fn parse_store_from_reg(s: &str) -> Option<(&str, i32)> {
    let rest = s.strip_prefix("movl ")?;
    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();
    if !src.starts_with("%e") {
        return None;
    }
    let inner = dst.strip_suffix("(%esp)")?;
    let disp: i32 = inner.trim().parse().ok()?;
    Some((src, disp))
}

/// Parse `cmpl $imm, %R` / `testl $imm, %R` returning (mnemonic, R).
fn parse_cmp_test_imm(s: &str) -> Option<(&'static str, &str)> {
    for mnem in ["cmpl", "testl"] {
        if let Some(rest) = s.strip_prefix(mnem).and_then(|r| r.strip_prefix(' ')) {
            let (imm, reg) = rest.split_once(',')?;
            let imm = imm.trim();
            let reg = reg.trim();
            if imm.starts_with('$') && reg.starts_with("%e") {
                return Some((mnem, reg));
            }
            return None;
        }
    }
    None
}

/// Extract the immediate text (without `$`) from `mnem $imm, ...`.
fn imm_of(s: &str) -> &str {
    let rest = s.split_once(' ').map(|(_, r)| r).unwrap_or("");
    let (imm, _) = rest.split_once(',').unwrap_or((rest, ""));
    imm.trim().trim_start_matches('$')
}

/// Parse `subl $imm, %R` returning imm.
fn parse_subl_imm_reg(s: &str, reg: &str) -> Option<i32> {
    let rest = s.strip_prefix("subl ")?;
    let (imm, r) = rest.split_once(',')?;
    if r.trim() != reg {
        return None;
    }
    imm.trim().strip_prefix('$')?.parse().ok()
}

/// Parse `cmpl $imm, %R` returning (imm, R-family) for any GP R.
fn parse_cmpl_imm_any(s: &str) -> Option<(i32, RegId)> {
    let rest = s.strip_prefix("cmpl ")?;
    let (imm, r) = rest.split_once(',')?;
    let imm: i32 = imm.trim().strip_prefix('$')?.parse().ok()?;
    let r = register_family(r.trim());
    if r > REG_GP_MAX {
        return None;
    }
    Some((imm, r))
}

/// Order/equality condition codes: every jCC whose result cannot depend on
/// OF (the flag that may differ between a 32-bit and a 16-bit compare).
fn is_order_cond_jcc(s: &str) -> bool {
    let Some((m, _)) = s.split_once(' ') else {
        return false;
    };
    matches!(
        m,
        "je" | "jz"
            | "jne"
            | "jnz"
            | "jb"
            | "jc"
            | "jnae"
            | "jnb"
            | "jnc"
            | "jae"
            | "jbe"
            | "jna"
            | "ja"
            | "jnbe"
    )
}

/// Walk backward from `before` (exclusive) looking for a zero-extending
/// definition of `reg` (`movzwl/movzbl SRC, %R`) within a small window, with
/// NO other reference to `reg` in between (any width alias voids the
/// canonicality).  Returns the def line index.
fn canonical_16_def(
    store: &LineStore,
    infos: &[LineInfo],
    before: usize,
    reg: RegId,
    floor: usize,
) -> Option<usize> {
    let mut k = before;
    let mut seen = 0usize;
    while k > floor && seen < 8 {
        k -= 1;
        if infos[k].is_nop() || is_debug_location(store, infos, k) {
            continue;
        }
        seen += 1;
        let line = trimmed(store, &infos[k], k);
        if infos[k].kind == LineKind::Label {
            return None; // block entry: canonicality unprovable
        }
        let is_def = line.starts_with("movzwl ") || line.starts_with("movzbl ");
        if is_def {
            let Some((_, dst)) = line.split_once(',') else {
                return None;
            };
            if register_family(dst.trim()) == reg {
                return Some(k);
            }
            return None; // a different movz def: not our register
        }
        // Intermediate lines may READ the register freely (a read preserves
        // the canonical value); only a REDEFINITION — direct move, RMW,
        // partial-alias write or implicit clobber — voids the proof.
        if writes_reg_family(line, &infos[k], reg) {
            return None;
        }
    }
    None
}

/// Does this line write any alias (32/16/8-bit) of the register family?
/// Reads are deliberately not writes: `movl %eax,%ebp` preserves %eax.
fn writes_reg_family(line: &str, info: &LineInfo, reg: RegId) -> bool {
    let kind_match = match info.kind {
        LineKind::Move { dst, .. } => dst == reg,
        LineKind::SelfMove => {
            // `movl %R,%R`: both operands are the same register family.
            line.split_whitespace()
                .nth(1)
                .and_then(|o| o.trim_end_matches(',').split_once(',').map(|(s, _)| s))
                .map(|s| register_family(s.trim_end_matches(',')) == reg)
                .unwrap_or(false)
        }
        LineKind::LoadEbp { reg: r, .. } => r == reg,
        LineKind::Pop { reg: r } => r == reg,
        LineKind::SetCC { reg: r } => r == reg,
        LineKind::Call => matches!(reg, REG_EAX | REG_ECX | REG_EDX),
        LineKind::Other { dest_reg } => dest_reg == reg,
        LineKind::Nop => false,
        _ => false,
    };
    let kind_writes = kind_match;
    kind_writes || line_writes_reg_implicitly(line, reg)
}

/// Parse `cmpl $imm, %R` returning imm.
fn parse_cmpl_imm_reg(s: &str, reg: &str) -> Option<i32> {
    let rest = s.strip_prefix("cmpl ")?;
    let (imm, r) = rest.split_once(',')?;
    if r.trim() != reg {
        return None;
    }
    imm.trim().strip_prefix('$')?.parse().ok()
}

/// True if the line is a conditional jump (`jCC label`).
fn is_cond_jcc(s: &str) -> bool {
    let Some((m, _)) = s.split_once(' ') else {
        return false;
    };
    matches!(
        m,
        "jo" | "jno"
            | "js"
            | "jns"
            | "je"
            | "jz"
            | "jne"
            | "jnz"
            | "jb"
            | "jc"
            | "jnae"
            | "jnb"
            | "jnc"
            | "jae"
            | "jbe"
            | "jna"
            | "ja"
            | "jnbe"
            | "jl"
            | "jnge"
            | "jge"
            | "jnl"
            | "jle"
            | "jng"
            | "jg"
            | "jnle"
            | "jp"
            | "jpe"
            | "jnp"
            | "jpo"
    )
}

/// True only when the line is a PURE write of `reg` (value replaced, old
/// value not read): mov/lea/setcc-style definitions.  Read-modify-write ALU
/// operations (addl/andl/...) and everything else return false — their
/// destination register is also a source, so deleting an earlier definition
/// that fed it would change observable state.
fn line_writes_reg_purely(line: &str, reg: RegId) -> bool {
    let Some((mnem, ops)) = line.split_once(' ') else {
        return false;
    };
    let pure_write = matches!(
        mnem,
        "movl" | "movw" | "movb" | "movzbl" | "movzwl" | "movsbl" | "movswl" | "leal" | "call"
    );
    if !pure_write {
        return false;
    }
    if mnem == "call" {
        // only %eax is defined by a plain call (return value)
        return reg == REG_EAX;
    }
    let Some((_, dst)) = ops.split_once(',') else {
        return false;
    };
    let dst = dst.trim();
    reg32_name(reg) == dst || format!("{}(%esp)", &reg32_name(reg)[1..]) == dst
}

/// Dominance-based dead-proof for a register whose definition is deleted or
/// rewritten by a staging-copy fold.  Returns true when EVERY read of `reg`
/// in `[fstart, fend)` is dominated, within its own basic block, by a later
/// pure write — i.e. no read can ever observe the deleted value.
///
/// Rules:
///  * `Label` starts a new block: entry state is unknown -> dominance resets.
///    (Both fallthrough and jump entrants are covered: a fallthrough entrant
///    executed all preceding dominated writes; a jump entrant skips to the
///    label, which is exactly the reset point.)
///  * `CondJmp` is transparent: the fallthrough continues in the same block.
///  * A pure write (mov/lea/movz/setcc-family, `call` for %eax) dominates
///    everything after it in the block.
///  * A read-modify-write or any other textual read of `reg` requires an
///    active dominance, otherwise the fold is refused.
///  * `call` ABI-clobbers %eax/%ecx/%edx, which counts as a defining write.
///  * Instructions with implicit register reads (rep-string ops, div/idiv,
///    one-operand mul, cltd, loop) are treated as reads.
///  * Lines in `excluded` (the staging window being deleted/rewritten) are
///    invisible: they neither read nor dominate.
fn staging_reads_safe(
    store: &LineStore,
    infos: &[LineInfo],
    fstart: usize,
    fend: usize,
    reg: RegId,
    excluded: &[usize],
) -> bool {
    // `pending_excluded` tracks whether a DELETED definition of `reg` has
    // been passed without an intervening pure redefinition.  A read of `reg`
    // that satisfies the forward `dominated` check can still observe the
    // deleted value when the dominating definition precedes the deleted
    // write: `movl 16(%esp),%eax; movl %esi,%eax; subl $-6,%eax` (deleted)
    // feeding `movl (%ecx,%eax,4),%eax` was judged "dominated" by the
    // earlier slot load, so the range-check fusion deleted the subl and the
    // jump-table fetch indexed a stale register
    // (gcc.c-torture execute/20011109-1.c @O1/O2).
    let mut dominated = false;
    let mut pending_excluded = false;
    let mut k = fstart;
    while k < fend {
        if infos[k].is_nop() || is_debug_location(store, infos, k) {
            k += 1;
            continue;
        }
        if excluded.contains(&k) {
            pending_excluded = true;
            k += 1;
            continue;
        }
        let line = trimmed(store, &infos[k], k);
        if infos[k].kind == LineKind::Label {
            dominated = false;
            k += 1;
            continue;
        }
        if line_implicitly_reads(line, reg) && !dominated {
            return false;
        }
        if line_references_reg(line, reg) {
            if line_writes_reg_purely(line, reg) {
                dominated = true;
                pending_excluded = false;
            } else {
                // A purely-architectural write is a REDEFINITION, not an
                // observation: `cltd` overwrites %edx without naming it,
                // so a staging copy into %edx that it follows can never be
                // observed by it (the oracle's read half is what makes
                // this exact — `idivl` both reads and writes %eax and is
                // correctly treated as an observer).
                let (imp_r, imp_w) = classify_implicit_operands(line);
                let bit = 1u8 << reg;
                let pure_redef = imp_w & bit != 0
                    && imp_r & bit == 0
                    && !line_references_reg_explicit(line, reg);
                if !pure_redef && (!dominated || pending_excluded) {
                    return false;
                }
                dominated = true;
                if pure_redef {
                    pending_excluded = false;
                }
            }
        }
        if infos[k].kind == LineKind::Call && matches!(reg, REG_EAX | REG_ECX | REG_EDX) {
            dominated = true;
            pending_excluded = false;
        }
        k += 1;
    }
    true
}

/// Count reads of `reg` (any width alias) in `[fstart, fend)`, excluding nop,
/// debug and `excluded` lines.  Pure writes of `reg` do not count; read-
/// modify-write forms do (they read the register they write).
fn census_reg_reads(
    store: &LineStore,
    infos: &[LineInfo],
    fstart: usize,
    fend: usize,
    reg: RegId,
    excluded: &[usize],
) -> usize {
    let mut n = 0;
    let mut k = fstart;
    while k < fend {
        if infos[k].is_nop() || is_debug_location(store, infos, k) || excluded.contains(&k) {
            k += 1;
            continue;
        }
        // Inline asm may read (or clobber) any register without naming it
        // in the template text — count it as a read of everything.
        if infos[k].kind == LineKind::InlineAsm {
            n += 1;
            k += 1;
            continue;
        }
        let line = trimmed(store, &infos[k], k);
        // A read is: an explicit textual mention that is not a pure write
        // of the family, or an architectural implicit READ (the oracle's
        // exact read half — `cltd` mentions %edx but does not read it,
        // while `stosl` genuinely consumes %eax).
        let explicitly = line_references_reg_explicit(line, reg);
        if (explicitly || line_implicitly_reads(line, reg))
            && !line_writes_reg_purely(line, reg)
        {
            n += 1;
        }
        k += 1;
    }
    n
}

/// Rewrite every branch in `[fstart, fend)` whose target is `from_name` to
/// target `to_name` instead.  Returns the number of rewritten branches.
///
/// Also rewrites whole-word references to `from_name` inside `.long`/`.quad`
/// data directives: jump tables hold ` .long LABEL - BASE` entries that bind
/// just as strongly as branch targets.  Threaded labels whose definition is
/// removed must not survive in table data, or the entry resolves to an
/// undefined symbol and the indirect jump lands on garbage
/// (gcc.c-torture execute/20011109-1.c @O1/O2, SIGSEGV in foo(1)).
fn retarget_label_refs(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    fstart: usize,
    fend: usize,
    from_name: &str,
    to_name: &str,
) -> usize {
    let mut n = 0;
    for i in fstart..fend {
        if infos[i].is_nop() {
            continue;
        }
        if matches!(infos[i].kind, LineKind::Directive) {
            // Jump-table data references: `.long LABEL - BASE` /
            // `.long BASE - LABEL`.  Replace whole-word label tokens only
            // (`.LBB7` must not match `.LBB77`).
            let line = trimmed(store, &infos[i], i);
            if !(line.starts_with(".long ") || line.starts_with(".quad ")) {
                continue;
            }
            if !label_referenced_whole_word(&line, from_name) {
                continue;
            }
            let new_line = replace_label_whole_word(&line, from_name, to_name);
            store.replace(i, format!("    {}", new_line));
            infos[i] = classify_line(store.get(i));
            n += 1;
            continue;
        }
        if !matches!(infos[i].kind, LineKind::Jmp | LineKind::CondJmp) {
            continue;
        }
        let line = trimmed(store, &infos[i], i);
        let targets_from = line
            .split_whitespace()
            .next_back()
            .is_some_and(|t| t == from_name);
        if targets_from {
            let prefix_len = line.len() - from_name.len();
            let new_line = format!("    {}{}", &line[..prefix_len], to_name);
            store.replace(i, new_line);
            infos[i] = classify_line(store.get(i));
            n += 1;
        }
    }
    n
}

/// Whether `name` appears in `text` as a whole word (identifier-boundary
/// delimited on both sides).  Used by retarget_label_refs for data directives.
fn label_referenced_whole_word(text: &str, name: &str) -> bool {
    let bytes = text.as_bytes();
    let nb = name.as_bytes();
    if nb.len() > bytes.len() {
        return false;
    }
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$';
    let mut start = 0usize;
    while start + nb.len() <= bytes.len() {
        if &bytes[start..start + nb.len()] == nb
            && (start == 0 || !is_word(bytes[start - 1]))
            && (start + nb.len() == bytes.len() || !is_word(bytes[start + nb.len()]))
        {
            return true;
        }
        start += 1;
    }
    false
}

/// Replace every whole-word occurrence of `from` with `to` in `text`.
fn replace_label_whole_word(text: &str, from: &str, to: &str) -> String {
    let bytes = text.as_bytes();
    let nb = from.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$';
    let mut out = String::with_capacity(text.len() + to.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if i + nb.len() <= bytes.len()
            && &bytes[i..i + nb.len()] == nb
            && (i == 0 || !is_word(bytes[i - 1]))
            && (i + nb.len() == bytes.len() || !is_word(bytes[i + nb.len()]))
        {
            out.push_str(to);
            i += nb.len();
        } else {
            let ch_len = utf8_char_len(bytes[i]);
            out.push_str(&text[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Jump threading through trivial blocks.  A trivial block is a `.L` label
/// whose body is exactly one unconditional `jmp X` (nops/directives aside)
/// that nothing falls into.  Every branch to the label is retargeted to X
/// and the block (label + jmp) is deleted.  Sound unconditionally: the
/// elided control transfer is flag-neutral and stack-neutral, so any entrant
/// observes identical state at X.  Iterated to a fixpoint by the caller, so
/// chains thread transitively.
fn thread_trivial_blocks(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    let mut fstart = 0usize;
    for idx in 0..=len {
        let at_end = idx == len;
        let is_boundary = at_end || {
            !infos[idx].is_nop() && {
                let s = trimmed(store, &infos[idx], idx);
                s == ".cfi_endproc" || s.starts_with(".size ")
            }
        };
        if !is_boundary {
            continue;
        }
        let fend = idx;

        // Label name -> line index, for target-existence checks.
        let mut labels: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for i in fstart..fend {
            if infos[i].is_nop() || infos[i].kind != LineKind::Label {
                continue;
            }
            let t = trimmed(store, &infos[i], i);
            if let Some(name) = t.strip_suffix(':') {
                labels.insert(name.to_string(), i);
            }
        }

        for (name, &label_idx) in labels.iter() {
            if infos[label_idx].is_nop() {
                continue; // already threaded in this pass
            }
            if !name.starts_with(".L") {
                continue;
            }
            // Body: exactly one Jmp among nops/directives before a Label.
            let mut body: Option<usize> = None;
            let mut k = label_idx + 1;
            let mut trivial = true;
            while k < fend {
                if infos[k].is_nop() || is_debug_location(store, infos, k) {
                    k += 1;
                    continue;
                }
                match infos[k].kind {
                    LineKind::Jmp if body.is_none() => {
                        body = Some(k);
                        k += 1;
                        continue;
                    }
                    LineKind::Label => break,
                    _ => {
                        trivial = false;
                        break;
                    }
                }
            }
            let Some(jmp_idx) = body else {
                continue;
            };
            if !trivial {
                continue;
            }
            let target = match trimmed(store, &infos[jmp_idx], jmp_idx)
                .split_whitespace()
                .next_back()
            {
                Some(t) => t.to_string(),
                None => continue,
            };
            if target == *name || !labels.contains_key(&target) {
                continue;
            }
            // Fallthrough guard: the previous real line must not enter the
            // block by falling through.  Ret/Jmp/JmpIndirect never do;
            // CondJmp and ordinary instructions both can.
            let mut guarded = label_idx == fstart;
            let mut b = label_idx;
            while b > fstart {
                b -= 1;
                if infos[b].is_nop() || is_debug_location(store, infos, b) {
                    continue;
                }
                match infos[b].kind {
                    LineKind::Directive | LineKind::Empty => continue,
                    LineKind::Ret | LineKind::Jmp | LineKind::JmpIndirect => {
                        guarded = true;
                    }
                    _ => {}
                }
                break;
            }
            if !guarded {
                continue;
            }
            // Census: branches referencing this label (label def excluded).
            let refs = (fstart..fend)
                .filter(|&i| {
                    !infos[i].is_nop()
                        && matches!(infos[i].kind, LineKind::Jmp | LineKind::CondJmp)
                        && trimmed(store, &infos[i], i)
                            .split_whitespace()
                            .next_back()
                            .is_some_and(|t| t == name)
                })
                .count();
            let _ = refs;
            retarget_label_refs(store, infos, fstart, fend, name, &target);
            infos[label_idx].kind = LineKind::Nop;
            infos[jmp_idx].kind = LineKind::Nop;
            changed = true;
        }

        fstart = idx + 1;
    }

    changed
}

/// Delete conditional branches whose outcome is irrelevant because BOTH
/// paths reach the same target: `jCC X` immediately followed by `jmp X`
/// (threading creates these), or `jCC X` immediately followed by `X:`.
/// The jCC itself is flag-neutral and side-effect-free, so deleting it is
/// unconditionally sound.
fn delete_redundant_cond_jumps(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].kind != LineKind::CondJmp {
            i += 1;
            continue;
        }
        let target = match trimmed(store, &infos[i], i).split_whitespace().next_back() {
            Some(t) => t.to_string(),
            None => {
                i += 1;
                continue;
            }
        };
        let j = next_non_nop(infos, i + 1);
        if j < len {
            let redundant = match infos[j].kind {
                LineKind::Jmp => trimmed(store, &infos[j], j)
                    .split_whitespace()
                    .next_back()
                    .is_some_and(|t| t == target),
                LineKind::Label => trimmed(store, &infos[j], j) == format!("{}:", target),
                _ => false,
            };
            if redundant {
                infos[i].kind = LineKind::Nop;
                changed = true;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    changed
}

fn fold_reg_copy_idioms(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    let mut fstart = 0usize;
    for idx in 0..=len {
        let at_end = idx == len;
        let is_boundary = at_end || {
            !infos[idx].is_nop() && {
                let s = trimmed(store, &infos[idx], idx);
                s == ".cfi_endproc" || s.starts_with(".size ")
            }
        };
        if !is_boundary {
            continue;
        }
        let fend = idx;

        let mut i = fstart;
        while i < fend {
            if infos[i].is_nop() || is_debug_location(store, infos, i) {
                i += 1;
                continue;
            }
            let s = trimmed(store, &infos[i], i).to_string();

            // Pattern 4: full-width slot load feeding a narrow zext/sext.
            //
            // Soundness (adjacent def-def shadowing): the deleted staging
            // load and its redefining extension are textually adjacent with
            // no label between (the parse requires it), so they execute as
            // an atomic pair.  Every path through the pair observes the
            // extension's result, never the staging value; every path NOT
            // through the pair never executed the staging write.  No read
            // anywhere can observe the deletion, so no liveness proof is
            // needed.
            let slot_load = regex_int_esp_full(&s);
            if let Some((disp, r32)) = slot_load {
                let j = next_non_nop(infos, i + 1);
                if j < fend {
                    let t = trimmed(store, &infos[j], j);
                    if let Some(new_insn) = narrow_ext_from_slot(t, disp, r32) {
                        store.replace(j, new_insn);
                        infos[j] = classify_line(store.get(j));
                        // Deletion convention: mark the staging load as a
                        // nop and leave the original text in place.
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
                i += 1;
                continue;
            }

            // Pattern 15: 16-bit canonical compare.  `movzwl SRC,%R` (or a
            // movzbl, which is also 16-bit-canonical) followed within a few
            // flag-neutral lines by `cmpl $imm,%R` with 128 <= imm <= 65535
            // and an order/equality conditional consumer: the 32-bit compare
            // is equivalent to `cmpw $imm,%r16` because both operands are
            // canonical 16-bit values, and the chosen condition codes (je/
            // jne/jb/jbe/ja/jae family) do not read OF, the one flag that
            // could differ between the widths.  `cmpw $imm16,%r16` encodes
            // one byte shorter than `cmpl $imm32,%r`.  Any other reference
            // to %R between definition and compare voids the canonicality.
            if let Some((imm_text, r)) = parse_cmpl_imm_any(&s) {
                if (128..=65535).contains(&imm_text) {
                    let i1 = next_non_nop(infos, i + 1);
                    if i1 < fend && is_order_cond_jcc(trimmed(store, &infos[i1], i1)) {
                        if let Some(def_idx) = canonical_16_def(store, infos, i, r, fstart) {
                            let r16 = low_subreg_name(reg32_name(r), 2).unwrap_or("%ax");
                            let new_cmp = format!("    cmpw ${}, {}", imm_text, r16);
                            store.replace(i, new_cmp);
                            infos[i] = classify_line(store.get(i));
                            changed = true;
                            i = i1;
                            continue;
                        }
                    }
                }
            }

            // Pattern 18: redundant re-zeroing.  Between two `xorl %X,%X`
            // whose only intervening lines are other xor-self zeroings
            // (flag-identical: zeroing sets ZF=1/CF=0/OF=0 regardless of the
            // register) or flag-neutral lines (mov/push/pop/lea) that do not
            // reference %X, the FIRST xor is invisible: the second produces
            // the same register value and the same flags.  Deleting the
            // earlier zero lets the fixpoint collapse whole RA spill-home
            // re-zero runs (`xor eax; xor edi; xor eax; xor ebp; xor eax`
            // -> one xor each).
            if let Some(x) = parse_xor_self(&s) {
                let mut k = i;
                let mut redundant = false;
                for _ in 0..6 {
                    let nk = next_non_nop(infos, k + 1);
                    if nk >= fend || nk - k > 8 {
                        break;
                    }
                    k = nk;
                    let t2 = trimmed(store, &infos[k], k);
                    if let Some(y) = parse_xor_self(t2) {
                        if y == x {
                            redundant = true;
                            break;
                        }
                        continue; // zeroing another register: flags identical
                    }
                    let neutral = t2.starts_with("movl ")
                        || t2.starts_with("movw ")
                        || t2.starts_with("movb ")
                        || t2.starts_with("pushl ")
                        || t2.starts_with("popl ")
                        || t2.starts_with("leal ");
                    if !neutral || line_references_reg(t2, reg_id_from_name(x)) {
                        break;
                    }
                }
                if redundant {
                    infos[i].kind = LineKind::Nop;
                    changed = true;
                    i += 1;
                    continue;
                }
            }

            let j = next_non_nop(infos, i + 1);
            if j >= fend {
                i += 1;
                continue;
            }
            let t = trimmed(store, &infos[j], j).to_string();

            // Pattern 2: zero through the copy (`xorl %X,%X; movl %X,%Y`).
            if let Some(x) = parse_xor_self(&s) {
                if let Some(y) = parse_mov_from(x, &t) {
                    if y != x {
                        let new_xor = format!("    xorl {}, {}", y, y);
                        store.replace(j, new_xor);
                        infos[j] = classify_line(store.get(j));
                        changed = true;
                    }
                    i = j + 1;
                    continue;
                }
                i += 1;
                continue;
            }

            // Patterns 1/3: staging move into a test or immediate ALU.
            if let Some((op_src, op_dst)) = parse_mov_reg_reg(&s) {
                if let Some(src2) = parse_test_pair(&t) {
                    if (src2 == (op_src, op_dst)) || (src2 == (op_dst, op_src)) {
                        let new_test = format!("    testl {}, {}", op_src, op_src);
                        store.replace(j, new_test);
                        infos[j] = classify_line(store.get(j));
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }
            if let Some((imm_text, c)) = parse_mov_imm_reg(&s) {
                if let Some((mnem, other)) = parse_alu_reg_reg(&t) {
                    if c == other.0 {
                        // `movl $imm,%C; andl %C,%X` -> `andl $imm,%X`
                        let new_alu = format!("    {} ${}, {}", mnem, imm_text, other.1);
                        store.replace(j, new_alu);
                        infos[j] = classify_line(store.get(j));
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                    // operand-order variant (`cmpl %X,%C` -> `cmpl %X,$imm`)
                    // is not encodable: immediates are always the second
                    // operand on x86. Skip to stay conservative.
                }
            }

            // Pattern 5: immediate staged through a register into a disp8
            // slot store (`movl $imm,%B; movl %B,N(%esp)`) ->
            // `movl $imm,N(%esp)`.  Fired only when %B has ZERO reads in the
            // whole function (census), which makes the staging definition's
            // deletion unconditionally invisible AND guarantees the dead-move
            // cleanup reclaims it — a guaranteed -5 bytes per site.
            if let Some((imm_text, b)) = parse_mov_imm_value(&s) {
                if let Some((src_reg, disp)) = parse_store_from_reg(&t) {
                    if src_reg == b
                        && (-128..=127).contains(&disp)
                        && census_reg_reads(
                            store,
                            infos,
                            fstart,
                            fend,
                            reg_id_from_name(b),
                            &[i, j],
                        ) == 0
                    {
                        let new_store = format!("    movl ${}, {}(%esp)", imm_text, disp);
                        store.replace(j, new_store);
                        infos[j] = classify_line(store.get(j));
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }

            // Pattern 16: constant staged through a register into a copy
            // (`movl $VAL,%B; movl %B,%C`) -> `movl $VAL,%C`, VAL a numeric
            // or symbol immediate.  Fired only when %B has ZERO reads in the
            // whole function (census), so the staging definition's deletion
            // is unconditionally invisible and the dead-move cleanup is
            // guaranteed to reclaim it: a guaranteed -2 bytes per site.
            // (A function-wide guarantee is required here: rewriting the
            // reader without the definition dying would trade a 2-byte
            // reg-reg copy for a 5-byte immediate mov — a net loss.)
            if let Some((val_text, b)) = parse_mov_imm_value(&s) {
                if let Some((src_reg, c)) = parse_mov_reg_reg(&t) {
                    if src_reg == b
                        && census_reg_reads(
                            store,
                            infos,
                            fstart,
                            fend,
                            reg_id_from_name(b),
                            &[i, j],
                        ) == 0
                    {
                        let new_copy = format!("    movl ${}, {}", val_text, c);
                        store.replace(j, new_copy);
                        infos[j] = classify_line(store.get(j));
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }

            // Pattern 6: value staged into a register for a compare or test
            // (`movl %A,%B; cmpl X,%B`) -> `cmpl X,%A`, X an immediate or a
            // register.  cmpl/testl write only flags, so %A survives and the
            // operand values are unchanged; identical values set identical
            // flags.  The staging move is deleted under the dominance proof.
            if let Some((op_src, op_dst)) = parse_mov_reg_reg(&s) {
                let mut handled = false;
                if let Some((mnem, dst)) = parse_cmp_test_imm(&t) {
                    if dst == op_dst
                        && staging_reads_safe(
                            store,
                            infos,
                            fstart,
                            fend,
                            reg_id_from_name(op_dst),
                            &[i, j],
                        )
                    {
                        let new_cmp = format!("    {} ${}, {}", mnem, imm_of(&t), op_src);
                        store.replace(j, new_cmp);
                        infos[j] = classify_line(store.get(j));
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = j + 1;
                        handled = true;
                    }
                } else if let Some((mnem, other, dst)) = parse_cmp_test_reg_reg(&t) {
                    if dst == op_dst
                        && other != op_src
                        && staging_reads_safe(
                            store,
                            infos,
                            fstart,
                            fend,
                            reg_id_from_name(op_dst),
                            &[i, j],
                        )
                    {
                        // When the compare's other operand IS the copy
                        // destination itself (`testl %D,%D` — the shape the
                        // boolean-select lowering emits after reloading a
                        // value through a staging copy), the rewrite must
                        // push BOTH operands through the alias:
                        // `testl %S,%S`.  Emitting `testl %D,%S` instead —
                        // leaving op_dst as the surviving first operand —
                        // keeps the compare reading a register that the
                        // `infos[i] = Nop` below just orphaned: any later
                        // redefinition of op_dst between the copy and the
                        // compare (e.g. `incl %eax` in the loop-increment)
                        // turns the condition into `defunct_dst & bool`.
                        // That was the deterministic 20071030-1.c[-O2]
                        // miscompile (CalcPing's count select observed the
                        // loop-increment value, count stayed 0, and the
                        // test aborted).  op_dst is a *name* string, so an
                        // exact string compare against the parsed operand
                        // text is the right equivalence test here.
                        let o1 = if other == op_dst { op_src } else { other };
                        let new_cmp = format!("    {} {}, {}", mnem, o1, op_src);
                        store.replace(j, new_cmp);
                        infos[j] = classify_line(store.get(j));
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = j + 1;
                        handled = true;
                    }
                }
                if handled {
                    continue;
                }
            }

            // Pattern 7: range check via subtract-then-compare
            // (`movl %A,%B; subl $a,%B; cmpl $b,%B; jCC L`)
            // -> `cmpl $(a+b),%A; jCC L`.  (x-a) CMP b and x CMP a+b set
            // identical flags for every condition code: both are the
            // comparison of the same two mathematical values, and the
            // borrow/overflow chains agree because the fused immediate
            // encodes exactly the same subtraction.  The staging pair is
            // deleted under the dominance proof for %B.
            if let Some((op_src, op_dst)) = parse_mov_reg_reg(&s) {
                let i1 = next_non_nop(infos, i + 1);
                let i2 = if i1 < fend {
                    next_non_nop(infos, i1 + 1)
                } else {
                    fend
                };
                let i3 = if i2 < fend {
                    next_non_nop(infos, i2 + 1)
                } else {
                    fend
                };
                if i3 < fend {
                    let t1 = trimmed(store, &infos[i1], i1);
                    let t2 = trimmed(store, &infos[i2], i2);
                    if let (Some(a), Some(b)) = (
                        parse_subl_imm_reg(t1, op_dst),
                        parse_cmpl_imm_reg(t2, op_dst),
                    ) {
                        let cond = trimmed(store, &infos[i3], i3);
                        if is_cond_jcc(cond) {
                            let fused = a.wrapping_add(b);
                            if (-128..=127).contains(&fused)
                                && staging_reads_safe(
                                    store,
                                    infos,
                                    fstart,
                                    fend,
                                    reg_id_from_name(op_dst),
                                    &[i, i1, i2],
                                )
                            {
                                let new_cmp = format!("    cmpl ${}, {}", fused, op_src);
                                store.replace(i2, new_cmp);
                                infos[i2] = classify_line(store.get(i2));
                                infos[i].kind = LineKind::Nop;
                                infos[i1].kind = LineKind::Nop;
                                changed = true;
                                i = i3;
                                continue;
                            }
                        }
                    }
                }
            }

            // Pattern 11: mask test through a dead copy
            // (`movl %A,%B; andl $imm,%B; jCC`) -> `testl $imm,%A` + jCC.
            // test computes the same AND product as andl and writes identical
            // SF/ZF/PF (CF/OF are zero for both); the product register %B is
            // deleted under the dominance proof.  (orl is NOT flag-equal to
            // test and is deliberately not matched.)
            if let Some((op_src, op_dst)) = parse_mov_reg_reg(&s) {
                // NOTE: the original gate scanned the andl line itself
                // (`next_non_nop(i + 1)`) for jCC-ness — never true — so
                // this pattern has been dormant since it landed.  The
                // corrected scan (`j + 1`) fires soundly (verified: flags
                // identical, staging dominance proof) but on the -g boot
                // corpus the fire perturbs the shared Phase 3.8 fixpoint:
                // three more staging copies survive than the fire reclaims
                // (net +9 in validate_cpu).  The dormant original gate is
                // restored until the pattern gets its own post-fire
                // re-fixpoint; see updates/followup for the analysis.
                let i1 = next_non_nop(infos, i + 1);
                if i1 < fend && is_cond_jcc(trimmed(store, &infos[i1], i1)) {
                    let imm = parse_andl_imm_reg(&t, &op_dst).map(|s| s.to_string());
                    if imm.is_some()
                        && staging_reads_safe(
                            store,
                            infos,
                            fstart,
                            fend,
                            reg_id_from_name(op_dst),
                            &[i, j],
                        )
                    {
                        let new_test = format!("    testl ${}, {}", imm.unwrap(), op_src);
                        store.replace(j, new_test);
                        infos[j] = classify_line(store.get(j));
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = i1;
                        continue;
                    }
                }
            }

            // Pattern 13: slot store feeding a widened slot load of the SAME
            // 4-byte slot (`movl %R,S(%esp); movzbl S(%esp),%C`) — the load
            // consumes exactly the low byte/half the store just wrote, so it
            // can read it straight from %R's subregister.  i386 encodes byte
            // and halfword subregisters only for eax/ecx/edx/ebx (no REX),
            // which bounds the match.  Pure rewrite: value, flags and both
            // lines' register effects are identical.
            if let Some((src_reg, disp)) = parse_store_from_reg(&s) {
                if let Some((mnem, width, dst)) = parse_widened_slot_load(&t, disp) {
                    if let Some(sub) = low_subreg_name(src_reg, width) {
                        let new_load = format!("    {} {}, {}", mnem, sub, dst);
                        store.replace(j, new_load);
                        infos[j] = classify_line(store.get(j));
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }

            // Pattern 14: doubling through a dead copy
            // (`movl %X,%Y; addl %Y,%X`) -> `addl %X,%X`.  %Y equals %X, so
            // sum and flags are identical; %Y is deleted under the dominance
            // proof.
            if let Some((x, y)) = parse_mov_reg_reg(&s) {
                if let Some((add_src, add_dst)) = parse_addl_reg_reg(&t) {
                    if add_src == y
                        && add_dst == x
                        && staging_reads_safe(
                            store,
                            infos,
                            fstart,
                            fend,
                            reg_id_from_name(y),
                            &[i, j],
                        )
                    {
                        let new_add = format!("    addl {}, {}", x, x);
                        store.replace(j, new_add);
                        infos[j] = classify_line(store.get(j));
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i = j + 1;
                        continue;
                    }
                }
            }

            i += 1;
        }

        // Pattern 8: identical adjacent block tail-merge.  When block X is
        // `[insns] jmp L`, is immediately followed by block Y holding the
        // SAME insns (text-identical), and Y falls through into L, X's body
        // becomes `jmp Y`; X's duplicated insns and X's original jump
        // disappear, while Y's copy survives and serves both predecessor
        // sets.  Text-identical insns set identical flags, so any flag
        // consumer at L observes the same state.
        {
            let mut i = fstart;
            while i < fend {
                if infos[i].is_nop()
                    || infos[i].kind != LineKind::Label
                    || is_debug_location(store, infos, i)
                {
                    i += 1;
                    continue;
                }
                // collect up to 4 insns after the label
                let mut insns: Vec<usize> = Vec::new();
                let mut k = i + 1;
                while k < fend && insns.len() < 4 {
                    if infos[k].is_nop() || is_debug_location(store, infos, k) {
                        k += 1;
                        continue;
                    }
                    match infos[k].kind {
                        LineKind::Label => break,
                        // block terminators end the straight-line body
                        LineKind::Jmp
                        | LineKind::CondJmp
                        | LineKind::Ret
                        | LineKind::JmpIndirect => break,
                        _ => {}
                    }
                    insns.push(k);
                    k += 1;
                }
                if insns.is_empty() || k >= fend || infos[k].kind != LineKind::Jmp {
                    i += 1;
                    continue;
                }
                let jmp_target = match trimmed(store, &infos[k], k).split_whitespace().next_back() {
                    Some(t) => t.to_string(),
                    None => {
                        i += 1;
                        continue;
                    }
                };
                let body: Vec<String> = insns
                    .iter()
                    .map(|&x| trimmed(store, &infos[x], x).to_string())
                    .collect();
                // Y must start right after the jmp
                let y = next_non_nop(infos, k + 1);
                if y >= fend || infos[y].kind != LineKind::Label {
                    i += 1;
                    continue;
                }
                // collect Y's insns
                let mut y_insns: Vec<usize> = Vec::new();
                let mut k2 = y + 1;
                while k2 < fend && y_insns.len() < body.len() {
                    if infos[k2].is_nop() || is_debug_location(store, infos, k2) {
                        k2 += 1;
                        continue;
                    }
                    match infos[k2].kind {
                        LineKind::Label => break,
                        LineKind::Jmp
                        | LineKind::CondJmp
                        | LineKind::Ret
                        | LineKind::JmpIndirect => break,
                        _ => {}
                    }
                    y_insns.push(k2);
                    k2 += 1;
                }
                if y_insns.len() != body.len() {
                    i += 1;
                    continue;
                }
                let y_body: Vec<String> = y_insns
                    .iter()
                    .map(|&x| trimmed(store, &infos[x], x).to_string())
                    .collect();
                if y_body != body {
                    i += 1;
                    continue;
                }
                // Y must fall through into the same jmp target L
                let l = next_non_nop(infos, k2);
                if l >= fend || infos[l].kind != LineKind::Label {
                    i += 1;
                    continue;
                }
                let lname = trimmed(store, &infos[l], l);
                let lname = match lname.strip_suffix(':') {
                    Some(n) => n.to_string(),
                    None => {
                        i += 1;
                        continue;
                    }
                };
                if lname != jmp_target {
                    i += 1;
                    continue;
                }
                // X -> jmp Y; X's duplicated body and X's original jmp are
                // reclaimed.  Y's body is the SURVIVING copy and MUST stay:
                // every predecessor of X (and X's own fall-in) now executes
                // it via the jmp, and Y's other predecessors always did.
                // Deleting it as well left the merged computation nowhere —
                // observed as a miscompile where both arms of a bool-merge
                // diamond stored the same constant and the store vanished
                // (upstream bool_thread_merges regression, `a && b` value
                // context: got stale stack garbage instead of the constant).
                // The jmp overwrites X's first body line (never i+1: the
                // collector may have skipped nops after the label).
                let xj = insns[0];
                let yname = {
                    let t = trimmed(store, &infos[y], y);
                    t[..t.len() - 1].to_string() // strip ':'
                };
                store.replace(xj, format!("    jmp {}", yname));
                infos[xj] = classify_line(store.get(xj));
                for &x in insns.iter().skip(1) {
                    infos[x].kind = LineKind::Nop;
                }
                infos[k].kind = LineKind::Nop;
                changed = true;
                i += 1;
            }
        }

        fstart = idx + 1;
    }

    changed
}

/// Parse `cmpl %C, %B` / `testl %C, %B` returning (mnemonic, C, B).
fn parse_cmp_test_reg_reg(s: &str) -> Option<(&'static str, &str, &str)> {
    for mnem in ["cmpl", "testl"] {
        if let Some(rest) = s.strip_prefix(mnem).and_then(|r| r.strip_prefix(' ')) {
            let (a, b) = rest.split_once(',')?;
            let a = a.trim();
            let b = b.trim();
            if a.starts_with("%e") && b.starts_with("%e") {
                return Some((mnem, a, b));
            }
            return None;
        }
    }
    None
}

/// Parse `andl $imm, %B` returning the immediate text (B given).
fn parse_andl_imm_reg<'a>(s: &'a str, reg: &str) -> Option<&'a str> {
    let rest = s.strip_prefix("andl ")?;
    let (imm, r) = rest.split_once(',')?;
    if r.trim() != reg {
        return None;
    }
    let imm = imm.trim().strip_prefix('$')?;
    if imm.is_empty() {
        return None;
    }
    Some(imm)
}

/// Parse `addl %A, %B` returning (A, B).
fn parse_addl_reg_reg(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("addl ")?;
    let (a, b) = rest.split_once(',')?;
    let a = a.trim();
    let b = b.trim();
    if a.starts_with("%e") && b.starts_with("%e") {
        Some((a, b))
    } else {
        None
    }
}

/// Parse `movzbl|movsbl|movzwl|movswl DISP(%esp), %D` where DISP matches;
/// returns (mnemonic, width-in-bytes, %D).  The slot must be a plain disp
/// (no registers), matching the store side exactly.
fn parse_widened_slot_load<'a>(s: &'a str, disp: i32) -> Option<(&'a str, i32, &'a str)> {
    let (mnem, ops) = s.split_once(' ')?;
    let (mem, dst) = ops.split_once(',')?;
    let dst = dst.trim();
    let expected = format!("{}(%esp)", disp);
    if mem.trim() != expected || !dst.starts_with("%e") {
        return None;
    }
    let width = match mnem {
        "movzbl" | "movsbl" => 1,
        "movzwl" | "movswl" => 2,
        _ => return None,
    };
    Some((mnem, width, dst))
}

/// Low subregister name for a 32-bit register at the given width, i386-
/// encodable set only (al/cl/dl/bl and ax/cx/dx/bx; sil/dil/bpl need REX).
fn low_subreg_name(reg32: &str, width: i32) -> Option<&'static str> {
    match (reg32, width) {
        ("%eax", 1) => Some("%al"),
        ("%ecx", 1) => Some("%cl"),
        ("%edx", 1) => Some("%dl"),
        ("%ebx", 1) => Some("%bl"),
        ("%eax", 2) => Some("%ax"),
        ("%ecx", 2) => Some("%cx"),
        ("%edx", 2) => Some("%dx"),
        ("%ebx", 2) => Some("%bx"),
        _ => None,
    }
}

/// Parse `movl %S, %D` (both 32-bit GPRs, S != D).
fn parse_mov_reg_reg(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("movl ")?;
    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();
    if !(src.starts_with("%e") && dst.starts_with("%e")) {
        return None;
    }
    if src == dst {
        return None;
    }
    Some((src, dst))
}

/// Parse `testl %A, %B` returning (A, B).
fn parse_test_pair(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("testl ")?;
    let (a, b) = rest.split_once(',')?;
    let a = a.trim();
    let b = b.trim();
    if a.starts_with("%e") && b.starts_with("%e") {
        Some((a, b))
    } else {
        None
    }
}

/// Parse `xorl %X, %X` returning X.
fn parse_xor_self(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("xorl ")?;
    let (a, b) = rest.split_once(',')?;
    let a = a.trim();
    let b = b.trim();
    if a == b && a.starts_with("%e") {
        Some(a)
    } else {
        None
    }
}

/// Parse `movl %X, %Y` where X is the given zeroed register.
fn parse_mov_from<'a>(x: &str, s: &'a str) -> Option<&'a str> {
    let rest = s.strip_prefix("movl ")?;
    let (src, dst) = rest.split_once(',')?;
    if src.trim() == x {
        let dst = dst.trim();
        if dst.starts_with("%e") {
            return Some(dst);
        }
    }
    None
}

/// Parse `movl $VAL, %C` where VAL is a plain integer or an assembler
/// symbol/absolute expression token (no commas, whitespace, parens or
/// memory syntax) — returns (VAL-without-dollar, C).
fn parse_mov_imm_value(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("movl ")?;
    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();
    let imm = src.strip_prefix('$')?;
    if imm.is_empty()
        || imm.contains(|ch: char| ch.is_whitespace() || matches!(ch, ',' | '(' | ')' | ':' | '+'))
        || !dst.starts_with("%e")
    {
        return None;
    }
    Some((imm, dst))
}

/// Parse `movl $imm, %C` returning (imm_text_without_dollar_prefix, C).
fn parse_mov_imm_reg(s: &str) -> Option<(&str, &str)> {
    let rest = s.strip_prefix("movl ")?;
    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();
    let imm = src.strip_prefix('$')?;
    if imm.is_empty() || !dst.starts_with("%e") {
        return None;
    }
    // Reject symbolic immediates; only plain integers are safe to re-encode.
    if !imm
        .bytes()
        .all(|c| c.is_ascii_digit() || c == b'-' || c == b'x' || (c as char).is_ascii_hexdigit())
    {
        return None;
    }
    Some((imm, dst))
}

/// Parse `ALU %A, %B` returning ((A, B), B-as-second).
fn parse_alu_reg_reg(s: &str) -> Option<(&str, (&str, &str))> {
    for mnem in ["andl", "orl", "xorl", "addl", "subl", "cmpl", "testl"] {
        let rest = s.strip_prefix(mnem).and_then(|r| r.strip_prefix(' '));
        if let Some(rest) = rest {
            let (a, b) = rest.split_once(',')?;
            let a = a.trim();
            let b = b.trim();
            if a.starts_with("%e") && b.starts_with("%e") {
                return Some((mnem, (a, b)));
            }
            return None;
        }
    }
    None
}

/// Recognize `movl N(%esp), %R32`; return (N, %R32).
fn regex_int_esp_full(s: &str) -> Option<(i32, &'static str)> {
    let rest = s.strip_prefix("movl ")?;
    let (mem, reg) = rest.split_once(',')?;
    let reg = reg.trim();
    let mem = mem.trim();
    let inner = mem.strip_suffix("(%esp)")?;
    let disp: i32 = inner.trim().parse().ok()?;
    let known = match reg {
        "%eax" => "%eax",
        "%ecx" => "%ecx",
        "%edx" => "%edx",
        "%ebx" => "%ebx",
        "%esi" => "%esi",
        "%edi" => "%edi",
        _ => return None,
    };
    Some((disp, known))
}

/// If `t` is `movz{b,w}l %r8/%r16,%R32` (or movs), where r8/r16 is the low
/// subregister of R32 and R32 matches, rewrite to a direct widened slot load.
fn narrow_ext_from_slot(t: &str, disp: i32, r32: &'static str) -> Option<String> {
    let (mnem, src, dst) = {
        let (mnem, ops) = t.split_once(' ')?;
        let (a, b) = ops.split_once(',')?;
        (mnem, a.trim(), b.trim())
    };
    let width: i32 = match mnem {
        "movzbl" | "movsbl" => 1,
        "movzwl" | "movswl" => 2,
        _ => return None,
    };
    let low: &str = match (r32, width) {
        ("%eax", 1) => "%al",
        ("%ecx", 1) => "%cl",
        ("%edx", 1) => "%dl",
        ("%ebx", 1) => "%bl",
        ("%eax", 2) => "%ax",
        ("%ecx", 2) => "%cx",
        ("%edx", 2) => "%dx",
        ("%ebx", 2) => "%bx",
        _ => return None,
    };
    if src != low || dst != r32 {
        return None;
    }
    Some(format!("    {} {}(%esp), {}", mnem, disp, r32))
}

fn fold_global_fnptr_calls(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    // Process one function at a time so the whole-function reference check
    // is scoped correctly.
    let mut fstart = 0usize;
    for idx in 0..=len {
        let at_end = idx == len;
        let is_boundary = at_end || {
            !infos[idx].is_nop() && {
                let s = trimmed(store, &infos[idx], idx);
                s == ".cfi_endproc" || s.starts_with(".size ")
            }
        };
        if !is_boundary {
            continue;
        }
        let fend = idx;

        // Phase 1: collect candidate triples (load_i, store_j, call_k, sym, disp).
        struct Triple {
            li: usize,
            sj: usize,
            ck: usize,
            sym: String,
            disp: i32,
        }
        let mut triples: Vec<Triple> = Vec::new();
        let mut i = fstart;
        'scan: while i < fend {
            if infos[i].is_nop() {
                i += 1;
                continue;
            }

            // Line i: `movl SYM, %reg` (classified Other{dest_reg}: symbolic
            // source, so it is neither Move nor LoadEbp).
            let LineKind::Other { dest_reg } = infos[i].kind else {
                i += 1;
                continue;
            };
            if dest_reg > REG_GP_MAX {
                i += 1;
                continue;
            }
            let li = trimmed(store, &infos[i], i);
            let Some(rest) = li.strip_prefix("movl ") else {
                i += 1;
                continue;
            };
            let Some((sym, dst)) = rest.split_once(',') else {
                i += 1;
                continue;
            };
            let sym = sym.trim();
            let dst = dst.trim();
            if register_family(dst) != dest_reg {
                i += 1;
                continue;
            }
            let sb = sym.as_bytes();
            if sb.is_empty() || !(sb[0].is_ascii_alphabetic() || sb[0] == b'_') {
                i += 1;
                continue;
            }
            if !sym
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'$' | b'+' | b'-'))
            {
                i += 1;
                continue;
            }

            // Line j: `movl %reg, N(%esp)`.
            let j = next_non_nop(infos, i + 1);
            if j >= fend {
                break;
            }
            let LineKind::StoreEbp {
                reg: sreg,
                offset,
                size: MoveSize::L,
            } = infos[j].kind
            else {
                i += 1;
                continue;
            };
            if sreg != dest_reg || offset < ESP_SLOT_BIAS / 2 {
                i += 1;
                continue;
            }
            let raw_disp = offset - ESP_SLOT_BIAS;

            // Forward scan for `call *N(%esp)`; only reg/imm-only data ops
            // that do not READ %reg may be crossed (a pure overwrite is
            // fine — MOV-family writes without reading; ALU ops read their
            // destination). No memory operands of any kind may be crossed,
            // which also rules out ESP changes and slot writes.
            let call_txt = format!("call *{}(%esp)", raw_disp);
            let mut k = j + 1;
            let mut found: Option<usize> = None;
            let mut cnt = 0;
            while k < fend && cnt < 12 {
                if infos[k].is_nop() || is_debug_location(store, infos, k) {
                    k += 1;
                    continue;
                }
                let s = trimmed(store, &infos[k], k);
                if s == call_txt {
                    found = Some(k);
                    break;
                }
                let mn = s.split_ascii_whitespace().next().unwrap_or("");
                const MOVS: &[&str] = &["movl", "movzbl", "movzwl", "movsbl", "movswl"];
                const ALUS: &[&str] = &[
                    "addl", "subl", "andl", "orl", "xorl", "imull", "shll", "shrl", "sarl", "incl",
                    "decl", "negl", "notl",
                ];
                let is_mov = MOVS.contains(&mn);
                let is_alu = ALUS.contains(&mn);
                let ok = (is_mov || is_alu) && {
                    let ops: Vec<&str> = s[mn.len()..].split(',').map(str::trim).collect();
                    ops.iter()
                        .all(|op| op.starts_with('%') || op.starts_with('$'))
                        && {
                            let n = ops.len();
                            // Zero idioms `xorl %r, %r` / `subl %r, %r`
                            // architecturally do NOT read %r (result is the
                            // constant 0) — they are pure overwrites and must
                            // not be treated as readers of the spilled
                            // register. Boot code stages zero arguments with
                            // `xorl %eax, %eax` between the fptr spill and
                            // the call (outb(0, 0x80) in empty_8042), which
                            // used to block every such fold.
                            let is_zero_idiom = is_alu
                                && n == 2
                                && (mn == "xorl" || mn == "subl")
                                && register_family(ops[0]) == register_family(ops[1]);
                            let reads_reg = ops.iter().enumerate().any(|(oi, op)| {
                                let is_dest = oi + 1 == n;
                                if is_dest && (is_mov || (is_zero_idiom && oi + 1 == n)) {
                                    return false;
                                }
                                if is_zero_idiom {
                                    // both operands are the zeroed register:
                                    // neither is a read.
                                    return false;
                                }
                                op.starts_with('%') && register_family(op) == dest_reg
                            });
                            !reads_reg
                        }
                };
                if !ok {
                    i += 1;
                    continue 'scan;
                }
                k += 1;
                cnt += 1;
            }
            let Some(ci) = found else {
                i += 1;
                continue;
            };
            triples.push(Triple {
                li: i,
                sj: j,
                ck: ci,
                sym: sym.to_string(),
                disp: raw_disp,
            });
            i = ci + 1;
        }

        // Phase 2: decide foldability per triple by one of two proofs.
        //
        // (a) SLOT OWNERSHIP: every textual reference to the displacement in
        //     the function is a member (store or call) of some candidate
        //     triple. With no foreign reference, no stale read can exist
        //     under ANY ESP numbering (raw-text matching over-approximates
        //     aliasing; an unmatched reference merely fails this proof).
        //     Triples on one slot are disjoint intervals (the store->call
        //     window crosses only reg/imm ops), so folding any subset is
        //     independent of the rest.
        //
        // (b) LINEAR SEAL: walking forward from the triple's call, crossing
        //     only lines that are not labels, not branches/rets, not
        //     push/pop/ESP arithmetic, and do not reference the slot, the
        //     FIRST reference to the slot is a full-width `movl` store.
        //     Then our store's value is observable only by our call: the
        //     span store->call crosses reg/imm ops only, and the span
        //     call->seal has no reader before the slot is overwritten.
        //     Chains stay sound by induction: if the sealing store belongs
        //     to a later triple that also folds, THAT triple's own proof
        //     guarantees the next access is again a seal (its window and
        //     walk are label/branch-free), so removing intermediate
        //     store/call pairs never exposes a reader.
        if !triples.is_empty() {
            let mut owned: Vec<i32> = Vec::new();
            {
                let mut disps: Vec<i32> = triples.iter().map(|t| t.disp).collect();
                disps.sort_unstable();
                disps.dedup();
                for d in disps {
                    let members: Vec<&Triple> = triples.iter().filter(|t| t.disp == d).collect();
                    // Overlap window [d-3, d+3]: any 1/2/4-byte access whose
                    // lowest address lands there can touch our 4 bytes.
                    let foreign = (fstart..fend).any(|t| {
                        !infos[t].is_nop()
                            && !members.iter().any(|m| m.sj == t || m.ck == t)
                            && (d - 3..=d + 3)
                                .any(|dd| references_esp_slot(trimmed(store, &infos[t], t), dd))
                    });
                    if !foreign {
                        owned.push(d);
                    }
                }
            }

            let linear_seal = |m: &Triple| -> bool {
                let mut k = m.ck + 1;
                while k < fend {
                    if infos[k].is_nop() || is_debug_location(store, infos, k) {
                        k += 1;
                        continue;
                    }
                    let s = trimmed(store, &infos[k], k);
                    match infos[k].kind {
                        LineKind::Empty | LineKind::Directive => {
                            k += 1;
                            continue;
                        }
                        LineKind::Label
                        | LineKind::Jmp
                        | LineKind::JmpIndirect
                        | LineKind::CondJmp
                        | LineKind::Ret
                        | LineKind::Push { .. }
                        | LineKind::Pop { .. } => return false,
                        LineKind::StoreEbp { offset, size, .. }
                            if offset == ESP_SLOT_BIAS + m.disp =>
                        {
                            // Exact full-width overwrite seals the lifetime.
                            return size == MoveSize::L;
                        }
                        _ => {}
                    }
                    // ESP arithmetic renumbers the slot; any textual
                    // reference OVERLAPPING our 4 bytes ([disp-3, disp+3]
                    // catches every 1/2/4-byte access that could touch
                    // them) is a potential read or partial overwrite.
                    if parse_esp_adjust(s, "subl").is_some()
                        || parse_esp_adjust(s, "addl").is_some()
                        || (s.contains("%esp") && !s.contains("(%esp)"))
                    {
                        return false;
                    }
                    if (m.disp - 3..=m.disp + 3).any(|d| references_esp_slot(s, d)) {
                        return false;
                    }
                    k += 1;
                }
                false
            };

            let fold_flags: Vec<bool> = triples
                .iter()
                .map(|m| owned.contains(&m.disp) || linear_seal(m))
                .collect();
            for (m, fold) in triples.iter().zip(fold_flags) {
                if !fold {
                    continue;
                }
                infos[m.li].kind = LineKind::Nop;
                infos[m.sj].kind = LineKind::Nop;
                store.replace(m.ck, format!("    call *{}", m.sym));
                infos[m.ck] = classify_line(store.get(m.ck));
                changed = true;
            }
        }

        fstart = fend + 1;
    }

    changed
}

// ── Pass: Never-read store elimination ───────────────────────────────────────

/// Find stack slots that are never loaded and remove all stores to them.
///
/// The analysis MUST be scoped per function (the x86-64 twin already is):
/// the module buffer holds every function of the TU, so a read of 24(%esp)
/// in one function would otherwise keep a dead store to 24(%esp) alive in
/// every other function, and the label/jump between two functions makes the
/// linear ESP walk give up (`esp_walk_ok = false`) for the whole buffer.
/// Function extents are delimited by the `.cfi_endproc` / `.size` markers the
/// emitter always writes (same delimiters the CFG-liveness pass keys on).
fn eliminate_never_read_stores(store: &LineStore, infos: &mut [LineInfo]) {
    let len = infos.len();
    let mut start = 0usize;
    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let s = trimmed(store, &infos[i], i);
        if s == ".cfi_endproc" || s.starts_with(".size ") {
            eliminate_never_read_stores_range(store, infos, start, i + 1);
            start = i + 1;
        }
    }
    if start < len {
        eliminate_never_read_stores_range(store, infos, start, len);
    }
}

/// The per-function body of [`eliminate_never_read_stores`]; analyses and
/// rewrites only `[start, end)`.
fn eliminate_never_read_stores_range(
    store: &LineStore,
    infos: &mut [LineInfo],
    start: usize,
    end: usize,
) {
    // ── ESP-offset soundness ────────────────────────────────────────────────
    // N(%esp) names DIFFERENT memory before and after any ESP change: a store
    // to 20(%esp) followed by two pushes is read back as 28(%esp). Comparing
    // raw ESP offsets across the whole function (as this pass once did)
    // deleted live parameter-capture stores in exactly that pattern.
    //
    // Every ESP-relative offset is therefore NORMALIZED to a frame-anchored
    // coordinate (offset − sp_down, where sp_down is how far ESP sits below
    // function entry at that line, tracked through pushl/popl and
    // subl/addl $imm,%esp). Two situations make the linear sp_down walk
    // unsound, and each falls back conservatively rather than guessing:
    //
    //  * a CALL may change net ESP invisibly (i686 sret callees pop the
    //    hidden pointer with `ret $4`): after any call, an ESP store is
    //    eliminated only if the function contains NO ESP-based reads at all
    //    (shift-immune criterion — a dead slot is dead under any numbering);
    //  * a label/jump reached with unbalanced pushes, or an ESP write that
    //    is not push/pop/subl/addl $imm: give up on ESP stores entirely.
    //
    // EBP-relative offsets are stable (EBP never moves inside the body), so
    // the EBP domain keeps full precision in all cases. The two domains are
    // kept apart by ESP_SLOT_BIAS exactly as in the windowed passes.
    let mut sp_down: i64 = 0;
    let mut sp_base: Option<i64> = None; // sp_down after first explicit allocation
    let mut esp_walk_ok = true;
    let mut has_call = false;
    let mut any_esp_read = false;
    let mut norm: Vec<i32> = vec![0; end - start]; // per-line ESP normalization (subtract from biased offset)

    for i in start..end {
        if infos[i].is_nop() {
            continue;
        }
        let s = trimmed(store, &infos[i], i);
        norm[i - start] = sp_down as i32;
        match infos[i].kind {
            LineKind::Push { .. } => {
                norm[i - start] = sp_down as i32;
                sp_down += 4;
            }
            LineKind::Pop { .. } => {
                any_esp_read = true;
                sp_down -= 4;
                norm[i - start] = sp_down as i32;
            }
            LineKind::Call => {
                has_call = true;
            }
            LineKind::Label | LineKind::Jmp | LineKind::CondJmp => {
                if let Some(base) = sp_base {
                    if sp_down != base {
                        esp_walk_ok = false;
                    }
                } else if sp_down != 0 {
                    // control flow before the frame allocation: give up
                    esp_walk_ok = false;
                }
            }
            _ => {
                if let Some(rest) = s.strip_prefix("subl $") {
                    if let Some(n) = rest
                        .strip_suffix(", %esp")
                        .and_then(|v| v.parse::<i64>().ok())
                    {
                        sp_down += n;
                        if sp_base.is_none() {
                            sp_base = Some(sp_down);
                        }
                        continue;
                    }
                }
                if let Some(rest) = s.strip_prefix("addl $") {
                    if let Some(n) = rest
                        .strip_suffix(", %esp")
                        .and_then(|v| v.parse::<i64>().ok())
                    {
                        sp_down -= n;
                        continue;
                    }
                }
                // Any other ESP write invalidates the walk.
                if matches!(infos[i].kind, LineKind::Other { dest_reg } if dest_reg == REG_ESP)
                    || (s.contains("%esp") && !s.contains("(%esp)"))
                {
                    esp_walk_ok = false;
                }
            }
        }
    }

    let is_esp_biased = |off: i32| off >= ESP_SLOT_BIAS / 2;

    // %ebp is the frame base only when the prologue establishes it. Under
    // -fomit-frame-pointer (kernel realmode) %ebp is a data register:
    // (%ebp) accesses are POINTER derefs that the classifier nevertheless
    // parses as ebp-domain frame slots. Those must be exempt from
    // elimination (they write through a pointer) but do not bail the pass
    // (no-escape discipline covers them).
    let ebp_is_frame = (start..end)
        .any(|i| !infos[i].is_nop() && trimmed(store, &infos[i], i) == "movl %esp, %ebp");

    // Collect all loaded byte ranges in normalized coordinates.
    let mut read_ranges: Vec<(i32, i32)> = Vec::new();
    let mut addr_taken = false;

    for i in start..end {
        if infos[i].is_nop() {
            continue;
        }
        let normalize = |off: i32| {
            if is_esp_biased(off) {
                off - norm[i - start]
            } else {
                off
            }
        };
        match infos[i].kind {
            LineKind::LoadEbp { offset, size, .. } => {
                if is_esp_biased(offset) {
                    any_esp_read = true;
                }
                read_ranges.push((normalize(offset), size.byte_size()));
            }
            _ => {
                let s = trimmed(store, &infos[i], i);
                // Escape discipline (ported from the x86-64 DSE): a frame
                // slot is only observable through an indirect access if a
                // frame address ESCAPED first — leal of a slot, or a raw
                // frame-pointer VALUE copied into a data register. Without
                // an escape event, pointer derefs target caller-owned
                // memory and cannot alias this fresh frame, so
                // has_indirect_mem alone is NOT a bail. Any escape still
                // disables the pass for the whole function.
                //
                // %ebp is only a frame base in FP mode (ebp_is_frame,
                // detected from the `movl %esp, %ebp` prologue). Under
                // -fomit-frame-pointer it is an ordinary data register:
                // its value moves are NOT escapes, and `leal N(%ebp)` is
                // pointer arithmetic on data, not a slot address.
                let leal_esp = s.starts_with("leal ") && s.contains("(%esp)");
                let leal_ebp = s.starts_with("leal ") && s.contains("(%ebp)");
                if leal_esp || (ebp_is_frame && leal_ebp) {
                    addr_taken = true;
                }
                // Raw %esp value flowing somewhere it can be observed
                // (`movl %esp, %reg`, `movl %esp, mem`). Frame adjustments
                // (subl/addl $imm) were consumed by the sp_down walk; the
                // FP prologue/epilogue moves reposition the stack, they do
                // not leak an address into data flow.
                if s.contains("%esp")
                    && !s.contains("(%esp)")
                    && !s.starts_with("subl $")
                    && !s.starts_with("addl $")
                    && s != "movl %esp, %ebp"
                    && s != "movl %ebp, %esp"
                    && !matches!(infos[i].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                {
                    addr_taken = true;
                }
                // Raw %ebp value as data — an escape only in FP mode.
                if ebp_is_frame
                    && s.contains("%ebp")
                    && !s.contains("(%ebp)")
                    && s != "movl %esp, %ebp"
                    && s != "movl %ebp, %esp"
                    && !matches!(infos[i].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                {
                    addr_taken = true;
                }
                // Track %ebp/%esp-relative reads from non-Load/Store lines
                // (e.g. folded memory operands like "cmpl -44(%ebp), %eax")
                let ebp_off = infos[i].ebp_offset;
                if ebp_off != EBP_OFFSET_NONE {
                    if is_esp_biased(ebp_off) {
                        any_esp_read = true;
                    }
                    // Conservatively treat as a 16-byte read. 4 bytes (the
                    // GP-store maximum) is NOT enough: x87 folded operands
                    // read past the first dword — `fldl 16(%esp)` reads
                    // 16..24 and `fldt` reads 10 bytes — and undersizing the
                    // range let the pass delete the live high-word store of
                    // a double parameter (d_i regparm miscompile). 16 bytes
                    // covers every x86 memory read that names this offset
                    // as its lowest address.
                    read_ranges.push((normalize(ebp_off), 16));
                } else if !matches!(infos[i].kind, LineKind::StoreEbp { .. })
                    && (s.contains("(%ebp)") || s.contains("(%esp)"))
                {
                    // Unknown frame reference - bail out
                    addr_taken = true;
                }
            }
        }
    }

    if addr_taken {
        return;
    }

    // Remove stores to slots whose byte range is never overlapped by any load.
    // Register stores classify as StoreEbp; immediate homes (`movl $N, slot`)
    // classify as Other but are equally dead when the slot is never read —
    // the accumulator backend stages a constant into an outgoing-argument or
    // local slot that every use then re-materializes directly, leaving the
    // store with no reader. Gather them here so the same liveness proof covers
    // both shapes.
    let imm_stores: Vec<(usize, i32, MoveSize)> = {
        let mut v = Vec::new();
        for i in start..end {
            if infos[i].is_nop() {
                continue;
            }
            if matches!(infos[i].kind, LineKind::Other { .. }) {
                if let Some((_, slot, size)) = parse_immediate_store(trimmed(store, &infos[i], i)) {
                    v.push((i, slot, size));
                }
            }
        }
        v
    };
    for i in start..end {
        if infos[i].is_nop() {
            continue;
        }
        if let LineKind::StoreEbp { offset, size, .. } = infos[i].kind {
            let esp_store = is_esp_biased(offset);
            if esp_store {
                // `%esp` top stores can feed an architectural stack read that
                // has no explicit memory operand.  Keep the canonical
                // retpoline/stack-transfer shape `movl %reg,(%esp); ret|pop`.
                let mut next = i + 1;
                while next < end
                    && (infos[next].is_nop()
                        || matches!(
                            infos[next].kind,
                            LineKind::Empty | LineKind::Directive | LineKind::Label
                        ))
                {
                    next += 1;
                }
                let consumed = next < end
                    && (matches!(infos[next].kind, LineKind::Pop { .. } | LineKind::Ret)
                        || trimmed(store, &infos[next], next).starts_with("ret "));
                if offset == ESP_SLOT_BIAS
                    && trimmed(store, &infos[i], i).ends_with("(%esp)")
                    && consumed
                {
                    continue;
                }
                if !esp_walk_ok {
                    continue;
                }
                if has_call {
                    continue;
                }
            } else if !ebp_is_frame {
                // ebp-domain "store" without a frame pointer = write through
                // the %ebp DATA register (pointer store). Never a dead slot.
                continue;
            }
            let store_bytes = size.byte_size();
            let norm_off = if esp_store {
                offset - norm[i - start]
            } else {
                offset
            };
            let is_read = read_ranges
                .iter()
                .any(|&(r_off, r_sz)| ranges_overlap(norm_off, store_bytes, r_off, r_sz));
            if !is_read {
                infos[i].kind = LineKind::Nop;
            }
        }
    }

    // Immediate homes: apply the identical function-scoped liveness proof.
    // (%esp) immediate stores require the same walk/call discipline the
    // register stores need; (%ebp) immediate stores are real frame slots only
    // under a frame pointer (otherwise %ebp is a data register and the line
    // could be a pointer write — never a dead home).
    for (i, offset, size) in imm_stores {
        let esp_store = is_esp_biased(offset);
        if esp_store {
            if !esp_walk_ok || has_call {
                continue;
            }
        } else if !ebp_is_frame {
            continue;
        }
        let store_bytes = size.byte_size();
        let norm_off = if esp_store {
            offset - norm[i - start]
        } else {
            offset
        };
        // Top-of-stack immediate store could feed a `ret`/pop architectural
        // read, mirroring the register-store guard above.
        if esp_store && offset == ESP_SLOT_BIAS && trimmed(store, &infos[i], i).ends_with("(%esp)")
        {
            let mut next = i + 1;
            while next < end
                && (infos[next].is_nop()
                    || matches!(
                        infos[next].kind,
                        LineKind::Empty | LineKind::Directive | LineKind::Label
                    ))
            {
                next += 1;
            }
            if next < end
                && (matches!(infos[next].kind, LineKind::Pop { .. } | LineKind::Ret)
                    || trimmed(store, &infos[next], next).starts_with("ret "))
            {
                continue;
            }
        }
        let is_read = read_ranges
            .iter()
            .any(|&(r_off, r_sz)| ranges_overlap(norm_off, store_bytes, r_off, r_sz));
        if !is_read {
            infos[i].kind = LineKind::Nop;
        }
    }
}

// ── Pass: Unused callee-saved register elimination ───────────────────────────

/// Parse the immediate from `subl $N, %esp` or `addl $N, %esp`.
fn parse_esp_adjust(line: &str, mnemonic: &str) -> Option<u32> {
    let rest = line.strip_prefix(mnemonic)?.trim_start();
    let rest = rest.strip_prefix('$')?;
    let (imm, dst) = rest.split_once(',')?;
    if dst.trim() != "%esp" {
        return None;
    }
    imm.trim().parse().ok()
}

/// Remove unused callee-save pairs in omit-frame-pointer functions while
/// preserving the exact body ESP.
///
/// Removing a push directly would shift every ESP-relative local and incoming
/// stack parameter.  Instead, fold its four bytes into the existing frame
/// `sub`/`add`: N removed pairs transform `sub $F`/`add $F` into
/// `sub $(F+4N)`/`add $(F+4N)`.  The body therefore sees the identical ESP,
/// while the remaining saved registers become a compact push/pop sequence.
fn eliminate_unused_callee_saves(store: &mut LineStore, infos: &mut [LineInfo]) {
    let len = infos.len();
    let mut start = 0usize;
    while start < len {
        // A non-local label starts a generated function.  Local basic-block
        // labels begin with `.L` and remain inside the current range.
        while start < len {
            if infos[start].kind == LineKind::Label {
                let line = trimmed(store, &infos[start], start);
                if line.ends_with(':') && !line.starts_with('.') {
                    break;
                }
            }
            start += 1;
        }
        if start >= len {
            break;
        }
        let mut end = start + 1;
        while end < len {
            let line = trimmed(store, &infos[end], end);
            if line == ".cfi_endproc" || line.starts_with(".size ") {
                break;
            }
            end += 1;
        }

        // A real frame pointer needs different CFA/frame-layout accounting.
        // Omit-frame-pointer code may still use EBP as an ordinary register.
        let has_frame_pointer = (start..end).any(|i| {
            if infos[i].is_nop() {
                return false;
            }
            let line = trimmed(store, &infos[i], i);
            line.contains("(%ebp)") || line == "movl %esp, %ebp"
        });
        if has_frame_pointer {
            start = end.saturating_add(1);
            continue;
        }

        // Prologue pushes: the consecutive Push run at the function top.
        let mut prologue_pushes: Vec<(usize, RegId)> = Vec::new();
        {
            let mut i = start + 1;
            while i < end {
                if infos[i].is_nop() {
                    i += 1;
                    continue;
                }
                match infos[i].kind {
                    LineKind::Push { reg } if reg <= REG_GP_MAX => {
                        prologue_pushes.push((i, reg));
                        i += 1;
                    }
                    LineKind::Directive | LineKind::Empty => {
                        i += 1;
                    }
                    _ => break,
                }
            }
        }
        if prologue_pushes.is_empty() {
            start = end.saturating_add(1);
            continue;
        }

        // Census of every explicit ESP adjustment in the function.
        let mut subls: Vec<(usize, u32)> = Vec::new();
        let mut addls: Vec<(usize, u32)> = Vec::new();
        for i in start..end {
            if infos[i].is_nop() {
                continue;
            }
            let line = trimmed(store, &infos[i], i);
            if let Some(v) = parse_esp_adjust(line, "subl") {
                subls.push((i, v));
            }
            if let Some(v) = parse_esp_adjust(line, "addl") {
                addls.push((i, v));
            }
        }

        // Two supported shapes:
        //  FRAMED:    exactly one subl (the alloc), one or more addl
        //             deallocs ALL of the same value (one per epilogue).
        //             Removing a pair grows both by 4 so body ESP — and
        //             with it every N(%esp) slot — is unchanged.
        //  FRAMELESS: no subl/addl at all AND no (%esp) reference anywhere
        //             (no slots exist, so shifting body ESP by 4k is
        //             unobservable; i386 needs no 16-byte call alignment
        //             here). bcmp under tail-call conversion is the
        //             canonical case: 4 dead push/pop pairs guarding a
        //             2-instruction body.
        enum Shape {
            Framed,
            Frameless,
        }
        let shape = if subls.len() == 1
            && !addls.is_empty()
            && addls.iter().all(|&(_, v)| v == subls[0].1)
            && addls.iter().all(|&(i, _)| i > subls[0].0)
        {
            // STACK-WINDOW SOUNDNESS GUARD for the framed transformation.
            //
            // Removing a prologue push/pop pair grows the sole alloc/subl
            // and every matching dealloc/addl by 4 * removed bytes. That
            // keeps every %esp-relative reference UNCHANGED only for
            // references living strictly inside the [alloc .. first
            // dealloc] envelope — the +N/-N growth cancels exactly there.
            //
            // References OUTSIDE that window break silently:
            //  * BEFORE the alloc: incoming-parameter homes addressed
            //    against the raw entry CFA. This function canonically
            //    materialises them when the callee stages params into
            //    register homes before opening its outgoing-argument area
            //    (`wrap(y){ return inc(y*2) }`, GCC torture nestfunc-2/
            //    -3/-5, 20000822-1: the load kept its displacement while
            //    the removed push raised the real esp by 4, so the loaded
            //    value shifted one slot up = stale/garbage argument).
            //  * AFTER the first dealloc: rarely shaped similarly once
            //    multiple epilogues exist.
            //
            // A text-level pass cannot renumber arbitrary outside
            // references safely (it lacks the slot-liveness map), so the
            // only sound choice is to leave such functions untouched.
            let alloc_idx = subls[0].0;
            let first_dealloc = addls.iter().map(|&(i, _)| i).min();
            let last_dealloc = addls.iter().map(|&(i, _)| i).max();
            let is_registered_adjust =
                |idx: usize| -> bool { subls.iter().chain(&addls).any(|&(i, _)| i == idx) };
            // Label name (without ':') -> line index, for resolving jump
            // targets in the envelope soundness check below.
            let mut label_pos: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            // Textual edges (source line index, target label name) of every
            // resolvable Jmp/CondJmp in the function. Needed to vet labels in
            // the interleaved zone (see the Label arm below).
            let mut edges: Vec<(usize, &str)> = Vec::new();
            for i in start..end {
                if infos[i].is_nop() {
                    continue;
                }
                match infos[i].kind {
                    LineKind::Label => {
                        let t = trimmed(store, &infos[i], i);
                        if let Some(name) = t.strip_suffix(':') {
                            label_pos.insert(name.to_string(), i);
                        }
                    }
                    LineKind::Jmp | LineKind::CondJmp => {
                        let t = trimmed(store, &infos[i], i);
                        if let Some(name) = t.split_whitespace().next_back() {
                            edges.push((i, name));
                        }
                    }
                    _ => {}
                }
            }
            let has_jmp_indirect =
                (start..end).any(|i| !infos[i].is_nop() && infos[i].kind == LineKind::JmpIndirect);
            // Opaque user asm may compute slot addresses, call, or depend on
            // the exact %esp geometry — never renumber a frame it can see.
            let has_inline_asm =
                (start..end).any(|i| !infos[i].is_nop() && infos[i].kind == LineKind::InlineAsm);
            if has_inline_asm {
                start = end.saturating_add(1);
                continue;
            }
            let mut envelope_ok = true;
            if let Some(ld) = last_dealloc {
                // 1) Geometry: every %esp engagement must live strictly
                //    inside the uniformly-shifted interior.
                // 2) Control flow: entries into the interior must all pass
                //    through the grown alloc. A label/branch edge into or
                //    out of the middle would execute the surrounding code
                //    under two different depths; text-level knowledge
                //    cannot prove both paths consistent.
                let mut k = start;
                while k < end {
                    if infos[k].is_nop() {
                        k += 1;
                        continue;
                    }
                    match infos[k].kind {
                        LineKind::Directive | LineKind::Empty => {
                            k += 1;
                            continue;
                        }
                        _ => {}
                    }
                    if is_registered_adjust(k) {
                        k += 1;
                        continue;
                    }
                    match infos[k].kind {
                        LineKind::Push { .. } | LineKind::Pop { .. } | LineKind::Call => {}
                        _ => {
                            let t = trimmed(store, &infos[k], k);
                            if t.contains("%esp") && !(k > alloc_idx && k <= ld) {
                                envelope_ok = false;
                                break;
                            }
                        }
                    }
                    if k > alloc_idx && k <= ld {
                        // Control flow INSIDE the envelope (which now spans
                        // alloc..LAST dealloc so interleaved alloc/dealloc
                        // sequences are covered). The transform's invariant
                        // is "esp identical at every program point whose
                        // path crossed the alloc and the pair push" (+4k
                        // alloc and +4k deallocs cancel the removed push
                        // everywhere between them), so:
                        //
                        //  * Jmp/CondJmp whose target lies strictly AFTER the
                        //    alloc executes at the identical depth in both
                        //    versions (multi-epilogue early-return shape).
                        //  * Ret inside the envelope returns at identical
                        //    esp; the dead register's epilogue pop is simply
                        //    skipped in both versions.
                        //  * A Label between the FIRST and LAST dealloc
                        //    (second-epilogue targets like `.L1:`) keeps
                        //    depth identity exactly when every textual edge
                        //    into it originates at an index > alloc (those
                        //    paths crossed the pair push + alloc, so their
                        //    arrival depth cancels to zero); fallthrough
                        //    from the preceding interior line qualifies.
                        //    Tail-region sources (past the last dealloc)
                        //    are fully unwound and likewise cancel.
                        //
                        // What is NOT provable at text level:
                        //  * A Label up to the FIRST dealloc: an edge
                        //    entering from before the alloc arrives 4 bytes
                        //    shallower (one fewer push) than the interior
                        //    expects.
                        //  * An interleaved-zone Label that has an edge from
                        //    at/before the alloc (uncrossed pair push), or
                        //    ANY JmpIndirect in the function (its target is
                        //    unprovable, so the label's entry depths are
                        //    unknowable): same depth mismatch.
                        //  * JmpIndirect / a jump to an unresolvable or
                        //    pre-alloc target: same depth mismatch.
                        match infos[k].kind {
                            LineKind::JmpIndirect => {
                                envelope_ok = false;
                                break;
                            }
                            LineKind::Label => {
                                // Interior labels between the alloc and the
                                // first dealloc are equally safe WHEN every
                                // textual edge into them originates after the
                                // alloc: those paths crossed the pair push +
                                // alloc and therefore observe the same depth
                                // before and after the transform (the +4k
                                // alloc growth cancels the removed push for
                                // them exactly as it does for fallthrough).
                                // Fallthrough into such a label comes from the
                                // preceding interior line, which also crossed
                                // the alloc. Edges from at/before the alloc
                                // are the only shallow-entry hazard; there are
                                // no edges from the push run itself (only
                                // directives/pushes there), so the edge-source
                                // test below is the complete proof.
                                let edge_sources_past_alloc =
                                    edges.iter().all(|&(src, name)| match label_pos.get(name) {
                                        Some(&ti) if ti == k => src > alloc_idx,
                                        _ => true,
                                    });
                                let interior_label_ok = (!has_jmp_indirect
                                    && edge_sources_past_alloc)
                                    && (k > first_dealloc.unwrap_or(usize::MAX) || k > alloc_idx);
                                if !interior_label_ok {
                                    envelope_ok = false;
                                    break;
                                }
                            }
                            LineKind::Jmp | LineKind::CondJmp => {
                                let t = trimmed(store, &infos[k], k);
                                let target = t.split_whitespace().next_back();
                                let ok = target
                                    .and_then(|name| label_pos.get(name))
                                    .is_some_and(|&ti| ti > alloc_idx);
                                if !ok {
                                    envelope_ok = false;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    k += 1;
                }
            } else {
                envelope_ok = false;
            }
            if !envelope_ok {
                start = end.saturating_add(1);
                continue;
            }
            Shape::Framed
        } else if subls.is_empty() && addls.is_empty() {
            // Any %esp engagement (slots or pointer copies) makes depth
            // shifting observable; frameless candidates must be pure.
            let any_esp_text = (start..end)
                .any(|i| !infos[i].is_nop() && trimmed(store, &infos[i], i).contains("%esp"));
            if any_esp_text {
                start = end.saturating_add(1);
                continue;
            }
            Shape::Frameless
        } else {
            start = end.saturating_add(1);
            continue;
        };

        // In the framed shape each dealloc is immediately followed by the
        // epilogue pop chain; a pop of `reg` is an EPILOGUE pop when only
        // other pops/nops/directives sit between it and a dealloc.
        let is_epilogue_pop = |idx: usize| -> bool {
            match shape {
                Shape::Frameless => true,
                Shape::Framed => {
                    let mut k = idx;
                    while k > start {
                        k -= 1;
                        if infos[k].is_nop() || is_debug_location(store, infos, k) {
                            continue;
                        }
                        match infos[k].kind {
                            LineKind::Pop { .. } | LineKind::Directive | LineKind::Empty => {
                                continue
                            }
                            _ => {
                                let line = trimmed(store, &infos[k], k);
                                return parse_esp_adjust(line, "addl").is_some();
                            }
                        }
                    }
                    false
                }
            }
        };

        let mut removals: Vec<(usize, Vec<usize>)> = Vec::new(); // (push_idx, pop_idxs)
        for &(push_idx, reg) in &prologue_pushes {
            if !matches!(reg, REG_EBX | REG_ESI | REG_EDI | REG_EBP) {
                continue;
            }
            if prologue_pushes.iter().filter(|&&(_, r)| r == reg).count() != 1 {
                continue;
            }
            let pops: Vec<usize> = (start..end)
                .filter(|&i| {
                    !infos[i].is_nop()
                        && matches!(infos[i].kind, LineKind::Pop { reg: r } if r == reg)
                })
                .collect();
            if pops.is_empty() {
                continue;
            }
            // Every epilogue must restore the register exactly once.
            let expected = match shape {
                Shape::Framed => addls.len(),
                Shape::Frameless => pops.len(), // all pops must qualify below
            };
            if pops.len() != expected {
                continue;
            }
            if !pops.iter().all(|&i| is_epilogue_pop(i)) {
                continue;
            }
            // The register must be unreferenced outside its push/pop lines.
            let used = (start..end).any(|i| {
                if infos[i].is_nop() || i == push_idx || pops.contains(&i) {
                    return false;
                }
                line_references_reg(trimmed(store, &infos[i], i), reg)
            });
            if used {
                continue;
            }
            removals.push((push_idx, pops));
        }

        if !removals.is_empty() {
            let bytes = 4 * removals.len() as u32;
            if let Shape::Framed = shape {
                // Encoding guard: `subl/addl $imm8, %esp` is 3 bytes only
                // while the value fits i8. Growing 124 -> 128 flips both the
                // alloc and every dealloc to the 6-byte imm32 form (+3 bytes
                // each), which can exceed the 2 bytes per removed pair. Skip
                // when the crossing costs more than the removals save.
                let (alloc_idx, alloc_value) = subls[0];
                let adjusted = alloc_value + bytes;
                let crossing = alloc_value <= 127 && adjusted > 127;
                let growth = if crossing { 3 * (1 + addls.len()) } else { 0 };
                let saving = 2 * removals.len();
                if growth >= saving {
                    start = end.saturating_add(1);
                    continue;
                }
                store.replace(
                    alloc_idx,
                    format!("    subl ${}, %esp", alloc_value + bytes),
                );
                infos[alloc_idx] = classify_line(store.get(alloc_idx));
                for &(idx, v) in &addls {
                    store.replace(idx, format!("    addl ${}, %esp", v + bytes));
                    infos[idx] = classify_line(store.get(idx));
                }
            }
            for (push, pops) in removals {
                infos[push].kind = LineKind::Nop;
                for p in pops {
                    infos[p].kind = LineKind::Nop;
                }
            }
        }

        start = end.saturating_add(1);
    }
}
/// Remove a function's stack-frame allocation/deallocation when the frame is
/// never used. The i686 accumulator backend emits a `subl $N, %esp` prologue
/// for almost every function and an `addl $N, %esp` before every return, even
/// when the register allocator kept every SSA value in registers and the
/// local-frame space is never touched (a typical `-m16 -Os` leaf helper is
/// `subl $12, %esp; movzwl %ax,%eax; incl %eax; addl $12, %esp; ret`). Those
/// two to four instructions are pure overhead — GCC emits none.
///
/// Soundness is deliberately conservative. A frame is only elided when ALL of:
///   * a single alloc (`subl $K, %esp`, constant immediate) and one or more
///     matching deallocs (`addl $K, %esp`, same K, every one at or after the
///     alloc position);
///   * no frame pointer in use (`movl %esp,%ebp` or any `(%ebp)` reference);
///   * no prologue/epilogue callee-save push/pop (those are handled — and
///     renumber the frame — by `eliminate_unused_callee_saves`; mixing the two
///     transforms here would desync esp math, so functions with pushes are
///     skipped and left to that pass);
///   * no `call`: a call needs the stack 16-byte aligned (SysV i386) and may
///     push/pop outgoing arguments, so a frame can legitimately exist for
///     alignment/argument space even with no `(%esp)` data access;
///   * no other %esp engagement anywhere — no `N(%esp)` slot access (the frame
///     really holds no live data), no register-driven esp adjustment (dynamic
///     alloca), no esp copies, no `ret $N` (sret/fastcall pop convention);
///   * no jump enters the function at or before the alloc: every body path
///     must cross the single alloc, otherwise one side runs with the frame
///     open and the other without it.
///
/// Under those constraints no stack memory is observed while the frame is
/// open, the ret pops the return address at the entry esp, and esp-relative
/// addressing (prologue parameter captures, which a pure regparm leaf emits
/// none of) is unaffected.
fn eliminate_dead_frame_allocation(store: &mut LineStore, infos: &mut [LineInfo]) {
    if std::env::var_os("CCC_NO_I686_DEAD_FRAME_ELISION").is_some() {
        return;
    }
    let len = infos.len();
    let mut start = 0usize;
    while start < len {
        // Find the next function: a non-local label (local basic blocks are
        // `.L*`).
        while start < len {
            if infos[start].kind == LineKind::Label {
                let line = trimmed(store, &infos[start], start);
                if line.ends_with(':') && !line.starts_with('.') {
                    break;
                }
            }
            start += 1;
        }
        if start >= len {
            break;
        }
        let mut end = start + 1;
        while end < len {
            let line = trimmed(store, &infos[end], end);
            if line == ".cfi_endproc" || line.starts_with(".size ") {
                break;
            }
            end += 1;
        }

        // Reject a frame-pointer function: esp-relative accounting differs.
        let has_frame_pointer = (start..end).any(|i| {
            if infos[i].is_nop() {
                return false;
            }
            let line = trimmed(store, &infos[i], i);
            line.contains("(%ebp)") || line == "movl %esp, %ebp"
        });
        if has_frame_pointer {
            start = end.saturating_add(1);
            continue;
        }

        // Census: frame adjusts, pushes/pops, calls, and other %esp uses.
        let mut subls: Vec<(usize, u32)> = Vec::new();
        let mut addls: Vec<(usize, u32)> = Vec::new();
        let mut has_call = false;
        let mut has_push_or_pop = false;
        let mut other_esp = false;
        for i in start..end {
            if infos[i].is_nop() {
                continue;
            }
            let line = trimmed(store, &infos[i], i);
            match infos[i].kind {
                // Opaque user asm may itself call, push, or depend on the
                // absolute %esp value/alignment — veto the elision.
                LineKind::Call | LineKind::InlineAsm => has_call = true,
                LineKind::Push { .. } | LineKind::Pop { .. } => has_push_or_pop = true,
                _ => {
                    if let Some(v) = parse_esp_adjust(line, "subl") {
                        subls.push((i, v));
                    } else if let Some(v) = parse_esp_adjust(line, "addl") {
                        addls.push((i, v));
                    } else if infos[i].kind != LineKind::Ret
                        && infos[i].kind != LineKind::Label
                        && infos[i].kind != LineKind::Directive
                        && infos[i].kind != LineKind::Jmp
                        && infos[i].kind != LineKind::CondJmp
                        && infos[i].kind != LineKind::JmpIndirect
                        && line.contains("%esp")
                    {
                        // Any remaining %esp reference (slot access, esp copy,
                        // dynamic alloca, and-l alignment, …) means the frame
                        // is observable.
                        other_esp = true;
                    }
                }
            }
        }

        // Exactly one alloc and ≥1 matching deallocs, no other esp traffic,
        // no calls and no callee-save push/pop to renumber.
        let shape_ok = subls.len() == 1
            && !addls.is_empty()
            && addls.iter().all(|&(_, v)| v == subls[0].1)
            && addls.iter().all(|&(ai, _)| ai >= subls[0].0)
            && !has_call
            && !has_push_or_pop
            && !other_esp;
        if !shape_ok {
            start = end.saturating_add(1);
            continue;
        }
        let alloc_idx = subls[0].0;

        // Control-flow soundness: every body path must cross the alloc. A
        // direct jump whose target is a local label at or before the alloc
        // enters the body without opening the frame; an indirect jump can
        // target anywhere and is unprovable. External (non-`.L*`) jumps are
        // tail calls leaving the function and need no label resolution.
        let mut labels_after_alloc = std::collections::HashSet::new();
        for i in start..end {
            if infos[i].kind == LineKind::Label && i > alloc_idx {
                let line = trimmed(store, &infos[i], i);
                if let Some(name) = line.strip_suffix(':') {
                    labels_after_alloc.insert(name.trim().to_string());
                }
            }
        }
        let mut flow_ok = true;
        'scan: for i in start..end {
            if infos[i].is_nop() {
                continue;
            }
            match infos[i].kind {
                LineKind::JmpIndirect => {
                    flow_ok = false;
                    break 'scan;
                }
                LineKind::Jmp | LineKind::CondJmp => {
                    let line = trimmed(store, &infos[i], i);
                    if let Some(target) = line.split_whitespace().next_back() {
                        if target.starts_with('.') && !labels_after_alloc.contains(target) {
                            flow_ok = false;
                            break 'scan;
                        }
                    }
                }
                _ => {}
            }
        }
        if !flow_ok {
            start = end.saturating_add(1);
            continue;
        }

        // All guards passed: drop the alloc and every matching dealloc.
        infos[alloc_idx].kind = LineKind::Nop;
        for &(ai, _) in &addls {
            infos[ai].kind = LineKind::Nop;
        }

        start = end.saturating_add(1);
    }
}

#[allow(dead_code)]
fn eliminate_push_pop_pairs(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    for i in 0..len {
        if infos[i].is_nop() {
            continue;
        }
        let push_reg = match infos[i].kind {
            LineKind::Push { reg } if reg <= REG_GP_MAX => reg,
            _ => continue,
        };

        // Scan forward for matching pop, checking reg is unmodified
        let mut j = i + 1;
        let mut depth = 0; // Track nested push/pops
        let mut safe = true;
        while j < len {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }

            match infos[j].kind {
                LineKind::Push { .. } => {
                    depth += 1;
                }
                LineKind::Pop { reg } if depth > 0 => {
                    depth -= 1;
                }
                LineKind::Pop { reg } if reg == push_reg && depth == 0 => {
                    // Found matching pop
                    if safe {
                        infos[i].kind = LineKind::Nop;
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                    }
                    break;
                }
                LineKind::Pop { .. } if depth == 0 => {
                    break;
                } // Different register popped
                _ => {}
            }

            // Check if the register is modified
            match infos[j].kind {
                LineKind::Move { dst, .. } if dst == push_reg => {
                    safe = false;
                }
                LineKind::LoadEbp { reg, .. } if reg == push_reg => {
                    safe = false;
                }
                LineKind::Other { dest_reg } if dest_reg == push_reg => {
                    safe = false;
                }
                LineKind::Other { .. } => {
                    // For unknown instructions, check if the raw text references the register
                    // or if it's an instruction that implicitly clobbers registers (rep movsb, etc.)
                    let raw = store.get(j).trim();
                    if raw.starts_with("rep") || raw.starts_with("cld") {
                        // rep movsb/movsl/stosb etc. clobber esi, edi, ecx
                        if push_reg == REG_ESI || push_reg == REG_EDI || push_reg == REG_ECX {
                            safe = false;
                        }
                    }
                    if line_references_reg(raw, push_reg) {
                        safe = false;
                    }
                }
                LineKind::Call => {
                    if is_caller_saved(push_reg) {
                        safe = false;
                    }
                }
                LineKind::SetCC { reg } if reg == push_reg => {
                    safe = false;
                }
                _ => {}
            }

            // Stop at barriers that change control flow
            if matches!(
                infos[j].kind,
                LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret | LineKind::Label
            ) {
                break;
            }

            j += 1;
        }
    }

    changed
}

// ── Utility ──────────────────────────────────────────────────────────────────

/// Find the next non-nop line after index `start`.
fn next_non_nop(infos: &[LineInfo], start: usize) -> usize {
    let mut i = start;
    while i < infos.len() && (infos[i].is_nop() || infos[i].kind == LineKind::Empty) {
        i += 1;
    }
    i
}

// ── Pattern 17: else-hoist for select-store diamonds ────────────────────────
//
// GCC's -Os leaves `if (c) slot = a; else slot = b;` as a diamond:
//
//     jCC  Lelse          T-stores are hoisted above the branch and the
//     <then: T-stores>    jump-to-join is deleted, saving the 2-byte jmp:
//     jmp  Ljoin
//   Lelse:                    <then: T-stores>
//     <else: E-stores>        jCC' Ljoin          (inverted condition)
//   Ljoin:                  Lelse:
//                             <else: E-stores>
//                           Ljoin:
//
// The transform is only fired when it is provably invisible:
//  * T and E bodies are pure `mov{l,w,b} reg|$imm, disp(%esp)` stores
//    (mov writes no flags, so the hoist cannot disturb the jCC' flags and
//    the flag producer itself must not touch stack slots — any `%esp`
//    operand there refuses the site);
//  * the T and E bodies write the SAME slot multiset — every path then
//    ends with the same final value per slot, so executing both stores on
//    one path (the else path also runs the hoisted T-stores) is identical;
//  * Ljoin is the fallthrough label right after the E-body;
//  * both spans stay inside rel8 range so the 2-byte branch forms are kept.

/// True when the flag producer line above `jcc_idx` (skipping labels and
/// blanks) has a (%esp) operand matching one of the hoisted `slots`.
/// Producers touching OTHER slots are unobservable for the transform.
fn producer_touches_slots(lines: &[String], jcc_idx: usize, slots: &[String]) -> bool {
    let mut p = jcc_idx;
    while p > 0 {
        p -= 1;
        let t = lines[p].trim();
        if t.is_empty() || t.ends_with(':') {
            continue;
        }
        let prod_slot = t.split(',').find_map(|op| {
            let (disp, base) = op.trim().rsplit_once('(')?;
            (base == "%esp)").then(|| disp.trim().to_string())
        });
        return match prod_slot {
            Some(ps) => slots.iter().any(|s| {
                s.rsplit_once('(')
                    .map(|(d, _)| d.trim() == ps)
                    .unwrap_or(false)
            }),
            None => false,
        };
    }
    false
}

fn else_hoist_diamonds(asm: String) -> String {
    // Classify a body line: Some(true) = pure slot store (dst is disp(%esp)),
    // Some(false) = register prep (no stack operand), None = not allowed.
    fn classify_body_line(l: &str) -> Option<bool> {
        let (mn, rest) = l.split_once(' ')?;
        const STORE_OK: &[&str] = &[
            "movl", "movw", "movb", "movzbl", "movzwl", "movsbl", "movswl", "andl", "andw", "andb",
            "orl", "orw", "orb", "xorl", "xorw", "xorb", "addl", "addw", "addb", "subl", "subw",
            "subb", "testl", "testw", "testb", "incl", "incw", "incb", "decl", "decw", "decb",
            "negl", "negw", "negb", "notl", "notw", "notb", "leal", "leaw",
        ];
        if !STORE_OK.contains(&mn) {
            return None;
        }
        let (srcop, dstop) = rest.split_once(',')?;
        let srcop = srcop.trim();
        let dstop = dstop.trim();
        let src_ok = srcop.starts_with('$')
            || srcop.starts_with('%')
            || (mn.starts_with("lea")
                && srcop.contains('(')
                && !srcop.contains("%esp")
                && !srcop.contains(','));
        if !src_ok {
            return None;
        }
        if let Some((disp, base)) = dstop.rsplit_once('(') {
            if base == "%esp)" && !disp.trim().is_empty() {
                return Some(true); // slot store
            }
            return None; // any other memory operand refuses the line
        }
        Some(false) // register prep — never hoisted
    }
    fn invert_jcc(l: &str) -> Option<(&str, &str)> {
        let (mn, target) = l.split_once(' ')?;
        let neg = match mn {
            "je" => "jne",
            "jne" => "je",
            "jb" => "jae",
            "jae" => "jb",
            "jbe" => "ja",
            "ja" => "jbe",
            "jl" => "jge",
            "jge" => "jl",
            "jle" => "jg",
            "jg" => "jle",
            "js" => "jns",
            "jns" => "js",
            "jp" => "jnp",
            "jnp" => "jp",
            _ => return None,
        };
        Some((neg, target.trim()))
    }

    let mut lines: Vec<String> = asm.split('\n').map(str::to_string).collect();
    let mut i = 0usize;
    while i < lines.len() {
        // The branch already selects the ELSE path; after hoisting the else
        // stores next to it, the SAME condition must jump to the join
        // (skipping the then body).  The inverted mnemonic would swap the
        // arms' values — a real miscompile (caught auditing cpucheck).
        let Some((jcc, else_target)) = invert_jcc(lines[i].trim()).map(|(neg, t)| {
            let orig = lines[i].trim().split(' ').next().unwrap_or("");
            (orig, t)
        }) else {
            i += 1;
            continue;
        };
        // Walk the then-body: prep lines and slot stores, no labels/calls.
        let mut j = i + 1;
        let mut then_slots: Vec<String> = Vec::new();
        while j < lines.len() {
            let t = lines[j].trim();
            if t.is_empty() || t.ends_with(':') {
                // Block labels inside the then-body are transparent: the
                // rewrite keeps the body in place, so every entry path
                // (fallthrough or jump) sees identical code.
                j += 1;
                continue;
            }
            match classify_body_line(t) {
                Some(true) => {
                    then_slots.push(t.rsplit_once(',').unwrap().1.trim().to_string());
                    j += 1;
                }
                Some(false) => j += 1,
                None => break,
            }
        }
        // Terminator: jmp Ljoin.
        let Some(("jmp", jt)) = lines.get(j).and_then(|l| l.trim().split_once(' ')) else {
            i += 1;
            continue;
        };
        let join_target = jt.trim();
        if then_slots.is_empty() {
            i += 1;
            continue;
        }
        // Lelse: label immediately after the jmp.
        let mut k = j + 1;
        while k < lines.len() && lines[k].trim().is_empty() {
            k += 1;
        }
        let else_lbl = format!("{}:", else_target);
        if lines.get(k).map(|l| l.trim()) != Some(else_lbl.as_str()) {
            i += 1;
            continue;
        }
        // Else-body: pure stores only (they hoist above the branch; prep
        // there would carry its flag writes across the jCC).
        let mut m = k + 1;
        let mut else_stores: Vec<usize> = Vec::new();
        let mut else_slots: Vec<String> = Vec::new();
        while m < lines.len() {
            let t = lines[m].trim();
            if t.is_empty() {
                m += 1;
                continue;
            }
            match classify_body_line(t) {
                Some(true) => {
                    else_stores.push(m);
                    else_slots.push(t.rsplit_once(',').unwrap().1.trim().to_string());
                    m += 1;
                }
                _ => break,
            }
        }
        // The else body must fall through into the join label.
        if lines.get(m).map(|l| l.trim()) != Some(format!("{}:", join_target).as_str())
            || else_stores.is_empty()
            || else_target == join_target
        {
            i += 1;
            continue;
        }
        // Slot multisets must match exactly: the then path re-runs the
        // hoisted else stores, so every slot must be overwritten by the
        // path's own final store.
        let mut ts = then_slots.clone();
        let mut es = else_slots.clone();
        ts.sort();
        es.sort();
        if ts != es {
            i += 1;
            continue;
        }
        // rel8 guard for both branch forms.
        let span_est = |a: usize, b: usize| -> usize {
            (a..=b).map(|r| lines[r].trim().len() + 2).sum::<usize>() / 6
        };
        if span_est(i + 1, j - 1) + 2 > 120 || span_est(k + 1, *else_stores.last().unwrap()) > 120 {
            i += 1;
            continue;
        }
        // The flag producer above the jCC (skipping labels/blanks) must not
        // READ or WRITE a slot the hoisted else stores write: the stores
        // now execute before the producer instead of after the branch.
        // A different slot is unobservable, so compare displacements.
        if producer_touches_slots(&lines, i, &else_slots) {
            i += 1;
            continue;
        }
        // The else block must have EXACTLY ONE predecessor: the jCC being
        // rewritten.  The else body sits between `jmp Ljoin` and `Ljoin:`,
        // so fallthrough entry is impossible, but another branch (the
        // merge-diamond shape `if (a) ok = 1; else if (b) ...` compiles to
        // two jCCs targeting the same constant-else block) can jump here
        // directly.  Hoisting its stores above this jCC silently removes
        // them from THAT predecessor's path (observed as a miscompile of
        // the upstream bool_thread_merges regression: the a==0 arm read
        // stale stack garbage instead of the constant).  Census every
        // branch target in the function: any reference to the else label
        // other than lines[i] vetoes the transform.
        let mut else_refs = 0usize;
        for (li, l) in lines.iter().enumerate() {
            if li == i {
                continue;
            }
            let t = l.trim();
            if t.ends_with(':') || t.is_empty() {
                continue;
            }
            let is_branch = t.starts_with("j") || t.starts_with("call") || t.starts_with("ret");
            if is_branch
                && t.split_whitespace()
                    .next_back()
                    .map(|x| x == else_target)
                    .unwrap_or(false)
            {
                else_refs += 1;
            }
        }
        if else_refs != 0 {
            i += 1;
            continue;
        }
        // Rewrite:
        //   ... producer          ... producer
        //   jCC Lelse             <else stores>        (hoisted)
        //   <then body>           jCC' Ljoin           (inverted)
        //   jmp Ljoin      =>     <then body>          (in place)
        //   Lelse:                Lelse:               (kept; now free)
        //   <else stores>
        //   Ljoin:                Ljoin:
        let mut out: Vec<String> = Vec::with_capacity(lines.len() - 1);
        out.extend_from_slice(&lines[..i]);
        out.extend(else_stores.iter().map(|&r| lines[r].clone()));
        out.push(format!("    {} {}", jcc, join_target));
        out.extend_from_slice(&lines[i + 1..j]); // then body
        out.extend_from_slice(&lines[j + 1..=k]); // blanks + Lelse label
        out.extend_from_slice(&lines[m..]); // join label onward
        let advanced = i + 1 + else_stores.len();
        lines = out;
        i = advanced.min(lines.len());
        continue;
    }
    lines.join("\n")
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Run peephole optimization on i686 assembly text.
/// Returns the optimized assembly string.
/// Reclassify every line between `#APP` and `#NO_APP` as
/// [`LineKind::InlineAsm`] so no pass rewrites user-authored assembly.
/// Ported from the x86-64 peephole, where the missing quarantine deleted a
/// deliberate `movq %rax, %rax` length template inside a kernel
/// ALTERNATIVE() block (objtool: "empty alternative entry").  The markers
/// themselves stay classified as comments and pass through untouched.
fn pin_inline_asm_regions(store: &LineStore, infos: &mut [LineInfo]) {
    let mut in_asm = false;
    for i in 0..store.len() {
        let t = trimmed(store, &infos[i], i);
        if t == "#APP" {
            in_asm = true;
            continue;
        }
        if t == "#NO_APP" {
            in_asm = false;
            continue;
        }
        if in_asm {
            infos[i] = LineInfo {
                kind: LineKind::InlineAsm,
                trim_start: infos[i].trim_start,
                has_indirect_mem: true,
                ebp_offset: EBP_OFFSET_NONE,
            };
        }
    }
}

pub fn peephole_optimize(asm: String) -> String {
    if std::env::var_os("CCC_NO_I686_PEEPHOLE").is_some() {
        return asm;
    }
    let mut store = LineStore::new(asm);
    let line_count = store.len();
    let mut infos: Vec<LineInfo> = (0..line_count)
        .map(|i| classify_line(store.get(i)))
        .collect();
    pin_inline_asm_regions(&store, &mut infos);

    // Phase 1: Iterative local passes
    let mut changed = true;
    let mut pass_count = 0;
    while changed && pass_count < MAX_LOCAL_PASS_ITERATIONS {
        changed = false;
        changed |= combined_local_pass(&mut store, &mut infos);
        pass_count += 1;
    }

    // Establish CFG liveness before the linear global passes.  Besides
    // deleting globally dead moves, this retargets accumulator update/copy-back
    // chains while all three instructions are still present.
    optimize_cfg_register_liveness(&mut store, &mut infos, false);

    // Phase 2: Global passes (run once)
    // Copy propagation first: it turns chained moves into dead moves, which
    // the very next pass then removes. Slot-load forwarding runs right after,
    // so freshly exposed store/load pairs are folded in the same phase.
    let global_changed = propagate_reg_copies(&mut store, &mut infos);
    let global_changed = global_changed | forward_slot_loads(&mut store, &mut infos);
    let global_changed = global_changed | forward_immediate_slot_loads(&mut store, &mut infos);
    let global_changed = global_changed | eliminate_dead_reg_moves(&store, &mut infos);
    let global_changed = global_changed | eliminate_dead_stores(&store, &mut infos);
    let global_changed = global_changed | fuse_compare_and_branch(&mut store, &mut infos);
    let global_changed = global_changed | fuse_setcc_branch(&mut store, &mut infos);
    let global_changed = global_changed | fold_memory_operands(&mut store, &mut infos);
    let global_changed = global_changed | eliminate_redundant_test_i686(&mut store, &mut infos);
    let global_changed = global_changed | fuse_sext_testb(&mut store, &mut infos);
    let global_changed = global_changed | collapse_slot_rmw_i686(&mut store, &mut infos);

    // Phase 3: Local cleanup after global passes
    if global_changed {
        let mut changed2 = true;
        let mut pass_count2 = 0;
        while changed2 && pass_count2 < MAX_POST_GLOBAL_ITERATIONS {
            changed2 = false;
            changed2 |= combined_local_pass(&mut store, &mut infos);
            changed2 |= propagate_reg_copies(&mut store, &mut infos);
            changed2 |= forward_slot_loads(&mut store, &mut infos);
            changed2 |= forward_immediate_slot_loads(&mut store, &mut infos);
            changed2 |= eliminate_dead_reg_moves(&store, &mut infos);
            changed2 |= eliminate_dead_stores(&store, &mut infos);
            changed2 |= fuse_setcc_branch(&mut store, &mut infos);
            changed2 |= fold_memory_operands(&mut store, &mut infos);
            changed2 |= eliminate_redundant_test_i686(&mut store, &mut infos);
            changed2 |= fuse_sext_testb(&mut store, &mut infos);
            pass_count2 += 1;
        }
    }

    // Phase 3.5: redundant zero-extension elimination (may expose dead
    // stores and moves by shortening def-use chains, so it runs before DSE
    // and the liveness cleanup).
    let pair_changed = fold_sext_zext_pairs(&mut store, &mut infos);
    let zext_changed = eliminate_redundant_zext_i686(&mut store, &mut infos);
    // Redundant SIGN-extension elimination runs right after: it turns
    // `movsbl %al,%REG` into a `movl` copy or a no-op, which the cleanup
    // loop below then propagates/deletes like any other move.
    let sext_changed = eliminate_redundant_sign_ext_i686(&mut store, &mut infos);
    if pair_changed || zext_changed || sext_changed {
        let mut changed3 = true;
        let mut pass_count3 = 0;
        while changed3 && pass_count3 < MAX_POST_GLOBAL_ITERATIONS {
            changed3 = false;
            changed3 |= combined_local_pass(&mut store, &mut infos);
            changed3 |= eliminate_dead_reg_moves(&store, &mut infos);
            changed3 |= eliminate_dead_stores(&store, &mut infos);
            pass_count3 += 1;
        }
    }

    // Phase 3.6: div/rem pair fusion.  Runs after the local cleanup folded
    // the URem result extraction (`movl %edx,%eax` + store → direct store),
    // which otherwise writes %eax between the two divisions and blocks the
    // window proof.  May expose dead staging moves for the next phases.
    let div_fused = fuse_div_rem_pairs(&mut store, &mut infos);
    if div_fused {
        let mut changed3b = true;
        let mut pass_count3b = 0;
        while changed3b && pass_count3b < MAX_POST_GLOBAL_ITERATIONS {
            changed3b = false;
            changed3b |= combined_local_pass(&mut store, &mut infos);
            changed3b |= propagate_reg_copies(&mut store, &mut infos);
            changed3b |= forward_slot_loads(&mut store, &mut infos);
            changed3b |= eliminate_dead_reg_moves(&store, &mut infos);
            changed3b |= eliminate_dead_stores(&store, &mut infos);
            changed3b |= fuse_sext_testb(&mut store, &mut infos);
            pass_count3b += 1;
        }
    }

    // Phase 3.7: fold indirect calls through freshly spilled global function
    // pointers (`movl SYM,%r; movl %r,N(%esp); call *N(%esp)` ->
    // `call *SYM`). Runs before phase 4 so the vacated slot can turn other
    // stores dead and unused callee-saves can drop.
    fold_global_fnptr_calls(&mut store, &mut infos);

    // Phase 3.8: staging-copy idiom folds (test/zero/immediate/mask through
    // a copy, narrow extension of a freshly loaded slot, widened slot loads
    // of fresh stores, range-check and doubling fusion) plus CFG cleanup:
    // jump threading through trivial blocks and redundant-conditional
    // deletion.  Every rewrite is flag- and value-identical and proven
    // invisible by the dominance dead-proof; threading is control-flow
    // neutral by construction.  The dead-move/dead-store machinery reclaims
    // the orphans, and the wider cleanup can in turn drop unused callee-save
    // pairs — so iterate to a bounded fixpoint.
    {
        let mut changed8 = fold_reg_copy_idioms(&mut store, &mut infos)
            | thread_trivial_blocks(&mut store, &mut infos)
            | delete_redundant_cond_jumps(&mut store, &mut infos);
        let mut pass_count8 = 0;
        while changed8 && pass_count8 < MAX_POST_GLOBAL_ITERATIONS {
            changed8 = false;
            changed8 |= fold_reg_copy_idioms(&mut store, &mut infos);
            changed8 |= thread_trivial_blocks(&mut store, &mut infos);
            changed8 |= delete_redundant_cond_jumps(&mut store, &mut infos);
            changed8 |= combined_local_pass(&mut store, &mut infos);
            // Copy propagation rewrites the readers of the staging moves the
            // folds above orphan (dispatch compares, relay copies); the dead-
            // move pass then actually reclaims them.
            changed8 |= propagate_reg_copies(&mut store, &mut infos);
            changed8 |= eliminate_dead_reg_moves(&store, &mut infos);
            changed8 |= eliminate_dead_stores(&store, &mut infos);
            pass_count8 += 1;
        }
    }

    // Phase 4: Never-read store elimination, then remove callee-save pairs
    // made unused by the dead-move cleanup above.
    eliminate_never_read_stores(&store, &mut infos);
    optimize_cfg_register_liveness(&mut store, &mut infos, true);
    eliminate_unused_callee_saves(&mut store, &mut infos);
    eliminate_dead_frame_allocation(&mut store, &mut infos);

    // Phase 5: tail-call conversion (after all epilogue-shape rewrites).
    optimize_tail_calls_i686(&mut store, &mut infos);

    // Phase 6: select-diamond else-hoist runs on the FINAL text — earlier
    // passes shape the diamond (fusing the flag producer, folding prep)
    // that this pass needs to see.
    else_hoist_diamonds(store.build_result(|i| infos[i].is_nop()))
}

// ── Pass: redundant zero-extension elimination (i686 redundant_ext port) ────

/// Whether a generic instruction defeats every tracked narrow-value fact.
///
/// `parse_dest_reg` is deliberately a comma-form parser. Some real writers
/// therefore have no parsed destination (single-operand shifts/bswap and
/// unknown inline-assembly instructions), while atomics such as cmpxchg/xadd
/// write registers in addition to the last textual operand. Narrow-value
/// elimination must fail closed for both classes: a stale fact can delete a
/// required extension and silently change the value.
fn invalidates_all_narrow_facts(text: &str, parsed_dest: RegId) -> bool {
    parsed_dest == REG_NONE
        || text.starts_with("mul")
        || text.starts_with("imul")
        || text.starts_with("div")
        || text.starts_with("idiv")
        || text == "cltd"
        || text == "cdq"
        || text.starts_with("xchg")
        || text.starts_with("xadd")
        || text.starts_with("cmpxchg")
        || text.starts_with("lock xadd")
        || text.starts_with("lock cmpxchg")
        || text.starts_with("rep")
        || text.starts_with("lods")
        || text.starts_with("stos")
        || text.starts_with("movs")
        || text.starts_with("cmps")
        || text.starts_with("scas")
}

#[inline]
fn is_low_byte_register(name: &str) -> bool {
    matches!(name, "%al" | "%bl" | "%cl" | "%dl")
}

/// Track which GP register families provably hold byte values (bits 8..31
/// zero) or 16-bit values (bits 16..31 zero), and remove/rewrite redundant
/// re-extensions:
///   * `movzbl %al, %eax` when %eax already holds a byte value  -> removed
///   * `movzbl %al, %REG` when %eax already holds a byte value  -> movl copy
///   * `movzwl %ax, %eax` when %eax already holds a 16-bit value -> removed
///
/// The setup-code i686 emitter re-extends after EVERY sub-int operation
/// (43 movzbl in video-bios.o alone, 15 of them provably redundant), because
/// `movsbl SRC, %R32` + `movzbl %R8, %R32` (same register family, adjacent)
/// → `movzbl SRC, %R32`: the zero-extension truncates the sign-extension's
/// high bits anyway, so the signed load is pure wasted work. The classic
/// source is `(u8)(i8)*p` char handling (strncmp/strchr) where the frontend
/// lowers the signed load and the unsigned cast as separate instructions.
/// Adjacency (nop lines aside) makes the sext's wider result provably dead:
/// nothing can read it between the two lines, and the movz overwrites the
/// register afterwards.
fn fold_sext_zext_pairs(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = infos.len();
    let mut i = 0;
    while i + 1 < len {
        if infos[i].is_nop() || infos[i].is_barrier() {
            i += 1;
            continue;
        }
        if infos[i + 1].is_nop() {
            // Allow the pair to be separated by nop lines: find the next
            // real instruction, but a barrier or label kills the fold.
            let mut j = i + 1;
            while j < len && infos[j].is_nop() {
                j += 1;
            }
            if j >= len || infos[j].is_barrier() {
                i = j;
                continue;
            }
            if !try_fold_sext_zext_at(store, infos, i, j) {
                i += 1;
            } else {
                changed = true;
                i = j + 1;
            }
            continue;
        }
        if infos[i + 1].is_barrier() {
            i += 2;
            continue;
        }
        if try_fold_sext_zext_at(store, infos, i, i + 1) {
            changed = true;
            i += 2;
        } else {
            i += 1;
        }
    }
    changed
}

fn try_fold_sext_zext_at(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    sext_idx: usize,
    zext_idx: usize,
) -> bool {
    let sext = trimmed(store, &infos[sext_idx], sext_idx);
    let zext = trimmed(store, &infos[zext_idx], zext_idx);
    // movs{b,w}l SRC, %DST  followed by  movz{b,w}l %SRC-low, %DST
    let (byte_reg, sext_mnem, zext_mnem) = if sext.starts_with("movsbl ") {
        (true, "movsbl ", "movzbl ")
    } else if sext.starts_with("movswl ") {
        (false, "movswl ", "movzwl ")
    } else {
        return false;
    };
    let Some((src, sdst)) = sext.split_once(',') else {
        return false;
    };
    let src = src.strip_prefix(sext_mnem).unwrap_or(src).trim();
    let sdst = sdst.trim();
    let Some((zsrc, zdst)) = zext.split_once(',') else {
        return false;
    };
    let zsrc = zsrc.strip_prefix(zext_mnem).unwrap_or(zsrc).trim();
    let zdst = zdst.trim();
    if !zext.starts_with(zext_mnem) {
        return false;
    }
    if sdst != zdst {
        return false;
    }
    let sf = register_family(sdst);
    if sf > REG_GP_MAX {
        return false;
    }
    // The zext source register must be the sext destination's low part.
    let low_ok = if byte_reg {
        is_low_byte_register(zsrc) && register_family(zsrc) == sf
    } else {
        register_family(zsrc) == sf && zsrc.ends_with('x')
    };
    if !low_ok {
        return false;
    }
    // Fold: replace the sext with the zero-extending load, nop the zext.
    store.replace(
        sext_idx,
        format!(
            "    {} {}, {}",
            if byte_reg { "movzbl" } else { "movzwl" },
            src,
            zdst
        ),
    );
    infos[sext_idx] = classify_line(store.get(sext_idx));
    infos[zext_idx].kind = LineKind::Nop;
    true
}

fn eliminate_redundant_zext_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    // Same state machine as the x86-64 pass (redundant_ext.rs): flags cleared
    // at labels/calls/barriers, updated by extensions, copies, and
    // byte-preserving andl; anything else clears the written family.
    // Fail-closed: implicit multi-register writers clear all.
    let mut changed = false;
    let len = infos.len();
    let mut is_byte = [false; 8]; // per family: value < 256
    let mut is_16 = [false; 8]; // per family: value < 65536

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if infos[i].is_barrier() {
            is_byte = [false; 8];
            is_16 = [false; 8];
            i += 1;
            continue;
        }
        let t = trimmed(store, &infos[i], i).to_string();

        // movzbl SRC, %DST
        if let Some(rest) = t.strip_prefix("movzbl ") {
            if let Some((src, dst)) = rest.split_once(',') {
                let src = src.trim();
                let dst = dst.trim();
                let df = register_family(dst);
                // A full-register byte fact applies to its low byte only.
                // If EAX < 256 then AH is zero, so deleting
                // `movzbl %ah,%eax` would leave AL instead of producing 0;
                // a cross-family movl rewrite is wrong for the same reason.
                if df <= REG_GP_MAX && is_low_byte_register(src) {
                    let sf = register_family(src);
                    if sf <= REG_GP_MAX && is_byte[sf as usize] {
                        if sf == df {
                            // pure no-op
                            infos[i].kind = LineKind::Nop;
                            changed = true;
                            i += 1;
                            continue;
                        }
                        // cross-register: plain 32-bit copy (shorter, no
                        // partial-register stall).
                        store.replace(
                            i,
                            format!("    movl {}, {}", reg32_name(sf), reg32_name(df)),
                        );
                        infos[i] = classify_line(store.get(i));
                        is_byte[df as usize] = true;
                        is_16[df as usize] = true;
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
                if df <= REG_GP_MAX {
                    is_byte[df as usize] = true;
                    is_16[df as usize] = true;
                }
                i += 1;
                continue;
            }
        }
        // movzwl SRC, %DST
        if let Some(rest) = t.strip_prefix("movzwl ") {
            if let Some((src, dst)) = rest.split_once(',') {
                let src = src.trim();
                let dst = dst.trim();
                let df = register_family(dst);
                if df <= REG_GP_MAX && !src.contains('(') && src.starts_with('%') {
                    let sf = register_family(src);
                    if sf <= REG_GP_MAX && is_16[sf as usize] && sf == df {
                        infos[i].kind = LineKind::Nop;
                        changed = true;
                        i += 1;
                        continue;
                    }
                }
                if df <= REG_GP_MAX {
                    is_byte[df as usize] = false;
                    is_16[df as usize] = true;
                }
                i += 1;
                continue;
            }
        }

        // Flag propagation / invalidation for everything else.
        match infos[i].kind {
            LineKind::Move { dst, src } => {
                is_byte[dst as usize] = is_byte[src as usize];
                is_16[dst as usize] = is_16[src as usize];
            }
            LineKind::LoadEbp { reg, .. } | LineKind::Pop { reg } => {
                if reg <= REG_GP_MAX {
                    is_byte[reg as usize] = false;
                    is_16[reg as usize] = false;
                }
            }
            LineKind::StoreEbp { .. }
            | LineKind::Push { .. }
            | LineKind::Cmp
            | LineKind::Empty
            | LineKind::SelfMove => {}
            LineKind::SetCC { reg } => {
                // setCC writes the low byte only; upper bits UNCHANGED —
                // conservatively clear (the canonical setcc+movzbl pair
                // re-establishes byte-ness via the movzbl above).
                if reg <= REG_GP_MAX {
                    is_byte[reg as usize] = false;
                    is_16[reg as usize] = false;
                }
            }
            _ => {
                let d = parse_dest_reg(&t);
                if d <= REG_GP_MAX && t.starts_with("andl $") {
                    // andl $imm: result <= imm, so byte/16 flags follow
                    // the immediate's range regardless of prior state.
                    let imm = t["andl $".len()..]
                        .split(',')
                        .next()
                        .and_then(|v| v.trim().parse::<i64>().ok());
                    is_byte[d as usize] = matches!(imm, Some(v) if (0..=255).contains(&v));
                    is_16[d as usize] = matches!(imm, Some(v) if (0..=65535).contains(&v));
                } else if d <= REG_GP_MAX {
                    is_byte[d as usize] = false;
                    is_16[d as usize] = false;
                }
                if invalidates_all_narrow_facts(&t, d) {
                    is_byte = [false; 8];
                    is_16 = [false; 8];
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Pass: redundant sign-extension elimination (i686) ────────────────────────

/// Track which GP register families provably hold sign-extended byte values
/// (bits 8..31 equal bit 7), and remove redundant re-sign-extensions:
///   * `movsbl %al, %eax` when %eax is already sign-extended  -> removed
///   * `movsbl %al, %REG` when %eax is already sign-extended  -> `movl` copy
///
/// The accumulator-based i686 codegen re-sign-extends after every sub-int
/// path: `movsbl (%esi),%eax; movsbl %al,%eax` (the load already sign-extends)
/// and `setbe %al; movzbl %al,%eax; movsbl %al,%eax` (a bool is its own sign
/// extension). Same fail-closed state machine as `eliminate_redundant_zext_i686`:
/// flags cleared at every barrier, updated by extensions, copies, and setcc;
/// anything else clears the written family.
fn eliminate_redundant_sign_ext_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = infos.len();
    let mut sign_ext = [false; 8]; // per family: bits 8..31 == sign(bit7)
    let mut is_bool = [false; 8]; // per family: low byte is 0/1 (setcc)

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if infos[i].is_barrier() {
            sign_ext = [false; 8];
            is_bool = [false; 8];
            i += 1;
            continue;
        }
        let t = trimmed(store, &infos[i], i).to_string();

        // movsbl SRC, %DST
        if let Some(rest) = t.strip_prefix("movsbl ") {
            if let Some((src, dst)) = rest.split_once(',') {
                let src = src.trim();
                let dst = dst.trim();
                let df = register_family(dst);
                if df <= REG_GP_MAX {
                    // A sign-extension fact describes the full register's
                    // low byte. High-byte aliases are different values and
                    // must never be replaced by a full-register copy.
                    if is_low_byte_register(src) {
                        let sf = register_family(src);
                        if sf <= REG_GP_MAX && sign_ext[sf as usize] {
                            if sf == df {
                                // Re-sign-extending an already-sign-extended
                                // value: pure no-op.
                                infos[i].kind = LineKind::Nop;
                                changed = true;
                                i += 1;
                                continue;
                            }
                            // Cross-register: a plain 32-bit copy suffices.
                            store.replace(
                                i,
                                format!("    movl {}, {}", reg32_name(sf), reg32_name(df)),
                            );
                            infos[i] = classify_line(store.get(i));
                            sign_ext[df as usize] = true;
                            is_bool[df as usize] = is_bool[sf as usize];
                            changed = true;
                            i += 1;
                            continue;
                        }
                    }
                    // Genuine sign extension of a non-sign-extended source.
                    sign_ext[df as usize] = true;
                    is_bool[df as usize] = false;
                    i += 1;
                    continue;
                }
            }
        }

        // movzbl SRC, %DST : a zero extension is its own sign extension only
        // when the source is a bool (0/1 == sign-extended 0/1).
        if let Some(rest) = t.strip_prefix("movzbl ") {
            if let Some((src, dst)) = rest.split_once(',') {
                let src = src.trim();
                let dst = dst.trim();
                let df = register_family(dst);
                if df <= REG_GP_MAX {
                    let src_bool = is_low_byte_register(src) && {
                        let sf = register_family(src);
                        sf <= REG_GP_MAX && is_bool[sf as usize]
                    };
                    sign_ext[df as usize] = src_bool;
                    is_bool[df as usize] = src_bool;
                    i += 1;
                    continue;
                }
            }
        }

        // Flag propagation / invalidation for everything else.
        match infos[i].kind {
            LineKind::Move { dst, src } => {
                sign_ext[dst as usize] = sign_ext[src as usize];
                is_bool[dst as usize] = is_bool[src as usize];
            }
            LineKind::LoadEbp { reg, .. } | LineKind::Pop { reg } => {
                if reg <= REG_GP_MAX {
                    sign_ext[reg as usize] = false;
                    is_bool[reg as usize] = false;
                }
            }
            LineKind::StoreEbp { .. }
            | LineKind::Push { .. }
            | LineKind::Cmp
            | LineKind::Empty
            | LineKind::SelfMove => {}
            LineKind::SetCC { reg } => {
                // setCC writes the low byte only; the value is a bool (0/1).
                if reg <= REG_GP_MAX {
                    sign_ext[reg as usize] = false;
                    is_bool[reg as usize] = true;
                }
            }
            _ => {
                let d = parse_dest_reg(&t);
                if d <= REG_GP_MAX {
                    // `movl $imm, %reg`: the 32-bit value is its own sign
                    // extension exactly when imm fits in a sign-extended byte
                    // ([-128,127]); it is a bool only for 0/1.
                    if let Some(imm) = t.strip_prefix("movl $").and_then(|r| {
                        r.split(',')
                            .next()
                            .and_then(|v| v.trim().parse::<i64>().ok())
                    }) {
                        sign_ext[d as usize] = (-128..=127).contains(&imm);
                        is_bool[d as usize] = matches!(imm, 0 | 1);
                    } else if let Some(imm) = t.strip_prefix("andl $").and_then(|r| {
                        r.split(',')
                            .next()
                            .and_then(|v| v.trim().parse::<i64>().ok())
                    }) {
                        // `andl $imm, %reg`: result is in [0, imm], so for
                        // imm <= 127 it is non-negative and hence its own sign
                        // extension; bool only for 0/1.
                        sign_ext[d as usize] = (0..=127).contains(&imm);
                        is_bool[d as usize] = matches!(imm, 0 | 1);
                    } else {
                        sign_ext[d as usize] = false;
                        is_bool[d as usize] = false;
                    }
                }
                if invalidates_all_narrow_facts(&t, d) {
                    sign_ext = [false; 8];
                    is_bool = [false; 8];
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Pass: tail-call conversion (call X; <epilogue>; ret → <epilogue>; jmp X) ──

/// i686 port of the x86-64 tail-call peephole.
///
/// Converts `call TARGET` immediately followed by a pure epilogue
/// (addl $N,%esp / popl callee-saved / popl %ebp / movl %ebp,%esp /
/// leal -N(%ebp),%esp) and `ret` into the epilogue followed by
/// `jmp TARGET`. Saves 1 insn + the return-address push/pop per site and
/// shortens the hot return path (GCC does this at -Os everywhere in the
/// kernel setup code).
///
/// i686-specific soundness rules:
///  * Suppressed for the whole function when a frame address escapes
///    (leal of an %esp/%ebp slot) or dynamic alloca adjusts %esp by a
///    register — the callee could dereference a dangling frame pointer.
///  * Suppressed when the bare `ret` is not plain (ret $N sret/fastcall
///    cleanup differs between caller and callee — LineKind::Ret only
///    matches plain `ret`, so this falls out naturally).
///  * Epilogue restores must not write %eax/%edx (32/64-bit return
///    value registers). popl of callee-saved regs never does.
///  * Indirect `call *%reg` converts only if %reg is NOT restored by the
///    epilogue between call and ret (callee-saved pointers are restored
///    before the jmp would use them). We only accept `call *%e{c,d}x` /
///    memory-free operands that no epilogue pop touches.
///
/// **Stack-window soundness (argument forwarding) — decisive rule.**
/// Converting a stack-argument call to a jmp silently *re-bases* where the
/// tail-callee reads its incoming arguments from. At `call time` the callee
/// would have read its parameter bytes at [C, C+N), where C = %esp at the
/// call (before the return address push). After conversion it instead reads
/// them at [E', E'+N) where E' = %esp after the epilogue unwind = function
/// entry CFA. Those windows differ whenever any caller-frame scratch /
/// outgoing-argument area existed below the CFA — which on i386 cdecl is
/// the *normal* shape (`return g(y*2)` stores y*2 into that area). The
/// converted code then hands the callee stale parent-incoming slots:
/// empirically this miscompiled `int f0(int (*fn)(int*), int*p)
/// { return (*fn)(p); }` into reading the trampoline template bytes as data
/// (GCC torture nestfunc-2/-3/-5, 20000822-1 at O1/O2; minimal repros
/// `wrap: return inc(y*2)` aborting because inc received y). A text-level
/// peephole has no interprocedural signature knowledge, so it can never
/// shuffle/move prepared outgoing arguments into place above the return
/// address soundly. The ONLY locally provable equivalence is window
/// identity: if the simulated %esp depth at the call is exactly 0 (equal to
/// the entry CFA) then unconverted reads [C=CFA ..] and converted reads
/// [E'=CFA ..] are the SAME addresses holding the SAME bytes, making the
/// transform observable-equivalent regardless of the callee's ABI.
/// Therefore conversion additionally requires simulated esp-depth == 0 at
/// the call site. Anything deeper (locals, outstanding pushes, any
/// outgoing-arg preparation) suppresses the conversion. Simulation is
/// deliberately conservative: entering an internal join label (.L*) marks
/// the depth unknown (= suppressed); control-flow joins without full
/// re-derivation cannot justify window identity.
fn optimize_tail_calls_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    let mut suppress = false;
    // Simulated net %esp displacement vs. function entry, in bytes.
    // `None` = unknown (after an internal control-flow join / mid-block
    // entry): treated as "unsafe to convert" per the soundness argument
    // above. Depth 0 with no outstanding scratch means the argument
    // window coincides bit-for-bit before and after the conversion.
    let mut depth: Option<i64> = None;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        match infos[i].kind {
            LineKind::Label => {
                let t = trimmed(store, &infos[i], i);
                if !t.starts_with(".L") {
                    suppress = false; // new function
                                      // Function boundary: known depth again. Any pending
                                      // outer depth is irrelevant here — a global label only
                                      // starts a new translation unit's function stream in
                                      // generated output.
                    depth = Some(0);
                } else {
                    // Internal join point: fallthrough or branch target.
                    // We do not trace predecessors, so the incoming depth
                    // must be assumed unequal/uncontrolled.
                    depth = None;
                }
                i += 1;
                continue;
            }
            LineKind::Directive => {
                let t = trimmed(store, &infos[i], i);
                if t == ".cfi_startproc" {
                    suppress = false;
                    depth = Some(0);
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        // Track ESP arithmetic so call-site depth stays meaningful. Only
        // constant-immediate %esp adjustments and push/pop contribute;
        // anything else (register forms, leave, and-l alignments)
        // invalidates knowledge conservatively.
        {
            let t = trimmed(store, &infos[i], i);
            if t.starts_with("pushl ") {
                if let Some(d) = depth.as_mut() {
                    *d += 4;
                }
            } else if t.starts_with("popl ") {
                let rhs = t["popl ".len()..].trim();
                if rhs == "%esp" {
                    // `popl %esp` reassigns the pointer itself.
                    depth = None;
                } else if let Some(d) = depth.as_mut() {
                    *d -= 4;
                }
            } else if (t.starts_with("subl $") || t.starts_with("addl $")) && t.ends_with(", %esp")
            {
                let num_part = &t["addl $".len()..t.len() - ", %esp".len()];
                if let Ok(n) = num_part.parse::<i64>() {
                    if let Some(d) = depth.as_mut() {
                        *d += if t.starts_with("addl $") { n } else { -n };
                    }
                } else {
                    depth = None;
                }
            } else if t.ends_with(", %esp") {
                // Any remaining %esp-ending form (movl reg,%esp, andl
                // alignment, xchg…) makes the displacement unknowable.
                // The classic FP-teardown `movl %ebp, %esp` only occurs in
                // epilogue windows which are skipped anyway, but treating
                // it uniformly here costs nothing and stays conservative.
                depth = None;
            }
        }

        if !suppress {
            let t = trimmed(store, &infos[i], i);
            if t.starts_with("leal ") && (t.contains("(%esp)") || t.contains("(%ebp)")) {
                suppress = true;
            }
            // dynamic alloca: subl %reg, %esp
            if t.starts_with("subl %") && t.ends_with(", %esp") {
                suppress = true;
            }
        }

        if infos[i].kind != LineKind::Call || suppress || depth != Some(0) {
            i += 1;
            continue;
        }

        // Pure-epilogue scan after the call.
        let limit = (i + 16).min(len);
        let mut j = i + 1;
        let mut ret_idx = None;
        let mut restored_regs: u32 = 0;
        while j < limit {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            match infos[j].kind {
                LineKind::Empty | LineKind::Directive => {
                    j += 1;
                }
                LineKind::Pop { reg } => {
                    if reg == REG_EAX || reg == REG_EDX {
                        break; // clobbers return value
                    }
                    if reg <= 31 {
                        restored_regs |= 1u32 << reg;
                    }
                    j += 1;
                }
                LineKind::Ret => {
                    ret_idx = Some(j);
                    break;
                }
                LineKind::Other { .. } => {
                    let t = trimmed(store, &infos[j], j);
                    let is_teardown = t == "movl %ebp, %esp"
                        || (t.starts_with("addl $")
                            && t.ends_with(", %esp")
                            && t["addl $".len()..t.len() - ", %esp".len()]
                                .bytes()
                                .all(|b| b.is_ascii_digit()))
                        || (t.starts_with("leal -") && t.ends_with("(%ebp), %esp"));
                    if is_teardown {
                        j += 1;
                        continue;
                    }
                    break;
                }
                _ => break,
            }
        }

        if let Some(ridx) = ret_idx {
            let call_t = trimmed(store, &infos[i], i).to_string();
            let jmp_text = if let Some(target) = call_t.strip_prefix("call ") {
                let target = target.trim();
                if let Some(reg) = target.strip_prefix("*%") {
                    // Indirect: the register must survive the epilogue (not
                    // be one of the restored callee-saved regs).
                    let fam = register_family(&format!("%{}", reg));
                    if fam <= REG_GP_MAX && (restored_regs & (1u32 << fam)) == 0 {
                        Some(format!("jmp {}", target))
                    } else {
                        None
                    }
                } else if target.starts_with('*') {
                    None // memory-indirect: frame may be gone
                } else {
                    Some(format!("jmp {}", target))
                }
            } else {
                None
            };
            if let Some(jmp) = jmp_text {
                infos[i].kind = LineKind::Nop;
                store.replace(ridx, format!("    {}", jmp));
                infos[ridx] = classify_line(store.get(ridx));
                changed = true;
            }
        }

        i += 1;
    }

    changed
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unary_rmw_breaks_copy_alias() {
        // `movl %edi,%eax; negl %eax; movl %eax,(%esi)` must NOT become
        // `movl %edi,(%esi)`: the single-operand RMW (negl) overwrites %eax,
        // and classify_line sees no destination for it (no second operand).
        // Found by the alias differential fuzz (struct-copy loops).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl $5, %edi\n",
            "    movl %edi, %eax\n",
            "    negl %eax\n",
            "    movl 4(%esp), %esi\n",
            "    movl %eax, (%esi)\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("negl %eax"), "negl must survive:\n{result}");
        // The store must still read the NEGATED register, not %edi.
        assert!(
            result.contains("movl %eax, (%esi)") || result.contains("movl %eax,(%esi)"),
            "store must keep the negated %eax:\n{result}"
        );
        assert!(
            !result.contains("movl %edi, (%esi)") && !result.contains("movl %edi,(%esi)"),
            "store must not use the stale alias %edi:\n{result}"
        );
    }

    #[test]
    fn alu_slot_use_forwards_to_register() {
        // `movl %eax,32(%esp); imull 32(%esp),%edx` with no other reader:
        // the imul must take %eax directly and the store must die.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $40, %esp\n",
            "    movl $7, %eax\n",
            "    movl %eax, 32(%esp)\n",
            "    imull 32(%esp), %edx\n",
            "    addl $40, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("imull %eax, %edx"),
            "imul not forwarded:\n{result}"
        );
        assert!(
            !result.contains("32(%esp)"),
            "store/load survived:\n{result}"
        );
    }

    #[test]
    fn slot_forwarding_crosses_loc_without_dropping_debug_metadata() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $12, %esp\n",
            "    movl $7, %eax\n",
            "    movl %eax, 8(%esp)\n",
            ".loc 1 42 3\n",
            "    movl 8(%esp), %edx\n",
            "    addl %edx, %ecx\n",
            "    addl $12, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains(".loc 1 42 3"),
            "debug metadata lost:\n{result}"
        );
        assert!(
            result.contains("addl %eax, %ecx"),
            "load was not forwarded:\n{result}"
        );
        assert!(
            !result.contains("8(%esp)"),
            "dead spill survived:\n{result}"
        );
    }

    #[test]
    fn alu_slot_use_not_forwarded_when_src_rewritten() {
        // %eax is redefined between store and use: forwarding would use the
        // stale register. The memory-operand imul must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $40, %esp\n",
            "    movl $7, %eax\n",
            "    movl %eax, 32(%esp)\n",
            "    movl $9, %eax\n",
            "    imull 32(%esp), %edx\n",
            "    addl $40, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("imull 32(%esp), %edx"),
            "imul must stay a memory op when %eax is rewritten:\n{result}"
        );
    }

    #[test]
    fn fold_reg_copy_test_through_copy() {
        // `movl %S,%D; testl %S,%D` -> `testl %S,%S`; the dead copy is then
        // reclaimed (S is the only remaining def, D unused).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $5, %eax\n",
            "    movl %eax, %edx\n",
            "    testl %eax, %edx\n",
            "    je .L1\n",
            "    movl $1, %ecx\n",
            ".L1:\n",
            "    movl %ecx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("testl %eax, %edx"),
            "test-through-copy must fold:\n{result}"
        );
        assert!(result.contains("testl %eax, %eax"), "{result}");
    }

    #[test]
    fn fold_reg_copy_zero_through_copy() {
        // `xorl %X,%X; movl %X,%Y` -> `xorl %Y,%Y` (identical flags).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    xorl %eax, %eax\n",
            "    movl %eax, %edx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("movl %eax, %edx"),
            "zero-through-copy must fold:\n{result}"
        );
        // The staging zero (or the copy) collapses to a single xor; with the
        // copy gone, an unused %edx home may be eliminated entirely.
        let xor_count = result.matches("xorl").count();
        assert!(xor_count <= 1, "expected at most one xor:\n{result}");
    }

    #[test]
    fn fold_reg_copy_widened_slot_load() {
        // `movl N(%esp),%eax; movzbl %al,%eax` -> `movzbl N(%esp),%eax`.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $16, %esp\n",
            "    movb $7, 4(%esp)\n",
            "    movl 4(%esp), %eax\n",
            "    movzbl %al, %eax\n",
            "    addl $16, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // The immediate byte store forwards straight into the widened
        // reload BECAUSE the adjacent movzbl terminates the widening: the
        // constant materializes directly (`movl $7,%eax`) and the zext of
        // the already-byte constant dies with it — strictly better than
        // folding the reload into a memory-source movzbl, and sound: the
        // fabricated upper bits never existed without the zext either.
        assert!(
            result.contains("movl $7, %eax"),
            "immediate slot store must forward to the widened load:\n{result}"
        );
    }

    #[test]
    fn forward_immediate_constant_to_reload() {
        // A constant staged to a slot and reloaded into a full-width result
        // must materialize directly, with no memory reload.
        let asm = "f:
    subl $12, %esp
    movl $3, 8(%esp)
    movl 8(%esp), %eax
    addl $12, %esp
    ret
"
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl $3, %eax"),
            "constant slot reload must fold to an immediate move:\n{result}"
        );
        // The reload is what mattered: it is gone, replaced by the immediate
        // materialization. The now-dead immediate store to the slot is left
        // for a dedicated dead-store pass (it writes memory conservatively).
        assert!(
            !result.contains("8(%esp), %eax"),
            "no slot RELOAD may remain for a pure constant:\n{result}"
        );
    }

    #[test]
    fn forward_immediate_stopped_by_call_barrier() {
        let asm = "f:
    subl $12, %esp
    movl $3, 8(%esp)
    call g
    movl 8(%esp), %eax
    addl $12, %esp
    ret
"
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("8(%esp)"),
            "a call can overwrite the slot; the reload must stay:\n{result}"
        );
    }

    #[test]
    fn fold_reg_copy_widened_slot_load_16bit_low_half() {
        // `movl N(%esp),%eax; movzwl %ax,%eax` -> `movzwl N(%esp),%eax`:
        // on little-endian, bytes N..N+1 of the 32-bit slot ARE the low
        // half of the stored value, so the widened load reads exactly the
        // half the movzwl consumes.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $16, %esp\n",
            "    movl 4(%esp), %eax\n",
            "    movzwl %ax, %eax\n",
            "    addl $16, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movzwl 4(%esp), %eax"),
            "widened 16-bit slot load must fold:\n{result}"
        );
    }

    #[test]
    fn fold_reg_copy_compare_through_copy() {
        // `movl %esi,%eax; cmpl $97,%eax` -> `cmpl $97,%esi` + dead move.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl 4(%esp), %esi\n",
            "    movl %esi, %eax\n",
            "    cmpl $97, %eax\n",
            "    jl .L1\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".L1:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("cmpl $97, %eax"),
            "compare-through-copy must fold:\n{result}"
        );
        assert!(result.contains("cmpl $97, %esi"), "{result}");
    }

    #[test]
    fn fold_reg_copy_range_check_fuses_sub_cmp() {
        // `movl %esi,%eax; subl $48,%eax; cmpl $9,%eax; ja` ->
        // `cmpl $57,%esi; ja` (identical flags, staging pair dead).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl 4(%esp), %esi\n",
            "    movl %esi, %eax\n",
            "    subl $48, %eax\n",
            "    cmpl $9, %eax\n",
            "    ja .L1\n",
            "    movl $1, %eax\n",
            "    ret\n",
            ".L1:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("subl $48, %eax"),
            "range-check staging must fold:\n{result}"
        );
        // The fused compare then meets the existing compare-with-memory
        // fold: the slot load itself disappears.
        assert!(result.contains("cmpl $57, 4(%esp)"), "{result}");
    }

    #[test]
    fn fold_reg_copy_range_check_keeps_live_stage() {
        // When %eax is READ after the compare (before any redefinition),
        // the staging subtract must survive: only the compare source may
        // change (here nothing folds).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl 4(%esp), %esi\n",
            "    movl %esi, %eax\n",
            "    subl $48, %eax\n",
            "    cmpl $9, %eax\n",
            "    ja .L1\n",
            "    addl $100, %eax\n",
            "    ret\n",
            ".L1:\n",
            "    movl $2, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("subl $48, %eax"),
            "live staging subtract must survive:\n{result}"
        );
    }

    #[test]
    fn fold_reg_copy_imm_store() {
        // `movl $512,%ecx; movl %ecx,76(%esp)` -> `movl $512,76(%esp)` when
        // %ecx is dead afterwards.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $96, %esp\n",
            "    movl $512, %ecx\n",
            "    movl %ecx, 76(%esp)\n",
            "    movb 76(%esp), %al\n",
            "    addl $96, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl $512, 76(%esp)"),
            "immediate store must fold:\n{result}"
        );
    }

    #[test]
    fn fold_reg_copy_duplicate_block_tail_merge() {
        // X: [xorl;xorl] jmp L;  Y: [xorl;xorl] falls into L
        // -> X becomes `jmp Y`, both duplicate bodies collapse.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    testl %eax, %eax\n",
            "    je .LY\n",
            ".LX:\n",
            "    xorl %eax, %eax\n",
            "    xorl %edx, %edx\n",
            "    jmp .LJ\n",
            ".LY:\n",
            "    xorl %eax, %eax\n",
            "    xorl %edx, %edx\n",
            ".LJ:\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        let xor_count = result.matches("xorl %edx, %edx").count();
        assert!(
            xor_count <= 1,
            "duplicate block bodies must tail-merge (found {}):\n{}",
            xor_count,
            result
        );
        // After the merge X's `jmp .LY` is a jump-to-next (the bodies and
        // the dead xors are reclaimed by the dead-move cleanup), so the
        // control flow collapses to `je .LY` falling straight through.
        assert!(
            !result.contains("xorl %eax, %eax\n    xorl %edx, %edx\n    jmp"),
            "{result}"
        );
    }

    #[test]
    fn fold_reg_copy_tail_merge_keeps_surviving_body() {
        // Pattern 8 must keep Y's body: it is the surviving copy.  Both
        // arms of this diamond store the same constant to a stack slot;
        // the merged code must still store it on every path (a regression
        // here miscompiled the bool-merge shape `int ok = a && b;` where
        // the constant 0/1 the arms staged was silently dropped and the
        // consumer read stale stack garbage).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    testl %eax, %eax\n",
            "    je .LY\n",
            ".LX:\n",
            "    movl $22, 8(%esp)\n",
            "    jmp .LJ\n",
            ".LY:\n",
            "    movl $22, 8(%esp)\n",
            ".LJ:\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        let stores = result.matches("movl $22, 8(%esp)").count();
        assert!(
            stores == 1,
            "tail-merge must keep exactly one surviving store (found {}):\n{}",
            stores,
            result
        );
        // .LY must still be reachable and hold the store: no empty Y.
        assert!(
            result.contains(".LY:\n    movl $22, 8(%esp)"),
            "surviving block body must not be deleted:\n{}",
            result
        );
    }

    #[test]
    fn fold_reg_copy_no_test_fold_when_copy_is_live() {
        // `movl %eax,%edx` where %edx is genuinely consumed must NOT have
        // the copy deleted; the test may still be rewritten (flag-equal),
        // but here the consumer is a real use of %edx, so the mov stays.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl 4(%esp), %eax\n",
            "    movl %eax, %edx\n",
            "    testl %eax, %edx\n",
            "    je .L1\n",
            "    addl $7, %edx\n",
            "    movl %edx, 8(%esp)\n",
            ".L1:\n",
            "    movl %edx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movl %eax, %edx"), "{result}");
        assert!(result.contains("testl %eax, %eax"), "{result}");
    }

    #[test]
    fn alu_slot_use_not_forwarded_across_partial_reader() {
        // A LIVE movzbl of the slot keeps the full-width store alive and
        // blocks register forwarding past it (the sub-word read must come
        // from the slot, and the store therefore survives the DSE).
        // The blocker reads the slot's HIGH half (16(%esp) overlaps the
        // 4 bytes at 12(%esp)): no register substitution can express it, so
        // the store stays and the memory-operand imul must stay too.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $40, %esp\n",
            "    movl $7, %eax\n",
            "    movl %eax, 12(%esp)\n",
            "    movzwl 14(%esp), %ecx\n",
            "    imull 12(%esp), %edx\n",
            "    addl %ecx, %edx\n",
            "    addl $40, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("imull 12(%esp), %edx"),
            "imul must survive behind a mid-slot reader:\n{result}"
        );
        assert!(result.contains("movzwl 14(%esp), %ecx"), "{result}");
    }

    #[test]
    fn alu_slot_forward_handles_negative_ebp_slots() {
        // ebp-based frames encode slots as plain negative offsets.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebp\n",
            "    movl %esp, %ebp\n",
            "    subl $8, %esp\n",
            "    movl $7, %eax\n",
            "    movl %eax, -8(%ebp)\n",
            "    addl -8(%ebp), %edx\n",
            "    movl %ebp, %esp\n",
            "    popl %ebp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("addl %eax, %edx"),
            "addl not forwarded:\n{result}"
        );
        assert!(!result.contains("-8(%ebp)"), "store survived:\n{result}");
    }
    #[test]
    fn cmp_with_memory_folds_when_register_dies_at_branch() {
        // `movl SLOT,%eax; testl %eax,%eax; je` with %eax dead on both
        // paths -> `cmpl $0, SLOT; je`. Needs CFG liveness (consumer is a
        // branch). Both branch targets overwrite %eax before reading it.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl 4(%esp), %eax\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    movl $3, %eax\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".L1:\n",
            "    xorl %eax, %eax\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("cmpl $0, 4(%esp)"), "{result}");
        assert!(!result.contains("movl 4(%esp), %eax"), "{result}");
    }

    #[test]
    fn cmp_with_memory_not_folded_when_register_stays_live() {
        // %eax is READ on the fallthrough path: the load must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl 4(%esp), %eax\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    incl %eax\n",
            ".L1:\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("4(%esp), %eax") || result.contains("testl %eax, %eax"),
            "load must survive while %eax is live:\n{result}"
        );
        assert!(!result.contains("cmpl $0, 4(%esp)"), "{result}");
    }

    #[test]
    fn cmp_with_memory_not_folded_for_byte_load() {
        // `movzbl SLOT,%eax; cmpl $4,%eax` must NOT become `cmpl $4, SLOT`:
        // the slot holds a 4-byte word, the byte load only the low byte.
        // Regression for structs_bitfields (union `uu.b[0] != 0x04`).
        let asm = concat!(
            "f:\\n",
            ".cfi_startproc\\n",
            "    subl $8, %esp\\n",
            "    movzbl 4(%esp), %eax\\n",
            "    cmpl $4, %eax\\n",
            "    jne .L1\\n",
            "    xorl %eax, %eax\\n",
            "    addl $8, %esp\\n",
            "    ret\\n",
            ".L1:\\n",
            "    movl $13, %eax\\n",
            "    addl $8, %esp\\n",
            "    ret\\n",
            ".cfi_endproc\\n",
            ".size f, .-f\\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movzbl 4(%esp), %eax"), "{result}");
        assert!(!result.contains("cmpl $4, 4(%esp)"), "{result}");
    }

    #[test]
    fn memory_operand_not_folded_for_word_load() {
        // `movzwl SLOT,%ecx; addl %ecx,%eax` must NOT fold to
        // `addl SLOT,%eax`: the slot is a 2-byte field, addl reads 4.
        let asm = concat!(
            "f:\\n",
            ".cfi_startproc\\n",
            "    subl $8, %esp\\n",
            "    movzwl 4(%esp), %ecx\\n",
            "    addl %ecx, %eax\\n",
            "    ret\\n",
            ".cfi_endproc\\n",
            ".size f, .-f\\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movzwl 4(%esp), %ecx"), "{result}");
        assert!(!result.contains("addl 4(%esp), %eax"), "{result}");
    }

    #[test]
    fn slot_forwarding_respects_wide_x87_folded_stores() {
        // spectral_norm -m32 -O2 miscompile: `fstpl 20(%esp)` writes
        // 20..28, so the forwarded mapping %edx -> 24(%esp) must die at
        // the fstpl. The reload of 24(%esp) must NOT become `movl %edx,
        // 48(%esp)` (stale high word of the previous iteration's value).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $56, %esp\n",
            "    movl (%ecx), %eax\n",
            "    movl 4(%ecx), %edx\n",
            "    movl %eax, 20(%esp)\n",
            "    movl %edx, 24(%esp)\n",
            "    fldl 12(%esp)\n",
            "    fldl 20(%esp)\n",
            "    faddp %st, %st(1)\n",
            "    fstpl 20(%esp)\n",
            "    movl 20(%esp), %eax\n",
            "    movl %eax, 44(%esp)\n",
            "    movl 24(%esp), %eax\n",
            "    movl %eax, 48(%esp)\n",
            "    addl $56, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("movl %edx, 48(%esp)"),
            "stale forwarded %edx must not survive the fstpl overwrite:\n{result}"
        );
        assert!(
            result.contains("movl 24(%esp), %eax") || result.contains("24(%esp)"),
            "the high word must be re-read from memory:\n{result}"
        );
    }

    #[test]
    fn global_fnptr_call_folds_load_spill_call() {
        // Two same-slot sites in one function must BOTH fold (every
        // reference to the displacement belongs to a candidate triple).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl pio_ops+8, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl %esi, %edx\n",
            "    movl $15879, %eax\n",
            "    call *0(%esp)\n",
            "    movl pio_ops+8, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl %esi, %edx\n",
            "    movl $57106, %eax\n",
            "    call *0(%esp)\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(result.matches("call *pio_ops+8").count(), 2, "{result}");
        assert!(!result.contains("call *0(%esp)"), "{result}");
        assert!(!result.contains("movl pio_ops+8, %eax"), "{result}");
    }

    #[test]
    fn global_fnptr_call_folds_when_slot_resealed_by_next_store() {
        // Slot 0 is REUSED: after the first call the very next slot access
        // is a full-width store (the second triple's spill). Linear-seal
        // proof folds BOTH triples even though the ownership proof fails
        // for neither alone... and the last one seals via ownership of the
        // remaining window. The early_serial_console shape.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl pio_ops+4, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl %edi, %edx\n",
            "    movl $3, %eax\n",
            "    call *0(%esp)\n",
            "    movl pio_ops+4, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl %edi, %edx\n",
            "    movl $5, %eax\n",
            "    call *0(%esp)\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(result.matches("call *pio_ops+4").count(), 2, "{result}");
        assert!(!result.contains("call *0(%esp)"), "{result}");
    }

    #[test]
    fn global_fnptr_linear_seal_blocked_by_branch_before_reuse() {
        // A conditional branch sits between the call and the resealing
        // store: the reader on the taken path is invisible to the linear
        // walk, so the FIRST triple must NOT fold. (The second one folds
        // via its own proof; slot 4 read at .L1 keeps ownership failing.)
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl pio_ops, %eax\n",
            "    movl %eax, 4(%esp)\n",
            "    call *4(%esp)\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    movl %ecx, 4(%esp)\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".L1:\n",
            "    movl 4(%esp), %eax\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("call *4(%esp)"),
            "must stay staged:\n{result}"
        );
        assert!(!result.contains("call *pio_ops"), "{result}");
    }

    #[test]
    fn global_fnptr_call_fold_blocked_by_foreign_slot_use() {
        // The slot is ALSO read by a non-call line: no triple may fold.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl pio_ops, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    call *0(%esp)\n",
            "    movl 0(%esp), %ebx\n",
            "    movl %ebx, (%ecx)\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("call *0(%esp)"), "{result}");
        assert!(!result.contains("call *pio_ops"), "{result}");
    }

    #[test]
    fn global_fnptr_call_fold_blocked_by_crossed_memory_op() {
        // A store to the global between load and call must block the fold
        // (the call must use the OLD pointer value).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subl $8, %esp\n",
            "    movl ops_tab, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl %ebx, ops_tab\n",
            "    call *0(%esp)\n",
            "    addl $8, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("call *0(%esp)"), "{result}");
        assert!(result.contains("movl %ebx, ops_tab"), "{result}");
    }

    #[test]
    fn copy_then_test_collapses_to_self_test() {
        // `movl %edx, %eax; testl %edx, %eax` computes edx&edx: rewrite to
        // `testl %edx, %edx` and drop the dead copy. (Regression: the
        // pattern matcher once built "testl %%edx, %eax" — reg32_name
        // already includes the '%' — so the pattern NEVER fired.)
        let asm = concat!(
            "f:\n",
            "    movl %edx, %eax\n",
            "    testl %edx, %eax\n",
            "    jne .L1\n",
            "    movl $3, %eax\n",
            "    ret\n",
            ".L1:\n",
            "    movl $4, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("testl %edx, %edx"), "{result}");
        assert!(!result.contains("movl %edx, %eax"), "{result}");
    }

    #[test]
    fn imm_store_fusion_folds_accumulator_immediate_into_slot_store() {
        // `movl $65535, %eax; movw %ax, S` -> `movw $65535, S` (truncated
        // immediate) when %eax is dead after; same for the movl form.
        let asm = concat!(
            "f:\n",
            "    movl $65535, %eax\n",
            "    movw %ax, 68(%esp)\n",
            "    movl $7, %eax\n",
            "    movl %eax, 32(%esp)\n",
            "    xorl %eax, %eax\n",
            "    call g\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movw $65535, 68(%esp)"), "{result}");
        assert!(result.contains("movl $7, 32(%esp)"), "{result}");
        assert!(!result.contains("%ax,"), "{result}");
    }

    #[test]
    fn imm_store_fusion_blocked_by_slot_reload_in_window() {
        // The reload of 8(%esp) is deleted by forwarding in the UNFUSED
        // shape (%eax still holds $5); fusing first would force a real
        // memory reload. The fusion must not fire.
        let asm = concat!(
            "f:\n",
            "    movl $5, %eax\n",
            "    movl %eax, 8(%esp)\n",
            "    movl 8(%esp), %eax\n",
            "    movl %eax, (%ecx)\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl $5, %eax") || result.contains("movl $5, (%ecx)"),
            "value must stay in a register path, not round-trip via a fused store:\n{result}"
        );
    }

    #[test]
    fn windowed_dse_kills_shadowed_store_across_source_reg_update() {
        // `movl %eax, 8(%esp)` is dead: the slot is overwritten before any
        // read. The intervening ALU on %eax must NOT protect the store
        // (register fate is irrelevant to memory deadness).
        // The call keeps the second store un-forwardable and the trailing
        // pointer store keeps %edx alive, so only the windowed DSE can act:
        // it must delete the FIRST store (shadowed by the second before any
        // read) and keep the second.
        let asm = concat!(
            "f:\n",
            "    movl %eax, 8(%esp)\n",
            "    andl $31, %eax\n",
            "    movl %eax, 8(%esp)\n",
            "    call g\n",
            "    movl 8(%esp), %edx\n",
            "    movl %edx, (%ecx)\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        let stores = result.matches(", 8(%esp)\n").count();
        assert_eq!(
            stores, 1,
            "first store must die, second must survive:\n{result}"
        );
        assert!(result.contains("movl 8(%esp), %edx"), "{result}");
    }

    #[test]
    fn never_read_store_dse_is_function_scoped() {
        // f stores 24(%esp) and never reads it; g READS 24(%esp).
        // Module-wide analysis kept f's dead store alive (cross-function
        // aliasing) and the inter-function label killed the ESP walk.
        let asm = concat!(
            "f:\n",
            "    subl $28, %esp\n",
            "    movl %eax, 24(%esp)\n",
            "    xorl %eax, %eax\n",
            "    addl $28, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
            "g:\n",
            "    subl $28, %esp\n",
            "    movl %ecx, 24(%esp)\n",
            "    call ext\n",
            "    movl 24(%esp), %eax\n",
            "    addl $28, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size g, .-g\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("movl %eax, 24(%esp)"),
            "f's never-read store must die despite g reading the same offset:\n{result}"
        );
        assert!(
            result.contains("movl %ecx, 24(%esp)"),
            "g's live store must survive:\n{result}"
        );
    }

    #[test]
    fn never_read_store_dse_respects_x87_wide_reads() {
        // The high dword of the double at 16(%esp) is only read by the
        // folded x87 operand `fldl 16(%esp)` (8-byte read). The store to
        // 20(%esp) must be treated as read (d_i regparm miscompile).
        let asm = concat!(
            "f:\n",
            "    subl $24, %esp\n",
            "    movl %eax, 16(%esp)\n",
            "    movl %edx, 20(%esp)\n",
            "    fldl 16(%esp)\n",
            "    fstpl 0(%esp)\n",
            "    movl 0(%esp), %eax\n",
            "    movl 4(%esp), %edx\n",
            "    addl $24, %esp\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %edx, 20(%esp)"),
            "high-dword store feeding fldl must survive:\n{result}"
        );
    }

    #[test]
    fn dead_callee_saved_move_crosses_epilogue_stack_adjustment() {
        let asm = concat!(
            "f:\n",
            "    movl %eax, %edi\n",
            "    addl $12, %esp\n",
            "    popl %ebp\n",
            "    popl %edi\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("movl %eax, %edi"));
        assert!(result.contains("addl $12, %esp"));
        assert!(result.contains("popl %edi"));
    }

    #[test]
    fn cfg_keeps_implicit_multiply_dividend() {
        let asm = concat!(
            "f:\n",
            "    movl %ebx, %eax\n",
            "    mull %ecx\n",
            "    movl %edx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movl %ebx, %eax"), "{result}");
        assert!(result.contains("mull %ecx"), "{result}");
    }

    #[test]
    fn cfg_retargets_dead_accumulator_update_before_loop_backedge() {
        let asm = concat!(
            "f:\n",
            ".Lloop:\n",
            "    movl (%ebx), %eax\n",
            "    testl %eax, %eax\n",
            "    je .Ldone\n",
            "    movl %ebx, %eax\n",
            "    incl %eax\n",
            "    movl %eax, %ebx\n",
            "    jmp .Lloop\n",
            ".Ldone:\n",
            "    movl %ebx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("incl %ebx"), "{result}");
        assert!(!result.contains("movl %ebx, %eax\n    incl %eax"));
        assert!(!result.contains("movl %eax, %ebx"));
    }

    #[test]
    fn cfg_retargets_all_explicit_accumulator_sources() {
        let asm = concat!(
            "f:\n",
            "    movl %ebx, %eax\n",
            "    addl %eax, %eax\n",
            "    movl %eax, %ebx\n",
            "    movl $0, %eax\n",
            "    pushl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("addl %ebx, %ebx"), "{result}");
        assert!(!result.contains("addl %eax, %ebx"), "{result}");
    }

    #[test]
    fn cfg_does_not_retarget_implicit_or_partial_accumulator_ops() {
        for op in ["imull %eax", "incw %ax"] {
            let asm = format!(
                "f:\n    movl %ebx, %eax\n    {op}\n    movl %eax, %ebx\n    xorl %eax, %eax\n    pushl %ebx\n    ret\n.cfi_endproc\n"
            );
            let result = peephole_optimize(asm.to_string());
            assert!(result.contains("movl %ebx, %eax"), "{op}: {result}");
            assert!(result.contains(op), "{op}: {result}");
        }
    }

    #[test]
    fn sign_extending_moves_are_not_classified_as_string_ops() {
        // The central oracle must not confuse the sign-extending MOVs with
        // the string primitives — they differ by one letter and one
        // semantic universe.
        assert_eq!(classify_implicit_operands("movsbl (%ecx), %eax"), (0, 0));
        assert_eq!(classify_implicit_operands("movswl (%ecx), %eax"), (0, 0));
        assert_eq!(
            classify_implicit_operands("movsl"),
            (1 << REG_ESI | 1 << REG_EDI, 1 << REG_ESI | 1 << REG_EDI)
        );
        assert_eq!(
            classify_implicit_operands("rep movsl").1,
            1 << REG_ECX | 1 << REG_ESI | 1 << REG_EDI
        );
    }

    #[test]
    fn pointer_relay_folds_into_source_clobbering_load() {
        let asm = concat!(
            "f:\n",
            "    movl %eax, %ecx\n",
            "    movsbl (%ecx), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("movl %eax, %ecx"));
        assert!(result.contains("movsbl (%eax), %eax"));
    }

    #[test]
    fn redundant_sign_ext_after_byte_load_removed() {
        // `movsbl (%esi),%eax` already sign-extends; the trailing
        // `movsbl %al,%eax` is a pure no-op.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsbl (%esi), %eax\n",
            "    movsbl %al, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movsbl (%esi), %eax"));
        assert!(!result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn redundant_sign_ext_after_setcc_movzbl_removed() {
        // `setbe %al; movzbl %al,%eax` leaves a bool (0/1 == its own sign
        // extension) in %eax; the trailing `movsbl %al,%eax` is a no-op.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    cmpl $9, %edx\n",
            "    setbe %al\n",
            "    movzbl %al, %eax\n",
            "    movsbl %al, %eax\n",
            "    movl %eax, 4(%esp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("setbe %al"));
        assert!(result.contains("movzbl %al, %eax"));
        assert!(!result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn sign_ext_not_removed_after_zero_extend_of_high_byte() {
        // `movzbl (%esi),%eax` zero-extends: bits 8..31 are 0, NOT the sign of
        // bit 7. A following `movsbl %al,%eax` is a REAL change (e.g. 0xFF ->
        // 0xFFFFFFFF) and must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movzbl (%esi), %eax\n",
            "    movsbl %al, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movzbl (%esi), %eax"));
        assert!(result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn sign_ext_high_byte_is_not_a_full_register_copy() {
        for dst in ["%eax", "%ecx"] {
            let asm = format!(
                "f:\n.cfi_startproc\n    movsbl (%esi), %eax\n    movsbl %ah, {dst}\n    ret\n.cfi_endproc\n"
            );
            let result = peephole_optimize(asm.to_string());
            assert!(
                result.contains(&format!("movsbl %ah, {dst}")),
                "high-byte sign extension must remain:\n{result}"
            );
        }
    }

    #[test]
    fn high_byte_zero_extend_does_not_inherit_low_byte_bool_fact() {
        // setcc proves AL is 0/1, but says nothing about AH. With AH=0x80,
        // movzbl %ah produces 128 and the following movsbl must produce -128.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $32768, %eax\n",
            "    cmpl $0, %edx\n",
            "    setne %al\n",
            "    movzbl %ah, %ecx\n",
            "    movsbl %cl, %ecx\n",
            "    movl %ecx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movzbl %ah, %ecx"), "{result}");
        assert!(result.contains("movsbl %cl, %ecx"), "{result}");
    }

    #[test]
    fn sign_ext_cross_register_becomes_movl_copy() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsbl (%esi), %eax\n",
            "    movsbl %al, %ecx\n",
            "    addl %ecx, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("movsbl %al"), "{result}");
        // REDUCED CONTRACT: with fallthrough copy propagation the whole
        // chain collapses further — the intermediate copy is propagated into
        // the add and dies, and because %eax is already sign-extended,
        // `addl %ecx,%eax` becomes the doubling `addl %eax,%eax`.
        assert!(result.contains("addl %eax, %eax"), "{result}");
    }

    #[test]
    fn sign_ext_removed_after_movl_small_positive_imm() {
        // `movl $32,%eax` leaves a sign-extended value (bits 8..31 == sign of
        // bit 7); the trailing `movsbl %al,%eax` is a no-op.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $32, %eax\n",
            "    movsbl %al, %eax\n",
            "    movl %eax, 4(%esp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn sign_ext_removed_after_andl_small_imm() {
        // `andl $32,%eax` yields 0..32 (non-negative, sign-extended); the
        // trailing `movsbl %al,%eax` is a no-op.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    andl $32, %eax\n",
            "    movsbl %al, %eax\n",
            "    movl %eax, 4(%esp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn sign_ext_kept_after_movl_negative_beyond_byte() {
        // `movl $-129,%eax` is 0xFFFFFF7F: bit 7 is 0 but bits 8..31 are 1,
        // so it is NOT its own sign extension and `movsbl %al,%eax` (0x7F ->
        // 0x0000007F) is a real change that must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $-129, %eax\n",
            "    movsbl %al, %eax\n",
            "    movl %eax, 4(%esp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn sign_ext_kept_after_andl_wide_imm() {
        // `andl $255,%eax` can leave 0x80..0xFF (bit 7 set, bits 8..31 zero),
        // so the value is not sign-extended; `movsbl %al,%eax` must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    andl $255, %eax\n",
            "    movsbl %al, %eax\n",
            "    movl %eax, 4(%esp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn cfg_dead_moves_across_branch_successors() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl %eax, %ecx\n",
            "    testl %eax, %eax\n",
            "    je .Lexit\n",
            "    movl %ebx, %ecx\n",
            "    jmp .Ldone\n",
            ".Lexit:\n",
            "    movl %esi, %ecx\n",
            ".Ldone:\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("%ecx"));
        assert!(result.contains("je .Lexit"));
        assert!(result.contains("jmp .Ldone"));
    }

    #[test]
    fn stack_top_store_consumed_by_ret_survives_dse() {
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl %eax, (%esp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movl %eax, (%esp)"), "{result}");
    }

    #[test]
    fn esp_store_survives_matching_load_across_push() {
        let asm = concat!(
            "f:\n",
            "    movl %eax, 4(%esp)\n",
            "    pushl %edx\n",
            "    movl 8(%esp), %eax\n",
            "    popl %edx\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("movl %eax, 4(%esp)"));
        assert!(result.contains("movl 8(%esp), %eax"));
    }

    #[test]
    fn has_indirect_memory_access_treats_indexed_frame_forms_as_indirect() {
        // REGRESSION PIN (990531-1 class): base-only frame operands stay
        // non-indirect; ANY indexed form anchored at a frame register is a
        // runtime-computed address and must be flagged indirect so every
        // conservative guard (store forwarding, DSE, fold windows) sees it.
        use super::has_indirect_memory_access as indirect;
        // Base-only frame forms: frame addressing, not pointer aliasing.
        assert!(!indirect("(%esp)"));
        assert!(!indirect("12(%esp)"));
        assert!(!indirect("-8(%ebp)"));
        assert!(!indirect("movl %esi, 12(%esp)"));
        // Non-frame bases: always indirect.
        assert!(indirect("(%ebx)"));
        // Indexed forms anchored at frame registers: runtime addresses.
        assert!(indirect("12(%esp, %ebx)"));
        assert!(indirect("12(%esp,%ebx)"));
        assert!(indirect("0(%ebp, %esi, 4)"));
        assert!(indirect("movb $0, 12(%esp, %ebx)"));
    }

    #[test]
    fn slot_load_not_forwarded_across_indexed_frame_store() {
        // END-TO-END REGRESSION PIN for gcc.c-torture/execute 990531-1:
        // `data.word = w; data.byte[reg] = 0; return data.word;` lowers to a
        // slot store, an INDEXED byte store aliasing that same slot, then a
        // reload. The reload must survive verbatim -- forwarding `%esi`
        // across the indexed store returned the stale word.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %esi\n",
            "    subl $20, %esp\n",
            "    movl 32(%esp), %ebx\n",
            "    movl 36(%esp), %esi\n",
            "    movl %esi, 12(%esp)\n",
            "    movb $0, 12(%esp, %ebx)\n",
            "    movl 12(%esp), %eax\n",
            "    addl $20, %esp\n",
            "    popl %esi\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl 12(%esp), %eax"),
            "reload must NOT be forwarded across the indexed aliased store:\n{result}"
        );
        assert!(
            !result.contains("movl %esi, %eax"),
            "stale-register substitution is exactly the miscompile:\n{result}"
        );
    }

    #[test]
    fn return_metadata_limits_edx_liveness_to_wide_functions() {
        let narrow =
            peephole_optimize("narrow:\n    movl %ebx, %edx\n    ret\n.cfi_endproc\n".to_string());
        assert!(!narrow.contains("movl %ebx, %edx"));

        let wide = peephole_optimize(
            concat!(
                "wide:\n",
                "# lccc-i686-return-uses-edx\n",
                "    movl %ebx, %edx\n",
                "    ret\n",
                ".cfi_endproc\n",
            )
            .to_string(),
        );
        assert!(wide.contains("movl %ebx, %edx"));
        assert!(!wide.contains("lccc-i686-return-uses-edx"));
    }

    #[test]
    fn unused_callee_saves_fold_into_omit_fp_frame() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %esi\n",
            "    pushl %edi\n",
            "    pushl %ebp\n",
            "    subl $12, %esp\n",
            "    addl %esi, %ebx\n",
            "    movl %ebx, %eax\n",
            "    addl $12, %esp\n",
            "    popl %ebp\n",
            "    popl %edi\n",
            "    popl %esi\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("pushl %edi"));
        assert!(!result.contains("pushl %ebp"));
        assert!(!result.contains("popl %edi"));
        assert!(!result.contains("popl %ebp"));
        assert!(result.contains("subl $20, %esp"));
        assert!(result.contains("addl $20, %esp"));
        assert!(result.contains("pushl %ebx"));
        assert!(result.contains("pushl %esi"));
    }

    #[test]
    fn test_redundant_store_load() {
        // With a frame-pointer prologue, (%ebp) slots are genuine frame
        // slots: the load is forwarded and the never-read store deleted.
        let asm = "    pushl %ebp\n    movl %esp, %ebp\n    movl %eax, -8(%ebp)\n    movl -8(%ebp), %eax\n    popl %ebp\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("-8(%ebp)"),
            "store/load pair should be fully eliminated, got:\n{}",
            result
        );
    }

    #[test]
    fn test_redundant_zext_removed() {
        // Second movzbl of an already-byte value is a no-op.
        let asm = "f:\n    movzbl (%ecx), %eax\n    movzbl %al, %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(
            result.matches("movzbl").count(),
            1,
            "redundant re-extension must be removed:\n{}",
            result
        );
    }

    #[test]
    fn test_zext_cleared_at_label() {
        // Byte-ness must not survive a control-flow merge.
        let asm =
            "f:\n    movzbl (%ecx), %eax\n.LBB1:\n    movzbl %al, %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(
            result.matches("movzbl").count(),
            2,
            "flags must clear at labels:\n{}",
            result
        );
    }

    #[test]
    fn test_zext_cleared_by_add() {
        // addl can overflow past 255: re-extension is REQUIRED.
        let asm =
            "f:\n    movzbl (%ecx), %eax\n    addl $200, %eax\n    movzbl %al, %eax\n    ret\n"
                .to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(
            result.matches("movzbl").count(),
            2,
            "addl invalidates byte-ness:\n{}",
            result
        );
    }

    #[test]
    fn test_zext_preserved_by_andl_imm8() {
        // andl $127 keeps the value in byte range: re-extension is dead.
        let asm =
            "f:\n    movzbl (%ecx), %eax\n    andl $127, %eax\n    movzbl %al, %eax\n    ret\n"
                .to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(
            result.matches("movzbl").count(),
            1,
            "andl imm8 preserves byte-ness:\n{}",
            result
        );
    }

    #[test]
    fn zext_high_byte_is_not_a_full_register_copy() {
        // EAX < 256 proves AH == 0, not that zero-extending AH is a no-op or
        // equivalent to copying the full EAX value.
        for (src, dst) in [("%ah", "%eax"), ("%ah", "%edx")] {
            let asm = format!("f:\n    movzbl (%ecx), %eax\n    movzbl {src}, {dst}\n    ret\n");
            let result = peephole_optimize(asm.to_string());
            assert!(
                result.contains(&format!("movzbl {src}, {dst}")),
                "high-byte extension must remain:\n{result}"
            );
        }
    }

    #[test]
    fn narrow_facts_die_at_unparsed_and_atomic_writers() {
        for writer in [
            "bswapl %eax",
            "lock cmpxchgl %edx, (%ecx)",
            "lock xaddl %eax, (%ecx)",
        ] {
            let asm = format!(
                "f:\n    movzbl (%ecx), %eax\n    {writer}\n    movzbl %al, %eax\n    ret\n"
            );
            let result = peephole_optimize(asm.to_string());
            assert_eq!(
                result.matches("movzbl").count(),
                2,
                "{writer} must invalidate byte-ness:\n{result}"
            );
        }
    }

    #[test]
    fn test_tail_call_direct_i686() {
        // The %ebx copy is dead (never read), so the liveness pass deletes
        // it; the unused-callee-save pass then removes the push/pop pair
        // (frameless shape), and the tail-call conversion turns the call
        // into a jmp. The whole function collapses to `jmp target`.
        let asm =
            "f:\n    pushl %ebx\n    movl %eax, %ebx\n    call target\n    popl %ebx\n    ret\n"
                .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("jmp target"),
            "expected tail call, got:\n{}",
            result
        );
        assert!(
            !result.contains("call target"),
            "call should be gone:\n{}",
            result
        );
        assert!(
            !result.contains("pushl %ebx") && !result.contains("popl %ebx"),
            "dead callee-save pair must go:\n{}",
            result
        );
        // If a restore had survived, it would have to precede the jmp; with
        // the pair gone the ordering constraint is vacuous.
    }

    #[test]
    fn test_tail_call_suppressed_outstanding_callee_save() {
        // STACK-WINDOW SOUNDNESS REGRESSION TEST. %ebx is live across the
        // call (the pair survives liveness cleanup), so the call happens at
        // simulated esp-depth 4 ≠ entry CFA. Converting to a jmp would make
        // the tail-callee read its incoming stack arguments from the
        // parent's incoming-argument window above the return address while
        // the pre-conversion code would have passed values prepared in the
        // caller's own frame below — observably different for every
        // non-register ABI. This is exactly the shape that miscompiled GCC
        // torture nestfunc-2/-3/-5 and 20000822-1 into reading trampoline
        // template bytes as data (see optimize_tail_calls_i686 doc comment).
        let asm = concat!(
            "f:\n",
            "    pushl %ebx\n",
            "    movl %eax, %ebx\n",
            "    incl %ebx\n",
            "    movl %ebx, %eax\n",
            "    call target\n",
            "    popl %ebx\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("jmp target"),
            "conversion must be suppressed at nonzero depth:\n{}",
            result
        );
        assert!(
            result.contains("call target"),
            "original call must survive:\n{}",
            result
        );
        assert!(
            result.contains("ret"),
            "plain ret must remain (no early transformation):\n{}",
            result
        );
    }

    #[test]
    fn test_tail_call_suppressed_outgoing_arg_store_below_cfa() {
        // The canonical i386 cdecl wrapper: `return inc(y * 2)` stores the
        // prepared argument into a scratch area BELOW the entry CFA and
        // unwinds it afterwards. At the call, depth ≠ 0, so the argument
        // window after an epilogue+jmp would be a DIFFERENT memory range —
        // conversion must stay off regardless of value coincidence
        // (`fwd: return inc(a)` used to pass by luck).
        let asm = concat!(
            "wrap:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %esi\n",
            "    movl 16(%esp), %esi\n",
            "    addl %esi, %esi\n",
            "    subl $20, %esp\n",
            "    movl %esi, 0(%esp)\n",
            "    call inc@PLT\n",
            "    addl $20, %esp\n",
            "    popl %esi\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("jmp inc@PLT"),
            "stack-arg forwarding must not convert:\n{result}"
        );
        assert!(
            result.contains("call inc@PLT"),
            "call must survive:\n{result}"
        );
    }

    #[test]
    fn test_tail_call_suppressed_after_internal_join_label() {
        // After an internal .L label the straight-line esp simulation ends:
        // fallthrough and branch predecessors may disagree on depth, so no
        // conversion may fire inside such a region.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    call helper\n",
            "    addl $4, %esp\n",
            ".Ljoin:\n",
            "    call target\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("jmp target"),
            "post-join calls have unknown depth; must not convert:\n{result}"
        );
    }

    #[test]
    fn unused_callee_saves_removed_in_frameless_multi_pop_function() {
        // bcmp shape: four dead pairs guarding a tail-jmp, no frame, no
        // (%esp) references. All four pairs must vanish.
        let asm = concat!(
            "bcmp:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %esi\n",
            "    pushl %edi\n",
            "    pushl %ebp\n",
            "    popl %ebp\n",
            "    popl %edi\n",
            "    popl %esi\n",
            "    popl %ebx\n",
            "    jmp memcmp\n",
            ".cfi_endproc\n",
            ".size bcmp, .-bcmp\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(!result.contains("pushl"), "{result}");
        assert!(!result.contains("popl"), "{result}");
        assert!(result.contains("jmp memcmp"), "{result}");
    }

    #[test]
    fn unused_callee_saves_kept_when_esp_slots_exist_in_frameless_fn() {
        // Frameless but the body reads 0(%esp): removing a push would shift
        // the slot. Pair must stay.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    movl 8(%esp), %eax\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("pushl %ebx"), "{result}");
        assert!(result.contains("popl %ebx"), "{result}");
    }

    #[test]
    fn unused_callee_saves_removed_across_multiple_epilogues() {
        // Framed shape with TWO epilogues (early return + fallthrough).
        //
        // STACK-WINDOW SOUNDNESS CONTRACT (PR #255 gate + multi-epilogue
        // relaxation): the envelope gate admits the whole-function transform
        // when every interior control edge is provably depth-neutral:
        //  * all entry paths cross the save-block linearly down to the
        //    single alloc (a Label inside the alloc..first-dealloc window,
        //    or a JmpIndirect there, still refuses);
        //  * an interior Jmp/CondJmp is accepted ONLY when its target is a
        //    resolvable label positioned AFTER the alloc -- such paths read
        //    identical addresses at identical depths because the grown
        //    alloc exactly cancels the removed push at every point that
        //    can load/store through %esp;
        //  * between a grown dealloc-addl and the reduced pop set nothing
        //    reads memory through %esp (fixed emitter epilogue shape).
        // Under this shape's `je .L1` (target after the first dealloc) the
        // unused %ebp pair therefore goes, BOTH dealloc grows move in
        // lockstep with the alloc (+4 each), and the live %ebx pair
        // survives verbatim.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            "    subl $16, %esp\n",
            "    movl %eax, %ebx\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    movl %ebx, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl 0(%esp), %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".L1:\n",
            "    xorl %eax, %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // EVOLVED CONTRACT: fallthrough copy propagation rewrites the sole
        // `movl %ebx,%eax` reader to %eax (a self-move, nopped), which makes
        // %ebx provably dead — so the callee-save pass now removes BOTH
        // unused pairs and the frame grows by 8. Strictly smaller code.
        assert!(
            !result.contains("%ebp") && !result.contains("%ebx"),
            "both unused pairs must go under a resolvable forward edge:\n{result}"
        );
        assert_eq!(
            result.matches("subl $24, %esp").count(),
            1,
            "alloc grows by 8 in lockstep (two pairs):\n{result}"
        );
        assert_eq!(
            result.matches("addl $24, %esp").count(),
            2,
            "BOTH deallocs grow by 8 (lockstep accounting):\n{result}"
        );
        assert_eq!(result.matches("ret").count(), 2, "{result}");
    }

    #[test]
    fn unused_callee_saves_refused_for_pre_alloc_label_target() {
        // The interior `je` targets a label positioned BEFORE the alloc
        // (loop back-edge re-entering the save block region). Every edge
        // whose target lies at or above the alloc executes part of the
        // function under the pre-transform depth; the transform is refused
        // and NOTHING changes.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            ".Lback:\n",
            "    subl $16, %esp\n",
            "    movl %eax, %ebx\n",
            "    testl %eax, %eax\n",
            "    je .Lback\n",
            "    movl %ebx, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("pushl %ebp") && result.contains("popl %ebp"),
            "pair must survive: interior edge targets pre-alloc label:\n{result}"
        );
        assert_eq!(
            result.matches("subl $16, %esp").count(),
            1,
            "alloc must not grow:\n{result}"
        );
        assert_eq!(
            result.matches("addl $20, %esp").count(),
            0,
            "no dealloc growth may leak out of a refused transform:\n{result}"
        );
        assert!(
            result.contains("pushl %ebx"),
            "live %ebx pair stays regardless:\n{result}"
        );
    }

    #[test]
    fn unused_callee_saves_removed_for_post_alloc_backedge() {
        // REFINED CONTRACT: a label BETWEEN the alloc and the first dealloc
        // is admitted when (a) no indirect jumps exist and (b) every
        // resolvable edge into the label originates after the alloc — both
        // entrant classes (fallthrough, post-alloc jumps) crossed the pair
        // push + grown alloc, so the +4k growth cancels the removed push at
        // the label for every arrival. The unused %ebp pair goes, alloc and
        // dealloc grow in lockstep, and the live %ebx pair stays.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            "    subl $16, %esp\n",
            ".Lback:\n",
            "    movl %eax, %ebx\n",
            "    testl %eax, %eax\n",
            "    je .Lback\n",
            "    movl %ebx, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // EVOLVED CONTRACT: with fallthrough copy propagation the staging
        // pair `movl %eax,%ebx` / `movl %ebx,%eax` collapses (the reader
        // becomes a self-move and both moves die), so %ebx is provably dead
        // and BOTH pairs are removed; alloc and dealloc grow by 8.
        assert!(
            !result.contains("%ebp") && !result.contains("%ebx"),
            "both unused pairs must be removed:\n{result}"
        );
        // FINAL EVOLVED CONTRACT: after both callee-save pairs are removed and
        // the never-read 0(%esp) store deleted, the grown frame is referenced
        // by nothing. eliminate_dead_frame_allocation then elides it too; the
        // guard admits this because the sole internal edge (.Lback -> .Lback)
        // targets a label positioned AFTER the alloc, so every body path
        // crosses the alloc. End state: no pushes/pops, no frame adjusts.
        assert!(
            !result.contains("subl") && !result.contains("addl"),
            "grown dead frame is elided after pair removal:\n{result}"
        );
        assert!(
            !result.contains("pushl") && !result.contains("popl"),
            "no prologue/epilogue traffic may remain:\n{result}"
        );
    }

    #[test]
    fn unused_callee_saves_removed_single_straight_line_epilogue() {
        // The CONTRACT-COMPLIANT counterpart of the multi-epilogue shape
        // above: one straight-line epilogue, no internal control flow, so
        // the envelope gate admits the transform: the unused %ebp pair goes,
        // the single alloc/dealloc pair grows by 4, and the live %ebx pair
        // survives untouched.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            "    subl $16, %esp\n",
            "    movl %eax, %ebx\n",
            "    movl %ebx, %eax\n",
            "    movl %eax, 0(%esp)\n",
            "    movl 0(%esp), %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // Observed contract on current main: with no internal control flow
        // the envelope gate admits the transform; additionally the synthetic
        // body's dataflow is fully dead (nothing ever consumes %eax), so the
        // liveness/dead-store phases erase BOTH save pairs and every body
        // instruction, growing the sole alloc/dealloc pair by 4 bytes per
        // removed register (2 x 4 = 8 here). Encode that end-to-end result.
        assert!(
            !result.contains("%ebp") && !result.contains("%ebx"),
            "both unreferenced pairs must go:\n{result}"
        );
        // FINAL EVOLVED CONTRACT: with the pairs removed and the fully-dead
        // body erased, the grown frame is referenced by nothing, so
        // eliminate_dead_frame_allocation elides the grown subl/addl too.
        assert!(
            !result.contains("subl") && !result.contains("addl"),
            "grown dead frame is elided after pair removal:\n{result}"
        );
        assert!(
            !result.contains("pushl") && !result.contains("popl"),
            "no prologue/epilogue traffic may remain:\n{result}"
        );
    }

    #[test]
    fn unused_callee_saves_refused_when_interleaved_label_edge_prealloc() {
        // Two-epilogue frame whose `.L1` label sits in the INTERLEAVED zone
        // (between the first and the last dealloc) -- but the edge INTO .L1
        // originates BEFORE the save block (source index <= alloc). Such a
        // path never crosses the pair push + alloc, so its arrival depth at
        // .L1 would shift by 4 after the transform (+4 alloc, -4 push do not
        // cancel on that path): the WHOLE transform must be refused -- pairs
        // stay, no growth anywhere.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            "    subl $16, %esp\n",
            "    movl %ebx, 0(%esp)\n",
            "    movl 0(%esp), %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".L1:\n",
            "    xorl %eax, %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // NOTE: the frame is otherwise well-formed (both epilogues restore
        // both pairs), so the ONLY refusal reason is the pre-alloc edge into
        // the interleaved-zone label -- exactly what this pin guards.
        assert!(
            result.contains("pushl %ebp") && result.contains("popl %ebp"),
            "pair must survive: an edge into the interleaved label comes \
             from before the alloc:\n{result}"
        );
        assert_eq!(
            result.matches("subl $16, %esp").count(),
            1,
            "alloc must not grow:\n{result}"
        );
        assert_eq!(
            result.matches("addl $16, %esp").count(),
            2,
            "deallocs must not grow:\n{result}"
        );
    }

    #[test]
    fn unused_callee_saves_refused_when_jmp_indirect_present_with_interleaved_label() {
        // Same multi-epilogue shape as the accepted forward-edge case, but a
        // `jmp *` exists in the tail. An indirect jump can target the
        // interleaved-zone label from any depth, which text-level analysis
        // cannot exclude: the conservative choice is to refuse the whole
        // transform rather than risk a shifted arrival depth.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            "    subl $16, %esp\n",
            "    movl %eax, %ebx\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    movl %ebx, 0(%esp)\n",
            "    movl %ebx, %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".L1:\n",
            "    xorl %eax, %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".Ltail:\n",
            "    jmp *%eax\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // The frame is otherwise well-formed (both epilogues restore both
        // pairs), so the only refusal reason is the unresolvable `jmp *`.
        assert!(
            result.contains("pushl %ebp") && result.contains("popl %ebp"),
            "pair must survive: jmp * makes the interleaved label's entry \
             depths unprovable:\n{result}"
        );
        assert_eq!(
            result.matches("subl $16, %esp").count(),
            1,
            "alloc must not grow:\n{result}"
        );
    }

    #[test]
    fn unused_callee_saves_removed_with_tail_edge_into_interleaved_label() {
        // Positive control for the interleaved-zone label rule: a tail-region
        // (past the LAST dealloc) edge into the interleaved label is safe --
        // paths reaching the tail have crossed the pair push + alloc and
        // fully unwound, so the arrival depth cancels to zero exactly like
        // interior edges. The unused %ebp pair still goes, both deallocs grow
        // in lockstep, and the live %ebx pair survives on both epilogues.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushl %ebx\n",
            "    pushl %ebp\n",
            "    subl $16, %esp\n",
            "    movl %eax, %ebx\n",
            "    testl %eax, %eax\n",
            "    je .L1\n",
            "    movl %ebx, 0(%esp)\n",
            "    movl %ebx, %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".L1:\n",
            "    xorl %eax, %eax\n",
            "    addl $16, %esp\n",
            "    popl %ebp\n",
            "    popl %ebx\n",
            "    ret\n",
            ".Ltail:\n",
            "    testl %edi, %edi\n",
            "    je .L1\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm.to_string());
        // EVOLVED CONTRACT: propagation proves %ebx dead (its only reader
        // was rewritten to a self-move), so BOTH unused pairs go; the frame
        // grows by 8 and the tail-edge shape is preserved.
        assert!(
            !result.contains("%ebp") && !result.contains("%ebx"),
            "both unused pairs must go:\n{result}"
        );
        assert_eq!(
            result.matches("subl $24, %esp").count(),
            1,
            "alloc grows by 8 in lockstep (two pairs):\n{result}"
        );
        assert_eq!(
            result.matches("addl $24, %esp").count(),
            2,
            "BOTH deallocs grow by 8 (lockstep accounting):\n{result}"
        );
    }

    #[test]
    fn test_tail_call_suppressed_by_leal_slot() {
        // Address of a frame slot escapes to the callee: converting would
        // hand the callee a dangling pointer after frame release.
        let asm = "f:\n    subl $12, %esp\n    leal 4(%esp), %eax\n    call target\n    addl $12, %esp\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("call target"),
            "must stay a call:\n{}",
            result
        );
    }

    #[test]
    fn test_tail_call_indirect_not_through_restored_reg() {
        // call *%ebx with %ebx restored by the epilogue: the pointer dies
        // before the jmp could use it — must NOT convert.
        let asm =
            "f:\n    pushl %ebx\n    movl %eax, %ebx\n    call *%ebx\n    popl %ebx\n    ret\n"
                .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("call *%ebx"),
            "must stay a call:\n{}",
            result
        );
    }

    #[test]
    fn test_ebp_pointer_store_kept_without_frame() {
        // WITHOUT the FP prologue (as under -fomit-frame-pointer), %ebp is a
        // data register: `movl %eax, -8(%ebp)` writes through a pointer and
        // must survive even though nothing in this function reads it back.
        let asm = "    movl %eax, -8(%ebp)\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %eax, -8(%ebp)"),
            "pointer-write through data %ebp must not be deleted, got:\n{}",
            result
        );
    }

    #[test]
    fn test_store_load_different_reg() {
        // The trailing `pushl %ecx` observes the loaded value: without a
        // reader, the census-gated dead-move pass correctly deletes the
        // load before forwarding can see it (nothing in a ret-free
        // snippet observes %ecx).
        let asm = "    movl %eax, -8(%ebp)\n    movl -8(%ebp), %ecx\n    pushl %ecx\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %eax, %ecx"),
            "should forward: {}",
            result
        );
        assert!(
            !result.contains("-8(%ebp), %ecx"),
            "should eliminate load: {}",
            result
        );
    }

    #[test]
    fn test_self_move() {
        let asm = "    movl %eax, %eax\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert_eq!(result.trim(), "");
    }

    /// idivl writes %eax implicitly (no comma operand -> Other{REG_NONE}).
    /// Copy propagation must NOT rewrite readers of %eax past the division:
    /// `movl %ebx,%eax; cltd; idivl %ecx; movl %eax,%edi` -- the last move
    /// consumes the QUOTIENT, not %ebx. (mulhaz.c: a + a/b returned 2*a.)
    #[test]
    fn test_copy_prop_stops_at_idivl() {
        let asm =
            "    movl %ebx, %eax\n    cltd\n    idivl %ecx\n    movl %eax, %edi\n    pushl %edi\n"
                .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("movl %ebx, %edi"),
            "quotient consumer must keep reading %eax: {}",
            result
        );
    }

    /// mull (one-operand) clobbers %eax and %edx implicitly; the alias from
    /// a copy of either must die at the multiply.
    #[test]
    fn test_copy_prop_stops_at_mull() {
        let asm =
            "    movl %esi, %eax\n    mull %ecx\n    movl %eax, %edi\n    pushl %edi\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("movl %esi, %edi"),
            "product consumer must keep reading %eax: {}",
            result
        );
    }

    #[test]
    fn test_redundant_jump() {
        let asm = "    jmp .Lfoo\n.Lfoo:\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("jmp"),
            "should eliminate redundant jmp: {}",
            result
        );
        assert!(result.contains(".Lfoo:"), "should keep label: {}", result);
    }

    #[test]
    fn test_branch_inversion() {
        let asm = [
            "    jl .LBB2",
            "    jmp .LBB4",
            ".LBB2:",
            "    movl %eax, %ecx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("jge .LBB4"),
            "should invert to jge: {}",
            result
        );
        assert!(
            !result.contains("jmp .LBB4"),
            "should remove jmp: {}",
            result
        );
    }

    #[test]
    fn test_compare_branch_fusion() {
        let asm = [
            "    cmpl %ecx, %eax",
            "    setl %al",
            "    movzbl %al, %eax",
            "    testl %eax, %eax",
            "    jne .LBB2",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(result.contains("jl .LBB2"), "should fuse to jl: {}", result);
        assert!(
            !result.contains("setl"),
            "should eliminate setl: {}",
            result
        );
    }

    #[test]
    fn test_compare_branch_fusion_with_store_load() {
        // Pattern: cmp + setCC + movzbl + store + load + test + jne
        // The store/load pair should be skipped, allowing fusion.
        let asm = [
            "    cmpl %ecx, %eax",
            "    setge %al",
            "    movzbl %al, %eax",
            "    movl %eax, -16(%ebp)",
            "    movl -16(%ebp), %eax",
            "    testl %eax, %eax",
            "    jne .LBB5",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("jge .LBB5"),
            "should fuse to jge: {}",
            result
        );
        assert!(
            !result.contains("setge"),
            "should eliminate setge: {}",
            result
        );
        assert!(
            !result.contains("movzbl"),
            "should eliminate movzbl: {}",
            result
        );
        assert!(
            !result.contains("testl"),
            "should eliminate testl: {}",
            result
        );
    }

    #[test]
    fn test_compare_branch_fusion_unmatched_store_bails() {
        // If the store has no matching load, we should NOT fuse (the boolean
        // escapes to another basic block).
        let asm = [
            "    cmpl %ecx, %eax",
            "    setge %al",
            "    movzbl %al, %eax",
            "    movl %eax, -16(%ebp)",
            "    testl %eax, %eax",
            "    jne .LBB5",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        // Should NOT fuse because the store has no matching load
        assert!(
            result.contains("setge"),
            "should keep setge (unmatched store): {}",
            result
        );
    }

    #[test]
    fn test_compare_branch_fusion_inverted() {
        // Test je (inverted condition)
        let asm = [
            "    cmpl %ecx, %eax",
            "    setl %al",
            "    movzbl %al, %eax",
            "    testl %eax, %eax",
            "    je .LBB3",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("jge .LBB3"),
            "should fuse to jge (inverted): {}",
            result
        );
        assert!(
            !result.contains("setl"),
            "should eliminate setl: {}",
            result
        );
    }

    #[test]
    fn test_dead_store() {
        // Two consecutive stores to the same slot: first is dead, second survives.
        // But never-read store elimination also removes the second store if no
        // loads exist. Use a load after the second store to keep it alive.
        let asm = [
            "    movl %eax, -8(%ebp)",
            "    movl %ecx, -8(%ebp)",
            "    movl -8(%ebp), %edx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("%eax, -8(%ebp)"),
            "first store dead: {}",
            result
        );
        assert!(result.contains("%ecx"), "second store alive: {}", result);
    }

    #[test]
    fn test_memory_fold() {
        let asm = ["    movl -48(%ebp), %ecx", "    addl %ecx, %eax"].join("\n") + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("addl -48(%ebp), %eax"),
            "should fold: {}",
            result
        );
    }

    // Note: store forwarding tests removed - global_store_forwarding is disabled
    // due to FP computation regressions.

    // ── P17 else-hoist: the else store hoists and the ORIGINAL condition
    // jumps to the join; the jmp disappears.  The inverted mnemonic would
    // swap the arms' values (miscompile caught auditing cpucheck).
    #[test]
    fn else_hoist_keeps_original_polarity() {
        let asm = "    andl $1, %edx\n    je .LBB12\n.LBB11:\n    movl $48, 112(%esp)\n    jmp .LBB13\n.LBB12:\n    movl $32, 112(%esp)\n.LBB13:\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        let store = r.find("movl $32, 112(%esp)").expect("else store hoisted");
        let br = r
            .find("je .LBB13")
            .expect("branch to join, original polarity");
        assert!(store < br, "hoisted store precedes branch: {}", r);
        assert!(!r.contains("jmp .LBB13"), "jmp deleted: {}", r);
        assert!(r.contains("movl $48, 112(%esp)"), "then store kept: {}", r);
    }

    #[test]
    fn else_hoist_refuses_unequal_slot_sets() {
        // then writes 112, else writes 116: hoisting would leak values.
        let asm = "    testl %eax, %eax\n    je .LBB12\n    movl $48, 112(%esp)\n    jmp .LBB13\n.LBB12:\n    movl $32, 116(%esp)\n.LBB13:\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("jmp .LBB13"), "diamond kept: {}", r);
    }

    // ── P18 redundant re-zeroing: an xor-self whose only intervening lines
    // are other xor-self zeroings is invisible (same value, same flags).
    #[test]
    fn redundant_rezero_collapse() {
        let asm = "    xorl %eax, %eax\n    xorl %edi, %edi\n    xorl %eax, %eax\n    xorl %ebp, %ebp\n    xorl %eax, %eax\n    xorl %esi, %esi\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert_eq!(
            r.matches("xorl %eax, %eax").count(),
            1,
            "one eax zero remains: {}",
            r
        );
        for reg in ["%edi", "%ebp", "%esi"] {
            assert!(
                r.contains(&format!("xorl {}, {}", reg, reg)),
                "{} kept: {}",
                reg,
                r
            );
        }
    }

    #[test]
    fn redundant_rezero_respects_x_consumers() {
        // The first zero IS observable when %eax is read in between.
        let asm = "    xorl %eax, %eax\n    pushl %eax\n    xorl %eax, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert_eq!(r.matches("xorl %eax, %eax").count(), 2, "both kept: {}", r);
    }

    // ── fuse_setcc_branch red-team tests ──
    //
    // Canonical fused shape (the emitter's real relay chain):
    //   orl; setne %al; movzbl %al,%eax; movl %eax,%edx; testl %eax,%edx; jne
    // The test is redundant: setCC→movzbl→movl all preserve EFLAGS, so the
    // jcc can read the producer's ZF (identical: bool==0 ⇔ or-result==0).

    #[test]
    fn setcc_fuse_drops_dead_window() {
        // Both carriers rewritten before any read or label: the whole
        // window (setCC..relay) is dead and disappears. invert_cc("ne")
        // == "e", so the rewritten je is textually unchanged but now
        // consumes the producer's flags.
        let asm = "f:\n    orl %ebx, %eax\n    setne %al\n    movzbl %al, %eax\n    movl %eax, %edx\n    testl %eax, %edx\n    je .L1\n    movl $7, %eax\n    movl $8, %edx\n    xorl %ecx, %ecx\n.L1:\n    movl %ecx, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(!r.contains("testl"), "test dropped: {}", r);
        assert!(!r.contains("setne"), "dead window NOPed: {}", r);
        assert!(r.contains("je .L1"), "branch onto producer flags: {}", r);
    }

    #[test]
    fn setcc_fuse_inverts_for_je() {
        // sete's CC is "e": je (bool==0) inverts to jne (producer != 0).
        let asm = "f:\n    cmpl $1, %ebx\n    sete %al\n    movzbl %al, %eax\n    movl %eax, %edx\n    testl %eax, %edx\n    je .L1\n    movl $7, %eax\n    movl $8, %edx\n    xorl %ecx, %ecx\n.L1:\n    movl %ecx, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(!r.contains("testl"), "test dropped: {}", r);
        assert!(
            r.contains("jne .L1"),
            "je(sete) -> jne(producer inverted): {}",
            r
        );
    }

    // REGRESSION (audit finding): setCC writes only the low BYTE of its
    // destination. `movzwl %ax, %r32` imports AH — garbage — into the
    // bool, so ZF would not reflect it. The fusion must be refused and
    // the window left untouched.
    #[test]
    fn setcc_fuse_refuses_movzwl_carrier() {
        let asm = "f:\n    orl %ebx, %eax\n    setne %al\n    movzwl %ax, %edx\n    testl %eax, %edx\n    je .L1\n    xorl %ecx, %ecx\n.L1:\n    movl %ecx, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("testl %eax, %edx"), "test kept: {}", r);
        assert!(r.contains("movzwl %ax, %edx"), "carrier kept: {}", r);
    }

    // REGRESSION (audit finding): a CondJmp after the fused branch is a
    // flags READER (reader-check must precede the barrier check): after
    // the rewrite it would consume the producer's flags instead of the
    // dropped test's. The fusion must be refused.
    #[test]
    fn setcc_fuse_refuses_jcc_reader_after_branch() {
        // Producer is a cmpl (no other pass may drop a cmpl-headed setCC
        // test), and a WRONG fusion would rewrite je -> jne (invert of
        // "e") — observable. The following jc is a flags reader the guard
        // must see (reader-check before the barrier check).
        let asm = "f:\n    cmpl $1, %ebx\n    sete %al\n    movzbl %al, %eax\n    movl %eax, %edx\n    testl %eax, %edx\n    je .L1\n    jc .L2\n.L1:\n    xorl %ecx, %ecx\n    jmp .L3\n.L2:\n    xorl %ecx, %ecx\n.L3:\n    movl %ecx, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        // The wrong-fusion signature is `jne` (invert of "e"). Other sound
        // passes may normalize the redundant test's operands, so assert on
        // the branch pair's semantics, not the test's exact spelling.
        assert!(!r.contains("jne .L1"), "fusion must be refused: {}", r);
        assert!(r.contains("testl"), "test kept: {}", r);
        assert!(r.contains("je .L1"), "jcc untouched: {}", r);
        assert!(r.contains("jc .L2"), "following reader untouched: {}", r);
    }

    // The guard skips block labels (the fallthrough physically enters the
    // next block): a setCC there would read the producer's flags instead
    // of the bool's. Refuse.
    #[test]
    fn setcc_fuse_refuses_setcc_reader_behind_label() {
        // The guard skips the block label (fallthrough enters the next
        // block) and must find the setCC reader there. cmpl producer: no
        // other pass may drop this test, and a wrong fusion would show
        // jne (invert of "e").
        let asm = "f:\n    cmpl $1, %ebx\n    sete %al\n    movzbl %al, %eax\n    movl %eax, %edx\n    testl %eax, %edx\n    je .L1\n.L1:\n    setne %cl\n    movzbl %cl, %ecx\n    movl %ecx, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(!r.contains("jne .L1"), "fusion must be refused: {}", r);
        assert!(r.contains("testl"), "test kept: {}", r);
        // (je .L1 may additionally be deleted by the branch-to-next rule —
        // also sound: the target IS the fallthrough here.)
    }

    // A flags WRITER between the branch and a later reader makes the fuse
    // safe: the reader sees the writer's flags either way.
    #[test]
    fn setcc_fuse_allows_writer_before_reader() {
        let asm = "f:\n    cmpl $1, %ebx\n    sete %al\n    movzbl %al, %eax\n    movl %eax, %edx\n    testl %eax, %edx\n    je .L1\n.L1:\n    cmpl $2, %ecx\n    setne %cl\n    movzbl %cl, %ecx\n    movl %ecx, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        // je targets the immediate fallthrough, so the (correct) branch
        // elimination drops it entirely; the writer (cmpl) makes dropping
        // the test safe for the later setCC reader.
        assert!(!r.contains("testl %eax, %edx"), "test dropped: {}", r);
        assert!(!r.contains("je .L1"), "branch to next removed: {}", r);
        assert!(r.contains("cmpl $2, %ecx"), "flag writer present: {}", r);
    }

    // The bool is alive after the branch (stored to a slot): only the test
    // may drop; the setCC/movzbl/relay window must survive intact.
    #[test]
    fn setcc_fuse_keeps_window_for_live_bool() {
        let asm = "f:\n    orl %ebx, %eax\n    setne %al\n    movzbl %al, %eax\n    movl %eax, %edx\n    testl %eax, %edx\n    je .L1\n.L1:\n    movl %edx, 8(%esp)\n    xorl %ecx, %ecx\n    movl 8(%esp), %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(!r.contains("testl %eax, %edx"), "test dropped: {}", r);
        assert!(r.contains("setne %al"), "setCC kept (bool live): {}", r);
        assert!(r.contains("movzbl %al, %eax"), "movzbl kept: {}", r);
        assert!(r.contains("movl %eax, %edx"), "relay kept: {}", r);
    }

    // ── P16 constant staging through a register (numeric and symbol).
    #[test]
    fn const_staging_numeric_folded() {
        let asm = "f:\n    movl $77, %ebx\n    movl %ebx, %edx\n    pushl %edx\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(
            r.contains("movl $77, %edx"),
            "value folds into reader: {}",
            r
        );
        assert_eq!(r.matches("movl").count(), 1, "staging gone: {}", r);
    }

    #[test]
    fn const_staging_symbol_folded() {
        let asm = "f:\n    movl $.Lstr, %ebx\n    movl %ebx, %edx\n    pushl %edx\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("movl $.Lstr, %edx"), "symbol folds: {}", r);
        assert_eq!(r.matches("movl").count(), 1, "staging gone: {}", r);
    }

    #[test]
    fn const_staging_respects_multi_reader() {
        // %ebx read twice: folding one reader would not kill the staging,
        // and P16 requires the census to be zero.
        let asm = "f:\n    movl $77, %ebx\n    movl %ebx, %edx\n    pushl %ebx\n    pushl %edx\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("movl $77, %ebx"), "staging kept: {}", r);
    }

    // ── Census fallback in eliminate_dead_reg_moves: a dead move whose
    // only "readers" the linear walk cannot see (label crossing) is
    // deleted; the caller-visible return registers are protected.
    #[test]
    fn census_fallback_kills_label_crossing_dead_move() {
        let asm = "f:\n    movl %eax, %ebx\n    testl %eax, %eax\n    je .L1\n.L1:\n    movl $9, %eax\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(
            !r.contains("movl %eax, %ebx"),
            "dead staging deleted: {}",
            r
        );
    }

    #[test]
    fn census_fallback_respects_edx_return() {
        // %edx is the high half of a 64-bit return: the function never
        // reads it, but the caller does at `ret`.  The compiler marks such
        // functions with the return-uses-edx comment, which both the CFG
        // liveness pass and the census fallback's ret-observation guard
        // honour.
        let asm = "f:\n    # lccc-i686-return-uses-edx\n    movl %eax, %edx\n    testl %eax, %eax\n    je .L1\n.L1:\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("movl %eax, %edx"), "edx return kept: {}", r);
    }

    // ── P15 16-bit canonical compare narrowing.
    #[test]
    fn canonical_16_cmp_narrows_to_cmpw() {
        // imm >= 128: cmpw $imm16 (4 B) beats cmpl $imm32 (6 B).
        let asm = "f:\n    movzwl 8(%esp), %eax\n    cmpl $200, %eax\n    je .L1\n.L1:\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("cmpw $200, %ax"), "narrowed: {}", r);
        assert!(!r.contains("cmpl $200"), "32-bit cmp gone: {}", r);
    }

    #[test]
    fn canonical_16_keeps_imm8_compares() {
        // imm < 128 already encodes as cmpl imm8 (3 B) — narrowing would grow.
        let asm = "f:\n    movzwl 8(%esp), %eax\n    cmpl $79, %eax\n    je .L1\n.L1:\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("cmpl $79, %eax"), "imm8 kept 32-bit: {}", r);
    }

    #[test]
    fn canonical_16_refuses_signed_consumers() {
        // jg reads SF/OF: the width change is not flag-safe for signed
        // values >= 0x8000.
        let asm = "f:\n    movzwl 8(%esp), %eax\n    cmpl $200, %eax\n    jg .L1\n.L1:\n    ret\n";
        let r = peephole_optimize(asm.to_string());
        assert!(r.contains("cmpl $200, %eax"), "kept signed: {}", r);
    }

    #[test]
    fn test_reverse_move_elimination() {
        let asm = ["    movl %eax, %ecx", "    movl %ecx, %eax"].join("\n") + "\n";
        let result = peephole_optimize(asm.to_string());
        // The reverse move must be gone: %eax already holds the value the
        // first move staged into %ecx.  In this ret-free fixture the
        // census fallback may additionally delete the now-unread staging
        // move itself (nothing observes %ecx), so only the reverse move
        // is asserted here.
        assert!(
            !result.contains("movl %ecx, %eax"),
            "should eliminate reverse: {}",
            result
        );
    }

    // Note: push/pop elimination test removed - eliminate_push_pop_pairs is disabled
    // due to callee-save/leal epilogue interactions.

    #[test]
    fn test_addl_1_to_incl() {
        let asm = "    addl $1, %eax\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("incl %eax"),
            "should convert to incl: {}",
            result
        );
        assert!(
            !result.contains("addl"),
            "should eliminate addl: {}",
            result
        );
    }

    #[test]
    fn test_subl_1_to_decl() {
        let asm = "    subl $1, %ecx\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("decl %ecx"),
            "should convert to decl: {}",
            result
        );
        assert!(
            !result.contains("subl"),
            "should eliminate subl: {}",
            result
        );
    }

    #[test]
    fn test_movl_0_to_xorl() {
        let asm = "    movl $0, %ebx\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("xorl %ebx, %ebx"),
            "should convert to xorl: {}",
            result
        );
        assert!(
            !result.contains("movl"),
            "should eliminate movl: {}",
            result
        );
    }

    #[test]
    fn test_redundant_movsbl() {
        let asm = ["    movsbl (%ecx), %eax", "    movsbl %al, %eax"].join("\n") + "\n";
        let result = peephole_optimize(asm.to_string());
        assert_eq!(
            result.matches("movsbl").count(),
            1,
            "should eliminate redundant movsbl: {}",
            result
        );
    }

    #[test]
    fn test_addl_neg1_to_decl() {
        let asm = "    addl $-1, %edx\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("decl %edx"),
            "should convert to decl: {}",
            result
        );
    }

    #[test]
    fn test_subl_neg1_to_incl() {
        let asm = "    subl $-1, %esi\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("incl %esi"),
            "should convert to incl: {}",
            result
        );
    }

    #[test]
    fn test_addl_1_not_incl_before_adcl() {
        // addl $1 followed by adcl must NOT be converted to incl,
        // because incl does not set the carry flag (CF).
        // This pattern is used in 64-bit negation: notl+notl+addl+adcl.
        let asm = [
            "    notl %eax",
            "    notl %edx",
            "    addl $1, %eax",
            "    adcl $0, %edx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("addl $1, %eax"),
            "must keep addl before adcl: {}",
            result
        );
        assert!(
            !result.contains("incl"),
            "must NOT convert to incl before adcl: {}",
            result
        );
    }

    #[test]
    fn test_subl_1_not_decl_before_sbbl() {
        // subl $1 followed by sbbl must NOT be converted to decl,
        // because decl does not set the carry flag.
        let asm = ["    subl $1, %eax", "    sbbl $0, %edx"].join("\n") + "\n";
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("subl $1, %eax"),
            "must keep subl before sbbl: {}",
            result
        );
        assert!(
            !result.contains("decl"),
            "must NOT convert to decl before sbbl: {}",
            result
        );
    }
    // ── Dead frame allocation elision: a `subl $N,%esp`/`addl $N,%esp`
    // frame that no instruction ever touches is pure overhead and must be
    // removed (GCC emits none for register-only leaves).

    #[test]
    fn test_dead_frame_elided_plain_leaf() {
        // The canonical `-m16 -Os` regparm leaf: the frame space is never
        // touched, so both frame adjusts vanish and the body survives.
        let asm = "plain:\n    subl $12, %esp\n    movzwl %ax, %eax\n    incl %eax\n    movzbl %al, %eax\n    addl $12, %esp\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("subl $12, %esp") && !result.contains("addl $12, %esp"),
            "dead frame adjusts must be elided:\n{result}"
        );
        assert!(result.contains("incl %eax"), "body must survive:\n{result}");
        assert!(result.contains("ret"), "ret must survive:\n{result}");
    }

    #[test]
    fn test_dead_frame_elided_multiple_epilogues() {
        // Two equal deallocs on two return paths are both dropped.
        let asm = "f:\n    subl $8, %esp\n    testl %eax, %eax\n    je .Lbb1\n    movl $1, %eax\n    addl $8, %esp\n    ret\n.Lbb1:\n    xorl %eax, %eax\n    addl $8, %esp\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("subl $8, %esp") && result.matches("addl $8, %esp").count() == 0,
            "alloc and both deallocs must be elided:\n{result}"
        );
    }

    #[test]
    fn test_dead_frame_kept_when_slot_accessed() {
        // A value spilled to the local frame and reloaded at a DIFFERENT size
        // makes the frame live: store-forwarding cannot rewrite a 32-bit
        // store feeding a 16-bit load, so the (%esp) slot access survives and
        // the frame must not be elided.
        // The local slot holds %ecx on one path and %ebx on the other; a join
        // load of the slot cannot be store-forwarded (which register? it
        // depends on the branch), so the (%esp) memory traffic is genuine and
        // the frame must be kept.
        let asm = "f:\n    subl $12, %esp\n    testl %edx, %edx\n    je .L1\n    movl %ecx, -4(%esp)\n    jmp .L2\n.L1:\n    movl %ebx, -4(%esp)\n.L2:\n    movl -4(%esp), %eax\n    addl $12, %esp\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("-4(%esp)"),
            "diamond slot access must not be forwarded:\n{result}"
        );
        assert!(
            result.contains("subl $12, %esp") && result.contains("addl $12, %esp"),
            "frame in use via (%esp) slot must be kept:\n{result}"
        );
    }

    #[test]
    fn test_dead_frame_kept_across_call() {
        // A call requires the stack alignment/argument space the frame
        // provides; never elide without proving the ABI unaffected.
        let asm = "f:\n    subl $12, %esp\n    call g\n    addl $12, %esp\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("subl $12, %esp") && result.contains("addl $12, %esp"),
            "frame across a call must be kept:\n{result}"
        );
    }

    #[test]
    fn test_dead_frame_kept_with_callee_save_push() {
        // Functions with push/pop callee saves are owned by
        // eliminate_unused_callee_saves; this pass must not renumber them.
        let asm = "f:\n    pushl %ebx\n    subl $16, %esp\n    addl %ebx, %eax\n    addl $16, %esp\n    popl %ebx\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("pushl %ebx") && result.contains("popl %ebx"),
            "callee-save pair must be left intact by this pass:\n{result}"
        );
    }

    #[test]
    fn test_dead_frame_kept_with_frame_pointer() {
        // The frame-pointer prologue itself disqualifies elision: the
        // (%ebp)-based slot is genuine frame memory read back into the return
        // register.
        let asm = "f:\n    pushl %ebp\n    movl %esp, %ebp\n    subl $8, %esp\n    movl %ecx, -4(%ebp)\n    movl -4(%ebp), %eax\n    leave\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        // The guarantee from THIS pass: a frame-pointer function's alloc is
        // never elided (the (%ebp)/push disqualify it). Other passes may
        // forward the slot traffic, so assert only the frame machinery.
        assert!(
            result.contains("subl $8, %esp") && result.contains("leave"),
            "frame-pointer function's alloc/epilogue must be kept:\n{result}"
        );
    }

    #[test]
    fn test_dead_frame_kept_on_backward_jump_across_alloc() {
        // A loop edge that branches backward to a label at/before the alloc
        // would re-enter the body without reopening the frame; conservatively
        // keep the frame.
        let asm = "f:\n.Ltop:\n    subl $12, %esp\n    decl %ecx\n    jnz .Ltop\n    addl $12, %esp\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("subl $12, %esp") && result.contains("addl $12, %esp"),
            "backward edge across alloc must keep the frame:\n{result}"
        );
    }

    // ── Widen store-forwarding: a narrower slot store feeding a wider
    // unsigned reload (`movb %al,slot` / `movl slot,%eax`) forwards to an
    // explicit movzbl so the redundant slot round-trip and the (subsequent)
    // zero-extension can be deleted.

    #[test]
    fn test_widen_forward_byte_store_to_l_reload_refused() {
        // A byte store feeding a 32-bit reload must NOT be forwarded: the
        // `movl` reload reads slot bytes 1..3 that the store never wrote
        // (adjacent locals or raw stack garbage — this is real-mode boot
        // code). Rewriting it to `movzbl %al, %eax` would fabricate zero
        // bits the original program never had.
        let asm = "f:\n    inb %dx, %al\n    movb %al, 0(%esp)\n    movl 0(%esp), %eax\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl 0(%esp), %eax"),
            "wider reload must keep reading the slot:\n{result}"
        );
        assert!(
            !result.contains("movzbl %al, %eax"),
            "zero-fabricating widening is unsound and must not appear:\n{result}"
        );
    }

    #[test]
    fn test_widen_forward_word_store_to_l_reload_refused() {
        let asm = "f:\n    movw %cx, %ax\n    movw %ax, 0(%esp)\n    movl 0(%esp), %edx\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl 0(%esp), %edx"),
            "wider reload must keep reading the slot:\n{result}"
        );
        assert!(
            !result.contains("movzwl %ax, %edx"),
            "zero-fabricating widening is unsound and must not appear:\n{result}"
        );
    }

    #[test]
    fn test_same_width_word_forward_preserves_dest_width() {
        // Unit-level contract for the (W,W) forward: the rewrite must keep
        // the reload's 16-bit destination.  `movw 0(%esp), %dx` writes only
        // %dx, so forwarding to `movzwl %ax, %edx` would clobber the high
        // half of %edx.  Driven through forward_slot_loads directly because
        // later pipeline passes deliberately fold under the backend's
        // narrow-write contract (high halves of 16-bit-typed values are
        // don't-care in backend-generated dataflow) and would mask the
        // exact rewrite shape under test.
        let asm = "f:\n    movw %ax, 0(%esp)\n    movw 0(%esp), %dx\n".to_string();
        let mut store = LineStore::new(asm);
        let n = store.len();
        let mut infos: Vec<LineInfo> = (0..n).map(|i| classify_line(store.get(i))).collect();
        let changed = forward_slot_loads(&mut store, &mut infos);
        assert!(changed, "same-width word pair must forward");
        let out: Vec<&str> = (0..store.len())
            .filter(|i| !infos[*i].is_nop())
            .map(|i| store.get(i).trim())
            .collect();
        assert!(
            out.iter().any(|l| *l == "movw %ax, %dx"),
            "word forward must stay 16-bit dest: {out:?}"
        );
    }

    #[test]
    fn test_widen_forward_with_zext_terminator_fuses() {
        // The `inb` idiom IS foldable when a zero-extension of exactly the
        // stored width immediately consumes the 32-bit reload — the bytes
        // above the store are dead on arrival, so the triple collapses:
        //   movb %al,S; movl S,%eax; movzbl %al,%eax  ->  movzbl %al,%eax
        let asm = "f:\n    inb %dx, %al\n    movb %al, 0(%esp)\n    movl 0(%esp), %eax\n    movzbl %al, %eax\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movzbl %al, %eax"),
            "zext-terminated widening must fuse:\n{result}"
        );
        assert!(
            !result.contains("0(%esp)"),
            "slot round-trip must be gone:\n{result}"
        );
    }

    #[test]
    fn test_immediate_forward_same_width_only() {
        // Same-width immediate forward materializes the constant directly.
        let asm = "f:\n    movl $3, 0(%esp)\n    movl 0(%esp), %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl $3, %eax"),
            "same-width immediate must forward:\n{result}"
        );
    }

    #[test]
    fn test_immediate_forward_wider_reload_refused() {
        // A byte-sized immediate store feeding a 32-bit reload must stay a
        // slot read: the upper three bytes are unwritten stack garbage, not
        // zeros.
        let asm = "f:\n    movb $3, 0(%esp)\n    movl 0(%esp), %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl 0(%esp), %eax"),
            "wider reload of a byte store must not materialize:\n{result}"
        );
    }

    #[test]
    fn test_inline_asm_region_is_never_rewritten() {
        // User-authored asm between #APP/#NO_APP must survive byte-for-byte
        // even when its text matches a foldable store/load shape, and it
        // must barrier forwarding windows around it.
        let asm = "f:\n    movl %eax, 0(%esp)\n#APP\n    movl 0(%esp), %ecx\n#NO_APP\n    movl 0(%esp), %edx\n    ret\n"
            .to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("#APP") && result.contains("#NO_APP"),
            "markers must survive:\n{result}"
        );
        let app = result.find("#APP").unwrap();
        let noapp = result.find("#NO_APP").unwrap();
        let region = &result[app..noapp];
        assert!(
            region.contains("movl 0(%esp), %ecx"),
            "asm body must be untouched:\n{result}"
        );
        assert!(
            !region.contains("movl %eax, %ecx"),
            "asm line must not be forwarded:\n{result}"
        );
    }

    #[test]
    fn test_widen_forward_blocked_when_source_clobbered() {
        // A barrier (call) between store and reload clobbers %al, so the
        // stored byte is not provably still in %al: the slot stays.
        let asm =
            "f:\n    movb %al, 0(%esp)\n    call g\n    movl 0(%esp), %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("0(%esp)"),
            "clobbered source must keep the slot reload:\n{result}"
        );
    }
    // ── Tests: the central implicit-operand oracle ───────────────────────

    #[test]
    fn the_widening_branch_writes_edx_and_only_reads_eax() {
        // The heart of the i686 div/rem lowering: `cltd` rewrote %edx
        // while being invisible to any textual scan.
        for m in ["cwd", "cwtd", "cltd", "cdq", "cqo"] {
            assert_eq!(classify_implicit_operands(m), (1 << REG_EAX, 1 << REG_EDX), "{m}");
        }
    }

    #[test]
    fn in_place_extensions_read_and_write_the_accumulator() {
        for m in ["cbw", "cbtw", "cwde", "cwtl"] {
            assert_eq!(classify_implicit_operands(m), (1 << REG_EAX, 1 << REG_EAX), "{m}");
        }
    }

    #[test]
    fn integer_division_names_its_implicit_pair_exactly() {
        let ax_dx = 1 << REG_EAX | 1 << REG_EDX;
        assert_eq!(classify_implicit_operands("idivl %ecx"), (ax_dx, ax_dx));
        assert_eq!(classify_implicit_operands("divl %ecx"), (ax_dx, ax_dx));
        assert_eq!(classify_implicit_operands("divw %cx"), (ax_dx, ax_dx));
        // Byte forms divide/multiply AX: quotient AL, remainder AH,
        // product AX — the %edx family is never touched.
        assert_eq!(classify_implicit_operands("idivb %cl"), (1 << REG_EAX, 1 << REG_EAX));
        assert_eq!(classify_implicit_operands("mulb %cl"), (1 << REG_EAX, 1 << REG_EAX));
        // SSE divides/multiplies have fully explicit operands: a prefix
        // match against "div"/"mul" would tax every FP loop with a
        // phantom accumulator pair.
        assert_eq!(classify_implicit_operands("divsd %xmm0, %xmm1"), (0, 0));
        assert_eq!(classify_implicit_operands("mulsd %xmm0, %xmm1"), (0, 0));
        assert_eq!(classify_implicit_operands("mull %ecx"), (ax_dx, ax_dx));
    }

    #[test]
    fn single_operand_imul_is_the_widening_form() {
        let eax = 1 << REG_EAX;
        let ax_dx = 1 << REG_EAX | 1 << REG_EDX;
        assert_eq!(classify_implicit_operands("imull %ecx"), (eax, ax_dx));
        assert_eq!(classify_implicit_operands("imulw %cx"), (eax, ax_dx));
        // The two/three-operand forms name every register they touch.
        assert_eq!(classify_implicit_operands("imull %ecx, %eax"), (0, 0));
        assert_eq!(classify_implicit_operands("imull $5, %ecx, %eax"), (0, 0));
    }

    #[test]
    fn system_instructions_have_their_contract() {
        let eax = 1 << REG_EAX;
        assert_eq!(classify_implicit_operands("rdtsc").1, eax | 1 << REG_EDX);
        // rdtscp additionally returns the TSC_AUX in %ecx; rdtsc does not.
        assert_eq!(classify_implicit_operands("rdtscp").1, eax | 1 << REG_ECX | 1 << REG_EDX);
        assert_eq!(
            classify_implicit_operands("cpuid").1,
            eax | 1 << REG_EBX | 1 << REG_ECX | 1 << REG_EDX
        );
        assert_eq!(classify_implicit_operands("cpuid").0, eax);
        // ia32 int $0x80: the handler's contract is unknown — fail closed.
        let all_args = 1 << REG_EAX
            | 1 << REG_EBX
            | 1 << REG_ECX
            | 1 << REG_EDX
            | 1 << REG_ESI
            | 1 << REG_EDI
            | 1 << REG_EBP;
        assert_eq!(classify_implicit_operands("int $0x80"), (all_args, all_args));
    }

    #[test]
    fn rep_prefixes_contribute_the_count_register() {
        let ecx_edi = 1 << REG_ECX | 1 << REG_EDI;
        let ecx = 1 << REG_ECX;
        assert_eq!(classify_implicit_operands("rep stosb").0, ecx | 1 << REG_EAX | 1 << REG_EDI);
        assert_eq!(classify_implicit_operands("rep stosb").1, ecx_edi);
        assert_eq!(classify_implicit_operands("repne scasl").1, ecx_edi);
        // `rep ret` / `rep nop` (pause) are branch-hint idioms, not
        // counted loops: they touch no register at all.
        assert_eq!(classify_implicit_operands("rep ret"), (0, 0));
        assert_eq!(classify_implicit_operands("rep nop"), (0, 0));
        // An unrecognized counted form fails closed on every family.
        assert_eq!(classify_implicit_operands("rep frobnicate"), (0xFF, 0xFF));
    }

    #[test]
    fn string_primitives_post_increment_their_pointers() {
        let si_di = 1 << REG_ESI | 1 << REG_EDI;
        // Bare (unrep'd) forms are invisible to a textual scan and were
        // missing from the legacy mentions table entirely.
        assert_eq!(classify_implicit_operands("movsl"), (si_di, si_di));
        assert_eq!(classify_implicit_operands("movsb"), (si_di, si_di));
        assert_eq!(classify_implicit_operands("lodsl").0, 1 << REG_ESI);
        assert_eq!(classify_implicit_operands("lodsl").1, 1 << REG_EAX | 1 << REG_ESI);
        // stos consumes %eax (the stored data) and post-increments %edi;
        // %esi must NOT be blamed for it.
        assert_eq!(classify_implicit_operands("stosl").0, 1 << REG_EAX | 1 << REG_EDI);
        assert_eq!(classify_implicit_operands("stosl").1, 1 << REG_EDI);
    }

    #[test]
    fn cmpxchg_reloads_the_accumulator_on_failure() {
        let eax = 1 << REG_EAX;
        assert_eq!(classify_implicit_operands("cmpxchgl %ebx, (%edi)"), (eax, eax));
        assert_eq!(classify_implicit_operands("lock cmpxchgl %ebx, (%edi)"), (eax, eax));
        // CMPXCHG8B compares edx:eax (reloaded on failure); ecx:ebx are
        // the read-only payload.
        let quad = eax | 1 << REG_EBX | 1 << REG_ECX | 1 << REG_EDX;
        assert_eq!(classify_implicit_operands("lock cmpxchg8b (%edi)").0, quad);
        assert_eq!(
            classify_implicit_operands("lock cmpxchg8b (%edi)").1,
            eax | 1 << REG_EDX
        );
    }

    #[test]
    fn frame_and_stack_instructions_have_their_contract() {
        let esp = 1 << REG_ESP;
        let ebp = 1 << REG_EBP;
        // `leave` copies %ebp into %esp (pure write of %esp — a dead-move
        // proof may rely on it) and pops %ebp (read-modify-write).
        assert_eq!(classify_implicit_operands("leave").0, ebp);
        assert_eq!(classify_implicit_operands("leave").1, esp | ebp);
        assert_eq!(classify_implicit_operands("enter").1, esp | ebp);
        // pusha reads all eight (including %esp) and writes %esp only.
        assert_eq!(classify_implicit_operands("pushad").0, 0xFF);
        assert_eq!(classify_implicit_operands("pushad").1, esp);
        assert_eq!(classify_implicit_operands("popad").1, 0xFF & !esp);
    }

    #[test]
    fn non_instructions_name_nothing() {
        assert_eq!(classify_implicit_operands(".LBB1:"), (0, 0));
        assert_eq!(classify_implicit_operands(".cfi_startproc"), (0, 0));
        assert_eq!(classify_implicit_operands("#APP"), (0, 0));
        assert_eq!(classify_implicit_operands(""), (0, 0));
        assert_eq!(classify_implicit_operands("frobnicate"), (0, 0));
        // lock alone contributes nothing; the prefixed mnemonic decides.
        assert_eq!(classify_implicit_operands("lock"), (0, 0));
    }

    #[test]
    fn the_union_reaches_line_references_reg() {
        // The classified mention set of a bare `cltd` used to be empty —
        // the exact hole behind the div-remainder miscompile class.
        assert!(line_references_reg("    cltd", REG_EDX));
        assert!(line_references_reg("    cltd", REG_EAX));
        assert!(line_references_reg("    idivl %ecx", REG_EDX));
        assert!(line_references_reg("    syscall", REG_ECX));
        assert!(line_references_reg("    rep movsb", REG_ECX));
        assert!(line_references_reg("    movsl", REG_ESI));
        // The Cmp kind funnels through the same scan (cmpxchg lives there).
        assert!(line_references_reg("    cmpxchgl %ebx, (%edi)", REG_EAX));
        // SSE div/mul name their operands and nothing else.
        assert!(!line_references_reg("    divsd %xmm0, %xmm1", REG_EDX));
    }

    #[test]
    fn the_write_half_reaches_line_writes_reg_implicitly() {
        assert!(line_writes_reg_implicitly("cltd", REG_EDX));
        assert!(!line_writes_reg_implicitly("cltd", REG_EAX));
        assert!(line_writes_reg_implicitly("idivl %ecx", REG_EDX));
        // cmpxchg's implicit %eax reload — the write that copy
        // propagation must not rename across.
        assert!(line_writes_reg_implicitly("cmpxchgl %ebx, (%edi)", REG_EAX));
        // XADD's register operand receives the old value — even under lock.
        assert!(line_writes_reg_implicitly("lock xaddl %ebx, (%eax)", REG_EBX));
        assert!(line_writes_reg_implicitly("xaddl %ebx, %eax", REG_EBX));
        // The explicit RMW single-operand forms still count.
        assert!(line_writes_reg_implicitly("negl %eax", REG_EAX));
        assert!(line_writes_reg_implicitly("incl %ecx", REG_ECX));
        // A pure write of another family stays clean.
        assert!(!line_writes_reg_implicitly("stosl", REG_EAX));
        assert!(line_writes_reg_implicitly("stosl", REG_EDI));
    }

    #[test]
    fn the_read_half_reaches_line_implicitly_reads() {
        assert!(line_implicitly_reads("cltd", REG_EAX));
        assert!(!line_implicitly_reads("cltd", REG_EDX));
        assert!(line_implicitly_reads("idivl %ecx", REG_EDX));
        assert!(line_implicitly_reads("rep stosb", REG_EAX));
        assert!(!line_implicitly_reads("stosl", REG_ECX));
        assert!(line_implicitly_reads("loop", REG_ECX));
        assert!(line_implicitly_reads("sahf", REG_EAX));
    }

    #[test]
    fn copy_propagation_does_not_rename_across_cmpxchg_eax_reload() {
        // `cmpxchg` rewrites %eax when the exchange FAILS.  The alias
        // `esi := eax` must not survive it: renaming the %esi reader onto
        // %eax would feed it the pre-exchange accumulator.  (The old
        // write table had no cmpxchg arm — the rename went through.)
        let asm = concat!(
            "f:\n",
            "    movl %eax, %esi\n",
            "    lock cmpxchgl %ebx, (%edi)\n",
            "    movl %esi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %esi, %eax"),
            "cmpxchg's implicit %eax write must break the alias:\n{result}"
        );
    }

    #[test]
    fn copy_propagation_does_not_rename_across_xadd_register_result() {
        // `lock xaddl %ebx, (%eax)` returns the OLD value in %ebx — a
        // register write invisible to the AT&T destination parse.  The
        // %esi reader must keep reading %esi, not the post-add %ebx.
        let asm = concat!(
            "f:\n",
            "    movl %ebx, %esi\n",
            "    lock xaddl %ebx, (%eax)\n",
            "    movl %esi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %esi, %eax"),
            "xadd's implicit register write must break the alias:\n{result}"
        );
    }

    #[test]
    fn copy_propagation_does_not_rename_dividend_across_cltd() {
        // The i686 twin of the x86-64 stress-lab intexpr repro: the alias
        // of the dividend must not survive the sign-extension that
        // overwrites %edx, or the quotient consumer reads the divisor.
        let asm = concat!(
            "f:\n",
            "    movl %edx, %esi\n",
            "    cltd\n",
            "    idivl %ecx\n",
            "    movl %esi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %esi, %eax"),
            "cltd's implicit %edx write must break the %edx alias:\n{result}"
        );
    }

    #[test]
    fn copy_propagation_still_renames_across_byte_division() {
        // POSITIVE precision test: the byte forms divide/multiply AX and
        // never touch %edx — an alias of %edx must survive `divb`.
        let asm = concat!(
            "f:\n",
            "    movl %edx, %esi\n",
            "    divb %cl\n",
            "    movl %esi, %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %edx, %eax"),
            "%edx alias must survive the byte divide:\n{result}"
        );
    }

    #[test]
    fn dead_move_into_edx_is_deleted_before_cltd() {
        // `cltd` is a purely-architectural write of %edx: a move into
        // %edx that it fully shadows is dead.
        let asm = concat!(
            "f:\n",
            "    movl $7, %eax\n",
            "    movl %ebx, %edx\n",
            "    cltd\n",
            "    idivl %ecx\n",
            "    movl %eax, %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            !result.contains("movl %ebx, %edx"),
            "cltd redefines %edx without reading it; the move is dead:\n{result}"
        );
    }

    #[test]
    fn dead_move_into_eax_survives_idivl() {
        // `idivl` READS %eax (the dividend): a move into %eax before it
        // is alive no matter what follows.
        let asm = concat!(
            "f:\n",
            "    movl %ebx, %eax\n",
            "    cltd\n",
            "    idivl %ecx\n",
            "    movl %eax, %ebx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %ebx, %eax"),
            "the dividend is consumed by idivl; the move must stay:\n{result}"
        );
    }

    #[test]
    fn dead_move_into_edi_survives_stosl() {
        // `stosl` post-increments %edi — a read-modify-write of the
        // pointer, not a redefinition.
        let asm = concat!(
            "f:\n",
            "    movl %ebx, %edi\n",
            "    stosl\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("movl %ebx, %edi"),
            "the string pointer is read-modify-write; the move must stay:\n{result}"
        );
    }

    // ── Whole-function GPR liveness oracle (GprLiveness) ────────────────────
    //
    // `fold_memory_operands` used to prove the loaded register dead with a
    // bounded forward scan that stopped at the first barrier and *assumed*
    // deadness there. Every test below contains a fold candidate whose
    // register is live PAST the barrier the old scan stopped at; the fold
    // must be refused. The positive controls at the end prove the oracle
    // still folds genuinely dead loads (exactness, not blanket conservatism).

    #[test]
    fn memfold_load_live_across_conditional_not_folded() {
        // The reproduced `maxi(a,b){return a>b?a:b;}` miscompile on 4927a774:
        // `movl 20(%esp),%esi` was folded into `cmpl 20(%esp),%ebx` and the
        // select's fall-through arm read a never-written %esi.
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl 16(%esp), %ebx\n",
            "    movl 20(%esp), %esi\n",
            "    cmpl %esi, %ebx\n",
            "    jg .Lsel_true_0\n",
            "    movl %esi, %eax\n",
            "    jmp .Lsel_end_0\n",
            ".Lsel_true_0:\n",
            "    movl %ebx, %eax\n",
            ".Lsel_end_0:\n",
            "    addl $8, %esp\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("cmpl 16(%esp), %ebx"),
            "fold must be refused: %esi is read by the fall-through arm:\n{result}"
        );
        assert!(
            result.contains("movl 20(%esp), %esi"),
            "the param home load must survive:\n{result}"
        );
        assert!(
            result.contains("cmpl %esi, %ebx"),
            "the consumer must keep the register operand:\n{result}"
        );
    }

    #[test]
    fn memfold_load_live_at_branch_target_not_folded() {
        // Same class, but the read sits behind the CONDITIONAL JUMP TARGET,
        // not the fall-through: the old scan stopped at the `je` and folded.
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl $0, %ecx\n",
            ".Lloop:\n",
            "    movl 16(%esp), %esi\n",
            "    cmpl %esi, %ebx\n",
            "    je .Ldone\n",
            "    incl %ecx\n",
            "    cmpl $10, %ecx\n",
            "    jl .Lloop\n",
            ".Ldone:\n",
            "    addl %esi, %edi\n",
            "    addl $8, %esp\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("cmpl 16(%esp), %ebx"),
            "fold must be refused: %esi is live at .Ldone:\n{result}"
        );
        assert!(
            result.contains("movl 16(%esp), %esi"),
            "the load must survive:\n{result}"
        );
    }

    #[test]
    fn memfold_load_feeding_regparm_call_not_folded() {
        // A regparm callee may read %eax/%ecx/%edx as incoming arguments. The
        // old scan treated `call` as an opaque barrier and folded, deleting a
        // value the call consumes.
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl 16(%esp), %ecx\n",
            "    cmpl %ecx, %ebx\n",
            "    call g\n",
            "    addl $8, %esp\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("cmpl 16(%esp), %ebx"),
            "fold must be refused: the call may read %ecx as an argument:\n{result}"
        );
        assert!(
            result.contains("movl 16(%esp), %ecx"),
            "the load feeding the call argument must survive:\n{result}"
        );
    }

    #[test]
    fn memfold_load_feeding_ret_not_folded() {
        // `ret` reads %eax (the scalar return value). The old scan stopped
        // there and folded, leaving the return value undefined.
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl 16(%esp), %eax\n",
            "    cmpl %eax, %ebx\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("cmpl 16(%esp), %ebx"),
            "fold must be refused: %eax is observed by the caller at ret:\n{result}"
        );
        assert!(
            result.contains("movl 16(%esp), %eax"),
            "the load feeding the return value must survive:\n{result}"
        );
    }

    #[test]
    fn memfold_load_before_jump_table_dispatch_not_folded() {
        // `jmp *table(,%eax,4)` (emit_switch_jump_table) reads every GPR (the
        // index, and any of them as a tail-call argument register); case
        // labels may read arbitrary register-allocated values. The dispatch
        // must act as a full liveness barrier.
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl 16(%esp), %esi\n",
            "    cmpl %esi, %ebx\n",
            "    jmp *jt(, %eax, 4)\n",
            ".L0:\n",
            "    addl %esi, %edi\n",
            "    addl $8, %esp\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("cmpl 16(%esp), %ebx"),
            "fold must be refused before an indirect dispatch:\n{result}"
        );
        assert!(
            result.contains("movl 16(%esp), %esi"),
            "the load must survive:\n{result}"
        );
    }

    #[test]
    fn memfold_dead_after_both_arms_still_folds() {
        // Positive control: %esi is read by NEITHER arm after the compare, so
        // the whole-function oracle must still fold (precision, not just
        // safety).
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl 16(%esp), %ebx\n",
            "    movl 20(%esp), %esi\n",
            "    cmpl %esi, %ebx\n",
            "    jg .Lsel_true_0\n",
            "    movl %ebx, %eax\n",
            "    jmp .Lsel_end_0\n",
            ".Lsel_true_0:\n",
            "    movl %ebx, %eax\n",
            ".Lsel_end_0:\n",
            "    addl $8, %esp\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            result.contains("cmpl 20(%esp), %ebx"),
            "dead load must fold into the compare:\n{result}"
        );
        assert!(
            !result.contains("movl 20(%esp), %esi"),
            "the dead load must be gone:\n{result}"
        );
    }

    #[test]
    fn memfold_dead_load_before_label_still_folds() {
        // Positive control across a label: the load is dead at the label (the
        // continuation re-materialises %ebx), so folding is exact even though
        // labels ended the old scan — the oracle distinguishes dead-at-label
        // from live-across-label precisely.
        let asm = concat!(
            "f:\n",
            "    subl $8, %esp\n",
            "    movl 20(%esp), %esi\n",
            "    cmpl %esi, %ebx\n",
            ".Lnext:\n",
            "    movl %ebx, %eax\n",
            "    addl $8, %esp\n",
            "    ret\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            result.contains("cmpl 20(%esp), %ebx"),
            "dead load must fold (dead-at-label is provable):\n{result}"
        );
        assert!(
            !result.contains("movl 20(%esp), %esi"),
            "the dead load must be gone:\n{result}"
        );
    }
}

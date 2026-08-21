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
    StoreEbp { reg: RegId, offset: i32, size: MoveSize },
    LoadEbp  { reg: RegId, offset: i32, size: MoveSize },
    Move { dst: RegId, src: RegId },
    SelfMove,
    Label,
    Jmp,
    JmpIndirect,
    CondJmp,
    Call,
    Ret,
    Push { reg: RegId },
    Pop { reg: RegId },
    SetCC { reg: RegId },
    Cmp,
    Directive,
    Other { dest_reg: RegId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveSize {
    L,   // movl (32-bit)
    W,   // movw (16-bit)
    B,   // movb (8-bit)
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
    fn is_nop(self) -> bool { self.kind == LineKind::Nop }
    #[inline]
    fn is_barrier(self) -> bool {
        matches!(self.kind,
            LineKind::Label | LineKind::Call | LineKind::Jmp | LineKind::JmpIndirect |
            LineKind::CondJmp | LineKind::Ret | LineKind::Directive |
            // ESP changes renumber every (%esp) slot: push/pop and explicit
            // %esp arithmetic fence all slot windows now that ESP-relative
            // slots participate in the peepholes. Pushes/pops only appear
            // in prologue/epilogue and around calls, so the cost is nil.
            LineKind::Push { .. } | LineKind::Pop { .. })
            || matches!(self.kind, LineKind::Other { dest_reg } if dest_reg == REG_ESP)
    }
}

#[inline]
fn line_info(kind: LineKind, ts: u16) -> LineInfo {
    LineInfo { kind, trim_start: ts, has_indirect_mem: false, ebp_offset: EBP_OFFSET_NONE }
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
    if !rest.starts_with('%') { return None; }
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
    if !is_ebp && !is_esp { return None; }
    // Reject indexed forms like 4(%esp,%eax,4): only plain offset(base).
    let paren = mem.find('(').unwrap_or(0);
    if mem[paren..].contains(',') { return None; }
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
    } else if let Some(r) = s.strip_prefix("movzbl ") {
        (r, MoveSize::L) // movzbl from stack, dest is 32-bit
    } else if let Some(r) = s.strip_prefix("movzwl ") {
        (r, MoveSize::L)
    } else if let Some(r) = s.strip_prefix("movsbl ") {
        (r, MoveSize::L)
    } else if let Some(r) = s.strip_prefix("movswl ") {
        (r, MoveSize::L)
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
    if offset_str.contains('(') { return None; }
    let after = rest[paren_start + 6..].trim();
    if !after.starts_with(',') { return None; }
    let reg = after[1..].trim();
    if !reg.starts_with('%') { return None; }
    Some((offset_str, reg, size))
}

/// Parse `movl %src, %dst` (register-to-register move).
fn parse_reg_to_reg_move(s: &str) -> Option<(RegId, RegId)> {
    let rest = s.strip_prefix("movl ")?.trim();
    if !rest.starts_with('%') { return None; }
    let comma = rest.find(',')?;
    let src_name = rest[..comma].trim();
    let dst_name = rest[comma + 1..].trim();
    if !dst_name.starts_with('%') { return None; }
    // Must not be memory operands
    if src_name.contains('(') || dst_name.contains('(') { return None; }
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
    if s.is_empty() { return 0; }
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
            // Check if it's (%ebp) or (%esp) - those are stack accesses, not indirect
            if i + 5 < bytes.len() && (&bytes[i + 1..i + 5] == b"%ebp" || &bytes[i + 1..i + 5] == b"%esp") {
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
    if s.contains("(%esp)") { ESP_SLOT_BIAS } else { 0 }
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
    let offset_start = before.rfind(|c: char| !c.is_ascii_digit() && c != '-').map(|p| p + 1).unwrap_or(0);
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

/// Check if a line references a specific register family.
/// This includes both explicit register operands and implicit register uses
/// by instructions like cltd, idivl, rep movsb, etc.
fn line_references_reg(s: &str, reg: RegId) -> bool {
    // Check explicit register operands
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
    for name in names {
        if s.contains(name) { return true; }
    }
    // Check implicit register uses by specific instructions
    if implicit_reg_use(s, reg) { return true; }
    false
}

/// Check if an instruction implicitly uses a register (not mentioned in text).
fn implicit_reg_use(s: &str, reg: RegId) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return false; }
    match bytes[0] {
        b'c' => {
            // cmpxchg8b (without lock prefix): reads/writes eax, edx, ecx, ebx
            if s.starts_with("cmpxchg8b") {
                return reg == REG_EAX || reg == REG_EDX || reg == REG_ECX || reg == REG_EBX;
            }
            // cmpxchg{l,w,b} (without lock prefix): implicitly reads eax
            if s.starts_with("cmpxchg") {
                return reg == REG_EAX;
            }
            // cltd/cdq: reads eax, writes edx
            if s == "cltd" || s == "cdq" {
                return reg == REG_EAX || reg == REG_EDX;
            }
            // cbw/cwde: reads/writes eax
            if s == "cbw" || s == "cwde" || s == "cwtl" {
                return reg == REG_EAX;
            }
        }
        b'i' => {
            // idivl/idivw: implicitly reads edx:eax, writes eax and edx
            if s.starts_with("idivl") || s.starts_with("idivw") || s.starts_with("idivb") {
                return reg == REG_EAX || reg == REG_EDX;
            }
            // imull with 1 operand: reads eax, writes edx:eax
            // imull with 2 or 3 operands has explicit regs
            if s.starts_with("imull ") && !s.contains(',') {
                return reg == REG_EAX || reg == REG_EDX;
            }
        }
        b'd' => {
            // divl/divw: implicitly reads edx:eax, writes eax and edx
            if s.starts_with("divl") || s.starts_with("divw") || s.starts_with("divb") {
                return reg == REG_EAX || reg == REG_EDX;
            }
        }
        b'm' => {
            // mul: reads eax, writes edx:eax
            if s.starts_with("mull ") || s.starts_with("mulw ") || s.starts_with("mulb ") {
                return reg == REG_EAX || reg == REG_EDX;
            }
        }
        b'r' => {
            // rep movsb/movsl: uses esi, edi, ecx
            // rep stosb/stosl: uses edi, ecx, eax
            if s.starts_with("rep") {
                if s.contains("movs") {
                    return reg == REG_ESI || reg == REG_EDI || reg == REG_ECX;
                }
                if s.contains("stos") {
                    return reg == REG_EAX || reg == REG_EDI || reg == REG_ECX;
                }
                if s.contains("scas") || s.contains("cmps") {
                    return reg == REG_ESI || reg == REG_EDI || reg == REG_ECX || reg == REG_EAX;
                }
                // Unknown rep instruction - assume all regs used
                return true;
            }
        }
        b'l' => {
            // lock cmpxchg8b: implicitly reads/writes eax, edx, ecx, ebx
            // cmpxchg8b compares edx:eax with memory, stores ecx:ebx on match
            if s.starts_with("lock cmpxchg8b") {
                return reg == REG_EAX || reg == REG_EDX || reg == REG_ECX || reg == REG_EBX;
            }
            // lock cmpxchg{l,w,b}: implicitly reads eax (compared with memory)
            if s.starts_with("lock cmpxchg") {
                return reg == REG_EAX;
            }
            // loop/loope/loopne: reads ecx
            if s.starts_with("loop") {
                return reg == REG_ECX;
            }
        }
        _ => {}
    }
    false
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
            return line_info(LineKind::Cmp, ts);
        }
    }

    if first == b'r' && s == "ret" {
        return line_info(LineKind::Ret, ts);
    }

    // test instructions
    if first == b't' && bytes.len() >= 5 && bytes[1] == b'e' && bytes[2] == b's' && bytes[3] == b't' {
        return line_info(LineKind::Cmp, ts);
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
    if first == b's' && bytes.len() >= 4 && bytes[1] == b'e' && bytes[2] == b't' && parse_setcc(s).is_some() {
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
    let ebp_off = if has_indirect { EBP_OFFSET_NONE } else { parse_ebp_offset_in_line(s) };
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
    if b.len() < 3 || b[0] != b'j' { return false; }
    // jCC where CC is one of: e, ne, l, le, g, ge, b, be, a, ae, s, ns, o, no, p, np, z, nz
    matches!(&s[1..2], "e" | "a" | "b" | "g" | "l" | "s" | "o" | "p" | "z" | "n")
        && s.contains(' ')
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
    if !s.starts_with('j') { return None; }
    let space = s.find(' ')?;
    let cc = &s[1..space];
    let target = s[space + 1..].trim();
    Some((cc, target))
}

/// Parse setCC instruction → condition code.
fn parse_setcc(s: &str) -> Option<&str> {
    if !s.starts_with("set") { return None; }
    let rest = &s[3..];
    let space = rest.find(' ')?;
    let cc = &rest[..space];
    // Validate it's a real condition code
    match cc {
        "e" | "ne" | "z" | "nz" | "l" | "le" | "g" | "ge" |
        "b" | "be" | "a" | "ae" | "s" | "ns" | "o" | "no" |
        "p" | "np" => Some(cc),
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
        if infos[i].is_nop() { i += 1; continue; }

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
            if dest_reg != REG_NONE && dest_reg <= REG_GP_MAX && dest_reg != REG_ESP && dest_reg != REG_EBP {
                let s = trimmed(store, &infos[i], i);
                let rn = reg32_name(dest_reg);
                // addl $1, %reg → incl %reg
                // SAFETY: incl does NOT set the carry flag (CF), so this
                // conversion is invalid if the next instruction reads CF
                // (e.g., adcl used in 64-bit add-with-carry chains).
                if s.starts_with("addl $1, ") && s.ends_with(rn)
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
                if s.starts_with("subl $1, ") && s.ends_with(rn)
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
                if s.starts_with("addl $-1, ") && s.ends_with(rn)
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
                if s.starts_with("subl $-1, ") && s.ends_with(rn)
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
            if dest_reg != REG_NONE && dest_reg <= REG_GP_MAX && dest_reg != REG_ESP && dest_reg != REG_EBP {
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
        while j < len && infos[j].is_nop() { j += 1; }
        if j >= len { i += 1; continue; }

        // Pattern 2w: immediate-to-scratch feeding a comparison --
        //   movl $IMM, %C ; cmpl %C, %R  ->  cmpl $IMM, %R
        // AT&T semantics match exactly: `cmpl %C, %R` computes R - C and
        // `cmpl $IMM, %R` computes R - IMM. Fires only when %C is dead
        // after the cmp (bounded written-before-read scan). GCC always
        // emits the immediate form; the scratch turnaround costs 5 bytes
        // and a register per comparison.
        if let LineKind::Other { dest_reg: c_reg } = infos[i].kind {
            if c_reg <= REG_GP_MAX && c_reg != REG_ESP && c_reg != REG_EBP
                && infos[j].kind == LineKind::Cmp {
                let s0 = trimmed(store, &infos[i], i).to_string();
                if s0.starts_with("movl $") && !s0.contains('(') {
                    let cn = reg32_name(c_reg);
                    if s0.ends_with(cn) {
                        let imm = s0[5..].split(',').next().unwrap_or("").trim().to_string();
                        let s1 = trimmed(store, &infos[j], j).to_string();
                        let pref = format!("cmpl {}, ", cn);
                        if let Some(rhs) = s1.strip_prefix(&pref) {
                            let rhs = rhs.trim();
                            if rhs.starts_with('%') && !rhs.contains('(')
                                && register_family(rhs) != c_reg {
                                // deadness of %C after the cmp
                                let mut k = j + 1;
                                let mut cnt = 0;
                                let mut dead = false;
                                while k < len && cnt < 24 {
                                    if infos[k].is_nop() { k += 1; continue; }
                                    if infos[k].is_barrier() { break; }
                                    let s2 = trimmed(store, &infos[k], k);
                                    let written = match infos[k].kind {
                                        LineKind::Other { dest_reg } => dest_reg == c_reg,
                                        LineKind::Move { dst, .. } => dst == c_reg,
                                        LineKind::LoadEbp { reg, .. } => reg == c_reg,
                                        LineKind::SetCC { reg } => reg == c_reg,
                                        _ => false,
                                    };
                                    if written && !line_reads_dest_source(s2, c_reg) { dead = true; break; }
                                    if line_references_reg(s2, c_reg) || line_writes_reg_implicitly(s2, c_reg) { break; }
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
        if let LineKind::Move { dst: t_reg, src: s_reg } = infos[i].kind {
            if t_reg <= REG_GP_MAX && s_reg <= REG_GP_MAX && t_reg != s_reg
                && t_reg != REG_ESP && t_reg != REG_EBP {
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
                        let is_cmp_imm = s2.starts_with("cmpl $") && s2.ends_with(rn)
                            && !s2[..s2.len()-rn.len()].contains('%');
                        if is_test_self || is_cmp_imm { Some(3u8) } else { None }
                    }
                    _ => None,
                };
                if let Some(kind2) = consumed {
                    // %T must be dead after j: written before any read.
                    let mut k = j + 1;
                    let mut cnt = 0;
                    let mut dead = false;
                    while k < len && cnt < 24 {
                        if infos[k].is_nop() { k += 1; continue; }
                        if infos[k].is_barrier() { break; }
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
        if let LineKind::LoadEbp { reg: lreg, offset: _loff, size: _lsize } = infos[i].kind {
            if let LineKind::Move { dst: mdst, src: msrc } = infos[j].kind {
                if msrc == lreg && mdst != lreg && mdst <= REG_GP_MAX {
                    // Is lreg written before any read after j?
                    let mut k = j + 1;
                    let mut cnt = 0;
                    let mut dead = false;
                    while k < len && cnt < 24 {
                        if infos[k].is_nop() { k += 1; continue; }
                        if infos[k].is_barrier() { break; }
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
                let is_imm = src_line.starts_with("movl $") && src_line.ends_with("%eax")
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
                    if let LineKind::StoreEbp { reg: sreg, offset, size } = infos[j].kind {
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
                                    if infos[k].is_nop() { k += 1; continue; }
                                    if infos[k].is_barrier() { break; }
                                    let s = trimmed(store, &infos[k], k);
                                    if matches!(infos[k].kind,
                                        LineKind::LoadEbp { offset: o, size: ls, .. }
                                            if ranges_overlap(offset, size.byte_size(), o, ls.byte_size()))
                                        || (infos[k].ebp_offset != EBP_OFFSET_NONE
                                            && ranges_overlap(offset, size.byte_size(), infos[k].ebp_offset, 16))
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
                                        size.mnemonic(), imm_out, disp, base
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
                    if let LineKind::Move { dst: mdst, src: msrc } = infos[j].kind {
                        if msrc == REG_EAX && mdst != REG_EAX && mdst <= REG_GP_MAX {
                            // Same bounded deadness scan as Pattern 2a.
                            let mut k = j + 1;
                            let mut cnt = 0;
                            let mut dead = false;
                            while k < len && cnt < 24 {
                                if infos[k].is_nop() { k += 1; continue; }
                                if infos[k].is_barrier() { break; }
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
                    if let LineKind::Move { dst: mdst, src: msrc } = infos[j].kind {
                        if msrc == REG_EAX && mdst != REG_EAX && mdst <= REG_GP_MAX {
                            let mut k = j + 1;
                            let mut cnt = 0;
                            let mut dead = false;
                            while k < len && cnt < 24 {
                                if infos[k].is_nop() { k += 1; continue; }
                                if infos[k].is_barrier() { break; }
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
        if let LineKind::Move { dst: mdst, src: msrc } = infos[i].kind {
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
                            if infos[k].is_nop() { k += 1; continue; }
                            if infos[k].is_barrier() { break; }
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
        if let LineKind::LoadEbp { reg: lreg, offset: loff, size: MoveSize::L } = infos[i].kind {
            if lreg == REG_EAX {
                // find op line j, then store line at k
                if let LineKind::Other { dest_reg: op_dst } = infos[j].kind {
                    if op_dst == REG_EAX {
                        let op_s = trimmed(store, &infos[j], j).to_string();
                        let is_imm_alu = (op_s.starts_with("addl $") || op_s.starts_with("subl $")
                            || op_s.starts_with("andl $") || op_s.starts_with("orl $")
                            || op_s.starts_with("xorl $") || op_s.starts_with("shll $")
                            || op_s.starts_with("sarl $") || op_s.starts_with("shrl $"))
                            && op_s.ends_with(", %eax");
                        let is_incdec = op_s == "incl %eax" || op_s == "decl %eax";
                        if is_imm_alu || is_incdec {
                            let mut k = j + 1;
                            while k < len && infos[k].is_nop() { k += 1; }
                            if k < len {
                                if let LineKind::StoreEbp { reg: sreg, offset: soff, size: MoveSize::L } = infos[k].kind {
                                    if sreg == REG_EAX && soff == loff {
                                        // %eax deadness after the store.
                                        let mut m = k + 1;
                                        let mut cnt = 0;
                                        let mut dead = false;
                                        while m < len && cnt < 24 {
                                            if infos[m].is_nop() { m += 1; continue; }
                                            if infos[m].is_barrier() { break; }
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
                                            let mem = load_s.strip_prefix("movl ")
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
        if let LineKind::StoreEbp { reg: store_reg, offset: store_off, size: store_size } = infos[i].kind {
            if let LineKind::LoadEbp { reg: load_reg, offset: load_off, size: load_size } = infos[j].kind {
                if store_off == load_off && store_size == load_size {
                    if store_reg == load_reg {
                        // movl %eax, -8(%ebp); movl -8(%ebp), %eax → keep store only
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                        i += 1;
                        continue;
                    } else {
                        // movl %eax, -8(%ebp); movl -8(%ebp), %ecx → movl %eax, -8(%ebp); movl %eax, %ecx
                        let new_line = format!("    {} {}, {}", store_size.mnemonic(), reg32_name(store_reg), reg32_name(load_reg));
                        store.replace(j, new_line);
                        infos[j] = LineInfo {
                            kind: LineKind::Move { dst: load_reg, src: store_reg },
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
            while k < len && infos[k].is_nop() { k += 1; }
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
        if let LineKind::Move { dst: dst1, src: src1 } = infos[i].kind {
            if let LineKind::Move { dst: dst2, src: src2 } = infos[j].kind {
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
        if infos[i].is_nop() { continue; }
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
        if infos[i].is_nop() { continue; }

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
            LineKind::LoadEbp { reg: load_reg, offset, size: load_size } => {
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
                            let new_line = format!("    {} {}, {}", load_size.mnemonic(), reg32_name(stored_reg), reg32_name(load_reg));
                            store.replace(i, new_line);
                            infos[i] = LineInfo {
                                kind: LineKind::Move { dst: load_reg, src: stored_reg },
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
                if forwarded { continue; }
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
                if s.contains(';') || s.starts_with("rdmsr") || s.starts_with("cpuid")
                    || s.starts_with("syscall") || s.starts_with("int ") || s.starts_with("int$")
                    || s.starts_with("rep") || s.starts_with("cld") {
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
        if infos[i].is_nop() { continue; }
        let LineKind::StoreEbp { reg: src_reg, offset: slot, size } = infos[i].kind else { continue; };
        if src_reg > REG_GP_MAX { continue; }
        let store_bytes = size.byte_size();

        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() { j += 1; continue; }
            if infos[j].is_barrier() { break; }

            match infos[j].kind {
                LineKind::LoadEbp { reg: dst_reg, offset, size: lsize }
                    if offset == slot && lsize == size =>
                {
                    if dst_reg == src_reg {
                        // Exact reload into the same register: pure no-op.
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                        j += 1;
                        count += 1;
                        continue;
                    } else if dst_reg <= REG_GP_MAX {
                        let text = format!("    movl {}, {}", reg32_name(src_reg), reg32_name(dst_reg));
                        store.replace(j, text);
                        infos[j] = classify_line(store.get(j));
                        changed = true;
                        j += 1;
                        count += 1;
                        continue;
                    } else {
                        break;
                    }
                }
                LineKind::LoadEbp { offset, size: lsize, .. }
                    if ranges_overlap(slot, store_bytes, offset, lsize.byte_size()) =>
                {
                    // Partial-overlap read: store live, no forwarding.
                    break;
                }
                LineKind::StoreEbp { offset, size: ssize, .. }
                    if ranges_overlap(slot, store_bytes, offset, ssize.byte_size()) => break,
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
            if infos[j].has_indirect_mem { break; }
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
                if line_writes_reg_implicitly(s, src_reg) { break; }
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
        return false;
    }
    // Everything else: conservatively a read.
    line_references_reg(s, reg)
}

fn is_implicit_string_op(s: &str) -> bool {
    let mut words = s.split_ascii_whitespace();
    let mut mnemonic = words.next().unwrap_or("");
    if matches!(mnemonic, "rep" | "repe" | "repz" | "repne" | "repnz") {
        mnemonic = words.next().unwrap_or("");
    }
    matches!(
        mnemonic,
        "movsb" | "movsw" | "movsl" | "movsq"
            | "stosb" | "stosw" | "stosl" | "stosq"
            | "lodsb" | "lodsw" | "lodsl" | "lodsq"
            | "scasb" | "scasw" | "scasl" | "scasq"
            | "cmpsb" | "cmpsw" | "cmpsl" | "cmpsq"
    )
}

fn line_writes_reg_implicitly(s: &str, reg: RegId) -> bool {
    let mn = s.split_whitespace().next().unwrap_or("");
    match mn {
        "mull" | "imull" if !s.contains(',') => reg == REG_EAX || reg == REG_EDX,
        "divl" | "idivl" => reg == REG_EAX || reg == REG_EDX,
        "cltd" | "cdq" => reg == REG_EDX,
        "xchgl" | "xchgw" | "xchgb" => line_references_reg(s, reg),
        // Single-operand read-modify-write forms (`negl %eax`, `incl %ecx`,
        // `bswapl %eax`): classify_line derives the destination from the
        // operand AFTER the last comma — these have none, so they classify
        // as Other{dest_reg: REG_NONE} and their overwrite was invisible.
        // Missing them let `movl %edi,%eax`-aliases survive `negl %eax` and
        // rewrite later %eax readers to the un-negated %edi (alias-fuzz
        // scenario 7, struct-copy loops): lccc computed 0x10 where gcc
        // computed 0x04.
        "negl" | "negw" | "negb"
        | "notl" | "notw" | "notb"
        | "incl" | "incw" | "incb"
        | "decl" | "decw" | "decb"
        | "bswapl" | "bswapw" => {
            let rest = s[mn.len()..].trim();
            !rest.contains(',') && rest.starts_with('%') && register_family(rest) == reg
        }
        _ if is_implicit_string_op(s) => {
            matches!(reg, REG_ECX | REG_ESI | REG_EDI | REG_EAX)
        }
        _ => false,
    }
}

fn eliminate_dead_stores(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    const WINDOW: usize = 16;

    for i in 0..len {
        if infos[i].is_nop() { continue; }
        if let LineKind::StoreEbp { offset: store_off, size: store_size, .. } = infos[i].kind {
            // Look ahead for another store to the same slot (meaning this one is dead)
            // or a load from the same slot (meaning this one is alive)
            let mut j = i + 1;
            let mut count = 0;
            while j < len && count < WINDOW {
                if infos[j].is_nop() { j += 1; continue; }

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
                if infos[j].is_barrier() { break; }
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
                if infos[j].has_indirect_mem { break; }
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
                if s.contains("(%ebp)") && !matches!(infos[j].kind, LineKind::StoreEbp { .. } | LineKind::LoadEbp { .. }) {
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
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    parts.push(cur);

    // A single operand is read-modify-write or a branch target: not a source.
    if parts.len() < 2 {
        return None;
    }

    let last = parts.len() - 1;
    let mut hit = false;
    for (idx, part) in parts.iter_mut().enumerate() {
        let is_dest = idx == last;
        // The destination may still be a MEMORY reference, whose registers are
        // read. A bare register destination must not be touched.
        let dest_is_memory = is_dest && part.contains('(');
        if is_dest && !dest_is_memory {
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
        if dst_reg == REG_ESP || dst_reg == REG_EBP
            || src_reg == REG_ESP || src_reg == REG_EBP
        {
            continue;
        }
        if dst_reg == src_reg {
            continue;
        }

        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() {
                j += 1;
                continue;
            }
            // No CFG here: anything that can transfer control ends the region
            // in which the alias is known to hold.
            if infos[j].is_barrier() {
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
                    if let Some(new_line) =
                        replace_att_source_reg(store.get(j), dst_name, src_name)
                    {
                        store.replace(j, new_line);
                        let re = classify_line(store.get(j));
                        infos[j] = LineInfo { trim_start: infos[j].trim_start, ..re };
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
                let dst_name = reg32_name(dst_reg);
                let src_name = reg32_name(src_reg);
                if let Some(new_line) = replace_att_source_reg(store.get(j), dst_name, src_name) {
                    store.replace(j, new_line);
                    let re = classify_line(store.get(j));
                    infos[j] = LineInfo { trim_start: infos[j].trim_start, ..re };
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
    let bit = |reg: RegId| -> u16 { if reg <= REG_GP_MAX { 1 << reg } else { 0 } };
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
    // delete a live dividend or string pointer.  Keep this table paired with
    // `line_writes_reg_implicitly`, which protects the linear passes.
    let mnemonic = line.split_ascii_whitespace().next().unwrap_or("");
    match mnemonic {
        "mull" | "imull" if !line.contains(',') => {
            uses |= bit(REG_EAX);
            defs |= bit(REG_EAX) | bit(REG_EDX);
        }
        "divl" | "idivl" => {
            uses |= bit(REG_EAX) | bit(REG_EDX);
            defs |= bit(REG_EAX) | bit(REG_EDX);
        }
        "cltd" | "cdq" => {
            uses |= bit(REG_EAX);
            defs |= bit(REG_EDX);
        }
        "xchgl" | "xchgw" | "xchgb" => {
            for reg in 0..=REG_GP_MAX {
                if line_references_reg(line, reg) {
                    uses |= bit(reg);
                    defs |= bit(reg);
                }
            }
        }
        _ if is_implicit_string_op(line) => {
            // The exact subset depends on the string opcode and prefixes;
            // keeping all architectural string registers live is cheap and
            // conservative for the tiny i686 functions handled here.
            for reg in [REG_EAX, REG_ECX, REG_ESI, REG_EDI] {
                uses |= bit(reg);
                defs |= bit(reg);
            }
        }
        _ => {}
    }
    (uses, defs)
}

fn direct_jump_target(line: &str) -> Option<&str> {
    let target = line.split_ascii_whitespace().last()?;
    if target.starts_with('.') || target.bytes().next().is_some_and(|b| b.is_ascii_alphabetic()) {
        Some(target.trim_end_matches(','))
    } else {
        None
    }
}

/// Use whole-function GPR liveness to remove dead moves and retarget safe
/// accumulator update/copy-back chains.  The earlier bounded scans intentionally
/// stop at branches, so they cannot reason about loop backedges or both sides of
/// a conditional.  This tiny eight-register backward dataflow can.
fn optimize_cfg_register_liveness(
    store: &mut LineStore, infos: &mut [LineInfo], consume_return_marker: bool,
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
            !infos[i].is_nop()
                && trimmed(store, &infos[i], i) == "# lccc-i686-return-uses-edx"
        });
        let mut labels = std::collections::HashMap::<&str, usize>::new();
        for i in start..end {
            if infos[i].kind == LineKind::Label {
                labels.insert(trimmed(store, &infos[i], i).trim_end_matches(':'), i - start);
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
                    LineKind::Ret | LineKind::JmpIndirect => {}
                    LineKind::Jmp => {
                        if let Some(target) = target {
                            out |= live_in[target];
                        } else {
                            // External direct jumps are tail calls.  Their ABI
                            // arguments may occupy any regparm register.
                            out = (1u16 << (REG_GP_MAX + 1)) - 1;
                        }
                    }
                    LineKind::CondJmp => {
                        if let Some(target) = target {
                            out |= live_in[target];
                        } else {
                            out = (1u16 << (REG_GP_MAX + 1)) - 1;
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
                    "incl" | "decl" | "negl" | "notl" | "addl" | "subl"
                        | "andl" | "orl" | "xorl" | "shll" | "shrl" | "sarl" | "sall"
                ) || (mnemonic == "imull" && line.contains(',')));
            if copy_back
                && safe_update
                && live_out[last_local] & (1u16 << REG_EAX) == 0
            {
                // EAX may also occur explicitly in a source operand, e.g.
                // `addl %eax,%eax` or `addl (%eax),%eax`.  It held `reg` at
                // this point, so every whole-register occurrence must follow
                // the retargeted value after the setup copy is removed.
                let replacement = store.get(mid).replace("%eax", reg32_name(reg));
                store.replace(mid, replacement);
                let re = classify_line(store.get(mid));
                infos[mid] = LineInfo { trim_start: infos[mid].trim_start, ..re };
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
                    matches!(mn, "movl" | "leal" | "movzbl" | "movzwl" | "movsbl" | "movswl")
                        && s.ends_with(", %eax")
                }
                _ => false,
            };
            if !retarget_ok {
                local += 1;
                continue;
            }
            let s = trimmed(store, &infos[i], i);
            let Some(src_part) = s.strip_suffix(", %eax") else { local += 1; continue; };
            if src_part.contains("%eax") || src_part.contains("%ax")
                || src_part.contains("%al") || src_part.contains("%ah")
            {
                local += 1;
                continue;
            }
            let Some(j_local) = (local + 1..n).find(|&p| !infos[start + p].is_nop()) else {
                break;
            };
            let j = start + j_local;
            let LineKind::Move { dst, src: REG_EAX } = infos[j].kind else { local += 1; continue; };
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
            infos[i] = LineInfo { trim_start: infos[i].trim_start, ..re };
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
            let LineKind::StoreEbp { reg: REG_EAX, offset, size: MoveSize::L } = infos[j].kind
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
            store.replace(j, format!("    movl {}, {}({})", reg32_name(src), disp, base));
            let re = classify_line(store.get(j));
            infos[j] = LineInfo { trim_start: infos[j].trim_start, ..re };
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
            let LineKind::LoadEbp { reg, offset, size: MoveSize::L } = infos[i].kind else {
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
            let Some(imm) = imm else { local += 1; continue; };
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
            infos[j] = LineInfo { trim_start: infos[j].trim_start, ..re };
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
        if infos[k].is_nop() {
            return false;
        }
        let l = trimmed(store, &infos[k], k);
        l.contains("(%ebp)") || l == "movl %esp, %ebp" || l.starts_with("movl %esp, %ebp")
    });

    for i in 0..len {
        if infos[i].is_nop() { continue; }
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
        if dst_reg == REG_ESP { continue; }
        if dst_reg == REG_EBP && frame_pointer_in_use { continue; }

        // Look ahead: if dst is overwritten before being read, this move is dead
        let mut j = i + 1;
        let mut count = 0;
        while j < len && count < WINDOW {
            if infos[j].is_nop() { j += 1; continue; }
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
            match infos[j].kind {
                LineKind::StoreEbp { reg, .. } if reg == dst_reg => {
                    // dst is read (stored to stack) - move is alive
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
                    break;
                }
                LineKind::Move { src, dst } => {
                    if src == dst_reg {
                        // dst is read - move is alive
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
                        break; // dst is read
                    }
                }
                _ => {
                    if line_references_reg(s, dst_reg) {
                        break; // dst is read
                    }
                }
            }

            j += 1;
            count += 1;
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
    infos: &[LineInfo], start_idx: usize, len: usize, out: &mut [usize; N],
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
        let rest_count = collect_non_nop_indices::<{ CMP_FUSION_LOOKAHEAD - 1 }>(infos, i, len, &mut rest);
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
        let mut store_offsets: [i32; MAX_TRACKED_STORE_LOAD_OFFSETS] = [0; MAX_TRACKED_STORE_LOAD_OFFSETS];
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
            None => { i += 1; continue; }
        };

        // If there are stores in the sequence, verify each has a matching load nearby.
        if store_count == usize::MAX {
            i += 1;
            continue;
        }
        if store_count > 0 {
            let range_start = seq_indices[1];
            let range_end = seq_indices[test_scan];
            let mut load_offsets: [i32; MAX_TRACKED_STORE_LOAD_OFFSETS] = [0; MAX_TRACKED_STORE_LOAD_OFFSETS];
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
            let has_unmatched_store = (0..store_count).any(|si| {
                !(0..load_count).any(|li| load_offsets[li] == store_offsets[si])
            });
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
        let jmp_line = trimmed(store, &infos[seq_indices[test_scan + 1]], seq_indices[test_scan + 1]);
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
                None => { i += 1; continue; }
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
                    let lhs = rest
                        .trim_start()
                        .split(',')
                        .next()
                        .unwrap_or("")
                        .trim();
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
        const GPR32: &[&str] = &[
            "%eax", "%ebx", "%ecx", "%edx", "%esi", "%edi", "%ebp",
        ];
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
            Some(s)
                if (s.ends_with("(%esp)") || s.ends_with("(%ebp)")) && !s.starts_with('(') =>
            {
                s
            }
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
        let store_slot = store_line.strip_prefix(store_prefix.as_str()).map(str::trim);
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

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() { i += 1; continue; }

        // Look for load from stack slot
        if let LineKind::LoadEbp { reg: load_reg, offset, size } = infos[i].kind {
            // Only fold scratch registers (eax, ecx, edx)
            if !is_caller_saved(load_reg) && load_reg != REG_EAX {
                i += 1; continue;
            }
            // Only full-width loads: movzbl/movsbl/movzwl/movswl read fewer
            // bytes than the folded memory operand would.
            if !is_full_width_load(trimmed(store, &infos[i], i)) {
                i += 1; continue;
            }

            // Find next non-nop instruction
            let j = next_non_nop(infos, i + 1);
            if j >= len { i += 1; continue; }

            // Check if next instruction uses this register as a source operand
            // Pattern: load into %ecx, then `addl %ecx, %eax` etc.
            let s = trimmed(store, &infos[j], j);
            if let Some(folded) = try_fold_memory_operand(s, load_reg, offset, size) {
                // Soundness: the fold deletes the load, so the loaded register
                // must be DEAD after the folded consumer. Copy propagation can
                // extend a slot load's register across several consumers
                // (`movl SLOT,%eax; movl %eax,%edx; cmpl %edx,%edi; subl
                // %edx,%ebp` -> `...; cmpl %eax,%edi; subl %eax,%ebp`), so a
                // later read of load_reg would see a stale value once the load
                // is gone. Scan forward: a read of load_reg (explicit or
                // implicit) before the next pure write to it is unsafe. A
                // label/call/ret invalidates the accumulator cache (the emitter
                // reloads afterwards), so the loaded value is provably dead
                // there — but a CONDITIONAL jump does NOT invalidate: its
                // fallthrough (e.g. a select arm) may still read the register.
                let mut safe = true;
                let mut k = j + 1;
                while k < len {
                    if infos[k].is_nop() { k += 1; continue; }
                    let lk = trimmed(store, &infos[k], k);
                    if infos[k].is_barrier() { break; }
                    // A pure write to load_reg ends its liveness. `movl $imm,%reg`
                    // and `leal ADDR,%reg` write without reading %reg; a generic
                    // read-modify-write (addl/subl/…) READS its destination and
                    // is correctly treated as a read below.
                    let pure_write = match infos[k].kind {
                        LineKind::Move { dst, src } => dst == load_reg && src != load_reg,
                        LineKind::LoadEbp { reg, .. } => reg == load_reg,
                        LineKind::Pop { reg } => reg == load_reg,
                        LineKind::SetCC { reg } => reg == load_reg,
                        LineKind::Other { dest_reg } => {
                            dest_reg == load_reg && (lk.starts_with("movl $") || lk.starts_with("leal "))
                        }
                        _ => false,
                    } || (line_writes_reg_implicitly(lk, load_reg)
                        && !line_references_reg(lk, load_reg));
                    if pure_write { break; }
                    if line_references_reg(lk, load_reg) {
                        safe = false;
                        break;
                    }
                    k += 1;
                }
                if !safe { i += 1; continue; }

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
fn try_fold_memory_operand(s: &str, load_reg: RegId, offset: i32, _size: MoveSize) -> Option<String> {
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
    for op in &["addl", "subl", "andl", "orl", "xorl", "cmpl", "testl", "imull"] {
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
        let val = if num.is_empty() { 0 } else { num.parse::<i32>().unwrap_or(i32::MIN) };
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
        if !is_boundary { continue; }
        let fend = idx;

        // Phase 1: collect candidate triples (load_i, store_j, call_k, sym, disp).
        struct Triple { li: usize, sj: usize, ck: usize, sym: String, disp: i32 }
        let mut triples: Vec<Triple> = Vec::new();
        let mut i = fstart;
        'scan: while i < fend {
            if infos[i].is_nop() { i += 1; continue; }

            // Line i: `movl SYM, %reg` (classified Other{dest_reg}: symbolic
            // source, so it is neither Move nor LoadEbp).
            let LineKind::Other { dest_reg } = infos[i].kind else { i += 1; continue; };
            if dest_reg > REG_GP_MAX { i += 1; continue; }
            let li = trimmed(store, &infos[i], i);
            let Some(rest) = li.strip_prefix("movl ") else { i += 1; continue; };
            let Some((sym, dst)) = rest.split_once(',') else { i += 1; continue; };
            let sym = sym.trim();
            let dst = dst.trim();
            if register_family(dst) != dest_reg { i += 1; continue; }
            let sb = sym.as_bytes();
            if sb.is_empty() || !(sb[0].is_ascii_alphabetic() || sb[0] == b'_') { i += 1; continue; }
            if !sym
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'$' | b'+' | b'-'))
            {
                i += 1;
                continue;
            }

            // Line j: `movl %reg, N(%esp)`.
            let j = next_non_nop(infos, i + 1);
            if j >= fend { break; }
            let LineKind::StoreEbp { reg: sreg, offset, size: MoveSize::L } = infos[j].kind else {
                i += 1;
                continue;
            };
            if sreg != dest_reg || offset < ESP_SLOT_BIAS / 2 { i += 1; continue; }
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
                if infos[k].is_nop() { k += 1; continue; }
                let s = trimmed(store, &infos[k], k);
                if s == call_txt {
                    found = Some(k);
                    break;
                }
                let mn = s.split_ascii_whitespace().next().unwrap_or("");
                const MOVS: &[&str] = &["movl", "movzbl", "movzwl", "movsbl", "movswl"];
                const ALUS: &[&str] = &[
                    "addl", "subl", "andl", "orl", "xorl", "imull",
                    "shll", "shrl", "sarl", "incl", "decl", "negl", "notl",
                ];
                let is_mov = MOVS.contains(&mn);
                let is_alu = ALUS.contains(&mn);
                let ok = (is_mov || is_alu) && {
                    let ops: Vec<&str> = s[mn.len()..].split(',').map(str::trim).collect();
                    ops.iter().all(|op| op.starts_with('%') || op.starts_with('$')) && {
                        let n = ops.len();
                        let reads_reg = ops.iter().enumerate().any(|(oi, op)| {
                            let is_dest = oi + 1 == n;
                            if is_dest && is_mov { return false; }
                            op.starts_with('%') && register_family(op) == dest_reg
                        });
                        !reads_reg
                    }
                };
                if !ok { i += 1; continue 'scan; }
                k += 1;
                cnt += 1;
            }
            let Some(ci) = found else { i += 1; continue; };
            triples.push(Triple { li: i, sj: j, ck: ci, sym: sym.to_string(), disp: raw_disp });
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
                            && (d - 3..=d + 3).any(|dd| {
                                references_esp_slot(trimmed(store, &infos[t], t), dd)
                            })
                    });
                    if !foreign {
                        owned.push(d);
                    }
                }
            }

            let linear_seal = |m: &Triple| -> bool {
                let mut k = m.ck + 1;
                while k < fend {
                    if infos[k].is_nop() { k += 1; continue; }
                    let s = trimmed(store, &infos[k], k);
                    match infos[k].kind {
                        LineKind::Empty | LineKind::Directive => { k += 1; continue; }
                        LineKind::Label | LineKind::Jmp | LineKind::JmpIndirect
                        | LineKind::CondJmp | LineKind::Ret
                        | LineKind::Push { .. } | LineKind::Pop { .. } => return false,
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
                if !fold { continue; }
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
        if infos[i].is_nop() { continue; }
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
        if infos[i].is_nop() { continue; }
        let s = trimmed(store, &infos[i], i);
        norm[i - start] = sp_down as i32;
        match infos[i].kind {
            LineKind::Push { .. } => { norm[i - start] = sp_down as i32; sp_down += 4; }
            LineKind::Pop { .. } => { any_esp_read = true; sp_down -= 4; norm[i - start] = sp_down as i32; }
            LineKind::Call => { has_call = true; }
            LineKind::Label | LineKind::Jmp | LineKind::CondJmp => {
                if let Some(base) = sp_base {
                    if sp_down != base { esp_walk_ok = false; }
                } else if sp_down != 0 {
                    // control flow before the frame allocation: give up
                    esp_walk_ok = false;
                }
            }
            _ => {
                if let Some(rest) = s.strip_prefix("subl $") {
                    if let Some(n) = rest.strip_suffix(", %esp").and_then(|v| v.parse::<i64>().ok()) {
                        sp_down += n;
                        if sp_base.is_none() { sp_base = Some(sp_down); }
                        continue;
                    }
                }
                if let Some(rest) = s.strip_prefix("addl $") {
                    if let Some(n) = rest.strip_suffix(", %esp").and_then(|v| v.parse::<i64>().ok()) {
                        sp_down -= n;
                        continue;
                    }
                }
                // Any other ESP write invalidates the walk.
                if matches!(infos[i].kind, LineKind::Other { dest_reg } if dest_reg == REG_ESP)
                    || (s.contains("%esp") && !s.contains("(%esp)")) {
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
    let ebp_is_frame = (start..end).any(|i| {
        !infos[i].is_nop() && trimmed(store, &infos[i], i) == "movl %esp, %ebp"
    });

    // Collect all loaded byte ranges in normalized coordinates.
    let mut read_ranges: Vec<(i32, i32)> = Vec::new();
    let mut addr_taken = false;

    for i in start..end {
        if infos[i].is_nop() { continue; }
        let normalize = |off: i32| if is_esp_biased(off) { off - norm[i - start] } else { off };
        match infos[i].kind {
            LineKind::LoadEbp { offset, size, .. } => {
                if is_esp_biased(offset) { any_esp_read = true; }
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
                if s.contains("%esp") && !s.contains("(%esp)")
                    && !s.starts_with("subl $") && !s.starts_with("addl $")
                    && s != "movl %esp, %ebp" && s != "movl %ebp, %esp"
                    && !matches!(infos[i].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                {
                    addr_taken = true;
                }
                // Raw %ebp value as data — an escape only in FP mode.
                if ebp_is_frame
                    && s.contains("%ebp") && !s.contains("(%ebp)")
                    && s != "movl %esp, %ebp" && s != "movl %ebp, %esp"
                    && !matches!(infos[i].kind, LineKind::Push { .. } | LineKind::Pop { .. })
                {
                    addr_taken = true;
                }
                // Track %ebp/%esp-relative reads from non-Load/Store lines
                // (e.g. folded memory operands like "cmpl -44(%ebp), %eax")
                let ebp_off = infos[i].ebp_offset;
                if ebp_off != EBP_OFFSET_NONE {
                    if is_esp_biased(ebp_off) { any_esp_read = true; }
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
                    && (s.contains("(%ebp)") || s.contains("(%esp)")) {
                    // Unknown frame reference - bail out
                    addr_taken = true;
                }
            }
        }
    }

    if addr_taken { return; }

    // Remove stores to slots whose byte range is never overlapped by any load
    for i in start..end {
        if infos[i].is_nop() { continue; }
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
                if !esp_walk_ok { continue; }
                if has_call && any_esp_read { continue; } // shift-immune rule only
            } else if !ebp_is_frame {
                // ebp-domain "store" without a frame pointer = write through
                // the %ebp DATA register (pointer store). Never a dead slot.
                continue;
            }
            let store_bytes = size.byte_size();
            let norm_off = if esp_store { offset - norm[i - start] } else { offset };
            let is_read = read_ranges.iter().any(|&(r_off, r_sz)| {
                ranges_overlap(norm_off, store_bytes, r_off, r_sz)
            });
            if !is_read {
                infos[i].kind = LineKind::Nop;
            }
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
                if infos[i].is_nop() { i += 1; continue; }
                match infos[i].kind {
                    LineKind::Push { reg } if reg <= REG_GP_MAX => {
                        prologue_pushes.push((i, reg));
                        i += 1;
                    }
                    LineKind::Directive | LineKind::Empty => { i += 1; }
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
            if infos[i].is_nop() { continue; }
            let line = trimmed(store, &infos[i], i);
            if let Some(v) = parse_esp_adjust(line, "subl") { subls.push((i, v)); }
            if let Some(v) = parse_esp_adjust(line, "addl") { addls.push((i, v)); }
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
        enum Shape { Framed, Frameless }
        let shape = if subls.len() == 1
            && !addls.is_empty()
            && addls.iter().all(|&(_, v)| v == subls[0].1)
            && addls.iter().all(|&(i, _)| i > subls[0].0)
        {
            Shape::Framed
        } else if subls.is_empty() && addls.is_empty() {
            let any_esp_slot = (start..end).any(|i| {
                !infos[i].is_nop() && trimmed(store, &infos[i], i).contains("(%esp)")
            });
            if any_esp_slot {
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
                        if infos[k].is_nop() { continue; }
                        match infos[k].kind {
                            LineKind::Pop { .. } | LineKind::Directive | LineKind::Empty => continue,
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
            if !matches!(reg, REG_EBX | REG_ESI | REG_EDI | REG_EBP) { continue; }
            if prologue_pushes.iter().filter(|&&(_, r)| r == reg).count() != 1 { continue; }
            let pops: Vec<usize> = (start..end)
                .filter(|&i| !infos[i].is_nop()
                    && matches!(infos[i].kind, LineKind::Pop { reg: r } if r == reg))
                .collect();
            if pops.is_empty() { continue; }
            // Every epilogue must restore the register exactly once.
            let expected = match shape {
                Shape::Framed => addls.len(),
                Shape::Frameless => pops.len(), // all pops must qualify below
            };
            if pops.len() != expected { continue; }
            if !pops.iter().all(|&i| is_epilogue_pop(i)) { continue; }
            // The register must be unreferenced outside its push/pop lines.
            let used = (start..end).any(|i| {
                if infos[i].is_nop() || i == push_idx || pops.contains(&i) {
                    return false;
                }
                line_references_reg(trimmed(store, &infos[i], i), reg)
            });
            if used { continue; }
            removals.push((push_idx, pops));
        }

        if !removals.is_empty() {
            let bytes = 4 * removals.len() as u32;
            if let Shape::Framed = shape {
                let (alloc_idx, alloc_value) = subls[0];
                store.replace(alloc_idx, format!("    subl ${}, %esp", alloc_value + bytes));
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
#[allow(dead_code)]
fn eliminate_push_pop_pairs(store: &LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    for i in 0..len {
        if infos[i].is_nop() { continue; }
        let push_reg = match infos[i].kind {
            LineKind::Push { reg } if reg <= REG_GP_MAX => reg,
            _ => continue,
        };

        // Scan forward for matching pop, checking reg is unmodified
        let mut j = i + 1;
        let mut depth = 0; // Track nested push/pops
        let mut safe = true;
        while j < len {
            if infos[j].is_nop() { j += 1; continue; }

            match infos[j].kind {
                LineKind::Push { .. } => { depth += 1; }
                LineKind::Pop { reg } if depth > 0 => { depth -= 1; }
                LineKind::Pop { reg } if reg == push_reg && depth == 0 => {
                    // Found matching pop
                    if safe {
                        infos[i].kind = LineKind::Nop;
                        infos[j].kind = LineKind::Nop;
                        changed = true;
                    }
                    break;
                }
                LineKind::Pop { .. } if depth == 0 => { break; } // Different register popped
                _ => {}
            }

            // Check if the register is modified
            match infos[j].kind {
                LineKind::Move { dst, .. } if dst == push_reg => { safe = false; }
                LineKind::LoadEbp { reg, .. } if reg == push_reg => { safe = false; }
                LineKind::Other { dest_reg } if dest_reg == push_reg => { safe = false; }
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
                LineKind::Call => { if is_caller_saved(push_reg) { safe = false; } }
                LineKind::SetCC { reg } if reg == push_reg => { safe = false; }
                _ => {}
            }

            // Stop at barriers that change control flow
            if matches!(infos[j].kind, LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret | LineKind::Label) {
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

// ── Main entry point ─────────────────────────────────────────────────────────

/// Run peephole optimization on i686 assembly text.
/// Returns the optimized assembly string.
pub fn peephole_optimize(asm: String) -> String {
    if std::env::var_os("CCC_NO_I686_PEEPHOLE").is_some() {
        return asm;
    }
    let mut store = LineStore::new(asm);
    let line_count = store.len();
    let mut infos: Vec<LineInfo> = (0..line_count).map(|i| classify_line(store.get(i))).collect();

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
    let global_changed = global_changed | eliminate_dead_reg_moves(&store, &mut infos);
    let global_changed = global_changed | eliminate_dead_stores(&store, &mut infos);
    let global_changed = global_changed | fuse_compare_and_branch(&mut store, &mut infos);
    let global_changed = global_changed | fold_memory_operands(&mut store, &mut infos);
    let global_changed = global_changed | eliminate_redundant_test_i686(&mut store, &mut infos);
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
            changed2 |= eliminate_dead_reg_moves(&store, &mut infos);
            changed2 |= eliminate_dead_stores(&store, &mut infos);
            changed2 |= fold_memory_operands(&mut store, &mut infos);
            changed2 |= eliminate_redundant_test_i686(&mut store, &mut infos);
            pass_count2 += 1;
        }
    }

    // Phase 3.5: redundant zero-extension elimination (may expose dead
    // stores and moves by shortening def-use chains, so it runs before DSE
    // and the liveness cleanup).
    let zext_changed = eliminate_redundant_zext_i686(&mut store, &mut infos);
    // Redundant SIGN-extension elimination runs right after: it turns
    // `movsbl %al,%REG` into a `movl` copy or a no-op, which the cleanup
    // loop below then propagates/deletes like any other move.
    let sext_changed = eliminate_redundant_sign_ext_i686(&mut store, &mut infos);
    if zext_changed || sext_changed {
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

    // Phase 3.7: fold indirect calls through freshly spilled global function
    // pointers (`movl SYM,%r; movl %r,N(%esp); call *N(%esp)` ->
    // `call *SYM`). Runs before phase 4 so the vacated slot can turn other
    // stores dead and unused callee-saves can drop.
    fold_global_fnptr_calls(&mut store, &mut infos);

    // Phase 4: Never-read store elimination, then remove callee-save pairs
    // made unused by the dead-move cleanup above.
    eliminate_never_read_stores(&store, &mut infos);
    optimize_cfg_register_liveness(&mut store, &mut infos, true);
    eliminate_unused_callee_saves(&mut store, &mut infos);

    // Phase 5: tail-call conversion (after all epilogue-shape rewrites).
    optimize_tail_calls_i686(&mut store, &mut infos);

    store.build_result(|i| infos[i].is_nop())
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
/// operand_to_eax cannot see the producer. Same state machine as the x86-64
/// pass (redundant_ext.rs): flags cleared at labels/calls/barriers, updated
/// by extensions, copies, and byte-preserving andl; anything else clears the
/// written family. Fail-closed: implicit multi-register writers clear all.
fn eliminate_redundant_zext_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = infos.len();
    let mut is_byte = [false; 8]; // per family: value < 256
    let mut is_16 = [false; 8];   // per family: value < 65536

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() { i += 1; continue; }
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
                        store.replace(i, format!(
                            "    movl {}, {}", reg32_name(sf), reg32_name(df)));
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
            LineKind::StoreEbp { .. } | LineKind::Push { .. }
            | LineKind::Cmp | LineKind::Empty | LineKind::SelfMove => {}
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
                        .split(',').next()
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
        if infos[i].is_nop() { i += 1; continue; }
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
                            store.replace(i, format!(
                                "    movl {}, {}", reg32_name(sf), reg32_name(df)));
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
            LineKind::StoreEbp { .. } | LineKind::Push { .. }
            | LineKind::Cmp | LineKind::Empty | LineKind::SelfMove => {}
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
                        r.split(',').next().and_then(|v| v.trim().parse::<i64>().ok())
                    }) {
                        sign_ext[d as usize] = (-128..=127).contains(&imm);
                        is_bool[d as usize] = matches!(imm, 0 | 1);
                    } else if let Some(imm) = t.strip_prefix("andl $").and_then(|r| {
                        r.split(',').next().and_then(|v| v.trim().parse::<i64>().ok())
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
fn optimize_tail_calls_i686(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;
    let mut suppress = false;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() { i += 1; continue; }

        match infos[i].kind {
            LineKind::Label => {
                let t = trimmed(store, &infos[i], i);
                if !t.starts_with(".L") {
                    suppress = false; // new function
                }
                i += 1;
                continue;
            }
            LineKind::Directive => {
                let t = trimmed(store, &infos[i], i);
                if t == ".cfi_startproc" {
                    suppress = false;
                }
                i += 1;
                continue;
            }
            _ => {}
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

        if infos[i].kind != LineKind::Call || suppress {
            i += 1;
            continue;
        }

        // Pure-epilogue scan after the call.
        let limit = (i + 16).min(len);
        let mut j = i + 1;
        let mut ret_idx = None;
        let mut restored_regs: u32 = 0;
        while j < limit {
            if infos[j].is_nop() { j += 1; continue; }
            match infos[j].kind {
                LineKind::Empty | LineKind::Directive => { j += 1; }
                LineKind::Pop { reg } => {
                    if reg == REG_EAX || reg == REG_EDX {
                        break; // clobbers return value
                    }
                    if reg <= 31 { restored_regs |= 1u32 << reg; }
                    j += 1;
                }
                LineKind::Ret => { ret_idx = Some(j); break; }
                LineKind::Other { .. } => {
                    let t = trimmed(store, &infos[j], j);
                    let is_teardown = t == "movl %ebp, %esp"
                        || (t.starts_with("addl $") && t.ends_with(", %esp")
                            && t["addl $".len()..t.len() - ", %esp".len()]
                                .bytes().all(|b| b.is_ascii_digit()))
                        || (t.starts_with("leal -") && t.ends_with("(%ebp), %esp"));
                    if is_teardown { j += 1; continue; }
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("imull %eax, %edx"), "imul not forwarded:\n{result}");
        assert!(!result.contains("32(%esp)"), "store/load survived:\n{result}");
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
        let result = peephole_optimize(asm);
        assert!(result.contains("imull 32(%esp), %edx"),
            "imul must stay a memory op when %eax is rewritten:\n{result}");
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
        let result = peephole_optimize(asm);
        assert!(result.contains("imull 12(%esp), %edx"),
            "imul must survive behind a mid-slot reader:\n{result}");
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
        let result = peephole_optimize(asm);
        assert!(result.contains("addl %eax, %edx"), "addl not forwarded:\n{result}");
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("4(%esp), %eax") || result.contains("testl %eax, %eax"),
            "load must survive while %eax is live:\n{result}");
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("call *4(%esp)"), "must stay staged:\n{result}");
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        let stores = result.matches(", 8(%esp)\n").count();
        assert_eq!(stores, 1, "first store must die, second must survive:\n{result}");
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("addl %ebx, %ebx"), "{result}");
        assert!(!result.contains("addl %eax, %ebx"), "{result}");
    }

    #[test]
    fn cfg_does_not_retarget_implicit_or_partial_accumulator_ops() {
        for op in ["imull %eax", "incw %ax"] {
            let asm = format!(
                "f:\n    movl %ebx, %eax\n    {op}\n    movl %eax, %ebx\n    xorl %eax, %eax\n    pushl %ebx\n    ret\n.cfi_endproc\n"
            );
            let result = peephole_optimize(asm);
            assert!(result.contains("movl %ebx, %eax"), "{op}: {result}");
            assert!(result.contains(op), "{op}: {result}");
        }
    }

    #[test]
    fn sign_extending_moves_are_not_classified_as_string_ops() {
        assert!(!is_implicit_string_op("movsbl (%ecx), %eax"));
        assert!(!is_implicit_string_op("movswl (%ecx), %eax"));
        assert!(is_implicit_string_op("movsl"));
        assert!(is_implicit_string_op("rep movsl"));
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("movzbl (%esi), %eax"));
        assert!(result.contains("movsbl %al"), "{result}");
    }

    #[test]
    fn sign_ext_high_byte_is_not_a_full_register_copy() {
        for dst in ["%eax", "%ecx"] {
            let asm = format!(
                "f:\n.cfi_startproc\n    movsbl (%esi), %eax\n    movsbl %ah, {dst}\n    ret\n.cfi_endproc\n"
            );
            let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(!result.contains("movsbl %al"), "{result}");
        assert!(result.contains("movl %eax, %ecx"), "{result}");
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("movl %eax, 4(%esp)"));
        assert!(result.contains("movl 8(%esp), %eax"));
    }

    #[test]
    fn return_metadata_limits_edx_liveness_to_wide_functions() {
        let narrow = peephole_optimize(
            "narrow:\n    movl %ebx, %edx\n    ret\n.cfi_endproc\n".to_string(),
        );
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(!result.contains("-8(%ebp)"),
            "store/load pair should be fully eliminated, got:\n{}", result);
    }

    #[test]
    fn test_redundant_zext_removed() {
        // Second movzbl of an already-byte value is a no-op.
        let asm = "f:\n    movzbl (%ecx), %eax\n    movzbl %al, %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.matches("movzbl").count(), 1,
            "redundant re-extension must be removed:\n{}", result);
    }

    #[test]
    fn test_zext_cleared_at_label() {
        // Byte-ness must not survive a control-flow merge.
        let asm = "f:\n    movzbl (%ecx), %eax\n.LBB1:\n    movzbl %al, %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.matches("movzbl").count(), 2,
            "flags must clear at labels:\n{}", result);
    }

    #[test]
    fn test_zext_cleared_by_add() {
        // addl can overflow past 255: re-extension is REQUIRED.
        let asm = "f:\n    movzbl (%ecx), %eax\n    addl $200, %eax\n    movzbl %al, %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.matches("movzbl").count(), 2,
            "addl invalidates byte-ness:\n{}", result);
    }

    #[test]
    fn test_zext_preserved_by_andl_imm8() {
        // andl $127 keeps the value in byte range: re-extension is dead.
        let asm = "f:\n    movzbl (%ecx), %eax\n    andl $127, %eax\n    movzbl %al, %eax\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.matches("movzbl").count(), 1,
            "andl imm8 preserves byte-ness:\n{}", result);
    }

    #[test]
    fn zext_high_byte_is_not_a_full_register_copy() {
        // EAX < 256 proves AH == 0, not that zero-extending AH is a no-op or
        // equivalent to copying the full EAX value.
        for (src, dst) in [("%ah", "%eax"), ("%ah", "%edx")] {
            let asm = format!(
                "f:\n    movzbl (%ecx), %eax\n    movzbl {src}, {dst}\n    ret\n"
            );
            let result = peephole_optimize(asm);
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
            let result = peephole_optimize(asm);
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
        let asm = "f:\n    pushl %ebx\n    movl %eax, %ebx\n    call target\n    popl %ebx\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("jmp target"), "expected tail call, got:\n{}", result);
        assert!(!result.contains("call target"), "call should be gone:\n{}", result);
        assert!(!result.contains("pushl %ebx") && !result.contains("popl %ebx"),
            "dead callee-save pair must go:\n{}", result);
        // If a restore had survived, it would have to precede the jmp; with
        // the pair gone the ordering constraint is vacuous.
    }

    #[test]
    fn test_tail_call_with_live_callee_save_restores_before_jmp() {
        // %ebx is genuinely used (feeds the tail-callee's argument), so the
        // pair must SURVIVE and the pop must precede the jmp.
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
        let result = peephole_optimize(asm);
        assert!(result.contains("jmp target"), "expected tail call, got:\n{}", result);
        let jmp_pos = result.find("jmp target").unwrap();
        let pop_pos = result.find("popl %ebx").unwrap();
        assert!(pop_pos < jmp_pos, "restore must precede jmp:\n{}", result);
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
        let result = peephole_optimize(asm);
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
        let result = peephole_optimize(asm);
        assert!(result.contains("pushl %ebx"), "{result}");
        assert!(result.contains("popl %ebx"), "{result}");
    }

    #[test]
    fn unused_callee_saves_removed_across_multiple_epilogues() {
        // Framed shape with TWO epilogues (early return + fallthrough):
        // %ebp is never referenced; both pops and the push must go, and
        // both addl deallocs plus the subl alloc grow by 4.
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
        let result = peephole_optimize(asm);
        assert!(!result.contains("%ebp"), "unused %ebp pair must go:\n{result}");
        assert_eq!(result.matches("subl $20, %esp").count(), 1, "{result}");
        assert_eq!(result.matches("addl $20, %esp").count(), 2, "{result}");
        assert!(result.contains("pushl %ebx"), "live %ebx pair stays:\n{result}");
    }

    #[test]
    fn test_tail_call_suppressed_by_leal_slot() {
        // Address of a frame slot escapes to the callee: converting would
        // hand the callee a dangling pointer after frame release.
        let asm = "f:\n    subl $12, %esp\n    leal 4(%esp), %eax\n    call target\n    addl $12, %esp\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("call target"), "must stay a call:\n{}", result);
    }

    #[test]
    fn test_tail_call_indirect_not_through_restored_reg() {
        // call *%ebx with %ebx restored by the epilogue: the pointer dies
        // before the jmp could use it — must NOT convert.
        let asm = "f:\n    pushl %ebx\n    movl %eax, %ebx\n    call *%ebx\n    popl %ebx\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("call *%ebx"), "must stay a call:\n{}", result);
    }

    #[test]
    fn test_ebp_pointer_store_kept_without_frame() {
        // WITHOUT the FP prologue (as under -fomit-frame-pointer), %ebp is a
        // data register: `movl %eax, -8(%ebp)` writes through a pointer and
        // must survive even though nothing in this function reads it back.
        let asm = "    movl %eax, -8(%ebp)\n    ret\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movl %eax, -8(%ebp)"),
            "pointer-write through data %ebp must not be deleted, got:\n{}", result);
    }

    #[test]
    fn test_store_load_different_reg() {
        let asm = "    movl %eax, -8(%ebp)\n    movl -8(%ebp), %ecx\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movl %eax, %ecx"), "should forward: {}", result);
        assert!(!result.contains("-8(%ebp), %ecx"), "should eliminate load: {}", result);
    }

    #[test]
    fn test_self_move() {
        let asm = "    movl %eax, %eax\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.trim(), "");
    }

    /// idivl writes %eax implicitly (no comma operand -> Other{REG_NONE}).
    /// Copy propagation must NOT rewrite readers of %eax past the division:
    /// `movl %ebx,%eax; cltd; idivl %ecx; movl %eax,%edi` -- the last move
    /// consumes the QUOTIENT, not %ebx. (mulhaz.c: a + a/b returned 2*a.)
    #[test]
    fn test_copy_prop_stops_at_idivl() {
        let asm = "    movl %ebx, %eax\n    cltd\n    idivl %ecx\n    movl %eax, %edi\n    pushl %edi\n".to_string();
        let result = peephole_optimize(asm);
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
        let asm = "    movl %esi, %eax\n    mull %ecx\n    movl %eax, %edi\n    pushl %edi\n".to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("movl %esi, %edi"),
            "product consumer must keep reading %eax: {}",
            result
        );
    }

    #[test]
    fn test_redundant_jump() {
        let asm = "    jmp .Lfoo\n.Lfoo:\n".to_string();
        let result = peephole_optimize(asm);
        assert!(!result.contains("jmp"), "should eliminate redundant jmp: {}", result);
        assert!(result.contains(".Lfoo:"), "should keep label: {}", result);
    }

    #[test]
    fn test_branch_inversion() {
        let asm = [
            "    jl .LBB2",
            "    jmp .LBB4",
            ".LBB2:",
            "    movl %eax, %ecx",
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("jge .LBB4"), "should invert to jge: {}", result);
        assert!(!result.contains("jmp .LBB4"), "should remove jmp: {}", result);
    }

    #[test]
    fn test_compare_branch_fusion() {
        let asm = [
            "    cmpl %ecx, %eax",
            "    setl %al",
            "    movzbl %al, %eax",
            "    testl %eax, %eax",
            "    jne .LBB2",
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("jl .LBB2"), "should fuse to jl: {}", result);
        assert!(!result.contains("setl"), "should eliminate setl: {}", result);
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
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("jge .LBB5"), "should fuse to jge: {}", result);
        assert!(!result.contains("setge"), "should eliminate setge: {}", result);
        assert!(!result.contains("movzbl"), "should eliminate movzbl: {}", result);
        assert!(!result.contains("testl"), "should eliminate testl: {}", result);
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
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        // Should NOT fuse because the store has no matching load
        assert!(result.contains("setge"), "should keep setge (unmatched store): {}", result);
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
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("jge .LBB3"), "should fuse to jge (inverted): {}", result);
        assert!(!result.contains("setl"), "should eliminate setl: {}", result);
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
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(!result.contains("%eax, -8(%ebp)"), "first store dead: {}", result);
        assert!(result.contains("%ecx"), "second store alive: {}", result);
    }

    #[test]
    fn test_memory_fold() {
        let asm = [
            "    movl -48(%ebp), %ecx",
            "    addl %ecx, %eax",
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("addl -48(%ebp), %eax"), "should fold: {}", result);
    }

    // Note: store forwarding tests removed - global_store_forwarding is disabled
    // due to FP computation regressions.

    #[test]
    fn test_reverse_move_elimination() {
        let asm = [
            "    movl %eax, %ecx",
            "    movl %ecx, %eax",
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(result.matches("movl").count(), 1, "should eliminate reverse: {}", result);
    }

    // Note: push/pop elimination test removed - eliminate_push_pop_pairs is disabled
    // due to callee-save/leal epilogue interactions.

    #[test]
    fn test_addl_1_to_incl() {
        let asm = "    addl $1, %eax\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("incl %eax"), "should convert to incl: {}", result);
        assert!(!result.contains("addl"), "should eliminate addl: {}", result);
    }

    #[test]
    fn test_subl_1_to_decl() {
        let asm = "    subl $1, %ecx\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("decl %ecx"), "should convert to decl: {}", result);
        assert!(!result.contains("subl"), "should eliminate subl: {}", result);
    }

    #[test]
    fn test_movl_0_to_xorl() {
        let asm = "    movl $0, %ebx\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("xorl %ebx, %ebx"), "should convert to xorl: {}", result);
        assert!(!result.contains("movl"), "should eliminate movl: {}", result);
    }

    #[test]
    fn test_redundant_movsbl() {
        let asm = [
            "    movsbl (%ecx), %eax",
            "    movsbl %al, %eax",
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(result.matches("movsbl").count(), 1,
            "should eliminate redundant movsbl: {}", result);
    }

    #[test]
    fn test_addl_neg1_to_decl() {
        let asm = "    addl $-1, %edx\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("decl %edx"), "should convert to decl: {}", result);
    }

    #[test]
    fn test_subl_neg1_to_incl() {
        let asm = "    subl $-1, %esi\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("incl %esi"), "should convert to incl: {}", result);
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
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("addl $1, %eax"), "must keep addl before adcl: {}", result);
        assert!(!result.contains("incl"), "must NOT convert to incl before adcl: {}", result);
    }

    #[test]
    fn test_subl_1_not_decl_before_sbbl() {
        // subl $1 followed by sbbl must NOT be converted to decl,
        // because decl does not set the carry flag.
        let asm = [
            "    subl $1, %eax",
            "    sbbl $0, %edx",
        ].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("subl $1, %eax"), "must keep subl before sbbl: {}", result);
        assert!(!result.contains("decl"), "must NOT convert to decl before sbbl: {}", result);
    }
}

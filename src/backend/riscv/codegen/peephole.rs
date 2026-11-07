//! RISC-V 64-bit peephole optimizer for assembly text.
//!
//! Operates on generated assembly text to eliminate redundant patterns from the
//! stack-based codegen. Lines are pre-parsed into `LineKind` enums so hot-path
//! pattern matching uses integer/enum comparisons instead of string parsing.
//!
//! ## Pass structure
//!
//! **Local passes** (iterative, up to 8 rounds): store/load elimination,
//! redundant jump removal, self-move elimination, move chain optimization,
//! and li-to-move chain elimination.
//!
//! **Global passes** (once): global store forwarding, register copy propagation,
//! and global dead store elimination.
//!
//! **Local cleanup** (up to 4 rounds): re-run local passes to clean up
//! opportunities exposed by global passes.
//!
//! ## Optimizations
//!
//! 1. **Adjacent store/load elimination**: `sd t0, off(s0)` followed by
//!    `ld t0, off(s0)` at the same offset — the load is redundant.
//!
//! 2. **Redundant jump elimination**: `jump .LBBN, t6` (or `j .LBBN`) when
//!    `.LBBN:` is the next non-empty line — the jump is redundant.
//!
//! 3. **Self-move elimination**: `mv tX, tX` is a no-op.
//!
//! 4. **Mv chain optimization**: `mv A, B; mv C, A` → redirect second to
//!    `mv C, B`, enabling the first mv to become dead if A is unused.
//!
//! 5. **Li-to-move chain**: `li rX, imm; mv rY, rX` → `li rY, imm` when
//!    rX is a temp register and the next instruction copies from it.
//!
//! 6. **Global store forwarding**: Tracks slot→register mappings within basic
//!    blocks. When a load reads a slot whose value is still in a register,
//!    the load is replaced with a register move (or eliminated if same reg).
//!
//! 7. **Register copy propagation**: After store forwarding converts loads to
//!    moves, propagate `mv dst, src` into the next instruction's source operands.
//!
//! 8. **Global dead store elimination**: Removes stores to stack slots that
//!    are never loaded anywhere in the function.
//!
//! 9. **Range-safe branch-over-jump fusion**: Inverts a conditional branch
//!    over an unconditional jump when the new B-type target is conservatively
//!    proven to fit its ±4 KiB range.
//!
//! 10. **Narrow-value lattice / redundant zero extensions**: Cross-pollinates
//!     AArch64 extension folding and x86 upper-bit knowledge across unsigned
//!     loads, constants, copies, comparisons, masks, logic and shifts; removes
//!     both redundant byte masks and RV64 `slli 32; srli 32` zext.w pairs.
//!
//! 11. **Trampoline threading**: a local label whose body is a single
//!     unconditional jump is bypassed (`jump .Ltramp, t6` -> `jump .Lreal, t6`)
//!     with chains resolved once and cycles refused. The far `jump` pseudo is
//!     retargeted freely; the near `j` form only under the same ±3 KB census
//!     that gates the branch-over-jump fusion.

// ── Line classification types ────────────────────────────────────────────────

/// Compact classification of an assembly line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    /// Deleted / blank
    Nop,
    /// `sd reg, offset(s0)` or `sw reg, offset(s0)` — store to frame slot
    StoreS0 { reg: u8, offset: i32, is_word: bool },
    /// `ld reg, offset(s0)` or `lw reg, offset(s0)` — load from frame slot
    LoadS0 { reg: u8, offset: i32, is_word: bool },
    /// `mv rdst, rsrc` — register-to-register move
    Move { dst: u8, src: u8 },
    /// `li rdst, imm` — load immediate
    LoadImm { dst: u8 },
    /// `jump .label, t6` or `j .label` — unconditional jump
    Jump,
    /// Branch instruction (beq, bne, bge, blt, bgeu, bltu, bnez, beqz)
    Branch,
    /// Label (`.LBBx:` etc.)
    Label,
    /// `ret`
    Ret,
    /// `call` or `jal ra, ...`
    Call,
    /// Assembler directive (lines starting with `.`)
    Directive,
    /// ALU instruction (add, sub, and, or, xor, sll, srl, sra, mul, etc.)
    Alu,
    /// `sext.w` instruction — sign-extend word
    SextW { dst: u8, src: u8 },
    /// `lla reg, symbol` or `la reg, symbol` — load address of symbol
    /// Must not be modified by copy propagation (symbol names contain
    /// register-like substrings such as `main.s1.0`).
    LoadAddr { dst: u8 },
    /// Any other instruction
    Other,
}

/// RISC-V register IDs for pattern matching.
/// We only track the registers that matter for our patterns.
const REG_NONE: u8 = 255;
const REG_T0: u8 = 0;
const REG_T1: u8 = 1;
const REG_T2: u8 = 2;
const REG_T3: u8 = 3;
const REG_T4: u8 = 4;
const REG_T5: u8 = 5;
const REG_T6: u8 = 6;
const REG_S0: u8 = 10; // frame pointer
const REG_S1: u8 = 11;
const REG_S2: u8 = 12;
const REG_S3: u8 = 13;
const REG_S4: u8 = 14;
const REG_S5: u8 = 15;
const REG_S6: u8 = 16;
const REG_S7: u8 = 17;
const REG_S8: u8 = 18;
const REG_S9: u8 = 19;
const REG_S10: u8 = 20;
const REG_S11: u8 = 21;
const REG_A0: u8 = 30;
const REG_A1: u8 = 31;
const REG_A2: u8 = 32;
const REG_A3: u8 = 33;
const REG_A4: u8 = 34;
const REG_A5: u8 = 35;
const REG_A6: u8 = 36;
const REG_A7: u8 = 37;
const REG_RA: u8 = 40;
const REG_SP: u8 = 41;
/// Maximum register ID we track (exclusive upper bound for arrays)
const NUM_REGS: usize = 42;

/// Parse a register name to our internal ID.
fn parse_reg(name: &str) -> u8 {
    match name {
        "t0" => REG_T0,
        "t1" => REG_T1,
        "t2" => REG_T2,
        "t3" => REG_T3,
        "t4" => REG_T4,
        "t5" => REG_T5,
        "t6" => REG_T6,
        "s0" => REG_S0,
        "s1" => REG_S1,
        "s2" => REG_S2,
        "s3" => REG_S3,
        "s4" => REG_S4,
        "s5" => REG_S5,
        "s6" => REG_S6,
        "s7" => REG_S7,
        "s8" => REG_S8,
        "s9" => REG_S9,
        "s10" => REG_S10,
        "s11" => REG_S11,
        "a0" => REG_A0,
        "a1" => REG_A1,
        "a2" => REG_A2,
        "a3" => REG_A3,
        "a4" => REG_A4,
        "a5" => REG_A5,
        "a6" => REG_A6,
        "a7" => REG_A7,
        "ra" => REG_RA,
        "sp" => REG_SP,
        _ => REG_NONE,
    }
}

fn reg_name(id: u8) -> &'static str {
    match id {
        REG_T0 => "t0",
        REG_T1 => "t1",
        REG_T2 => "t2",
        REG_T3 => "t3",
        REG_T4 => "t4",
        REG_T5 => "t5",
        REG_T6 => "t6",
        REG_S0 => "s0",
        REG_S1 => "s1",
        REG_S2 => "s2",
        REG_S3 => "s3",
        REG_S4 => "s4",
        REG_S5 => "s5",
        REG_S6 => "s6",
        REG_S7 => "s7",
        REG_S8 => "s8",
        REG_S9 => "s9",
        REG_S10 => "s10",
        REG_S11 => "s11",
        REG_A0 => "a0",
        REG_A1 => "a1",
        REG_A2 => "a2",
        REG_A3 => "a3",
        REG_A4 => "a4",
        REG_A5 => "a5",
        REG_A6 => "a6",
        REG_A7 => "a7",
        REG_RA => "ra",
        REG_SP => "sp",
        _ => "??",
    }
}

// ── Implicit register operands (central oracle) ─────────────────────────────
//
// RISC-V names nearly every operand explicitly, so its implicit surface is
// small — but it is exactly the surface a textual scan misses: the link
// register.  `ret` jumps through `ra` (implicit READ), `call sym` and the
// bare `jal target` pseudo write `ra` (jal's rd defaults to ra), and the
// one-operand `jalr rs1` / two-operand `jalr rs1, imm` forms read rs1 and
// write ra while naming neither half of that contract.  `jr rs1` reads its
// target and writes nothing; `ecall`/`ebreak`/`fence` touch no GP register;
// `jalr rd, rs1, imm` names everything it touches.
//
// Like the x86-64/x86/A64 oracles this is written against the ISA, not the
// emitter's current subset: the next emitter site (an indirect tail call,
// a computed jump) is covered the day it appears, and the conservative
// fallbacks the passes keep for unclassified lines stay as the safety net.
fn classify_implicit_operands_rv(line: &str) -> (u64, u64) {
    const RA: u64 = 1 << REG_RA;
    let t = line.trim();
    if t.is_empty() {
        return (0, 0);
    }
    let (mnem, rest) = match t.split_once(char::is_whitespace) {
        Some((m, r)) => (m, r.trim()),
        None => (t, ""),
    };
    if mnem.starts_with('.') || mnem.starts_with('#') || mnem.starts_with('/') {
        // Directives, comments and labels name no instruction.
        return (0, 0);
    }
    match mnem {
        "ret" => (RA, 0),
        "call" | "tail" => (0, RA),
        // `jal target` — the rd defaults to ra.  With a comma the rd is
        // named (`jal ra, label` / `jal x0, label`) and nothing is implicit.
        "jal" if !rest.contains(',') => (0, RA),
        // `jalr rs1` / `jalr rs1, imm` — reads the target, writes ra.
        "jalr" if !rest.contains(',') => {
            let r = parse_reg(rest);
            (if r != REG_NONE { 1u64 << r } else { 0 }, RA)
        }
        // `jalr rs1, imm` — the FIRST operand is the target register.
        "jalr" if rest.split(',').count() == 2 => {
            let rs1 = rest.split(',').next().unwrap_or("").trim();
            let r = parse_reg(rs1);
            (if r != REG_NONE { 1u64 << r } else { 0 }, RA)
        }
        "jalr" => (0, 0), // `jalr rd, rs1, imm` — fully explicit
        "jr" => {
            let r = parse_reg(rest);
            (if r != REG_NONE { 1u64 << r } else { 0 }, 0)
        }
        _ => (0, 0),
    }
}

/// True for a line that calls: control returns to the next line with every
/// caller-saved register clobbered, after the callee read a0-a7.
///
/// That is the `call` pseudo and every `jal`/`jalr` that links -- writes the
/// return address into a register other than `zero`.  The emitter's indirect
/// call is `jalr ra, t2, 0`; it used to classify as an ordinary instruction, so
/// the passes that track a call's clobbers by `LineKind::Call` carried values
/// across it in caller-saved registers.  Entry-copy coalescing renamed a
/// parameter's callee-saved home onto the a-register it arrived in, past the
/// call that destroys that register: `long f(long (*fp)(void), long x)
/// { return fp() + x; }` added the callee's leftover a1 instead of `x`.
///
/// Operand forms (GNU as): `jal label` and `jalr rs1` / `jalr rs1, imm` link
/// through ra; `jal rd, label`, `jalr rd, rs1`, `jalr rd, imm(rs1)` and
/// `jalr rd, rs1, imm` name the link register first.
fn is_call_line(trimmed: &str) -> bool {
    let (mnem, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((m, r)) => (m, r.trim()),
        None => (trimmed, ""),
    };
    let links = |rd: &str| !matches!(rd.trim(), "zero" | "x0");
    match mnem {
        "call" => true,
        "jal" => match rest.split_once(',') {
            None => !rest.is_empty(),
            Some((rd, _)) => links(rd),
        },
        "jalr" => {
            let ops: Vec<&str> = rest.split(',').map(str::trim).collect();
            match ops.as_slice() {
                [rs1] => !rs1.is_empty(),
                // `jalr rs1, imm` links through ra; with a register or an
                // `imm(rs1)` address second, the first operand is rd.
                [first, second] => parse_asm_i64(second).is_some() || links(first),
                [rd, _, _] => links(rd),
                _ => true, // unknown shape: assume the worst
            }
        }
        _ => false,
    }
}

/// True when the line implicitly writes register `id` (the write half of
/// [`classify_implicit_operands_rv`]).
fn implicitly_writes_rv(line: &str, id: u8) -> bool {
    id < NUM_REGS as u8 && (classify_implicit_operands_rv(line).1 >> id) & 1 != 0
}

// ── Line classification ──────────────────────────────────────────────────────

/// Classify a single assembly line into a LineKind.
fn classify_line(line: &str) -> LineKind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineKind::Nop;
    }
    // The prologue's return-register and argument-extension contracts: read
    // by the liveness and the extension tracking, never emitted.
    if is_internal_marker(trimmed) {
        return LineKind::Nop;
    }

    // Directives
    if trimmed.starts_with('.') {
        // Check if it's a label like `.LBB3:`
        if trimmed.ends_with(':') {
            return LineKind::Label;
        }
        return LineKind::Directive;
    }

    // Labels (e.g., `func_name:`)
    if trimmed.ends_with(':') && !trimmed.starts_with(' ') && !trimmed.starts_with('\t') {
        return LineKind::Label;
    }

    // Instructions (indented lines)
    // Store: sd/sw reg, offset(s0)
    if let Some(rest) = trimmed
        .strip_prefix("sd ")
        .or_else(|| trimmed.strip_prefix("sw "))
    {
        let is_word = trimmed.starts_with("sw");
        if let Some((reg_str, addr)) = rest.split_once(", ") {
            let reg = parse_reg(reg_str.trim());
            if reg != REG_NONE {
                if let Some(offset) = parse_s0_offset(addr.trim()) {
                    return LineKind::StoreS0 {
                        reg,
                        offset,
                        is_word,
                    };
                }
            }
        }
    }

    // Load: ld/lw reg, offset(s0)
    if let Some(rest) = trimmed
        .strip_prefix("ld ")
        .or_else(|| trimmed.strip_prefix("lw "))
    {
        let is_word = trimmed.starts_with("lw");
        if let Some((reg_str, addr)) = rest.split_once(", ") {
            let reg = parse_reg(reg_str.trim());
            if reg != REG_NONE {
                if let Some(offset) = parse_s0_offset(addr.trim()) {
                    return LineKind::LoadS0 {
                        reg,
                        offset,
                        is_word,
                    };
                }
            }
        }
    }

    // Move: mv dst, src
    if let Some(rest) = trimmed.strip_prefix("mv ") {
        if let Some((dst_str, src_str)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            let src = parse_reg(src_str.trim());
            if dst != REG_NONE && src != REG_NONE {
                return LineKind::Move { dst, src };
            }
        }
    }

    // Load immediate: li dst, imm
    if let Some(rest) = trimmed.strip_prefix("li ") {
        if let Some((dst_str, _)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            if dst != REG_NONE {
                return LineKind::LoadImm { dst };
            }
        }
    }

    // sext.w dst, src
    if let Some(rest) = trimmed.strip_prefix("sext.w ") {
        if let Some((dst_str, src_str)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            let src = parse_reg(src_str.trim());
            if dst != REG_NONE && src != REG_NONE {
                return LineKind::SextW { dst, src };
            }
        }
    }

    // Unconditional jump: `jump .label, t6` or `j .label`
    if trimmed.starts_with("jump ") || trimmed.starts_with("j ") {
        // Don't match `jal`, `jalr`, etc.
        if trimmed.starts_with("j ") || trimmed.starts_with("jump ") {
            return LineKind::Jump;
        }
    }

    // Branch instructions, pseudoinstructions included: the emitter writes
    // `ble`/`bgt` too, and as `Other` they were invisible as control flow.
    if trimmed
        .split_once(' ')
        .is_some_and(|(op, _)| invert_branch_mnemonic(op).is_some())
    {
        return LineKind::Branch;
    }

    // ret
    if trimmed == "ret" {
        return LineKind::Ret;
    }

    // call -- direct, or indirect through `jalr` (see `is_call_line`)
    if is_call_line(trimmed) {
        return LineKind::Call;
    }

    // ALU instructions
    if trimmed.starts_with("add ")
        || trimmed.starts_with("addi ")
        || trimmed.starts_with("addw ")
        || trimmed.starts_with("addiw ")
        || trimmed.starts_with("sub ")
        || trimmed.starts_with("subw ")
        || trimmed.starts_with("and ")
        || trimmed.starts_with("andi ")
        || trimmed.starts_with("or ")
        || trimmed.starts_with("ori ")
        || trimmed.starts_with("xor ")
        || trimmed.starts_with("xori ")
        || trimmed.starts_with("sll ")
        || trimmed.starts_with("slli ")
        || trimmed.starts_with("srl ")
        || trimmed.starts_with("srli ")
        || trimmed.starts_with("sra ")
        || trimmed.starts_with("srai ")
        || trimmed.starts_with("sllw ")
        || trimmed.starts_with("slliw ")
        || trimmed.starts_with("srlw ")
        || trimmed.starts_with("srliw ")
        || trimmed.starts_with("sraw ")
        || trimmed.starts_with("sraiw ")
        || trimmed.starts_with("mul ")
        || trimmed.starts_with("mulw ")
        || trimmed.starts_with("div ")
        || trimmed.starts_with("divu ")
        || trimmed.starts_with("divw ")
        || trimmed.starts_with("divuw ")
        || trimmed.starts_with("rem ")
        || trimmed.starts_with("remu ")
        || trimmed.starts_with("remw ")
        || trimmed.starts_with("remuw ")
        || trimmed.starts_with("slt ")
        || trimmed.starts_with("sltu ")
        || trimmed.starts_with("slti ")
        || trimmed.starts_with("sltiu ")
        || trimmed.starts_with("neg ")
        || trimmed.starts_with("negw ")
        || trimmed.starts_with("not ")
        || trimmed.starts_with("snez ")
        || trimmed.starts_with("seqz ")
        || trimmed.starts_with("lui ")
    {
        return LineKind::Alu;
    }

    // Load address: lla/la reg, symbol
    // These must not have their symbol operands modified by copy propagation,
    // since symbol names can contain register-like substrings (e.g. `main.s1.0`).
    if let Some(rest) = trimmed
        .strip_prefix("lla ")
        .or_else(|| trimmed.strip_prefix("la "))
    {
        if let Some((reg_str, _)) = rest.split_once(", ") {
            let dst = parse_reg(reg_str.trim());
            if dst != REG_NONE {
                return LineKind::LoadAddr { dst };
            }
        }
    }

    LineKind::Other
}

/// Parse `offset(s0)` and return the offset if it references s0.
fn parse_s0_offset(addr: &str) -> Option<i32> {
    // Format: `-24(s0)` or `48(s0)` etc.
    if !addr.ends_with("(s0)") {
        return None;
    }
    let offset_str = &addr[..addr.len() - 4]; // strip "(s0)"
    offset_str.parse::<i32>().ok()
}

/// Extract the jump target label from a jump instruction line.
fn jump_target(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("jump ") {
        // `jump .LBBN, t6` -> `.LBBN`
        if let Some((label, _)) = rest.split_once(", ") {
            return Some(label.trim());
        }
    }
    if let Some(rest) = trimmed.strip_prefix("j ") {
        return Some(rest.trim());
    }
    None
}

/// Extract the label name from a label line (strip trailing `:`)
fn label_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed.strip_suffix(':')
}

/// Invert a RISC-V conditional branch mnemonic.
fn invert_branch_mnemonic(op: &str) -> Option<&'static str> {
    // The B-type instructions and every branch pseudoinstruction of the
    // RISC-V Unprivileged ISA (`bgt`/`ble`/`bgtu`/`bleu` swap the operands of
    // `blt`/`bge`/`bltu`/`bgeu`; the `z` forms compare with x0).
    match op {
        "beq" => Some("bne"),
        "bne" => Some("beq"),
        "blt" => Some("bge"),
        "bge" => Some("blt"),
        "bltu" => Some("bgeu"),
        "bgeu" => Some("bltu"),
        "bgt" => Some("ble"),
        "ble" => Some("bgt"),
        "bgtu" => Some("bleu"),
        "bleu" => Some("bgtu"),
        "beqz" => Some("bnez"),
        "bnez" => Some("beqz"),
        "bltz" => Some("bgez"),
        "bgez" => Some("bltz"),
        "blez" => Some("bgtz"),
        "bgtz" => Some("blez"),
        _ => None,
    }
}

/// Parse a conditional branch into (mnemonic, operands before target, target).
fn branch_parts(line: &str) -> Option<(&str, &str, &str)> {
    let trimmed = line.trim();
    let (op, args) = trimmed.split_once(' ')?;
    invert_branch_mnemonic(op)?;
    let comma = args.rfind(',')?;
    Some((op, args[..comma].trim_end(), args[comma + 1..].trim()))
}

fn parse_asm_i64(text: &str) -> Option<i64> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()
    } else if let Some(hex) = text.strip_prefix("-0x") {
        i64::from_str_radix(hex, 16).ok()?.checked_neg()
    } else {
        text.parse().ok()
    }
}

/// Parse the destination register from an ALU-style instruction.
/// RISC-V ALU instructions have the form: `mnemonic rd, rs1, rs2/imm`
/// The destination is the first operand.
fn parse_alu_dest(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    let space_pos = trimmed.find(' ')?;
    let args = &trimmed[space_pos + 1..];
    let first_arg = if let Some(comma) = args.find(',') {
        args[..comma].trim()
    } else {
        args.trim()
    };
    let reg = parse_reg(first_arg);
    if reg != REG_NONE { Some(reg) } else { None }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Find complete functions containing IR-marked volatile accesses.  When CFI
/// boundaries or marker nesting cannot establish a whole function, return
/// None: the caller must keep the entire translation unit unchanged instead
/// of guessing which surrounding code may feed an observable access.
fn volatile_function_ranges(lines: &[String]) -> Option<Vec<std::ops::Range<usize>>> {
    let mut ranges = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut has_volatile = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t == ".cfi_startproc" {
            if start.is_some() {
                return None;
            }
            start = Some(i);
            has_volatile = false;
        } else if t == "# LCCC_VOLATILE_BEGIN" {
            start?;
            depth = depth.checked_add(1)?;
            has_volatile = true;
        } else if t == "# LCCC_VOLATILE_END" {
            start?;
            depth = depth.checked_sub(1)?;
        } else if t == ".cfi_endproc" {
            let begin: usize = start.take()?;
            if depth != 0 {
                return None;
            }
            if has_volatile {
                // Accept only the exact shape generated by the backend:
                // `.type NAME, @function; NAME:; .cfi_startproc ...
                // .cfi_endproc; .size NAME, .-NAME`. A user-written CFI
                // interval by itself is not proof that it spans the function.
                let name = lines.get(begin.checked_sub(1)?)?.trim().strip_suffix(':')?;
                if name.is_empty() || name.contains(char::is_whitespace) {
                    return None;
                }
                let (type_name, type_kind) = lines
                    .get(begin.checked_sub(2)?)?
                    .trim()
                    .strip_prefix(".type ")?
                    .split_once(',')?;
                let (size_name, size_expr) = lines
                    .get(i + 1)?
                    .trim()
                    .strip_prefix(".size ")?
                    .split_once(',')?;
                if type_name.trim() != name
                    || type_kind.trim() != "@function"
                    || size_name.trim() != name
                    || size_expr.trim().strip_prefix(".-") != Some(name)
                {
                    return None;
                }
                ranges.push(begin..i + 1);
            }
        } else if t.starts_with("# LCCC_VOLATILE_")
            || t.starts_with(".cfi_startproc")
            || t.starts_with(".cfi_endproc")
        {
            // A malformed marker or a different CFI spelling is ambiguous.
            return None;
        }
    }
    if start.is_some() || depth != 0 {
        return None;
    }
    Some(ranges)
}

// Opaque lines must remain barriers even for passes that scan by symbol
// rather than by LineKind. Never emit this internal placeholder.
const OPAQUE_LINE: &str = "# riscv_volatile_opaque";

/// Run peephole optimization on RISC-V assembly text.
/// Returns the optimized assembly string.
pub fn peephole_optimize(asm: String) -> String {
    if std::env::var_os("CCC_NO_PEEPHOLE").is_some() {
        return strip_internal_markers(asm);
    }
    let has_markers = asm.contains("# LCCC_VOLATILE_");
    // Either marker: a stray `#NO_APP` is malformed input, not plain text.
    let has_inline_asm = asm.contains("#APP") || asm.contains("#NO_APP");
    let mut lines: Vec<String> = asm.lines().map(String::from).collect();
    let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
    let n = lines.len();

    if n == 0 {
        return asm;
    }
    let mut ranges = if has_markers {
        match volatile_function_ranges(&lines) {
            Some(ranges) => ranges,
            None => return strip_internal_markers(asm),
        }
    } else {
        Vec::new()
    };
    // Functions holding inline assembly are opaque for the same reason: the
    // statement's operands are bound by constraints the text does not show.
    if has_inline_asm {
        match crate::backend::peephole_common::inline_asm_protected_ranges(&lines) {
            Some(asm_ranges) => ranges.extend(asm_ranges),
            None => return strip_internal_markers(asm),
        }
    }
    let has_opaque_functions = !ranges.is_empty();
    let mut protected = Vec::new();
    // A function with both a volatile access and inline asm is listed twice;
    // each line is hidden (and later restored) exactly once.
    let mut hidden = vec![false; n];
    for range in ranges {
        for i in range {
            if kinds[i] == LineKind::Label || hidden[i] {
                // Preserve labels as control-flow boundaries. Block merging is
                // disabled below because references inside opaque functions
                // are deliberately invisible to the optimizer's CFG.
                continue;
            }
            hidden[i] = true;
            // A whole volatile function is opaque, not only the access: a
            // prior store or register definition may feed it. Other clears
            // register/slot facts and cannot match a textual peephole pattern.
            protected.push((i, std::mem::replace(&mut lines[i], OPAQUE_LINE.to_string())));
            kinds[i] = LineKind::Other;
        }
    }
    crate::backend::peephole_common::hide_debug_rows(&lines, &mut kinds, LineKind::Nop);

    let merge_blocks =
        !has_opaque_functions && std::env::var("CCC_NO_IDENTICAL_BLOCK_MERGE").is_err();
    let coalesce_entry = std::env::var("CCC_NO_ENTRY_COPY_COALESCE").is_err();
    let retarget_accum = std::env::var("CCC_NO_RV_ACCUMULATOR_RETARGET").is_err();

    // Phase 1: Iterative local passes (up to 8 rounds)
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 8 {
        changed = false;
        changed |= eliminate_adjacent_store_load(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_jumps(&lines, &mut kinds, n);
        // These passes reject intervals containing opaque placeholders: a
        // hidden directive can emit an unbounded number of bytes. Ordinary
        // functions remain eligible when their own ranges are visible.
        changed |= rewrite_far_jumps_to_near(&mut lines, &mut kinds, n);
        changed |= fuse_branch_over_jump(&mut lines, &mut kinds, n);
        changed |= thread_trampolines(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_sext_w(&mut lines, &mut kinds, n);
        changed |= fold_zext32_into_sext_w(&lines, &mut kinds, n);
        changed |= eliminate_redundant_zext_byte(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_sign_extensions(&mut lines, &mut kinds, n);
        changed |= fold_sign_masks(&mut lines, &mut kinds, n);
        changed |= eliminate_self_moves(&mut kinds, n);
        changed |= eliminate_redundant_mv_chain(&mut lines, &mut kinds, n);
        if merge_blocks {
            changed |= merge_identical_blocks(&mut lines, &mut kinds, n);
        }
        if coalesce_entry {
            changed |= coalesce_entry_copies(&mut lines, &mut kinds, n);
        }
        if retarget_accum {
            changed |= retarget_dying_accumulator_moves(&mut lines, &mut kinds, n);
        }
        changed |= eliminate_li_mv_chain(&mut lines, &mut kinds, n);
        rounds += 1;
    }

    // Branch-over-jump fusion above is deliberately range-gated: B-type
    // branches reach only ±4 KiB, unlike the far `jump` pseudo.

    // Phase 2: Global passes (run once)
    let mut global_changed = false;
    global_changed |= global_store_forwarding(&mut lines, &mut kinds, n);
    global_changed |= propagate_register_copies(&mut lines, &mut kinds, n);
    global_changed |= eliminate_dead_reg_moves(&lines, &mut kinds, n);
    global_changed |= propagate_copies_within_blocks(&mut lines, &mut kinds, n);
    global_changed |= propagate_zero_constants(&mut lines, &mut kinds, n);
    global_changed |= eliminate_dead_defs_global(&lines, &mut kinds, n);
    global_changed |= eliminate_undemanded_extensions(&mut lines, &mut kinds, n);
    global_changed |= fold_into_memory_operands(&mut lines, &mut kinds, n);
    global_changed |= forward_fp_bit_moves(&mut lines, &mut kinds, n);
    global_changed |= fold_fp_bounces(&mut lines, &mut kinds, n);
    global_changed |= forward_fp_slot_reloads(&mut lines, &mut kinds, n);
    global_changed |= promote_fp_frame_slots(&mut lines, &mut kinds, n);
    global_changed |= forward_fp_copies(&mut lines, &mut kinds, n);
    global_changed |= retarget_fp_results(&mut lines, &mut kinds, n);
    // Per function; a function with an opaque body is skipped whole.
    global_changed |= global_dead_store_elimination(&lines, &mut kinds, n);

    // Phase 3: Local cleanup after global passes (up to 4 rounds)
    if global_changed {
        let mut changed2 = true;
        let mut rounds2 = 0;
        while changed2 && rounds2 < 4 {
            changed2 = false;
            changed2 |= eliminate_adjacent_store_load(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_jumps(&lines, &mut kinds, n);
            changed2 |= rewrite_far_jumps_to_near(&mut lines, &mut kinds, n);
            changed2 |= fuse_branch_over_jump(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_sext_w(&mut lines, &mut kinds, n);
            changed2 |= fold_zext32_into_sext_w(&lines, &mut kinds, n);
            changed2 |= eliminate_redundant_zext_byte(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_sign_extensions(&mut lines, &mut kinds, n);
            changed2 |= fold_sign_masks(&mut lines, &mut kinds, n);
            changed2 |= eliminate_self_moves(&mut kinds, n);
            changed2 |= eliminate_redundant_mv_chain(&mut lines, &mut kinds, n);
            if merge_blocks {
                changed2 |= merge_identical_blocks(&mut lines, &mut kinds, n);
            }
            if coalesce_entry {
                changed2 |= coalesce_entry_copies(&mut lines, &mut kinds, n);
            }
            if retarget_accum {
                changed2 |= retarget_dying_accumulator_moves(&mut lines, &mut kinds, n);
            }
            changed2 |= eliminate_li_mv_chain(&mut lines, &mut kinds, n);
            changed2 |= eliminate_dead_reg_moves(&lines, &mut kinds, n);
            changed2 |= propagate_copies_within_blocks(&mut lines, &mut kinds, n);
            changed2 |= propagate_zero_constants(&mut lines, &mut kinds, n);
            changed2 |= eliminate_dead_defs_global(&lines, &mut kinds, n);
            changed2 |= eliminate_undemanded_extensions(&mut lines, &mut kinds, n);
            changed2 |= fold_into_memory_operands(&mut lines, &mut kinds, n);
            changed2 |= forward_fp_bit_moves(&mut lines, &mut kinds, n);
            changed2 |= fold_fp_bounces(&mut lines, &mut kinds, n);
            changed2 |= forward_fp_slot_reloads(&mut lines, &mut kinds, n);
            changed2 |= promote_fp_frame_slots(&mut lines, &mut kinds, n);
            changed2 |= forward_fp_copies(&mut lines, &mut kinds, n);
            changed2 |= retarget_fp_results(&mut lines, &mut kinds, n);
            // The forwarding passes above leave the stores they answered
            // dead; phase 2 judged the slots before they ran.
            changed2 |= global_dead_store_elimination(&lines, &mut kinds, n);
            rounds2 += 1;
        }
    }

    // Homes the copy propagation above left unread go with their saves; the
    // copies that fed them die in turn.
    if eliminate_unread_callee_saved_homes(&lines, &mut kinds, n) {
        eliminate_dead_defs_global(&lines, &mut kinds, n);
        eliminate_self_moves(&mut kinds, n);
    }
    drop_orphaned_save_rows(&lines, &mut kinds, n);
    // Last: only now is it known which frames the body no longer touches.
    eliminate_vestigial_frames(&lines, &mut kinds, n);

    // Restore protected functions byte-for-byte, including their original -g
    // rows and CFI. No placeholder is permitted to survive the optimizer.
    for (i, original) in protected {
        kinds[i] = if is_internal_marker(original.trim_start()) {
            LineKind::Nop
        } else {
            LineKind::Other
        };
        lines[i] = original;
    }
    // The call annotations end with the passes that read them.
    for line in lines.iter_mut() {
        if let Some(at) = line.find(CALL_READS_TAG) {
            line.truncate(at);
        }
    }
    let optimized = crate::backend::peephole_common::join_with_debug_rows(&lines[..n], |i| {
        kinds[i] != LineKind::Nop
    });
    crate::backend::peephole_common::strip_volatile_markers(optimized)
}

/// The text a bail-out returns -- and `LCCC_NO_PEEPHOLE`'s raw output: the
/// volatile markers and the prologue's return-register annotation exist only
/// for the passes.
pub(crate) fn strip_internal_markers(asm: String) -> String {
    let asm = crate::backend::peephole_common::strip_volatile_markers(asm);
    if !asm.contains(RET_REGS_MARKER)
        && !asm.contains(ARG_EXT_MARKER)
        && !asm.contains(CALL_READS_TAG)
    {
        return asm;
    }
    asm.split_inclusive('\n')
        .filter(|line| !is_internal_marker(line.trim_start()))
        .map(|line| match line.find(CALL_READS_TAG) {
            Some(at) => format!(
                "{}{}",
                &line[..at],
                if line.ends_with('\n') { "\n" } else { "" }
            ),
            None => line.to_string(),
        })
        .collect()
}

/// Drop each callee-saved `.cfi_offset` row whose save the passes deleted.
///
/// `emit_prologue_impl` emits `.cfi_offset sN, OFF` right after the `sd`
/// that saves sN. The entry coalescer deletes the restore of a coalesced
/// callee-saved home and the dead-store pass then deletes the save, whose
/// slot nothing reads any more; sN keeps the caller's value for the whole
/// function, so the right unwind rule is "same value", and a stale row would
/// make the unwinder load sN from a slot nothing wrote. A row survives
/// exactly when the nearest preceding live instruction is a store naming sN.
fn drop_orphaned_save_rows(lines: &[String], kinds: &mut [LineKind], n: usize) {
    fn callee_saved(reg: &str) -> bool {
        let num = |p: &str| {
            reg.strip_prefix(p)
                .and_then(|d| d.parse::<u8>().ok().filter(|v| v.to_string() == d))
        };
        num("s").is_some_and(|r| (1..=11).contains(&r)) || num("fs").is_some_and(|r| r <= 11)
    }
    let mut last: Option<usize> = None;
    for i in 0..n {
        match kinds[i] {
            LineKind::Nop => {}
            LineKind::Directive => {
                let Some(rest) = lines[i].trim().strip_prefix(".cfi_offset") else {
                    continue;
                };
                let reg = rest.split(',').next().unwrap_or("").trim();
                if !callee_saved(reg) {
                    continue;
                }
                let saved = last.is_some_and(|j| {
                    let t = lines[j].trim();
                    matches!(
                        t.split_whitespace().next(),
                        Some("sd" | "sw" | "fsd" | "fsw")
                    ) && t
                        .split('(')
                        .next()
                        .unwrap_or("")
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .any(|tok| tok == reg)
                });
                if !saved {
                    kinds[i] = LineKind::Nop;
                }
            }
            LineKind::Label => last = None,
            _ => last = Some(i),
        }
    }
}

// ── Pass 1: Adjacent store/load elimination ──────────────────────────────────
//
// Pattern: sd tX, off(s0)  ;  ld tX, off(s0)  (same reg, same offset)
// The load is redundant because the value is already in the register.
// Also handles: sd rX, off(s0)  ;  ld rY, off(s0)  → mv rY, rX
//
// A word pair is NOT an identity: `sw` stores the low 32 bits, and `lw`
// returns them SIGN-EXTENDED, whatever the register's upper half held.  LCCC
// keeps an `unsigned int` zero-extended, so `(int)u` through a frame slot is
// `sw t0; lw t0` with a real sign extension in the load (0x8000_0000 must
// come back as 0xffff_ffff_8000_0000).  The word form therefore becomes
// `sext.w rY, rX` -- one ALU op instead of a load, the same rule as AArch64's
// `str wN; ldrsw xM` -> `sxtw xM, wN` -- and `eliminate_redundant_sext_w`
// deletes it when rX is provably sign-extended already.

fn eliminate_adjacent_store_load(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::StoreS0 {
            reg: store_reg,
            offset: store_off,
            is_word: store_word,
        } = kinds[i]
        {
            // Look ahead for the matching load (skip Nops)
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j < n {
                if let LineKind::LoadS0 {
                    reg: load_reg,
                    offset: load_off,
                    is_word: load_word,
                } = kinds[j]
                {
                    if store_off == load_off && store_word == load_word {
                        forward_stored_value(lines, kinds, j, load_reg, store_reg, store_word);
                        changed = true;
                    }
                }
            }
        }
        i += 1;
    }
    changed
}

/// Replace the frame load at `j` with the value a same-sized store left in
/// `stored`: a doubleword reload is a copy (or nothing), a word reload is the
/// sign extension `lw` performs (see the pass header).
fn forward_stored_value(
    lines: &mut [String],
    kinds: &mut [LineKind],
    j: usize,
    load_reg: u8,
    stored: u8,
    is_word: bool,
) {
    if is_word {
        lines[j] = format!("    sext.w {}, {}", reg_name(load_reg), reg_name(stored));
        kinds[j] = LineKind::SextW {
            dst: load_reg,
            src: stored,
        };
    } else if load_reg == stored {
        kinds[j] = LineKind::Nop;
    } else {
        lines[j] = format!("    mv {}, {}", reg_name(load_reg), reg_name(stored));
        kinds[j] = LineKind::Move {
            dst: load_reg,
            src: stored,
        };
    }
}

// ── Pass 6: far-jump → near-jump rewrite (auipc;jr → jal) ───────────────────
//
// Codegen emits every unconditional transition as `jump .Lxxx, t6`, which the
// assembler encodes as auipc+jalr (2 instructions, 8 bytes, and a t6
// dependency). For targets inside the same function, the plain `j` pseudo
// (jal x0) is one 4-byte instruction — the recorded census item
// "auipc;jr vs j". jal's range is ±1 MiB; a rewrite is only sound when the
// target label provably lives in the same function AND the function is far
// below the range limit, so cross-function and huge-function jumps keep the
// unlimited-range form. Function extents are delimited by column-0 labels.

fn rewrite_far_jumps_to_near(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_JUMP_NEAR").is_ok() {
        return false;
    }
    // Function extents: [start, end) pairs delimited by column-0 labels.
    let mut changed = false;
    let mut start = 0;
    while start < n {
        let mut end = start + 1;
        while end < n
            && !(lines[end].starts_with(char::is_alphabetic) && kinds[end] != LineKind::Nop)
        {
            end += 1;
        }
        // A local-style function symbol does not necessarily delimit this
        // census. If any part is opaque, hidden directives can invalidate
        // its size estimate or hide the target's true function boundary.
        let mut range_ok = !lines[start..end].iter().any(|line| line == OPAQUE_LINE);
        // Budget up to 32 bytes per emitted instruction or pseudo, plus a
        // generous margin below the ±1 MiB jal limit. The old 100,000-line
        // limit was not safe even for its claimed 4x expansion allowance.
        let mut insn_count = 0usize;
        for k in start..end {
            if !matches!(
                kinds[k],
                LineKind::Label | LineKind::Nop | LineKind::Directive
            ) {
                insn_count += 1;
                if insn_count > 25_000 {
                    range_ok = false;
                    break;
                }
            }
        }
        if range_ok {
            // Label positions within this function.
            for k in start..end {
                if kinds[k] != LineKind::Jump {
                    continue;
                }
                let Some(target) = jump_target(&lines[k]) else {
                    continue;
                };
                if !lines[k].trim_start().starts_with("jump ") {
                    continue;
                }
                // Local label only (never a function symbol / tail call).
                if !target.starts_with(".L") {
                    continue;
                }
                let Some(target_pos) = (start..end).find(|&m| {
                    kinds[m] == LineKind::Label && label_name(&lines[m]) == Some(target)
                }) else {
                    continue;
                };
                let (a, b) = (k.min(target_pos), k.max(target_pos));
                // A directive such as `.space`, `.incbin`, `.rept` or `.insn`
                // can add an unbounded number of bytes and fails closed; the
                // bounded ones (CFI, `.loc`, alignment) are charged their
                // worst case against the function's instruction budget,
                // which stays far inside the jal's +/-1 MiB.
                let mut padding = 0usize;
                let mut bounded = true;
                for m in a + 1..b {
                    if kinds[m] == LineKind::Directive {
                        match directive_max_bytes(&lines[m]) {
                            Some(bytes) => padding += bytes,
                            None => {
                                bounded = false;
                                break;
                            }
                        }
                    }
                }
                if !bounded || insn_count.saturating_mul(32).saturating_add(padding) > 900_000 {
                    continue;
                }
                lines[k] = format!("    j {}", target);
                changed = true;
            }
        }
        start = end;
    }
    changed
}

// ── Branch-over-jump fusion ──────────────────────────────────────────────────
//
//     b<cond> rs..., .Lskip       b<!cond> rs..., .Ltarget
//     j/jump .Ltarget       ->
// .Lskip:                    .Lskip:
//
// This is the RV counterpart of ARM's branch-over-branch pass, but RV B-type
// branches have only a ±4 KiB reach.  Count every intervening source
// instruction as 32 bytes (the worst expansion of the pseudos emitted here)
// and retain a generous 1 KiB linker/relaxation margin.  The deliberately
// conservative bound trades a few missed folds for relocation-proof output.
/// Is `target` reachable from `from` inside the ±3 KB budget the range-gated
/// rewrites use?
///
/// The census charges a 32-byte worst case per line and fails closed on any
/// interval that contains a directive, a global label (a function boundary) or
/// an opaque line: `.space`, `.rept`, inline `.insn` and large alignments can
/// contribute an unbounded number of bytes that a line census cannot estimate.
/// A target that is not defined in this text has no census at all.
/// The first definition of every label, by name: built once per pass so
/// that resolving a branch target is a lookup, not a scan of the unit (the
/// scan made `fuse_branch_over_jump` quadratic -- 52 s of the 87 s that
/// gcc.c-torture execute/memclr.c's 2,816 functions took at -O0).
fn label_positions(lines: &[String], kinds: &[LineKind], n: usize) -> FxHashMap<String, usize> {
    let mut positions = FxHashMap::default();
    for m in 0..n {
        if kinds[m] == LineKind::Label {
            if let Some(name) = label_name(&lines[m]) {
                positions.entry(name.to_string()).or_insert(m);
            }
        }
    }
    positions
}

/// Whether a branch at `from` can reach the label at `to` in its +/-4 KiB:
/// no function boundary, directive or opaque line in between, and at most
/// 3,000 bytes of instructions counted at 32 bytes each (the far pseudo-ops
/// expand).  The scan stops at the first reason to refuse, so its cost is
/// bounded by the budget, not by the distance.
fn interval_is_near(lines: &[String], kinds: &[LineKind], from: usize, to: usize) -> bool {
    let (a, b) = if from < to { (from, to) } else { (to, from) };
    let mut insns = 0usize;
    let mut padding = 0usize;
    for m in a..=b {
        let interior = m > a && m < b;
        if interior
            && ((kinds[m] == LineKind::Label
                && !label_name(&lines[m]).unwrap_or("").starts_with('.'))
                || lines[m] == OPAQUE_LINE)
        {
            return false;
        }
        if interior && kinds[m] == LineKind::Directive {
            match directive_max_bytes(&lines[m]) {
                Some(bytes) => padding += bytes,
                None => return false,
            }
        }
        if !matches!(
            kinds[m],
            LineKind::Nop | LineKind::Label | LineKind::Directive
        ) {
            insns += 1;
        }
        if insns.saturating_mul(32).saturating_add(padding) > 3_000 {
            return false;
        }
    }
    true
}

/// The most bytes a directive between a branch and its target can add, or
/// `None` when it is not bounded by its own text (`.space`, `.rept`, `.insn`,
/// a section switch, anything unknown: fail closed).
///
/// The loop-alignment requests the code generator places before every loop
/// header -- `.p2align 4,,10` then `.p2align 3` -- are bounded: GAS pads to
/// the boundary but never by more than the alignment minus one byte, nor by
/// more than an explicit max-skip.  Refusing them, as the census used to,
/// kept every loop's entry and exit branches in their far-jump form
/// (`bge ...; jump .Lbody, t6; .Lskip: jump .Lexit, t6`).  RISC-V GAS reads
/// `.align N` as a power of two, like `.p2align`.
fn directive_max_bytes(line: &str) -> Option<usize> {
    let t = line.trim();
    let (name, args) = match t.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((name, args)) => (name, args.trim()),
        None => (t, ""),
    };
    if name.starts_with(".cfi_") || matches!(name, ".loc" | ".file") {
        return Some(0);
    }
    let mut fields = args.split(',').map(str::trim);
    let first: usize = fields.next()?.parse().ok()?;
    let _fill = fields.next();
    let max_skip: Option<usize> = match fields.next() {
        Some(text) if !text.is_empty() => Some(text.parse().ok()?),
        _ => None,
    };
    let span = match name {
        ".p2align" | ".align" => {
            if first > 12 {
                return None;
            }
            (1usize << first) - 1
        }
        ".balign" => first.checked_sub(1)?,
        _ => return None,
    };
    Some(max_skip.map_or(span, |m| span.min(m)))
}

fn fuse_branch_over_jump(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_BRANCH_OVER_JUMP").is_ok() {
        return false;
    }
    // Labels are never rewritten here, so one map serves the whole sweep.
    let labels = label_positions(lines, kinds, n);
    // Labels some other line names.  An unreferenced label is entered only by
    // falling into it, so stepping over it changes no path: the far-branch
    // expansion leaves one between the branch and the jump (`.Lskip_N:`) once
    // its own branch has been fused, and the loop header after the jump is
    // preceded by alignment requests.
    let mut referenced: FxHashSet<String> = FxHashSet::default();
    for k in 0..n {
        if !matches!(kinds[k], LineKind::Label | LineKind::Nop) {
            for tok in
                lines[k].split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            {
                if tok.starts_with(".L") {
                    referenced.insert(tok.to_string());
                }
            }
        }
    }
    fn skippable(
        lines: &[String],
        kinds: &[LineKind],
        k: usize,
        referenced: &FxHashSet<String>,
    ) -> bool {
        match kinds[k] {
            LineKind::Nop => true,
            LineKind::Directive => directive_max_bytes(&lines[k]).is_some(),
            LineKind::Label => label_name(&lines[k])
                .is_some_and(|name| name.starts_with(".L") && !referenced.contains(name)),
            _ => false,
        }
    }
    let mut changed = false;
    for i in 0..n {
        if kinds[i] != LineKind::Branch {
            continue;
        }
        let Some((op, operands, skip)) = branch_parts(&lines[i]) else {
            continue;
        };
        let mut j = i + 1;
        while j < n && skippable(lines, kinds, j, &referenced) {
            j += 1;
        }
        if j >= n || kinds[j] != LineKind::Jump {
            continue;
        }
        // The branch's own target must be the first label after the jump,
        // with nothing but byte-bounded padding and unreferenced labels
        // between.
        let mut k = j + 1;
        while k < n
            && (kinds[k] == LineKind::Nop
                || (kinds[k] == LineKind::Directive && directive_max_bytes(&lines[k]).is_some())
                || (kinds[k] == LineKind::Label
                    && label_name(&lines[k]) != Some(skip)
                    && skippable(lines, kinds, k, &referenced)))
        {
            k += 1;
        }
        if k >= n || kinds[k] != LineKind::Label {
            continue;
        }
        if label_name(&lines[k]) != Some(skip) {
            continue;
        }
        let Some(target) = jump_target(&lines[j]) else {
            continue;
        };
        // Only local labels in this function are candidates.  The census also
        // refuses intervals that cross a global symbol (a function boundary).
        if !target.starts_with(".L") {
            continue;
        }
        let Some(&to) = labels.get(target) else {
            continue;
        };
        if !interval_is_near(lines, kinds, i, to) {
            continue;
        }
        let Some(inv) = invert_branch_mnemonic(op) else {
            continue;
        };
        lines[i] = format!("    {} {}, {}", inv, operands, target);
        kinds[i] = LineKind::Branch;
        kinds[j] = LineKind::Nop;
        changed = true;
    }
    changed
}

// ── Pass 8c: Trampoline threading ────────────────────────────────────────────
//
// A local label whose body is nothing but an unconditional jump is a
// trampoline: every branch to it pays an extra fetch and an extra taken jump.
// Block shuffling, select lowering and identical-block merging leave these
// behind. Redirect the jumps straight at the end of the chain.
//
// The far `jump` pseudo-op reaches the whole text, so retargeting it can never
// change a range. The near `j` form is only retargeted when the same ±3 KB
// instruction census that gates the branch-over-jump fusion proves the new
// target reachable. The trampoline instruction itself is left in place:
// deleting a block is a control-flow change (fall-through entry, `lla` label
// identity) that the i686 donor pass proves with a module-wide reference
// census. The unreachable copies are cleaned up by the dead-code passes of
// later rounds.

/// Maximum number of trampoline hops followed before a chain is given up on.
const TRAMPOLINE_CHAIN_LIMIT: usize = 16;

/// Follow trampolines forwards from `start` to the first label whose body is not
/// a trampoline. Returns `None` for a chain that revisits itself.
fn resolve_trampoline<'a>(start: &str, direct: &'a FxHashMap<&'a str, &'a str>) -> Option<&'a str> {
    let mut current: &'a str = direct.get(start).copied()?;
    for _ in 0..TRAMPOLINE_CHAIN_LIMIT {
        if current == start {
            // The chain came back to the label it started from.
            return None;
        }
        match direct.get(current).copied() {
            Some(further) => current = further,
            None => return Some(current),
        }
    }
    None
}

/// Replace the target of a `j` / `jump` line, keeping the far pseudo's register
/// operand.
fn retarget_jump_line(line: &str, new_target: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("jump ") {
        let (_, tail) = rest.split_once(", ")?;
        return Some(format!("    jump {}, {}", new_target, tail));
    }
    trimmed.strip_prefix("j ")?;
    Some(format!("    j {}", new_target))
}

fn thread_trampolines(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_TRAMPOLINE_THREAD").is_ok() {
        return false;
    }
    // The analysis only borrows the text; the rewrites are the only mutation.
    let rewrites: Vec<(usize, String)> = {
        // A name that is defined twice cannot be resolved locally.
        let mut definitions: FxHashMap<&str, usize> = FxHashMap::default();
        for i in 0..n {
            if kinds[i] != LineKind::Label {
                continue;
            }
            if let Some(name) = label_name(&lines[i]) {
                *definitions.entry(name).or_insert(0) += 1;
            }
        }
        // Local label -> the target of the single unconditional jump it guards.
        let mut direct: FxHashMap<&str, &str> = FxHashMap::default();
        for i in 0..n {
            if kinds[i] != LineKind::Label {
                continue;
            }
            let Some(name) = label_name(&lines[i]) else {
                continue;
            };
            if !name.starts_with('.') || definitions.get(name) != Some(&1) {
                continue;
            }
            let Some(j) = (i + 1..n).find(|&pos| kinds[pos] != LineKind::Nop) else {
                continue;
            };
            if kinds[j] != LineKind::Jump {
                continue;
            }
            let Some(target) = jump_target(&lines[j]) else {
                continue;
            };
            if target != name {
                direct.insert(name, target);
            }
        }
        if direct.is_empty() {
            return false;
        }
        let mut resolved: FxHashMap<&str, &str> = FxHashMap::default();
        for &name in direct.keys() {
            if let Some(final_target) = resolve_trampoline(name, &direct) {
                resolved.insert(name, final_target);
            }
        }
        let positions = label_positions(lines, kinds, n);
        let mut rewrites = Vec::new();
        for i in 0..n {
            if kinds[i] != LineKind::Jump {
                continue;
            }
            let Some(target) = jump_target(&lines[i]) else {
                continue;
            };
            let Some(final_target) = resolved.get(target).copied() else {
                continue;
            };
            // The far `jump` pseudo reaches the whole text; the near `j` form
            // must be proven to reach the new target by the same census that
            // gates the branch-over-jump fusion.
            let is_far_pseudo = lines[i].trim().starts_with("jump ");
            let near = || {
                positions
                    .get(final_target)
                    .is_some_and(|&to| interval_is_near(lines, kinds, i, to))
            };
            if !is_far_pseudo && !near() {
                continue;
            }
            if let Some(rewritten) = retarget_jump_line(&lines[i], final_target) {
                rewrites.push((i, rewritten));
            }
        }
        rewrites
    };
    let changed = !rewrites.is_empty();
    for (i, text) in rewrites {
        lines[i] = text;
    }
    changed
}

// ── Pass 7: redundant sext.w elimination ────────────────────────────────────
//
// `sext.w rD, rS` re-sign-extends the low 32 bits. It is a no-op when rS
// already holds a sign-extended-32 value ("sext-wide"). Within one basic
// block we track the sext-wide set conservatively:
//   li (imm12 sign-extended), lui, sext.w, W-suffix ALU (addw/subw/mulw/
//   divw/remw/sllw/srlw/sraw and their u-forms), lw, slli/srli with n>=1
//   (zero upper bits ⇒ sext-wide), srai, and mv from a sext-wide register
//   all PRODUCE sext-wide values; ld and 64-bit ALU (add/sub/mul/...),
//   lla/la, jal/jalr results do not. A call clears every caller-saved
//   register; labels and branches clear everything (join point).
// Only registers whose value provably equals signext(low32) may have the
// following sext.w deleted — a missed deletion is a missed opt, never a
// miscompile.

fn sext_producing_def(line: &str, kind: LineKind) -> Option<(u8, bool)> {
    // (dst, is_sext_wide_producer) — None for unknown lines.
    let t = line.trim();
    let (mnem, rest) = t.split_once(' ')?;
    match kind {
        // `li` writes its 64-bit constant: a sign-extended word only inside
        // the signed 32-bit range. `int f(void) { long c = 0x100000001; return
        // (int)c; }` at -O0 is `li t0, 4294967297; sext.w a0, t0`, and the
        // sign extension is the psABI's int result (1), not a copy.
        LineKind::LoadImm { dst } => {
            let wide = rest
                .split_once(',')
                .and_then(|(_, imm)| parse_asm_i64(imm.trim()))
                .is_some_and(|v| i32::try_from(v).is_ok());
            Some((dst, wide))
        }
        LineKind::SextW { dst, .. } => Some((dst, true)),
        LineKind::Move { dst, .. } => Some((dst, false)), // actual update below
        LineKind::LoadS0 { reg, is_word, .. } => Some((reg, is_word)), // lw yes, ld no
        LineKind::LoadAddr { dst } => Some((dst, false)),
        LineKind::Alu => {
            let dst = parse_reg(rest.split(',').next()?.trim());
            if dst == REG_NONE {
                return None;
            }
            // Sext-wide producers: only instructions whose RV64 semantics
            // GUARANTEE sext32 canonical results whatever their input — the
            // *w family (register and immediate forms), lui (sign-extended
            // imm20), and `srai` by 32 or more, which leaves at least 33
            // copies of bit 63 on top.  A shorter `srai` keeps sext32 only
            // of a sext32 input, which this def-local view cannot see:
            // `srai rX,rX,3` of 0x10_0000_0000 is 0x2_0000_0000, so a
            // following `sext.w` (a `long` -> `int` truncation) is real
            // code.  slli/srli are 64-bit shifts: their results are NOT
            // sext32 in general (e.g. `slli rX,rX,32` of a negative sext32
            // value zero-fills the top half), so a following
            // `sext.w rX,rX` is real code, never a no-op.
            let long_srai = mnem == "srai"
                && rest
                    .rsplit(',')
                    .next()
                    .and_then(|imm| parse_asm_i64(imm.trim()))
                    .is_some_and(|k| (32..64).contains(&k));
            let wide = matches!(
                mnem,
                "addw"
                    | "subw"
                    | "mulw"
                    | "divw"
                    | "divuw"
                    | "remw"
                    | "remuw"
                    | "sllw"
                    | "srlw"
                    | "sraw"
                    | "addiw"
                    | "slliw"
                    | "srliw"
                    | "sraiw"
            ) || mnem == "lui"
                || long_srai;
            Some((dst, wide))
        }
        // FP results in a general register: the word conversions and
        // `fmv.x.w` sign-extend their 32-bit result to XLEN, the comparisons
        // write 0 or 1 and `fclass` a 10-bit mask; the doubleword forms are
        // full 64-bit values.
        LineKind::Other => {
            let dst = rv_fp_gp_dest(line)?;
            let wide = match mnem.split('.').collect::<Vec<_>>()[..] {
                ["fcvt", int, _] => matches!(int, "w" | "wu"),
                ["fmv", "x", fmt] => fmt == "w",
                [op, _] => matches!(op, "feq" | "flt" | "fle" | "fclass"),
                _ => false,
            };
            Some((dst, wide))
        }
        _ => None,
    }
}

/// What a line writes to the general registers, as far as the
/// function-wide sign-extension facts of [`rv_invariantly_wide`] care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordDef {
    /// Writes no general register.
    Nothing,
    /// Writes the register with a sign-extended word.
    Wide(u8),
    /// `mv dst, src`: a sign-extended word exactly when `src` holds one.
    Copy(u8, u8),
    /// Writes these registers with values that need not be sign-extended
    /// words.
    Clobbers(u64),
    /// A line whose general-register writes this cannot tell.
    Unknown,
}

fn rv_word_def(line: &str, kind: LineKind) -> WordDef {
    match kind {
        LineKind::Nop
        | LineKind::Label
        | LineKind::Directive
        | LineKind::StoreS0 { .. }
        | LineKind::Ret
        | LineKind::Branch => return WordDef::Nothing,
        LineKind::Call => return WordDef::Clobbers(RV_CALLER_SAVED),
        LineKind::Move { dst, src } => return WordDef::Copy(dst, src),
        LineKind::Jump => {
            // `jump label, t6` writes its scratch register, `j label` none.
            let t = line.trim();
            if !t.starts_with("jump ") {
                return WordDef::Nothing;
            }
            let scratch = t
                .rsplit(',')
                .next()
                .map_or(REG_NONE, |r| parse_reg(r.trim()));
            return if scratch == REG_NONE {
                WordDef::Unknown
            } else {
                WordDef::Clobbers(rv_bit(scratch))
            };
        }
        _ => {}
    }
    if rv_is_store(line) || (kind == LineKind::Other && rv_is_pure_fp_line(line)) {
        return WordDef::Nothing;
    }
    if kind == LineKind::Other
        && let Some(dst) = rv_load_dest(line)
    {
        // `lw`/`lh`/`lb` sign-extend from bit 31 or below, `lhu`/`lbu`
        // zero-extend fewer than 32 bits; `ld` and `lwu` do neither.
        let mnemonic = line.trim().split_whitespace().next().unwrap_or("");
        return if matches!(mnemonic, "lw" | "lh" | "lb" | "lhu" | "lbu") {
            WordDef::Wide(dst)
        } else {
            WordDef::Clobbers(rv_bit(dst))
        };
    }
    match sext_producing_def(line, kind) {
        Some((dst, true)) => WordDef::Wide(dst),
        Some((dst, false)) => WordDef::Clobbers(rv_bit(dst)),
        None => WordDef::Unknown,
    }
}

/// The general registers that hold a sign-extended word at every point of
/// the function `[start, end)` that reads them: no path reads the value the
/// register had on entry (it is not live there), and every line that
/// writes it writes such a word -- directly, or by copying a register of
/// the same set (the greatest such set: a register whose only writes are
/// copies of each other is never read before one of them executes, and the
/// first to execute would read a register live on entry). A hoisted loop
/// constant (`li a3, 50` in the preheader) is the case the block-local
/// tracking of [`eliminate_redundant_sext_w`] cannot see: its `int` compare
/// re-extended it on every iteration (`sext.w t2, a3`). Any line whose
/// writes are not modelled, or an unresolvable CFG, gives no facts.
fn rv_invariantly_wide(lines: &[String], kinds: &[LineKind], start: usize, end: usize) -> u64 {
    let mut defined = 0u64;
    let mut clobbered = 0u64;
    let mut copies: Vec<(u8, u8)> = Vec::new();
    for k in start..end {
        match rv_word_def(&lines[k], kinds[k]) {
            WordDef::Nothing => {}
            WordDef::Wide(r) => defined |= rv_bit(r),
            WordDef::Copy(dst, src) => {
                defined |= rv_bit(dst);
                copies.push((dst, src));
            }
            WordDef::Clobbers(mask) => {
                defined |= mask;
                clobbered |= mask;
            }
            WordDef::Unknown => return 0,
        }
    }
    let candidates = defined & !clobbered & !rv_bit(REG_SP) & !rv_bit(REG_RA);
    if candidates == 0 {
        return 0;
    }
    let Some(live) = RvLiveness::build(lines, kinds, start, end) else {
        return 0;
    };
    let mut wide = candidates & !live.live_before.first().copied().unwrap_or(RV_LIVE_ALL);
    loop {
        let next = copies.iter().fold(wide, |acc, &(dst, src)| {
            if wide & rv_bit(src) == 0 {
                acc & !rv_bit(dst)
            } else {
                acc
            }
        });
        if next == wide {
            return wide;
        }
        wide = next;
    }
}

fn eliminate_redundant_sext_w(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_SEXT_ELIM").is_ok() {
        return false;
    }
    let mut changed = false;
    // The registers that hold a sign-extended word wherever their function
    // reads them (`rv_invariantly_wide`): facts no join or boundary ends.
    let mut invariant = vec![0u64; n];
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        let wide = rv_invariantly_wide(lines, kinds, start, end);
        if wide != 0 {
            invariant[start..end].fill(wide);
        }
    }
    // sext-wide set: bit per register id (0..=31 GPRs).
    let mut wide: u64 = 0;
    let mut i = 0;
    while i < n {
        match kinds[i] {
            LineKind::Label | LineKind::Branch | LineKind::Ret | LineKind::Jump => {
                wide = invariant[i]; // join point / boundary
                i += 1;
                continue;
            }
            LineKind::Call => {
                // Caller-saved: t0-t6 (0..=6) and a0-a7 (30..=37). Keep s-regs.
                for r in 0..=6u8 {
                    wide &= !(1u64 << r);
                }
                for r in 30..=37u8 {
                    wide &= !(1u64 << r);
                }
                i += 1;
                continue;
            }
            // The entry contract: these argument registers arrive
            // sign-extended from bit 31. The line follows the function label
            // (which cleared the set) and precedes every instruction.
            LineKind::Nop if lines[i].trim_start().starts_with(ARG_EXT_MARKER) => {
                for (reg, _) in arg_ext_facts(lines[i].trim_start()) {
                    wide |= 1u64 << reg;
                }
                i += 1;
                continue;
            }
            // A frame store defines no register: every fact survives it, so
            // the `sext.w` a forwarded word reload became (see
            // `forward_stored_value`) still meets the producer's proof.
            LineKind::Nop | LineKind::Directive | LineKind::StoreS0 { .. } => {
                i += 1;
                continue;
            }
            _ => {}
        }
        // sext.w candidate. Even when the operation is a value no-op the
        // instruction DEFINES rD, and cross-register consumers (`bne t2, t1`
        // after `sext.w t2, t0`) read that definition -- deleting the
        // cross-register form leaves them with a stale register
        // (cfg_phi_web miscompile class). So the self form `sext.w rX, rX`
        // of a provably sext-wide rX goes, and the cross-register form
        // becomes the copy it amounts to (which copy propagation and the
        // dead-definition passes then fold): `unsigned parity(unsigned x)`
        // read its sign-extended argument through `sext.w t0, a0`.
        if let LineKind::SextW { dst, src } = kinds[i] {
            if wide & (1u64 << src) != 0 {
                if dst == src {
                    kinds[i] = LineKind::Nop;
                } else {
                    kinds[i] = LineKind::Move { dst, src };
                    lines[i] = format!("    mv {}, {}", reg_name(dst), reg_name(src));
                }
                changed = true;
            }
            wide |= 1u64 << dst; // sext.w always produces a sext-wide dest
            i += 1;
            continue;
        }
        // Any other store defines no GP register either (RISC-V has no
        // writeback addressing): keep every fact across it -- and so does an
        // FP computation, load or move that writes no general register.
        if rv_is_store(lines[i].trim())
            || (kinds[i] == LineKind::Other && rv_is_pure_fp_line(&lines[i]))
        {
            i += 1;
            continue;
        }
        // Def tracking.
        let (dst, producer_sext) = match sext_producing_def(&lines[i], kinds[i]) {
            Some(x) => x,
            None => {
                // Unknown line: clear nothing specific, be safe — clear all
                // (but the function-wide facts, which such a line would have
                // prevented).
                wide = invariant[i];
                i += 1;
                continue;
            }
        };
        match kinds[i] {
            LineKind::Move { src, .. } => {
                if wide & (1u64 << src) != 0 {
                    wide |= 1u64 << dst;
                } else {
                    wide &= !(1u64 << dst);
                }
            }
            _ => {
                if producer_sext {
                    wide |= 1u64 << dst;
                } else {
                    wide &= !(1u64 << dst);
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Zero extension consumed only by a sign extension ─────────────────────────
//
// `slli rX, rX, 32; srli rX, rX, 32; sext.w rX, rX` is `sext.w rX, rX`: the
// sign extension reads only the low word, which the zero extension leaves
// unchanged, and it overwrites rX, so nothing observes the pair.  This is an
// `unsigned int` parameter handed back as is: the entry normalises it to
// LCCC's internal zero-extended form and the psABI return sign-extends it
// again (`unsigned f(unsigned x) { return x; }`: 5 -> 2 instructions with the
// dead-temporary rule in `eliminate_dead_reg_moves`; GCC: `ret`).  The three
// lines must be adjacent apart from deleted ones: a label between them is a
// join whose other predecessors this local view cannot see.
fn fold_zext32_into_sext_w(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for k in 0..n {
        let LineKind::SextW { dst, src } = kinds[k] else {
            continue;
        };
        if dst != src || dst == REG_NONE {
            continue;
        }
        let Some(j) = previous_live_line(kinds, k) else {
            continue;
        };
        let Some(i) = previous_live_line(kinds, j) else {
            continue;
        };
        if is_self_shift_by_32(&lines[j], "srli", dst)
            && is_self_shift_by_32(&lines[i], "slli", dst)
        {
            kinds[i] = LineKind::Nop;
            kinds[j] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

/// The nearest line before `k` that has not been deleted.
fn previous_live_line(kinds: &[LineKind], k: usize) -> Option<usize> {
    (0..k).rev().find(|&i| kinds[i] != LineKind::Nop)
}

/// `MNEM rX, rX, 32` where rX is the tracked register `reg`.
fn is_self_shift_by_32(line: &str, mnem: &str, reg: u8) -> bool {
    let Some((m, rest)) = line.trim().split_once(' ') else {
        return false;
    };
    if m != mnem {
        return false;
    }
    let mut ops = rest.split(',').map(str::trim);
    matches!(
        (ops.next(), ops.next(), ops.next(), ops.next()),
        (Some(d), Some(s), Some(amount), None)
            if parse_reg(d) == reg && parse_reg(s) == reg && parse_asm_i64(amount) == Some(32)
    )
}

// ── Narrow-value lattice and redundant zero-extension elimination ────────────
//
// This is the RV64 adaptation of AArch64's repeated-extension elimination and
// x86's upper-bit knowledge: track the maximum number of potentially nonzero
// low bits in each register.  The lattice recognises ten target constructs:
// lbu/lhu/lwu, nonnegative li, mv, seqz/snez/slt*, positive andi masks,
// logical and/or/xor, srli, slli, and the canonical slli+srli zext.w pair.
// Unknown instructions and CFG joins fail closed.  Calls retain only the
// callee-saved s-register facts.
fn eliminate_redundant_zext_byte(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_ZEXT_BYTE_ELIM").is_ok()
        || std::env::var("CCC_NO_NARROW_VALUE_ELIM").is_ok()
    {
        return false;
    }
    fn bits_for_nonnegative(v: i64) -> u8 {
        if v <= 0 {
            1
        } else {
            (64 - (v as u64).leading_zeros()) as u8
        }
    }
    fn set_width(widths: &mut [u8; NUM_REGS], reg: u8, width: u8) {
        if (reg as usize) < NUM_REGS {
            widths[reg as usize] = if width == 0 || width >= 64 { 0 } else { width };
        }
    }

    let mut widths = [0u8; NUM_REGS]; // 0 = unknown/full 64-bit
    let mut live = RvLazyLiveness::new(lines, kinds, n);
    let mut changed = false;
    let mut i = 0;
    while i < n {
        match kinds[i] {
            LineKind::Label | LineKind::Branch | LineKind::Jump | LineKind::Ret => {
                widths.fill(0);
                i += 1;
                continue;
            }
            LineKind::Call => {
                for r in 0..=6usize {
                    widths[r] = 0;
                }
                for r in 30..=37usize {
                    widths[r] = 0;
                }
                widths[REG_RA as usize] = 0;
                i += 1;
                continue;
            }
            // The entry contract (see `ARG_EXT_MARKER`): an `unsigned
            // char`/`unsigned short` argument arrives zero-extended.
            LineKind::Nop if lines[i].trim_start().starts_with(ARG_EXT_MARKER) => {
                for (reg, ext) in arg_ext_facts(lines[i].trim_start()) {
                    if let ArgExt::Zero(bits) = ext {
                        set_width(&mut widths, reg, bits);
                    }
                }
                i += 1;
                continue;
            }
            LineKind::Nop | LineKind::Directive => {
                i += 1;
                continue;
            }
            _ => {}
        }

        // Own the current text because successful folds rewrite `lines[i]`
        // while mnemonic/operand slices remain live for lattice bookkeeping.
        let text = lines[i].trim().to_owned();
        let (mnem, operands) = match text.split_once(' ') {
            Some(parts) => parts,
            None => {
                widths.fill(0);
                i += 1;
                continue;
            }
        };
        let ops: Vec<&str> = operands.split(',').map(str::trim).collect();

        // Canonical RV64 zero-extensions: slli d,s,K; srli d,d,K for K = 32
        // (zext.w), 48 (a halfword) and 56 (a byte).  If s is already that
        // narrow, preserve the destination definition with a move (or delete
        // both for the self form).
        let zext_shift = (mnem == "slli" && ops.len() == 3)
            .then(|| parse_asm_i64(ops[2]))
            .flatten()
            .filter(|k| matches!(k, 32 | 48 | 56));
        if let Some(k) = zext_shift {
            let dst = parse_reg(ops[0]);
            let src = parse_reg(ops[1]);
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if dst != REG_NONE
                && src != REG_NONE
                && widths[src as usize] != 0
                && i64::from(widths[src as usize]) <= 64 - k
                && j < n
            {
                let jt = lines[j].trim();
                let jops: Vec<&str> = jt
                    .strip_prefix("srli ")
                    .map(|s| s.split(',').map(str::trim).collect())
                    .unwrap_or_default();
                // `srli e, dst, K` into another register leaves `dst` holding
                // the shifted value, which must then be dead (the pair is
                // `slli t0, t0, 32; srli a2, t0, 32` after `srliw t0, t0, 7`).
                let e = jops.first().map_or(REG_NONE, |o| parse_reg(o));
                if jops.len() == 3
                    && e != REG_NONE
                    && parse_reg(jops[1]) == dst
                    && parse_asm_i64(jops[2]) == Some(k)
                    && (e == dst || live.dead_after(lines, kinds, j, dst))
                {
                    if e == src {
                        kinds[i] = LineKind::Nop;
                    } else {
                        lines[i] = format!("    mv {}, {}", reg_name(e), reg_name(src));
                        kinds[i] = LineKind::Move { dst: e, src };
                    }
                    kinds[j] = LineKind::Nop;
                    let src_width = widths[src as usize];
                    set_width(&mut widths, e, src_width);
                    live.invalidate();
                    changed = true;
                    i = j + 1;
                    continue;
                }
            }
        }

        // Unsigned loads establish width regardless of their addressing mode.
        if let Some(width) = match mnem {
            "lbu" => Some(8),
            "lhu" => Some(16),
            "lwu" | "zext.w" => Some(32),
            _ => None,
        } {
            let dst = parse_reg(ops.first().copied().unwrap_or(""));
            if dst == REG_NONE {
                widths.fill(0);
            } else {
                set_width(&mut widths, dst, width);
            }
            i += 1;
            continue;
        }
        // Signed/full-width loads kill only their destination proof; ordinary
        // stores define no GP register on RISC-V.  Keeping unrelated facts
        // across these fully modelled instructions is safe and materially
        // improves AND-with-a-narrow-operand cases.
        if matches!(mnem, "lb" | "lh" | "lw" | "ld") {
            let dst = parse_reg(ops.first().copied().unwrap_or(""));
            if dst == REG_NONE {
                widths.fill(0);
            } else {
                set_width(&mut widths, dst, 0);
            }
            i += 1;
            continue;
        }
        if matches!(mnem, "sb" | "sh" | "sw" | "sd") {
            i += 1;
            continue;
        }

        if mnem == "andi" && ops.len() == 3 {
            let dst = parse_reg(ops[0]);
            let src = parse_reg(ops[1]);
            let imm = parse_asm_i64(ops[2]);
            if dst != REG_NONE && src != REG_NONE {
                if imm == Some(255) && widths[src as usize] != 0 && widths[src as usize] <= 8 {
                    if dst == src {
                        kinds[i] = LineKind::Nop;
                    } else {
                        lines[i] = format!("    mv {}, {}", reg_name(dst), reg_name(src));
                        kinds[i] = LineKind::Move { dst, src };
                    }
                    changed = true;
                }
                if let Some(v) = imm.filter(|v| *v >= 0) {
                    set_width(&mut widths, dst, bits_for_nonnegative(v));
                    i += 1;
                    continue;
                }
            }
        }

        match kinds[i] {
            LineKind::Move { dst, src } => {
                let src_width = widths[src as usize];
                set_width(&mut widths, dst, src_width);
            }
            LineKind::LoadImm { dst } => {
                let width = ops
                    .get(1)
                    .and_then(|imm| parse_asm_i64(imm))
                    .filter(|v| *v >= 0)
                    .map(bits_for_nonnegative)
                    .unwrap_or(0);
                set_width(&mut widths, dst, width);
            }
            LineKind::Alu
                if matches!(mnem, "seqz" | "snez" | "slt" | "sltu" | "slti" | "sltiu") =>
            {
                if let Some(dst) = parse_alu_dest(&lines[i]) {
                    set_width(&mut widths, dst, 1);
                } else {
                    widths.fill(0);
                }
            }
            LineKind::Alu if matches!(mnem, "and" | "or" | "xor") && ops.len() == 3 => {
                let dst = parse_reg(ops[0]);
                let a = parse_reg(ops[1]);
                let b = parse_reg(ops[2]);
                if dst == REG_NONE || a == REG_NONE || b == REG_NONE {
                    widths.fill(0);
                } else {
                    let (wa, wb) = (widths[a as usize], widths[b as usize]);
                    let w = if mnem == "and" {
                        match (wa, wb) {
                            (0, 0) => 0,
                            (0, x) | (x, 0) => x,
                            (x, y) => x.min(y),
                        }
                    } else if wa != 0 && wb != 0 {
                        wa.max(wb)
                    } else {
                        0
                    };
                    set_width(&mut widths, dst, w);
                }
            }
            // `srliw d, s, K` (K >= 1) is the low word shifted right and
            // sign-extended from a cleared bit 31: at most 32 - K low bits.
            LineKind::Alu if mnem == "srliw" && ops.len() == 3 => {
                let dst = parse_reg(ops[0]);
                let src = parse_reg(ops[1]);
                let sh = parse_asm_i64(ops[2]).filter(|v| (1..32).contains(v));
                match (dst, src, sh) {
                    (d, s, Some(sh)) if d != REG_NONE && s != REG_NONE => {
                        let sw = widths[s as usize];
                        let sh = sh as u8;
                        let w = if sw == 0 || sw > 32 {
                            32 - sh
                        } else {
                            sw.saturating_sub(sh).max(1)
                        };
                        set_width(&mut widths, d, w);
                    }
                    (d, _, _) if d != REG_NONE => set_width(&mut widths, d, 0),
                    _ => widths.fill(0),
                }
            }
            LineKind::Alu if mnem == "srli" && ops.len() == 3 => {
                let dst = parse_reg(ops[0]);
                let src = parse_reg(ops[1]);
                let sh = parse_asm_i64(ops[2]).filter(|v| (0..64).contains(v));
                if dst == REG_NONE || src == REG_NONE || sh.is_none() {
                    widths.fill(0);
                } else {
                    let sh = sh.unwrap() as u8;
                    let sw = widths[src as usize];
                    let w = if sw == 0 {
                        64 - sh
                    } else {
                        sw.saturating_sub(sh).max(1)
                    };
                    set_width(&mut widths, dst, w);
                }
            }
            LineKind::Alu if mnem == "slli" && ops.len() == 3 => {
                let dst = parse_reg(ops[0]);
                let src = parse_reg(ops[1]);
                let sh = parse_asm_i64(ops[2]).filter(|v| (0..64).contains(v));
                if dst == REG_NONE || src == REG_NONE || sh.is_none() {
                    widths.fill(0);
                } else {
                    let sw = widths[src as usize];
                    let w = if sw != 0 {
                        sw.saturating_add(sh.unwrap() as u8)
                    } else {
                        0
                    };
                    set_width(&mut widths, dst, w);
                }
            }
            _ => match sext_producing_def(&lines[i], kinds[i]) {
                Some((dst, _)) => set_width(&mut widths, dst, 0),
                None => widths.fill(0),
            },
        }
        i += 1;
    }
    changed
}

// ── Value ranges: redundant sign extensions and masks ────────────────────────
//
// The signed and interval counterpart of `eliminate_redundant_zext_byte`:
// track, per register, an interval [lo, hi] its 64-bit value lies in (the
// whole i64 range when nothing is known).  The loads give their type's range,
// comparisons [0, 1], non-negative masks [0, mask], `srli`/`srai` shift the
// bounds, `addi` and an in-range `addiw` offset them, the other `*w`
// producers and `lui` give the signed word range, moves copy.  Three
// redundancies follow:
//
// * the sign-extension `slli d,s,K; srai d,d,K` of a value that already fits
//   64 - K bits -- the compare staging of a `short` loaded by `lh` (`lh t0;
//   slli t1,t0,48; srai t1,t1,48; slti`);
// * `sext.w d,s` of a value in [0, 2^31), which the word-def proof of
//   `eliminate_redundant_sext_w` does not model (`andi t0,t0,0xff; sext.w
//   t1,t0`).  Values that may be negative stay with that pass: a `lw` +
//   `sext.w` pair is the shape the memory-operand fold retargets into the
//   load;
// * a zero-extension of `s` to n bits -- `andi t,s,2^n-1` or `slli t,s,64-n;
//   srli t,t,64-n` -- whose only reader is `sltiu`/`slti d,t,k` or `xori
//   d,t,k` + a zero test, k in [0, 2047], when the range of `s` makes its low
//   n bits decide the comparison exactly as the whole register does:
//
//       t <u k  ==  s <u k   for every s in [k - 2^n, 2^n - 1]
//       t == k  ==  s == k   for every s in (k - 2^n, k + 2^n)
//
//   (below zero, t = s + 2^n is at least k while s is unsigned-huge; the
//   equality's window holds one value congruent to k).  `t` is non-negative,
//   so `slti` and `sltiu` agree on it; the compare then reads `s`, unsigned.
//   `(unsigned char)(c - 'a') <= 25` and `(unsigned)(c - '0') < 10` are
//   `addiw; sltiu` (Clang 23.1.5), no mask.
//
// Block-local facts: labels, branches, jumps and returns reset every range,
// calls the caller-saved ones, unknown instructions all of them.  Whether the
// dropped extension's result is dead comes from the function's global
// liveness, rebuilt after every rewrite.

/// An interval [lo, hi] a register's 64-bit value lies in.
type RvRange = (i64, i64);
const RV_FULL: RvRange = (i64::MIN, i64::MAX);
const RV_WORD: RvRange = (i32::MIN as i64, i32::MAX as i64);

/// Bits every value of `r` needs as a sign-extended number.
fn rv_range_width(r: RvRange) -> u32 {
    let signed_bits = |v: i64| 65 - (v ^ (v >> 63)).leading_zeros();
    signed_bits(r.0).max(signed_bits(r.1))
}

fn rv_range_within(r: RvRange, outer: RvRange) -> bool {
    outer.0 <= r.0 && r.1 <= outer.1
}

/// [0, 2^bits - 1], `bits` in 1..=63.
fn rv_unsigned_range(bits: u32) -> RvRange {
    (0, i64::MAX >> (63 - bits.clamp(1, 63)))
}

/// [-2^(bits-1), 2^(bits-1) - 1], `bits` in 1..=64.
fn rv_signed_range(bits: u32) -> RvRange {
    let shift = 64 - bits.clamp(1, 64);
    (i64::MIN >> shift, i64::MAX >> shift)
}

/// `r + imm` without wrapping, or the whole range.
fn rv_offset_range(r: RvRange, imm: i64) -> RvRange {
    match (r.0.checked_add(imm), r.1.checked_add(imm)) {
        (Some(lo), Some(hi)) => (lo, hi),
        _ => RV_FULL,
    }
}

/// The first line at or after `from` that is not deleted.
fn rv_next_kept_line(kinds: &[LineKind], from: usize, n: usize) -> usize {
    let mut j = from;
    while j < n && matches!(kinds[j], LineKind::Nop) {
        j += 1;
    }
    j
}

/// The operands of `line` when its mnemonic is `mnemonic`.
fn rv_operands_of(line: &str, mnemonic: &str) -> Vec<String> {
    line.trim()
        .strip_prefix(mnemonic)
        .filter(|rest| rest.starts_with(' '))
        .map(|rest| rest.split(',').map(|o| o.trim().to_string()).collect())
        .unwrap_or_default()
}

/// The liveness of one function of the text, built on demand and dropped
/// whenever a rewrite changes the function.
struct RvLazyLiveness {
    funcs: Vec<(usize, usize)>,
    cached: Option<(usize, Option<RvLiveness>)>,
}

impl RvLazyLiveness {
    fn new(lines: &[String], kinds: &[LineKind], n: usize) -> Self {
        RvLazyLiveness {
            funcs: rv_function_ranges(lines, kinds, n),
            cached: None,
        }
    }

    fn invalidate(&mut self) {
        self.cached = None;
    }

    /// Whether `reg` is dead after line `k` -- after a branch, on both edges.
    /// Without a resolvable CFG only a scratch register's block-local proof
    /// answers, and it sees no taken edge.
    fn dead_after(&mut self, lines: &[String], kinds: &[LineKind], k: usize, reg: u8) -> bool {
        let Some(f) = self.funcs.iter().position(|&(s, e)| s <= k && k < e) else {
            return false;
        };
        if self.cached.as_ref().map(|c| c.0) != Some(f) {
            let (s, e) = self.funcs[f];
            self.cached = Some((f, RvLiveness::build(lines, kinds, s, e)));
        }
        match self.cached.as_ref().and_then(|c| c.1.as_ref()) {
            Some(live) => reg != REG_NONE && live.live_after(k) & rv_bit(reg) == 0,
            None => {
                reg <= REG_T6
                    && !matches!(kinds[k], LineKind::Branch | LineKind::Jump)
                    && scratch_dead_after(lines, kinds, k + 1, reg, self.funcs[f].1)
            }
        }
    }
}

/// The zero-extension at line `i` (`andi t,s,2^n-1`, or `slli t,s,64-n` +
/// `srli t,t,64-n`): `(t, s, n, last line)`.
fn rv_zero_extension(
    lines: &[String],
    kinds: &[LineKind],
    n: usize,
    i: usize,
) -> Option<(u8, u8, u32, usize)> {
    let text = lines[i].trim();
    let (mnem, rest) = text.split_once(' ')?;
    let ops: Vec<&str> = rest.split(',').map(str::trim).collect();
    if ops.len() != 3 {
        return None;
    }
    let (t, s) = (parse_reg(ops[0]), parse_reg(ops[1]));
    if t == REG_NONE || s == REG_NONE {
        return None;
    }
    let imm = parse_asm_i64(ops[2])?;
    match mnem {
        "andi" if (1..=2047).contains(&imm) && (imm & (imm + 1)) == 0 => {
            Some((t, s, (imm + 1).trailing_zeros(), i))
        }
        "slli" if (32..64).contains(&imm) => {
            let j = rv_next_kept_line(kinds, i + 1, n);
            let srli = rv_operands_of(lines.get(j).map_or("", String::as_str), "srli");
            (j < n
                && srli.len() == 3
                && parse_reg(&srli[0]) == t
                && parse_reg(&srli[1]) == t
                && parse_asm_i64(&srli[2]) == Some(imm))
            .then_some((t, s, (64 - imm) as u32, j))
        }
        _ => None,
    }
}

/// Drop the zero-extension at line `i` when its only reader is a compare its
/// source's range `r` decides the same way (see the section comment), and
/// rewrite the compare to read the source.  The compare's line on success.
fn rv_fold_extension_into_compare(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
    i: usize,
    rng: &[RvRange; NUM_REGS],
    live: &mut RvLazyLiveness,
) -> Option<usize> {
    let (t, src, bits, last) = rv_zero_extension(lines, kinds, n, i)?;
    let r = rng.get(src as usize).copied().unwrap_or(RV_FULL);
    let m = 1i64 << bits;
    let c = rv_next_kept_line(kinds, last + 1, n);
    let ctext = lines.get(c)?.trim().to_owned();
    let (cm, crest) = ctext.split_once(' ')?;
    let cops: Vec<&str> = crest.split(',').map(str::trim).collect();
    if cops.len() != 3 || parse_reg(cops[1]) != t {
        return None;
    }
    let cdst = parse_reg(cops[0]);
    let k = parse_asm_i64(cops[2]).filter(|k| (0..=2047).contains(k))?;
    if cdst == REG_NONE {
        return None;
    }
    let replacement = match cm {
        "sltiu" | "slti" if k <= m && rv_range_within(r, (k - m, m - 1)) => {
            format!("    sltiu {}, {}, {k}", reg_name(cdst), reg_name(src))
        }
        "xori" if k < m && rv_range_within(r, (k - m + 1, k + m - 1)) => {
            // The difference itself differs: only a zero test of it may
            // follow, and nothing else may read it.
            let z = rv_next_kept_line(kinds, c + 1, n);
            let line = lines.get(z).map_or("", String::as_str);
            let mut zops = rv_operands_of(line, "seqz");
            if zops.is_empty() {
                zops = rv_operands_of(line, "snez");
            }
            let zero_test_only = zops.len() == 2
                && parse_reg(&zops[1]) == cdst
                && (parse_reg(&zops[0]) == cdst || live.dead_after(lines, kinds, z, cdst));
            if !zero_test_only {
                return None;
            }
            format!("    xori {}, {}, {k}", reg_name(cdst), reg_name(src))
        }
        _ => return None,
    };
    if cdst != t && !live.dead_after(lines, kinds, c, t) {
        return None;
    }
    lines[c] = replacement;
    kinds[i..=last].fill(LineKind::Nop);
    live.invalidate();
    Some(c)
}

/// How a line reads only the sign of register `t`: `slti d,t,0`,
/// `slt d,t,zero` and `srli d,t,63` leave the sign bit in `d`; `bltz`/`bgez`
/// (and their `blt`/`bge`/`bgt`/`ble` spellings against `zero`) branch on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RvSignReader {
    Value(u8),
    Branch,
}

fn rv_sign_only_reader(line: &str, t: u8) -> Option<RvSignReader> {
    let (mnem, rest) = line.trim().split_once(' ')?;
    let ops: Vec<&str> = rest.split(',').map(str::trim).collect();
    let is_t = |o: &str| parse_reg(o) == t;
    let is_zero = |o: &str| o == "zero" || o == "x0";
    let value = |d: &str| {
        let d = parse_reg(d);
        (d != REG_NONE).then_some(RvSignReader::Value(d))
    };
    match (mnem, ops.as_slice()) {
        ("slti", [d, s, k]) if is_t(s) && parse_asm_i64(k) == Some(0) => value(d),
        ("slt", [d, s, z]) if is_t(s) && is_zero(z) => value(d),
        ("srli", [d, s, k]) if is_t(s) && parse_asm_i64(k) == Some(63) => value(d),
        ("bltz" | "bgez", [s, _]) if is_t(s) => Some(RvSignReader::Branch),
        ("blt" | "bge", [s, z, _]) if is_t(s) && is_zero(z) => Some(RvSignReader::Branch),
        ("bgt" | "ble", [z, s, _]) if is_t(s) && is_zero(z) => Some(RvSignReader::Branch),
        _ => None,
    }
}

/// `slli t,s,K; srai t,t,K` (from line `i`) read only by a sign test: the
/// arithmetic shift leaves bit 63 where the left shift put it, so the test
/// reads the same bit without it (`(unsigned)(signed char)x < 300`, which
/// `cross_domain_sign_test` turns into `(signed char)x >= 0`, kept a
/// three-instruction extension-and-compare).  When `s` is a non-negative
/// value of at most 64 - K bits (an `lbu` for K = 56), a value-producing
/// test is that value's top bit, `srli d,s,63-K`, and the left shift goes
/// too; when it is negated next (`xori e,d,1`) and the bit fits an `andi`
/// immediate, the pair is `andi d,s,1<<(63-K); seqz e,d` -- Clang 23.1.5's
/// `andi a0, a0, 128; seqz a0, a0` for `(signed char)x >= 0`.  The line to
/// resume the scan at.
fn rv_sign_test_of_extension(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
    i: usize,
    src_range: RvRange,
    live: &mut RvLazyLiveness,
) -> Option<usize> {
    let shl = rv_operands_of(&lines[i], "slli");
    let (t, s) = (parse_reg(shl.first()?), parse_reg(shl.get(1)?));
    let k = parse_asm_i64(shl.get(2)?).filter(|k| (1..64).contains(k))?;
    if t == REG_NONE || s == REG_NONE {
        return None;
    }
    let j = rv_next_kept_line(kinds, i + 1, n);
    let sra = rv_operands_of(lines.get(j)?, "srai");
    if sra.len() != 3
        || parse_reg(&sra[0]) != t
        || parse_reg(&sra[1]) != t
        || parse_asm_i64(&sra[2]) != Some(k)
    {
        return None;
    }
    let c = rv_next_kept_line(kinds, j + 1, n);
    let reader = rv_sign_only_reader(lines.get(c)?, t)?;
    if reader != RvSignReader::Value(t) && !live.dead_after(lines, kinds, c, t) {
        return None;
    }
    let top = 63 - k;
    let RvSignReader::Value(d) = reader else {
        kinds[j] = LineKind::Nop;
        live.invalidate();
        return Some(i);
    };
    let negation = rv_next_kept_line(kinds, c + 1, n);
    let not = lines
        .get(negation)
        .map_or(Vec::new(), |l| rv_operands_of(l, "xori"));
    let e = not.first().map_or(REG_NONE, |o| parse_reg(o));
    if top <= 10
        && not.len() == 3
        && parse_reg(&not[1]) == d
        && parse_asm_i64(&not[2]) == Some(1)
        && e != REG_NONE
        && (e == d || live.dead_after(lines, kinds, negation, d))
    {
        lines[i] = format!("    andi {}, {}, {}", reg_name(d), reg_name(s), 1i64 << top);
        lines[negation] = format!("    seqz {}, {}", reg_name(e), reg_name(d));
        kinds[i] = classify_line(&lines[i]);
        kinds[negation] = classify_line(&lines[negation]);
        kinds[j] = LineKind::Nop;
        kinds[c] = LineKind::Nop;
    } else if rv_range_within(src_range, rv_unsigned_range((64 - k) as u32)) {
        lines[i] = format!("    srli {}, {}, {top}", reg_name(d), reg_name(s));
        kinds[i] = classify_line(&lines[i]);
        kinds[j] = LineKind::Nop;
        kinds[c] = LineKind::Nop;
    } else {
        kinds[j] = LineKind::Nop;
    }
    live.invalidate();
    Some(i)
}

/// `l{b,h,w}[u] s, M; slli t, s, K; srai d, t, K` with K = 64 - the load's
/// width (the shift at line `i`): the load's value extended the other way
/// round is the load of that signedness, `lb d, M` -- and a zero-extension
/// (`srli`) makes it `lbu`/`lhu`/`lwu`.  The access reads the same bytes
/// either way; GCC and Clang load `(signed char)p[i]` with `lb`, where the
/// emitter stages `lbu` and a shift pair.  The load must be the previous
/// kept line, `s` die at the left shift unless the pair overwrites it, and
/// `t` at the right shift unless it is `d`.  Returns the load's line, where
/// the scan resumes to record `d`'s new range.
fn rv_retype_extended_load(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
    i: usize,
    live: &mut RvLazyLiveness,
) -> Option<usize> {
    let shl = rv_operands_of(&lines[i], "slli");
    let [t, s, k] = shl.as_slice() else {
        return None;
    };
    let (t, s, k) = (parse_reg(t), parse_reg(s), parse_asm_i64(k)?);
    if t == REG_NONE || s == REG_NONE {
        return None;
    }
    let j = rv_next_kept_line(kinds, i + 1, n);
    let (signed, back) = match lines.get(j) {
        Some(line) if line.trim().starts_with("srai ") => (true, rv_operands_of(line, "srai")),
        Some(line) => (false, rv_operands_of(line, "srli")),
        None => return None,
    };
    let d = back.first().map_or(REG_NONE, |o| parse_reg(o));
    if back.len() != 3
        || d == REG_NONE
        || parse_reg(&back[1]) != t
        || parse_asm_i64(&back[2]) != Some(k)
    {
        return None;
    }
    let p = (0..i).rev().find(|&q| kinds[q] != LineKind::Nop)?;
    let (m, ops) = lines[p].trim().split_once(' ')?;
    let (dest, mem) = ops.split_once(',')?;
    let mem = mem.trim();
    if parse_reg(dest.trim()) != s || !mem.ends_with(')') {
        return None;
    }
    let width = match m {
        "lb" | "lbu" => 8,
        "lh" | "lhu" => 16,
        "lw" | "lwu" => 32,
        _ => return None,
    };
    if k != 64 - width
        || (s != t && s != d && !live.dead_after(lines, kinds, i, s))
        || (t != d && !live.dead_after(lines, kinds, j, t))
    {
        return None;
    }
    let base = &m[..2];
    let new_m = if signed {
        base.to_string()
    } else {
        format!("{base}u")
    };
    lines[p] = format!("    {new_m} {}, {mem}", reg_name(d));
    kinds[p] = classify_line(&lines[p]);
    kinds[i] = LineKind::Nop;
    kinds[j] = LineKind::Nop;
    live.invalidate();
    Some(p)
}

/// `slti t, x, 0; neg d, t` broadcasts the sign of `x`: `srai d, x, 63`.
/// `negw`, `sub d, zero, t` and `subw d, zero, t` alike -- the negation of 0
/// or 1 is 0 or all ones in either width.  Clang 23.1.5 spells `-(x < 0)`
/// as the one shift (`srai`), where the emitter stages a compare and a
/// negation.  The negation must be the compare's next kept line and `t` die
/// there unless it is `d`; the shift writes `d` where the compare wrote
/// `t`, with nothing between.
fn fold_sign_masks(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut live = RvLazyLiveness::new(lines, kinds, n);
    let mut changed = false;
    for i in 0..n {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        let cmp = rv_operands_of(&lines[i], "slti");
        let [t, x, zero] = cmp.as_slice() else {
            continue;
        };
        let (t, x) = (parse_reg(t), parse_reg(x));
        if t == REG_NONE || x == REG_NONE || parse_asm_i64(zero) != Some(0) {
            continue;
        }
        let j = rv_next_kept_line(kinds, i + 1, n);
        let Some(line) = lines.get(j) else {
            continue;
        };
        let (m, ops) = line.trim().split_once(' ').unwrap_or(("", ""));
        let ops: Vec<&str> = ops.split(',').map(str::trim).collect();
        let d = match (m, ops.as_slice()) {
            ("neg" | "negw", [d, s]) if parse_reg(s) == t => parse_reg(d),
            ("sub" | "subw", [d, "zero" | "x0", s]) if parse_reg(s) == t => parse_reg(d),
            _ => continue,
        };
        if d == REG_NONE || (t != d && !live.dead_after(lines, kinds, j, t)) {
            continue;
        }
        lines[i] = format!("    srai {}, {}, 63", reg_name(d), reg_name(x));
        kinds[i] = classify_line(&lines[i]);
        kinds[j] = LineKind::Nop;
        live.invalidate();
        changed = true;
    }
    changed
}

/// The range a recognised producer leaves in its destination; `None` for an
/// instruction this table does not model.
fn rv_produced_range(mnem: &str, imm: Option<i64>, src: RvRange) -> Option<RvRange> {
    Some(match mnem {
        "lb" => rv_signed_range(8),
        "lh" => rv_signed_range(16),
        "lw" => RV_WORD,
        "lbu" => rv_unsigned_range(8),
        "lhu" => rv_unsigned_range(16),
        "lwu" | "zext.w" => rv_unsigned_range(32),
        "ld" => RV_FULL,
        "seqz" | "snez" | "slt" | "sltu" | "slti" | "sltiu" => (0, 1),
        // `s & m` of a non-negative `m` is in [0, m], and below `s` when
        // `s` is non-negative too.
        "andi" => match imm.filter(|m| *m >= 0) {
            Some(m) if src.0 >= 0 => (0, m.min(src.1)),
            Some(m) => (0, m),
            None => RV_FULL,
        },
        "srli" => match imm.filter(|k| (1..64).contains(k)) {
            Some(k) if src.0 >= 0 => (src.0 >> k, src.1 >> k),
            Some(k) => rv_unsigned_range((64 - k) as u32),
            None => RV_FULL,
        },
        "srai" => match imm.filter(|k| (0..64).contains(k)) {
            Some(k) => (src.0 >> k, src.1 >> k),
            None => RV_FULL,
        },
        // The low word shifted, then sign-extended from bit 31: a logical
        // shift by K >= 1 clears that bit, so the result is the word's
        // unsigned value shifted -- the source's range shifted when it is a
        // non-negative word, else everything below 2^(32 - K).
        "srliw" => match imm.filter(|k| (1..32).contains(k)) {
            Some(k) if src.0 >= 0 && src.1 <= i64::from(i32::MAX) => (src.0 >> k, src.1 >> k),
            Some(k) => rv_unsigned_range((32 - k) as u32),
            None => RV_WORD,
        },
        "sraiw" => match imm.filter(|k| (0..32).contains(k)) {
            Some(k) if rv_range_within(src, RV_WORD) => (src.0 >> k, src.1 >> k),
            Some(k) => (i64::from(i32::MIN) >> k, i64::from(i32::MAX) >> k),
            None => RV_WORD,
        },
        "addi" => imm.map_or(RV_FULL, |v| rv_offset_range(src, v)),
        // The low word plus `v`, sign-extended: the offset range itself when
        // both it and the source are words.
        "addiw" => match imm.map(|v| rv_offset_range(src, v)) {
            Some(r) if rv_range_within(src, RV_WORD) && rv_range_within(r, RV_WORD) => r,
            _ => RV_WORD,
        },
        _ => return None,
    })
}

fn eliminate_redundant_sign_extensions(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    fn get(rng: &[RvRange; NUM_REGS], reg: u8) -> RvRange {
        rng.get(reg as usize).copied().unwrap_or(RV_FULL)
    }
    fn set(rng: &mut [RvRange; NUM_REGS], reg: u8, r: RvRange) {
        if let Some(slot) = rng.get_mut(reg as usize) {
            *slot = r;
        }
    }
    let mut live = RvLazyLiveness::new(lines, kinds, n);
    let mut rng = [RV_FULL; NUM_REGS];
    let mut changed = false;
    let mut i = 0;
    while i < n {
        match kinds[i] {
            LineKind::Label | LineKind::Branch | LineKind::Jump | LineKind::Ret => {
                rng.fill(RV_FULL);
                i += 1;
                continue;
            }
            LineKind::Call => {
                for r in (0..=6usize).chain(30..=37).chain([REG_RA as usize]) {
                    rng[r] = RV_FULL;
                }
                i += 1;
                continue;
            }
            LineKind::Nop if lines[i].trim_start().starts_with(ARG_EXT_MARKER) => {
                for (reg, ext) in arg_ext_facts(lines[i].trim_start()) {
                    let r = match ext {
                        ArgExt::Word => RV_WORD,
                        ArgExt::Zero(bits) => rv_unsigned_range(u32::from(bits)),
                        ArgExt::Sign(bits) => rv_signed_range(u32::from(bits)),
                    };
                    set(&mut rng, reg, r);
                }
                i += 1;
                continue;
            }
            LineKind::Nop | LineKind::Directive => {
                i += 1;
                continue;
            }
            _ => {}
        }
        if let Some(c) = rv_fold_extension_into_compare(lines, kinds, n, i, &rng, &mut live) {
            // The compare is read next with its new source; the extension's
            // destination was never written, so its range stands.
            changed = true;
            i = c;
            continue;
        }
        let text = lines[i].trim().to_owned();
        let Some((mnem, rest)) = text.split_once(' ') else {
            rng.fill(RV_FULL);
            i += 1;
            continue;
        };
        let ops: Vec<&str> = rest.split(',').map(str::trim).collect();
        let reg = |k: usize| ops.get(k).map_or(REG_NONE, |o| parse_reg(o));
        let imm = |k: usize| ops.get(k).and_then(|o| parse_asm_i64(o));

        // `slli d,s,K; srai d,d,K` of a value already 64 - K bits wide.
        if mnem == "slli" && ops.len() == 3 {
            let (dst, src) = (reg(0), reg(1));
            let j = rv_next_kept_line(kinds, i + 1, n);
            if let Some(k) = imm(2).filter(|k| (1..64).contains(k))
                && dst != REG_NONE
                && src != REG_NONE
                && j < n
                && rv_range_width(get(&rng, src)) <= (64 - k) as u32
            {
                // `srai e, dst, K` into another register leaves `dst` holding
                // the shifted value, which must then be dead.
                let pair = rv_operands_of(&lines[j], "srai");
                let e = pair.first().map_or(REG_NONE, |o| parse_reg(o));
                if pair.len() == 3
                    && e != REG_NONE
                    && parse_reg(&pair[1]) == dst
                    && parse_asm_i64(&pair[2]) == Some(k)
                    && (e == dst || live.dead_after(lines, kinds, j, dst))
                {
                    if e == src {
                        kinds[i] = LineKind::Nop;
                    } else {
                        lines[i] = format!("    mv {}, {}", reg_name(e), reg_name(src));
                        kinds[i] = LineKind::Move { dst: e, src };
                    }
                    kinds[j] = LineKind::Nop;
                    let r = get(&rng, src);
                    set(&mut rng, e, r);
                    live.invalidate();
                    changed = true;
                    i = j + 1;
                    continue;
                }
            }
            // Otherwise the pair is real, unless it extends a load (which
            // can extend by itself) or only a sign test reads it.  The scan
            // resumes at the rewritten line, which records its own range
            // like any other.
            if let Some(resume) = rv_retype_extended_load(lines, kinds, n, i, &mut live) {
                changed = true;
                i = resume;
                continue;
            }
            if let Some(resume) =
                rv_sign_test_of_extension(lines, kinds, n, i, get(&rng, src), &mut live)
            {
                changed = true;
                i = resume;
                continue;
            }
        }

        if let LineKind::SextW { dst, src } = kinds[i] {
            let r = get(&rng, src);
            if rv_range_within(r, (0, i64::from(i32::MAX))) {
                if dst == src {
                    kinds[i] = LineKind::Nop;
                } else {
                    lines[i] = format!("    mv {}, {}", reg_name(dst), reg_name(src));
                    kinds[i] = LineKind::Move { dst, src };
                }
                live.invalidate();
                changed = true;
            }
            set(
                &mut rng,
                dst,
                if rv_range_within(r, RV_WORD) {
                    r
                } else {
                    RV_WORD
                },
            );
            i += 1;
            continue;
        }

        let dst = reg(0);
        if let Some(r) = rv_produced_range(mnem, imm(2), get(&rng, reg(1))) {
            if dst == REG_NONE {
                rng.fill(RV_FULL);
            } else {
                set(&mut rng, dst, r);
            }
            i += 1;
            continue;
        }
        if matches!(mnem, "sb" | "sh" | "sw" | "sd") {
            i += 1;
            continue;
        }
        match kinds[i] {
            LineKind::Move { dst, src } => {
                let r = get(&rng, src);
                set(&mut rng, dst, r);
            }
            LineKind::LoadImm { dst } => set(&mut rng, dst, imm(1).map_or(RV_FULL, |v| (v, v))),
            _ => match sext_producing_def(&lines[i], kinds[i]) {
                Some((dst, wide)) => set(&mut rng, dst, if wide { RV_WORD } else { RV_FULL }),
                None => rng.fill(RV_FULL),
            },
        }
        i += 1;
    }
    changed
}

// ── Pass 2: Redundant jump elimination ───────────────────────────────────────
//
// Pattern: jump .LBBN, t6  ;  .LBBN:  (jump to the immediately next label)
// The jump is redundant because execution falls through anyway.

fn eliminate_redundant_jumps(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        // A conditional branch to the label that immediately follows it reaches
        // the same instruction whether it is taken or not, so the branch itself
        // is dead (the same argument the unconditional `j` case uses).
        let target = match kinds[i] {
            LineKind::Jump => jump_target(&lines[i]),
            LineKind::Branch => branch_parts(&lines[i]).map(|(_, _, target)| target),
            _ => continue,
        };
        let Some(target) = target else { continue };
        let Some(j) = (i + 1..n).find(|&pos| kinds[pos] != LineKind::Nop) else {
            continue;
        };
        if kinds[j] == LineKind::Label && label_name(&lines[j]) == Some(target) {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

// ── Pass 3: Self-move elimination ────────────────────────────────────────────
//
// Pattern: mv tX, tX  (move to itself is a no-op)

fn eliminate_self_moves(kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        if let LineKind::Move { dst, src } = kinds[i] {
            if dst == src {
                kinds[i] = LineKind::Nop;
                changed = true;
                continue;
            }
            // Identical consecutive duplicate: `mv D, S` immediately after
            // `mv D, S` (store-result staging followed by the identical
            // return-value staging).  The second move is a no-op — same
            // source still live, same destination.  Scan only across Nops
            // so any intervening instruction (which may read D or redefine
            // S) keeps the move.
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j < n && kinds[j] == (LineKind::Move { dst, src }) {
                kinds[j] = LineKind::Nop;
                changed = true;
            }
        }
    }
    changed
}

// ── Pass 4: Redundant mv chain elimination ───────────────────────────────────
//
// Pattern: mv tX, sN  ;  mv tY, tX  → mv tY, sN  (when tX is a temp not used again)
// Also: mv sN, t0  ;  mv t1, t0  → keep both (t0 is still live)
// The key insight: if we see `mv A, B` followed by `mv C, A` and A is a temp
// register, we can redirect to `mv C, B` and eliminate the first mv if A
// is not used elsewhere.

/// Instructions whose effects are not fully described by their textual
/// operands: system, atomic and memory-ordering traffic.  A register rename
/// cannot reach into any of them, so a candidate crossing one is rejected.
fn has_unmodelled_effects(line: &str) -> bool {
    let m = line.trim().split_whitespace().next().unwrap_or("");
    matches!(
        m,
        "ecall"
            | "ebreak"
            | "fence"
            | "fence.i"
            | "sfence.vma"
            | "wfi"
            | "mret"
            | "sret"
            | "uret"
            | "csrr"
            | "csrw"
            | "csrs"
            | "csrc"
            | "csrrw"
            | "csrrs"
            | "csrrc"
            | "csrrwi"
            | "csrrsi"
            | "csrrci"
    ) || m.starts_with("lr.")
        || m.starts_with("sc.")
        || m.starts_with("amo")
}

/// `sb`/`sh`/`sw`/`sd` and the FP stores: they read every register they name.
fn is_gp_store(line: &str) -> bool {
    matches!(
        line.trim().split_whitespace().next().unwrap_or(""),
        "sb" | "sh" | "sw" | "sd" | "fsh" | "fsw" | "fsd" | "fsq"
    )
}

/// The GP register a line names as its destination, if any.
///
/// A RISC-V instruction that writes a GP register names it first, so outside
/// stores, branches and jumps the first operand is the destination.  This
/// over-approximates on purpose (`jr t0` reports t0): callers use it to refuse
/// a transformation, never to license one.
fn named_gp_dest(line: &str, kind: LineKind) -> Option<u8> {
    match kind {
        LineKind::Nop
        | LineKind::Directive
        | LineKind::Label
        | LineKind::Branch
        | LineKind::Jump
        | LineKind::Ret
        | LineKind::StoreS0 { .. } => None,
        LineKind::Move { dst, .. }
        | LineKind::LoadImm { dst }
        | LineKind::SextW { dst, .. }
        | LineKind::LoadAddr { dst }
        | LineKind::LoadS0 { reg: dst, .. } => Some(dst),
        LineKind::Call | LineKind::Alu | LineKind::Other => {
            if is_gp_store(line) {
                None
            } else {
                parse_alu_dest(line)
            }
        }
    }
}

/// Whole-function register-copy coalescing for the entry shuffle.
///
/// Ported from the ARM backend, which ports the x86 pass
/// (`x86/codegen/peephole/passes/copy_coalesce.rs`).  RISC-V had no coalescing
/// at all, so its functions paid for an entry shuffle both other backends
/// delete: the register allocator hands parameters homes that do not match the
/// a0-a7 registers they arrive in, so functions open with `mv rD, rS` copies
/// whose destination really is live for the rest of the function — no local
/// pass can delete those.  Coalescing renames the destination onto the source
/// everywhere after the copy and drops the copy.
///
/// Legality, all required:
///  1. The copy sits in the straight-line entry run (no label, jump, branch,
///     call or return before it), so every earlier line runs exactly once and
///     cannot be re-entered through a back edge.
///  2. `src` is mentioned nowhere else in the function — no later write
///     clobbers the coalesced value and no other live range is disturbed.  A
///     parameter's arrival in `src` is implicit, not a mention.
///  3. After the copy, `dst` has no unrenamable reader or writer: no implicit
///     operand, no `ret` or call with `dst` in a0-a7 (the return-value and
///     argument registers), no call while the value lives in a caller-saved
///     register *and may still be used afterwards* -- named past the
///     straight-line run that follows the copy -- and no unmodelled
///     instruction.  A far `jump` uses t6 as its scratch, so t6 is never
///     renamed across one.
///  4. s0 (frame pointer), ra and sp are never renamed.
///  5. After the rename every later write of `dst` lands in `src`, so `src`
///     must be dead wherever such a write runs.  Rule 2 excludes the readers
///     that name `src`; the ABI's implicit ones are a call (a0-a7, the
///     arguments) and `ret` (a0/a1, the return value).  An epilogue restore of
///     a callee-saved `dst` is not renamed at all: `dst` is never written again,
///     so it still holds the caller's value and the restore is deleted.
fn coalesce_entry_copies(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    fn mentions(line: &str, reg: u8) -> bool {
        let name = reg_name(reg);
        !name.is_empty()
            && line
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|tok| tok == name)
    }
    /// Caller-saved: clobbered by `call`.  t0-t6, the unnamed temporaries,
    /// and a0-a7.
    fn caller_saved(reg: u8) -> bool {
        reg <= 9 || (REG_A0..=REG_A7).contains(&reg)
    }
    fn renamable(reg: u8) -> bool {
        (reg as usize) < NUM_REGS && reg != REG_S0 && reg != REG_RA && reg != REG_SP
    }
    if n == 0 {
        return false;
    }
    let mut starts: Vec<usize> = (0..n)
        .filter(|&i| {
            lines[i].ends_with(':')
                && !lines[i].starts_with('.')
                && !lines[i].starts_with(char::is_whitespace)
        })
        .collect();
    if starts.is_empty() {
        return false;
    }
    starts.push(n);
    let mut changed = false;
    for w in 0..starts.len() - 1 {
        let (fstart, fend) = (starts[w], starts[w + 1]);
        // This pass reasons across an entire symbol-delimited function, and
        // otherwise cannot see register uses hidden in an opaque CFI interval.
        // A local-style function symbol may not delimit `starts`, so skip any
        // such window rather than treating the placeholder as an empty comment.
        if lines[fstart..fend].iter().any(|line| line == OPAQUE_LINE) {
            continue;
        }
        let mut eend = fend;
        for k in fstart + 1..fend {
            if matches!(
                kinds[k],
                LineKind::Label
                    | LineKind::Jump
                    | LineKind::Branch
                    | LineKind::Call
                    | LineKind::Ret
            ) {
                eend = k;
                break;
            }
        }
        let mut i = fstart + 1;
        while i < eend {
            i += 1;
            let LineKind::Move { dst, src } = kinds[i - 1] else {
                continue;
            };
            let i = i - 1;
            if dst == src || !renamable(dst) || !renamable(src) {
                continue;
            }
            // The rename rewrites every later mention of `dst` -- including
            // the epilogue restore of a callee-saved `dst` -- so `dst` is never
            // written again and the caller gets back whatever it held just
            // before the copy. That is the caller's own value only when nothing
            // in the (straight-line) entry run touched it first; the prologue
            // save is a store, and RISC-V stores never write a GP register.
            // `mv s1, a0; mv s2, s1; mv s1, a1` would return a0 in s1.
            if (REG_S1..=REG_S11).contains(&dst)
                && (fstart + 1..i).any(|k| {
                    !matches!(
                        kinds[k],
                        LineKind::Nop | LineKind::Directive | LineKind::Label
                    ) && (implicitly_writes_rv(&lines[k], dst)
                        || (mentions(&lines[k], dst) && !is_gp_store(&lines[k])))
                })
            {
                continue;
            }
            // Rule 5's restores: the frame slot the entry run saved a
            // callee-saved `dst` to.  A doubleword load of that slot back into
            // `dst` reinstates the caller's value, which `dst` keeps once the
            // rename stops writing it, so such a line is deleted, not renamed.
            let save_slot = if (REG_S1..=REG_S11).contains(&dst) {
                (fstart + 1..i).find_map(|k| match kinds[k] {
                    LineKind::StoreS0 {
                        reg,
                        offset,
                        is_word: false,
                    } if reg == dst => Some(offset),
                    _ => None,
                })
            } else {
                None
            };
            let is_restore = |kind: LineKind| {
                save_slot.is_some()
                    && matches!(kind, LineKind::LoadS0 { reg, offset, is_word: false }
                        if reg == dst && Some(offset) == save_slot)
            };
            // Rule 5: every other write of `dst` after the copy becomes a
            // write of `src`.  Rule 2 leaves `src` without a reader that names
            // it, but an argument register has implicit ones: every call reads
            // a0-a7 and `ret` reads a0/a1.  The epilogue restore was the write
            // that bit before restores were deleted: in
            // `foo(x) { asm("" :: "r"(x)); return x; }` the copy `mv s1, a0`
            // was coalesced, `ld s1, -32(s0)` became `ld a0, -32(s0)`, and foo
            // returned the caller's s1 (gcc.c-torture/execute/pr40022.c looped
            // forever at -O1 and -O2).  The copy sits in the straight-line entry
            // run, so any later line may run after any such write -- a back
            // edge reaches lines above it -- and an implicit reader anywhere
            // after the copy counts.
            if (REG_A0..=REG_A7).contains(&src) {
                let returns_src = src == REG_A0 || src == REG_A1;
                let implicitly_read = (i + 1..fend).any(|k| match kinds[k] {
                    LineKind::Ret => returns_src,
                    LineKind::Call => true,
                    _ => false,
                });
                if implicitly_read
                    && (i + 1..fend).any(|k| {
                        kinds[k] != LineKind::Nop
                            && !is_restore(kinds[k])
                            && named_gp_dest(&lines[k], kinds[k]) == Some(dst)
                    })
                {
                    continue;
                }
            }
            let mut ok = true;
            for k in fstart..fend {
                if k == i || kinds[k] == LineKind::Nop {
                    continue;
                }
                if mentions(&lines[k], src) {
                    ok = false;
                    break;
                }
            }
            if ok {
                // After the rename the value lives in `src`, and a call
                // destroys a caller-saved `src`: the value must then be dead
                // after every call on every path.  Text order is not control
                // flow -- a back edge carries a use above a call to the point
                // after it -- so only the straight-line run that follows the
                // copy is trusted: past the first label, jump, branch, call or
                // return, nothing but a (deleted) restore may name `dst`.
                let run_end = (i + 1..fend)
                    .find(|&k| {
                        matches!(
                            kinds[k],
                            LineKind::Label
                                | LineKind::Jump
                                | LineKind::Branch
                                | LineKind::Call
                                | LineKind::Ret
                        )
                    })
                    .unwrap_or(fend);
                let named_past_run = (run_end..fend).any(|m| {
                    kinds[m] != LineKind::Nop && !is_restore(kinds[m]) && mentions(&lines[m], dst)
                });
                for k in i + 1..fend {
                    match kinds[k] {
                        LineKind::Nop | LineKind::Directive | LineKind::Label => {}
                        LineKind::Ret => {
                            if (REG_A0..=REG_A7).contains(&dst) {
                                ok = false;
                                break;
                            }
                        }
                        LineKind::Jump => {
                            if dst == REG_T6 {
                                ok = false;
                                break;
                            }
                        }
                        LineKind::Call => {
                            // A call also reads a0-a7: an argument-register
                            // `dst` the rename stops writing would reach the
                            // callee stale.
                            if (caller_saved(src) && named_past_run)
                                || (REG_A0..=REG_A7).contains(&dst)
                            {
                                ok = false;
                                break;
                            }
                        }
                        _ => {}
                    }
                    if !ok || has_unmodelled_effects(&lines[k]) {
                        ok = false;
                        break;
                    }
                    let (ireads, iwrites) = classify_implicit_operands_rv(&lines[k]);
                    if (ireads | iwrites) & (1u64 << dst) != 0 {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            let d = reg_name(dst).to_string();
            let s = reg_name(src).to_string();
            for k in i + 1..fend {
                if kinds[k] == LineKind::Nop {
                    continue;
                }
                if is_restore(kinds[k]) {
                    kinds[k] = LineKind::Nop;
                    continue;
                }
                let new =
                    crate::backend::peephole_common::rename_register_operand(&lines[k], &d, &s);
                if new != lines[k] {
                    lines[k] = new;
                    kinds[k] = classify_line(&lines[k]);
                }
            }
            kinds[i] = LineKind::Nop;
            changed = true;
            break; // the rename changes the picture: re-scan this function
        }
    }
    changed
}

/// True when a block defines every register it reads before the read, so its
/// result cannot depend on the state it is entered with.  Two identical blocks
/// that are both live-in independent are interchangeable regardless of their
/// predecessors.  Calls and returns are excluded: a call reads the argument
/// registers implicitly and a `ret` reads a0-a7.
fn block_is_live_in_independent(
    lines: &[String],
    kinds: &[LineKind],
    start: usize,
    end: usize,
) -> bool {
    let mut defined: u64 = 0;
    for k in start..end {
        match kinds[k] {
            LineKind::Nop | LineKind::Directive | LineKind::Label => continue,
            LineKind::Call | LineKind::Ret => return false,
            _ => {}
        }
        let line = &lines[k];
        let (ireads, iwrites) = classify_implicit_operands_rv(line);
        if ireads & !defined != 0 {
            return false;
        }
        let out = sext_producing_def(line, kinds[k]).map(|(d, _)| d);
        let mentions_dst_once = match out {
            Some(r) => {
                let name = reg_name(r);
                !name.is_empty()
                    && line
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .filter(|t| *t == name)
                        .count()
                        == 1
            }
            None => false,
        };
        for tok in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            let r = parse_reg(tok);
            if r == REG_NONE || (r as usize) >= NUM_REGS {
                continue;
            }
            if mentions_dst_once && out == Some(r) {
                continue; // sole mention is the definition
            }
            if defined & (1u64 << r) == 0 {
                return false;
            }
        }
        if let Some(r) = out {
            defined |= 1u64 << r;
        }
        defined |= iwrites;
    }
    true
}

/// Merge identical basic blocks.
///
/// Ported from the ARM backend, which ports it from x86's `identical_blocks`.
/// Two blocks with the same instruction text are interchangeable when either
/// their predecessor sets are equal (so the live-in state is identical) or both
/// are live-in independent (so the live-in state is irrelevant).  The deleted
/// block must be entered only by explicit jumps — a fall-through edge cannot be
/// retargeted by renaming a label — and a function's entry block is never
/// deleted.  Empty predecessor sets are never treated as equal: a block can
/// look orphaned only because an edge was missed.
fn merge_identical_blocks(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut starts: Vec<usize> = vec![0];
    for i in 0..n {
        let after_transfer = i != 0
            && matches!(
                kinds[i - 1],
                LineKind::Jump | LineKind::Branch | LineKind::Ret
            );
        if i != 0 && (kinds[i] == LineKind::Label || after_transfer) {
            starts.push(i);
        }
    }
    let nb = starts.len();
    if nb < 2 {
        return false;
    }
    let end = |b: usize| -> usize { if b + 1 < nb { starts[b + 1] } else { n } };
    let mut block_of = vec![0usize; n];
    for b in 0..nb {
        for k in starts[b]..end(b) {
            block_of[k] = b;
        }
    }
    let mut labels: Vec<String> = vec![String::new(); nb];
    // `FxHashMap`, not `std::collections::HashMap`: the standard hasher is
    // seeded per process, and iterating a randomly-ordered map made the
    // emitted assembly differ between identical compilations. Lookup-only
    // here, but the same map feeds the identical-block pass below, where the
    // iteration order decides which label survives a retarget.
    let mut label_block: FxHashMap<&str, usize> = FxHashMap::default();
    for i in 0..n {
        if kinds[i] == LineKind::Label {
            let name = lines[i].trim().trim_end_matches(':');
            label_block.insert(name, block_of[i]);
            labels[block_of[i]] = (*name).to_string();
        }
    }
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); nb];
    let mut falls_through = vec![false; nb];
    let mut unknown_out = vec![false; nb];
    for b in 0..nb {
        let e = end(b);
        let mut t = e;
        while t > starts[b] && kinds[t - 1] == LineKind::Nop {
            t -= 1;
        }
        if t == starts[b] {
            if b + 1 < nb {
                succs[b].push(b + 1);
                falls_through[b] = true;
            }
            continue;
        }
        let last = t - 1;
        match kinds[last] {
            LineKind::Ret => {}
            // `jump L, t6` names its scratch register last, so the label is
            // the first argument — `jump_target` knows both spellings.
            LineKind::Jump => {
                match jump_target(&lines[last]).and_then(|t| label_block.get(t).copied()) {
                    Some(tb) => succs[b].push(tb),
                    None => unknown_out[b] = true,
                }
            }
            LineKind::Branch => {
                let tgt = lines[last].trim().split_whitespace().next_back();
                match tgt.and_then(|t| label_block.get(t).copied()) {
                    Some(tb) => succs[b].push(tb),
                    None => unknown_out[b] = true,
                }
                if b + 1 < nb {
                    succs[b].push(b + 1);
                    falls_through[b] = true;
                }
            }
            _ => {
                if b + 1 < nb {
                    succs[b].push(b + 1);
                    falls_through[b] = true;
                }
            }
        }
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for b in 0..nb {
        for &s in &succs[b] {
            preds[s].push(b);
        }
    }
    let mut unusable: Vec<bool> = (0..nb)
        .map(|b| unknown_out[b] || preds[b].iter().any(|&p| unknown_out[p]))
        .collect();
    // Jump-table case labels have incoming references from DATA, not from a
    // Branch line. Deleting such a block would leave an undefined relocation
    // even when its body is identical to another arm. Treat every directive
    // reference to a known label as an unresolved incoming edge; this also
    // covers assembler expressions beyond the currently emitted table forms.
    for i in 0..n {
        if kinds[i] != LineKind::Directive {
            continue;
        }
        for token in lines[i]
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'))
        {
            if let Some(&b) = label_block.get(token) {
                unusable[b] = true;
            }
        }
    }
    let body: Vec<String> = (0..nb)
        .map(|b| {
            let mut t = String::new();
            for k in starts[b]..end(b) {
                if matches!(
                    kinds[k],
                    LineKind::Nop | LineKind::Directive | LineKind::Label
                ) {
                    continue;
                }
                t.push_str(lines[k].trim());
                t.push('\n');
            }
            t
        })
        .collect();
    // Deterministic iteration is a correctness property, not a nicety: this
    // map's `values()` order decides the order in which duplicate blocks are
    // retargeted, and two groups can touch the same label. With a seeded
    // hash the same source produced different `.Lskip_N` labels and different
    // block placement on every run (RS-1: five serial compilations, five
    // distinct SHA-256s), which breaks reproducible builds, cached objects and
    // every `diff`-based gate. `FxHashMap` is the tree's deterministic map.
    // Blocks merge only within their function.  A branch into another
    // function's copy would run under that function's CFI (wrong unwind
    // tables for every backtrace through it) and could leave the +/-4 KiB
    // range of a near branch; and grouping the whole unit made the pairwise
    // search below quadratic in a large file (memclr.c's 2,816 functions
    // all end in the same blocks).
    let mut function_of = vec![0usize; nb];
    let mut function = 0usize;
    for b in 0..nb {
        let first = (starts[b]..end(b)).find(|&k| kinds[k] != LineKind::Nop);
        if let Some(k) = first {
            if kinds[k] == LineKind::Label && !lines[k].trim_start().starts_with('.') {
                function += 1;
            }
        }
        function_of[b] = function;
    }
    let mut groups: FxHashMap<(usize, &str), Vec<usize>> = FxHashMap::default();
    for b in 0..nb {
        if unusable[b] || body[b].trim().is_empty() {
            continue;
        }
        groups
            .entry((function_of[b], body[b].as_str()))
            .or_default()
            .push(b);
    }
    for members in groups.values() {
        if members.len() < 2 {
            continue;
        }
        for (i, &keep) in members.iter().enumerate() {
            for &dup in members.iter().skip(i + 1) {
                let from = labels[dup].clone();
                let to = labels[keep].clone();
                if keep == dup || from.is_empty() || to.is_empty() {
                    continue;
                }
                // A function's entry block is the symbol itself.
                if lines[starts[dup]].ends_with(':') && !lines[starts[dup]].starts_with('.') {
                    continue;
                }
                // A fall-through successor is semantically part of a block
                // even though it is absent from the textual body.  Identical
                // instruction sequences with different following blocks are
                // therefore not interchangeable.
                if (falls_through[keep] || falls_through[dup]) && succs[keep] != succs[dup] {
                    continue;
                }
                let same_preds = {
                    let mut a = preds[keep].clone();
                    let mut b = preds[dup].clone();
                    a.sort_unstable();
                    a.dedup();
                    b.sort_unstable();
                    b.dedup();
                    !a.is_empty() && a == b
                };
                if !same_preds
                    && !(block_is_live_in_independent(lines, kinds, starts[keep], end(keep))
                        && block_is_live_in_independent(lines, kinds, starts[dup], end(dup)))
                {
                    continue;
                }
                // `dup` must not be entered by fall-through.
                let mut fallthrough_in = false;
                for &p in &preds[dup] {
                    let e = end(p);
                    let mut t = e;
                    while t > starts[p] && kinds[t - 1] == LineKind::Nop {
                        t -= 1;
                    }
                    let term = if t > starts[p] {
                        kinds[t - 1]
                    } else {
                        LineKind::Nop
                    };
                    if !matches!(term, LineKind::Jump | LineKind::Ret) && p + 1 == dup {
                        fallthrough_in = true;
                        break;
                    }
                }
                if fallthrough_in {
                    continue;
                }
                for k in 0..n {
                    if !matches!(kinds[k], LineKind::Jump | LineKind::Branch) {
                        continue;
                    }
                    let new =
                        crate::backend::peephole_common::replace_whole_word(&lines[k], &from, &to);
                    if new != lines[k] {
                        lines[k] = new;
                        kinds[k] = classify_line(&lines[k]);
                    }
                }
                for k in starts[dup]..end(dup) {
                    kinds[k] = LineKind::Nop;
                }
                return true;
            }
        }
    }
    false
}

fn eliminate_redundant_mv_chain(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::Move {
            dst: dst1,
            src: src1,
        } = kinds[i]
        {
            // Find next non-Nop instruction
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j < n {
                if let LineKind::Move {
                    dst: dst2,
                    src: src2,
                } = kinds[j]
                {
                    // Pattern: mv dst1, src1 ; mv dst2, dst1
                    // → redirect second: mv dst2, src1
                    // Keep the first mv (dst1 might be used later).
                    if src2 == dst1 && dst2 != src1 {
                        lines[j] = format!("    mv {}, {}", reg_name(dst2), reg_name(src1));
                        kinds[j] = LineKind::Move {
                            dst: dst2,
                            src: src1,
                        };
                        changed = true;
                        // The first mv may become dead. The self-move elimination or
                        // unused callee-save passes may clean it up later.
                    }
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Pass 5: Li-to-move chain elimination ─────────────────────────────────────
//
// Pattern: li tX, imm ; mv tY, tX → li tY, imm
// This avoids the intermediate temp register for load-immediate sequences.

fn eliminate_li_mv_chain(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::LoadImm { dst: imm_dst } = kinds[i] {
            // Only optimize when the immediate dest is a temp register (t0-t6)
            if imm_dst <= REG_T6 {
                let mut j = i + 1;
                while j < n && kinds[j] == LineKind::Nop {
                    j += 1;
                }
                if j < n {
                    if let LineKind::Move {
                        dst: mv_dst,
                        src: mv_src,
                    } = kinds[j]
                    {
                        if mv_src == imm_dst {
                            // Retarget the li to the move destination
                            let trimmed = lines[i].trim();
                            if let Some(new_line) = retarget_li(trimmed, mv_dst) {
                                lines[j] = format!("    {}", new_line);
                                kinds[j] = LineKind::LoadImm { dst: mv_dst };
                                changed = true;
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    changed
}

/// Retarget a `li` instruction to a different destination register.
fn retarget_li(line: &str, new_dst: u8) -> Option<String> {
    if let Some(rest) = line.strip_prefix("li ") {
        if let Some((_, imm_part)) = rest.split_once(", ") {
            return Some(format!("li {}, {}", reg_name(new_dst), imm_part));
        }
    }
    None
}

// ── Dying-accumulator producer retarget ─────────────────────────────────────
//
// The emitter computes every result in the accumulator (`t0`) and then copies
// it to the value's home (`emit_store_result` -> `store_t0_to`).  For a value
// whose home is not the accumulator that copy is a full instruction per
// operation, and it is pure overhead: `t0`..`t6` are scratches -- no value is
// ever homed there, and no ABI contract names them -- so a producer may write
// the destination directly:
//
//     addw t0, t1, a1   ->   addw a0, t1, a1
//     mv   a0, t0            (deleted)
//
// Each of the following is checked rather than assumed:
//
//   * the producer names exactly one destination, its shape is one this pass
//     can rewrite (`mv`, `li`, `sext.w`, a frame load, or a plain ALU line),
//     and that destination is an accumulator;
//   * the consumer is the next live line of the same symbol-delimited window
//     and is `mv D, tK` with `D != tK`;
//   * `tK` is dead after the consumer -- see [`scratch_dead_after`].
//
// A retargeted producer that becomes a self-move (`mv t0, a0` + `mv a0, t0`)
// takes the consumer with it: the destination already holds the value.
//
// `CCC_NO_RV_ACCUMULATOR_RETARGET=1` disables the pass, which is how a
// differential gate can hold it constant while measuring another rule (a
// length comparison between two builds of the same function must vary one
// knob at a time).

/// Token-exact test for a register name inside a line.
fn mentions_reg(line: &str, reg: u8) -> bool {
    let name = reg_name(reg);
    !name.is_empty()
        && line
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|tok| tok == name)
}

/// True when `line` merely overwrites `reg`: the register is its destination
/// and appears nowhere else in the instruction.
///
/// A register-to-register form with the operand in the destination slot reads
/// the register as well (`sext.w t0, t0` consumes t0's value), so it is a read
/// and not an overwrite.  Treating it as an overwrite is a miscompile, not a
/// missed optimisation: the value the reader consumes is the one the retarget
/// removed, so `srli t0, t0, 3; mv a0, t0; sext.w t0, t0` would keep the
/// pre-shift value in t0 and sign-extend that instead of the shift result.
/// Only definitions that read nothing are pure overwrites.
fn dest_only_write(line: &str, kind: LineKind, reg: u8) -> bool {
    match kind {
        LineKind::LoadImm { dst } | LineKind::LoadS0 { reg: dst, .. } => dst == reg,
        LineKind::Move { dst, src } | LineKind::SextW { dst, src } => dst == reg && src != reg,
        LineKind::Alu => {
            if parse_alu_dest(line) != Some(reg) {
                return false;
            }
            // The destination is the first operand, so only a later mention
            // would be a read (`add t0, t0, t1`).
            match line.trim().split_once(char::is_whitespace) {
                Some((_, rest)) => match rest.trim_start().split_once(',') {
                    Some((_, tail)) => !mentions_reg(tail, reg),
                    None => true,
                },
                None => true,
            }
        }
        _ => false,
    }
}

/// True when `reg` cannot be read again inside the window: the next thing that
/// happens to it is an overwrite, or the function returns.  A scratch register
/// is never a return register, so `ret` ends the search successfully.
///
/// Labels, branches, jumps and calls end it with a refusal: the first three
/// leave the straight-line run, and a callee is entitled to clobber a scratch
/// under the ABI while this pass is not entitled to assume the emitter relies
/// on it.
fn scratch_dead_after(
    lines: &[String],
    kinds: &[LineKind],
    start: usize,
    reg: u8,
    fend: usize,
) -> bool {
    let mask = 1u64 << reg;
    for k in start..fend {
        match kinds[k] {
            LineKind::Nop | LineKind::Directive => continue,
            LineKind::Ret => return true,
            LineKind::Label | LineKind::Branch | LineKind::Jump | LineKind::Call => return false,
            _ => {}
        }
        if has_unmodelled_effects(&lines[k]) {
            return false;
        }
        let (ireads, iwrites) = classify_implicit_operands_rv(&lines[k]);
        if ireads & mask != 0 {
            return false;
        }
        if iwrites & mask != 0 {
            return true;
        }
        if !mentions_reg(&lines[k], reg) {
            continue;
        }
        return dest_only_write(&lines[k], kinds[k], reg);
    }
    true
}

/// Replace the first operand of a `mnemonic rd, ...` line with `new_dst`.
fn retarget_first_operand(line: &str, new_dst: u8) -> Option<String> {
    let trimmed = line.trim();
    let space = trimmed.find(char::is_whitespace)?;
    let (mnem, rest) = trimmed.split_at(space);
    let (_, tail) = rest.trim_start().split_once(',')?;
    let indent = &line[..line.len() - line.trim_start().len()];
    Some(format!("{indent}{mnem} {},{}", reg_name(new_dst), tail))
}

fn retarget_dying_accumulator_moves(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    /// The destination this pass may rewrite: a single-destination producer of
    /// a shape it understands, writing an accumulator.
    fn producible_dest(line: &str, kind: LineKind) -> Option<u8> {
        let dst = match kind {
            LineKind::Move { dst, .. }
            | LineKind::LoadImm { dst }
            | LineKind::SextW { dst, .. }
            | LineKind::LoadS0 { reg: dst, .. } => dst,
            LineKind::Alu => parse_alu_dest(line)?,
            // An FP result in a general register (`fcvt.l.d t0, ft0, rtz`,
            // `flt.d t0, ft0, ft1`, `fmv.x.d t0, ft0`) or a plain load
            // (`lbu t0, 0(t0)`, which reads its base before it writes):
            // its first operand.
            LineKind::Other => rv_fp_gp_dest(line).or_else(|| {
                let m = line.trim().split_whitespace().next()?;
                matches!(m, "lb" | "lbu" | "lh" | "lhu" | "lw" | "lwu" | "ld")
                    .then(|| rv_load_dest(line))
                    .flatten()
            })?,
            _ => return None,
        };
        (dst <= REG_T6).then_some(dst)
    }
    if n == 0 {
        return false;
    }
    let mut starts: Vec<usize> = (0..n)
        .filter(|&i| {
            lines[i].ends_with(':')
                && !lines[i].starts_with('.')
                && !lines[i].starts_with(char::is_whitespace)
        })
        .collect();
    if starts.is_empty() {
        return false;
    }
    starts.push(n);
    let mut changed = false;
    for w in 0..starts.len() - 1 {
        let (fstart, fend) = (starts[w], starts[w + 1]);
        // An opaque body hides the register uses this pass reasons about.
        if lines[fstart..fend].iter().any(|line| line == OPAQUE_LINE) {
            continue;
        }
        let mut live: Option<Option<RvLiveness>> = None;
        let mut i = fstart + 1;
        while i + 1 < fend {
            let Some(tmp) = producible_dest(&lines[i], kinds[i]) else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            while j < fend && kinds[j] == LineKind::Nop {
                j += 1;
            }
            let Some(LineKind::Move { dst, src }) = kinds.get(j).copied() else {
                i += 1;
                continue;
            };
            // A value home is never a scratch, and `s0`, `sp` and `ra` carry
            // meaning of their own: leave those consumers alone.
            if src != tmp || dst == tmp || dst == REG_S0 || dst == REG_SP || dst == REG_RA {
                i += 1;
                continue;
            }
            // The scratch must die at the move: the local scan settles
            // straight-line code, the function's liveness what lies past a
            // branch or a label (built once, on the first such question --
            // each rewrite only shortens the scratch's live range).
            let dead = scratch_dead_after(lines, kinds, j + 1, tmp, fend) || {
                if live.is_none() {
                    live = Some(RvLiveness::build(lines, kinds, fstart, fend));
                }
                live.as_ref()
                    .and_then(Option::as_ref)
                    .is_some_and(|l| l.live_after(j) & rv_bit(tmp) == 0)
            };
            if !dead {
                i += 1;
                continue;
            }
            let self_move = matches!(kinds[i], LineKind::Move { src, .. } if src == dst);
            if self_move {
                kinds[i] = LineKind::Nop;
            } else {
                let Some(new_line) = retarget_first_operand(&lines[i], dst) else {
                    i += 1;
                    continue;
                };
                lines[i] = new_line;
                kinds[i] = classify_line(&lines[i]);
            }
            kinds[j] = LineKind::Nop;
            changed = true;
            i += 1;
        }
    }
    changed
}

// ── Global store forwarding ──────────────────────────────────────────────────
//
// Tracks slot→register mappings as we scan forward within each basic block.
// When we see a load from a stack slot that has a known register value,
// we replace the load with a register move (or eliminate it if same register).
//
// At labels (which may be branch targets), all mappings are invalidated.
// This ensures correctness: we only forward within straight-line basic blocks.

/// A tracked store mapping: stack slot at `offset` holds the value from `reg`.
#[derive(Clone, Copy)]
struct SlotMapping {
    reg: u8,
    is_word: bool,
    active: bool,
}

/// Maximum number of tracked slot mappings.
const MAX_SLOT_ENTRIES: usize = 64;

fn global_store_forwarding(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if n == 0 {
        return false;
    }

    // Slot tracking: Vec of (offset, SlotMapping)
    let mut slots: Vec<(i32, SlotMapping)> = Vec::new();
    // Reverse mapping: register -> list of offsets it backs
    let mut reg_slots: Vec<Vec<i32>> = vec![Vec::new(); NUM_REGS];
    let mut changed = false;

    for i in 0..n {
        if kinds[i] == LineKind::Nop {
            continue;
        }

        match kinds[i] {
            LineKind::Label => {
                // Conservatively invalidate all mappings at every label.
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }

            LineKind::StoreS0 {
                reg,
                offset,
                is_word,
            } => {
                // Invalidate any existing mapping that overlaps this store
                let store_size: i32 = if is_word { 4 } else { 8 };
                gsf_invalidate_overlapping(&mut slots, &mut reg_slots, offset, store_size);
                // Record new mapping
                if (reg as usize) < NUM_REGS {
                    slots.push((
                        offset,
                        SlotMapping {
                            reg,
                            is_word,
                            active: true,
                        },
                    ));
                    reg_slots[reg as usize].push(offset);
                }
                if slots.len() > MAX_SLOT_ENTRIES {
                    slots.retain(|&(_, m)| m.active);
                }
            }

            LineKind::LoadS0 {
                reg: load_reg,
                offset: load_off,
                is_word: load_word,
            } => {
                // Try to forward from a stored value
                let mapping = slots
                    .iter()
                    .rev()
                    .find(|(off, m)| m.active && *off == load_off && m.is_word == load_word)
                    .map(|(_, m)| *m);

                if let Some(mapping) = mapping {
                    // A word reload sign-extends: `sext.w`, never a plain copy.
                    forward_stored_value(lines, kinds, i, load_reg, mapping.reg, mapping.is_word);
                    changed = true;
                }
                // The load overwrites load_reg, invalidate any slot backed by it
                if (load_reg as usize) < NUM_REGS {
                    gsf_invalidate_reg(&mut slots, &mut reg_slots, load_reg);
                }
            }

            LineKind::Jump | LineKind::Branch | LineKind::Ret => {
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }

            LineKind::Call => {
                // Calls invalidate everything: callee may access stack via pointers.
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }

            LineKind::Move { dst, .. } => {
                // Move writes to dst, invalidate any slot backed by it
                if (dst as usize) < NUM_REGS {
                    gsf_invalidate_reg(&mut slots, &mut reg_slots, dst);
                }
            }

            LineKind::LoadImm { dst }
            | LineKind::SextW { dst, .. }
            | LineKind::LoadAddr { dst } => {
                if (dst as usize) < NUM_REGS {
                    gsf_invalidate_reg(&mut slots, &mut reg_slots, dst);
                }
            }

            LineKind::Alu => {
                // ALU writes to destination register
                if let Some(dst) = parse_alu_dest(&lines[i]) {
                    if (dst as usize) < NUM_REGS {
                        gsf_invalidate_reg(&mut slots, &mut reg_slots, dst);
                    }
                }
            }

            LineKind::Directive | LineKind::Nop => {
                // No effect on register state.
            }

            LineKind::Other => {
                // For any unclassified instruction, conservatively invalidate all.
                gsf_invalidate_all(&mut slots, &mut reg_slots);
            }
        }
    }

    changed
}

/// Invalidate all slot mappings.
fn gsf_invalidate_all(slots: &mut Vec<(i32, SlotMapping)>, reg_slots: &mut [Vec<i32>]) {
    slots.clear();
    for rs in reg_slots.iter_mut() {
        rs.clear();
    }
}

/// Invalidate slot mappings whose byte range overlaps [store_off, store_off + store_size).
fn gsf_invalidate_overlapping(
    slots: &mut [(i32, SlotMapping)],
    reg_slots: &mut [Vec<i32>],
    store_off: i32,
    store_size: i32,
) {
    let store_end = store_off + store_size;
    for &mut (off, ref mut m) in slots.iter_mut() {
        if !m.active {
            continue;
        }
        let mapping_size: i32 = if m.is_word { 4 } else { 8 };
        let mapping_end = off + mapping_size;
        if off < store_end && store_off < mapping_end {
            let r = m.reg as usize;
            if r < reg_slots.len() {
                reg_slots[r].retain(|&o| o != off);
            }
            m.active = false;
        }
    }
}

/// Invalidate all slot mappings backed by a specific register.
fn gsf_invalidate_reg(slots: &mut [(i32, SlotMapping)], reg_slots: &mut [Vec<i32>], reg: u8) {
    let r = reg as usize;
    if r >= reg_slots.len() {
        return;
    }
    for &offset in &reg_slots[r] {
        for &mut (off, ref mut m) in slots.iter_mut().rev() {
            if off == offset && m.active && m.reg == reg {
                m.active = false;
                break;
            }
        }
    }
    reg_slots[r].clear();
}

// ── Register copy propagation ────────────────────────────────────────────────
//
// After store forwarding converts loads into register moves, propagate those
// copies into subsequent instructions. For `mv dst, src`, replace references
// to dst with src in the immediately following instruction (within the same
// basic block).

fn propagate_register_copies(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;

    for i in 0..n {
        // Only process register-to-register moves
        let (dst, src) = match kinds[i] {
            LineKind::Move { dst, src } => {
                // Don't propagate frame pointer or stack pointer moves
                if dst == REG_S0 || src == REG_S0 || dst == REG_SP || src == REG_SP {
                    continue;
                }
                (dst, src)
            }
            _ => continue,
        };

        // Find the next non-Nop instruction
        let mut j = i + 1;
        while j < n && kinds[j] == LineKind::Nop {
            j += 1;
        }
        if j >= n {
            continue;
        }

        // Don't propagate across control flow boundaries or into instructions
        // with symbol names (LoadAddr) that could be corrupted by register
        // name replacement (e.g. `lla t0, main.s1.0` contains "s1").
        match kinds[j] {
            LineKind::Label
            | LineKind::Jump
            | LineKind::Ret
            | LineKind::Directive
            | LineKind::Call
            | LineKind::LoadAddr { .. } => continue,
            _ => {}
        }
        // A line that implicitly writes the copy's source or destination
        // (a bare `jal` overwriting ra, an indirect `jalr`) must break the
        // alias: renaming a reader onto a register the intervening
        // instruction redefined feeds it the wrong value, and redirecting
        // an implicit write onto the alias source corrupts it.
        if implicitly_writes_rv(&lines[j], src) || implicitly_writes_rv(&lines[j], dst) {
            continue;
        }

        let dst_name = reg_name(dst);
        if !lines[j].contains(dst_name) {
            continue;
        }

        let src_name = reg_name(src);

        // For move instructions, replace the source operand
        match kinds[j] {
            LineKind::Move {
                dst: dst2,
                src: src2,
            } if src2 == dst => {
                // mv X, dst → mv X, src
                if dst2 != src {
                    lines[j] = format!("    mv {}, {}", reg_name(dst2), src_name);
                    kinds[j] = LineKind::Move { dst: dst2, src };
                    changed = true;
                }
            }
            _ => {
                // General case: try to replace the register in source positions
                if let Some(new_line) =
                    replace_source_reg_in_instruction(&lines[j], dst_name, src_name)
                {
                    lines[j] = new_line;
                    kinds[j] = classify_line(&lines[j]);
                    changed = true;
                }
            }
        }
    }
    changed
}

// ── Dead register move elimination ──────────────────────────────────────────
//
// After copy propagation, some `mv` instructions may have dead destinations:
// the destination register is overwritten before being read. Scan forward
// from each `mv` instruction (within the same basic block, up to a window)
// to check if the destination is read before being overwritten.
//
// If the destination is overwritten without being read, the move is dead.

fn eliminate_dead_reg_moves(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let window = 16; // look ahead up to 16 instructions

    for i in 0..n {
        let (dst, _src) = match kinds[i] {
            LineKind::Move { dst, src } => (dst, src),
            _ => continue,
        };

        // Temporaries (t0-t6) and argument registers (a0-a7) only: the scan
        // below proves deadness by a straight-line overwrite, which holds for
        // any register, but its read model does not see the `s0` base of a
        // frame access, and `sp`/`ra`/callee-saved registers are never worth
        // the risk.  At `ret` only the temporaries are dead: a0/a1 carry the
        // return value.
        let is_temp = dst <= REG_T6;
        if !is_temp && !(REG_A0..=REG_A7).contains(&dst) {
            continue;
        }

        let dst_name = reg_name(dst);

        // Scan forward to check if dst is read before being overwritten
        let mut is_dead = false;
        let mut count = 0;
        let mut j = i + 1;

        while j < n && count < window {
            if kinds[j] == LineKind::Nop {
                j += 1;
                continue;
            }

            // Stop at control flow boundaries and any unclassified instruction
            match kinds[j] {
                LineKind::Ret => {
                    // `ret` leaves only the return registers (a0/a1, fa0/fa1)
                    // and the callee-saved ones live; a temporary not read
                    // on the way is dead.  User asm (a naked function's
                    // custom convention) never reaches here: functions
                    // holding it are opaque.
                    is_dead = is_temp;
                    break;
                }
                LineKind::Label
                | LineKind::Jump
                | LineKind::Branch
                | LineKind::Call
                | LineKind::Directive => break,
                LineKind::Other => {
                    // Conservatively assume Other instructions may read the register
                    break;
                }
                _ => {}
            }

            // For well-classified instructions, check reads precisely
            let reads_dst = match kinds[j] {
                LineKind::Move { src, .. } => src == dst,
                LineKind::SextW { src, .. } => src == dst,
                LineKind::StoreS0 { reg: store_reg, .. } => store_reg == dst,
                LineKind::LoadS0 { .. } => false, // only writes to dest
                LineKind::LoadImm { .. } => false, // only writes
                LineKind::LoadAddr { .. } => false, // only writes (lla/la)
                LineKind::Alu => {
                    // Check source positions (after first comma)
                    let trimmed = lines[j].trim();
                    if let Some(comma_pos) = trimmed.find(',') {
                        let after_dest = &trimmed[comma_pos + 1..];
                        has_whole_word(after_dest, dst_name)
                    } else {
                        // Single-operand: conservative
                        true
                    }
                }
                _ => true, // conservative for anything else
            };

            if reads_dst {
                break;
            }

            // Check if this instruction writes to dst (overwrites it)
            let dest_of_j = match kinds[j] {
                LineKind::Move { dst: d, .. } => Some(d),
                LineKind::LoadImm { dst: d } => Some(d),
                LineKind::SextW { dst: d, .. } => Some(d),
                LineKind::LoadS0 { reg: d, .. } => Some(d),
                LineKind::LoadAddr { dst: d } => Some(d),
                LineKind::Alu => parse_alu_dest(&lines[j]),
                _ => None,
            };

            if dest_of_j == Some(dst) {
                // dst is overwritten without being read: the move is dead!
                is_dead = true;
                break;
            }

            count += 1;
            j += 1;
        }

        if is_dead {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

// ── Global register liveness ────────────────────────────────────────────────
//
// The local passes above prove a definition dead only by a straight-line
// overwrite inside its block, so every copy the emitter's accumulator style
// leaves at a block end -- `mv t1, a3; mv t0, a1; mv t2, a1` before a loop's
// back-edge branch -- survived: the scan stops at the branch.  Backward
// liveness over the function's CFG answers the question exactly (ported from
// the AArch64 and x86 peepholes), and a forward copy propagation inside each
// block turns `mv t1, a2; add a2, t1, a5` into `add a2, a2, a5` so that the
// copy itself becomes one of those dead definitions.

/// The prologue's `# lccc-ret-regs a0 a1|a0|-` line: the general registers
/// the function's `ret` hands back as its value.  An internal annotation, not
/// a configuration knob: every exit of `peephole_optimize` removes it.
/// Appended by the call emitter to a direct `call` of an IR call: the GP
/// argument registers it reads (decimal mask, bit k = a_k).  The peephole
/// charges an annotated call with exactly those, the static chain t2 and
/// sp, instead of every caller-saved register; the annotation is stripped
/// before the text leaves the backend.
pub(super) const CALL_READS_TAG: &str = " # lccc-call-reads ";

/// The registers (liveness bits) the call on `line` reads, when the emitter
/// annotated it: its argument registers, t2 (a nested callee's static
/// chain) and sp.
fn rv_annotated_call_reads(line: &str) -> Option<u64> {
    let mask: u64 = line
        .split_once(CALL_READS_TAG)?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(((mask & 0xff) << REG_A0) | rv_bit(REG_T2) | rv_bit(REG_SP))
}

const RET_REGS_MARKER: &str = "# lccc-ret-regs";

/// The prologue's `# lccc-arg-ext a0 a1/z16 a2/s8 ...` line (see
/// `emit_arg_extension_marker`): argument registers that arrive sign-extended
/// from bit 31, `/z8` and `/z16` marking those also zero-extended from 8 or 16
/// bits, `/s8` and `/s16` those sign-extended from bit 7 or 15. It sits in the
/// entry block before any instruction, so the facts are the entry state the
/// forward extension trackers start from.
const ARG_EXT_MARKER: &str = "# lccc-arg-ext";

/// A line of the prologue's internal contracts (never emitted).
fn is_internal_marker(trimmed: &str) -> bool {
    trimmed.starts_with(RET_REGS_MARKER) || trimmed.starts_with(ARG_EXT_MARKER)
}

/// How an argument register arrives (one `ARG_EXT_MARKER` fact).  Every
/// kind is also a sign-extended word: the psABI widens a narrow integer by
/// its type's signedness to 32 bits, then sign-extends that word to 64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArgExt {
    /// Sign-extended from bit 31 only (`int`, `unsigned int`).
    Word,
    /// Zero-extended from bit `n - 1` (`unsigned char`, `unsigned short`).
    Zero(u8),
    /// Sign-extended from bit `n - 1` (`signed char`, `short`).
    Sign(u8),
}

/// The argument-extension facts of an `ARG_EXT_MARKER` line.
fn arg_ext_facts(trimmed: &str) -> impl Iterator<Item = (u8, ArgExt)> + '_ {
    trimmed
        .strip_prefix(ARG_EXT_MARKER)
        .unwrap_or("")
        .split_whitespace()
        .filter_map(|token| {
            let (name, tag) = token.split_once('/').unwrap_or((token, ""));
            let reg = parse_reg(name);
            let ext = match tag {
                "" => ArgExt::Word,
                "z8" => ArgExt::Zero(8),
                "z16" => ArgExt::Zero(16),
                "s8" => ArgExt::Sign(8),
                "s16" => ArgExt::Sign(16),
                _ => return None,
            };
            (REG_A0..REG_A0 + 8).contains(&reg).then_some((reg, ext))
        })
}

/// Every general register the liveness tracks (`parse_reg`'s ids).
const RV_LIVE_ALL: u64 =
    0x7f | (0xfff << REG_S0) | (0xff << REG_A0) | (1 << REG_RA) | (1 << REG_SP);
/// Caller-saved registers: t0-t6, a0-a7, ra.
const RV_CALLER_SAVED: u64 = 0x7f | (0xff << REG_A0) | (1 << REG_RA);
/// Callee-saved registers a return hands back to the caller: s0-s11, sp.
const RV_CALLEE_SAVED: u64 = (0xfff << REG_S0) | (1 << REG_SP);

fn rv_bit(reg: u8) -> u64 {
    if (reg as usize) < NUM_REGS {
        1u64 << reg
    } else {
        0
    }
}

/// Registers a line names, and whether it names one the liveness does not
/// model (`x5`, `fp`, `gp`, `tp`): such a line is treated as reading
/// everything.  Symbol operands may produce false register tokens
/// (`main.s1.0`); those only over-approximate the reads.
fn rv_named_regs(line: &str) -> (u64, bool) {
    let mut mask = 0u64;
    let mut unmodelled = false;
    for tok in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')) {
        for part in tok.split('.') {
            let id = parse_reg(part);
            if id != REG_NONE {
                mask |= rv_bit(id);
            } else if matches!(part, "fp" | "gp" | "tp")
                || (part.len() > 1
                    && part.starts_with('x')
                    && part[1..].bytes().all(|b| b.is_ascii_digit()))
            {
                unmodelled = true;
            }
        }
    }
    (mask, unmodelled)
}

/// Integer instructions whose only effect is writing their first operand:
/// never a trap (RISC-V integer division does not trap), a memory access,
/// a CSR or an FP-flag update.  Only these may be deleted when that operand
/// is dead.
fn rv_is_pure_int_op(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "add"
            | "addi"
            | "addw"
            | "addiw"
            | "sub"
            | "subw"
            | "and"
            | "andi"
            | "or"
            | "ori"
            | "xor"
            | "xori"
            | "sll"
            | "slli"
            | "sllw"
            | "slliw"
            | "srl"
            | "srli"
            | "srlw"
            | "srliw"
            | "sra"
            | "srai"
            | "sraw"
            | "sraiw"
            | "slt"
            | "slti"
            | "sltu"
            | "sltiu"
            | "lui"
            | "auipc"
            | "mul"
            | "mulh"
            | "mulhu"
            | "mulhsu"
            | "mulw"
            | "div"
            | "divu"
            | "divw"
            | "divuw"
            | "rem"
            | "remu"
            | "remw"
            | "remuw"
            | "mv"
            | "li"
            | "neg"
            | "negw"
            | "not"
            | "seqz"
            | "snez"
            | "sltz"
            | "sgtz"
            | "sext.w"
            | "sext.b"
            | "sext.h"
            | "zext.b"
            | "zext.h"
            | "zext.w"
            | "andn"
            | "orn"
            | "xnor"
            | "clz"
            | "clzw"
            | "ctz"
            | "ctzw"
            | "cpop"
            | "cpopw"
            | "max"
            | "maxu"
            | "min"
            | "minu"
            | "rol"
            | "rolw"
            | "ror"
            | "rori"
            | "roriw"
            | "rorw"
            | "sh1add"
            | "sh2add"
            | "sh3add"
            | "sh1add.uw"
            | "sh2add.uw"
            | "sh3add.uw"
            | "add.uw"
            | "slli.uw"
            | "rev8"
            | "orc.b"
            | "czero.eqz"
            | "czero.nez"
    )
}

/// The destination of an integer load (`lb`..`ld`, `lbu`/`lhu`/`lwu`, the
/// compressed and load-reserved forms), which `classify_line` leaves as
/// `Other` unless the base is s0.  FP loads write no tracked register.
fn rv_load_dest(line: &str) -> Option<u8> {
    let t = line.trim();
    let (m, ops) = t.split_once(|c: char| c.is_ascii_whitespace())?;
    let m = m.strip_prefix("c.").unwrap_or(m);
    let base = m.split('.').next().unwrap_or(m);
    let is_load = matches!(
        m,
        "lb" | "lh" | "lw" | "ld" | "lbu" | "lhu" | "lwu" | "lwsp" | "ldsp"
    ) || matches!(base, "lr");
    if !is_load {
        return None;
    }
    let first = ops.split(',').next()?.trim();
    let id = parse_reg(first);
    (id != REG_NONE).then_some(id)
}

/// The general-register destination of an FP instruction that writes one
/// (`fmv.x.d`, `fcvt.l.d`, `feq.s`, `fclass.d`, ...).
fn rv_fp_gp_dest(line: &str) -> Option<u8> {
    let t = line.trim();
    let (m, ops) = t.split_once(|c: char| c.is_ascii_whitespace())?;
    let writes_gp = m.starts_with("fmv.x.")
        || (m.starts_with("fcvt.") && matches!(m.split('.').nth(1), Some("w" | "wu" | "l" | "lu")))
        || matches!(m.split('.').next(), Some("feq" | "flt" | "fle" | "fclass"));
    if !writes_gp {
        return None;
    }
    let id = parse_reg(ops.split(',').next()?.trim());
    (id != REG_NONE).then_some(id)
}

/// Does `line` read general registers only as FP-instruction sources
/// (`fmv.d.x f, r`, `fcvt.d.l f, r`): its first operand is an FP register.
fn rv_fp_reads_gp_sources(line: &str) -> bool {
    let t = line.trim();
    let Some((m, ops)) = t.split_once(|c: char| c.is_ascii_whitespace()) else {
        return false;
    };
    let reads = (m.starts_with("fmv.") && m.ends_with(".x"))
        || (m.starts_with("fcvt.") && matches!(m.split('.').nth(2), Some("w" | "wu" | "l" | "lu")));
    reads
        && ops
            .split(',')
            .next()
            .is_some_and(|d| d.trim().starts_with('f'))
}

/// Is `line` a store (`sb`/`sh`/`sw`/`sd`, their FP and compressed forms):
/// every register it names is read.
fn rv_is_store(line: &str) -> bool {
    let m = line.split_whitespace().next().unwrap_or("");
    let m = m.strip_prefix("c.").unwrap_or(m);
    matches!(
        m,
        "sb" | "sh" | "sw" | "sd" | "fsw" | "fsd" | "fsh" | "fsq" | "swsp" | "sdsp"
    )
}

/// (uses, must-defs) of one line, over the tracked registers.
///
/// Uses over-approximate (every named register, the conservative implicit
/// reads), definitions under-approximate (only a destination named once by a
/// classified write): exactly the directions backward liveness needs.
fn rv_uses_defs(line: &str, kind: LineKind) -> (u64, u64) {
    match kind {
        LineKind::Nop | LineKind::Label | LineKind::Directive => return (0, 0),
        _ => {}
    }
    if line == OPAQUE_LINE {
        return (RV_LIVE_ALL, 0);
    }
    let (named, unmodelled) = rv_named_regs(line);
    if unmodelled {
        return (RV_LIVE_ALL, 0);
    }
    let (ireads, iwrites) = classify_implicit_operands_rv(line);
    match kind {
        // A call may read any argument or scratch register (t2 carries a
        // nested function's static chain; helper conventions use t-regs)
        // and the stack, and clobbers every caller-saved register -- unless
        // the emitter published exactly which argument registers it reads.
        LineKind::Call => (
            (named & !rv_bit(REG_RA))
                | rv_annotated_call_reads(line)
                    .unwrap_or(RV_CALLER_SAVED & !rv_bit(REG_RA) | rv_bit(REG_SP)),
            RV_CALLER_SAVED,
        ),
        // The return value (a0/a1), the link register and every register
        // the caller expects back.
        LineKind::Ret => (
            rv_bit(REG_A0) | rv_bit(REG_A1) | rv_bit(REG_RA) | RV_CALLEE_SAVED,
            0,
        ),
        _ => {
            let mut uses = named | ireads;
            let mut defs = iwrites;
            let first = line.trim().split_whitespace().next().unwrap_or("");
            if first == "ecall" {
                uses |= 0xff << REG_A0;
            }
            // `jump label, t6` writes its scratch register.
            if kind == LineKind::Jump && first == "jump" {
                if let Some(scratch) = line.trim().rsplit(',').next().map(str::trim) {
                    let id = parse_reg(scratch);
                    if id != REG_NONE {
                        uses &= !rv_bit(id);
                        defs |= rv_bit(id);
                    }
                }
                return (uses, defs);
            }
            if matches!(kind, LineKind::Branch | LineKind::Jump) || rv_is_store(line) {
                return (uses, defs);
            }
            let dest = match kind {
                LineKind::Move { dst, .. }
                | LineKind::LoadImm { dst }
                | LineKind::LoadAddr { dst }
                | LineKind::SextW { dst, .. } => Some(dst),
                LineKind::LoadS0 { reg, .. } => Some(reg),
                LineKind::Alu => parse_alu_dest(line),
                LineKind::Other => rv_load_dest(line).or_else(|| rv_fp_gp_dest(line)),
                _ => None,
            };
            if let Some(d) = dest
                && d != REG_NONE
                && (d as usize) < NUM_REGS
            {
                let name = reg_name(d);
                let mentions = line
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .filter(|t| *t == name)
                    .count();
                if mentions == 1 {
                    uses &= !rv_bit(d);
                    defs |= rv_bit(d);
                }
            }
            (uses, defs)
        }
    }
}

/// One function's control-flow graph over its lines, shared by the backward
/// problems below (register liveness, upper-word demand).
struct RvCfg {
    /// First line of each block, ascending.
    starts: Vec<usize>,
    succs: Vec<Vec<usize>>,
    /// The block leaves through a tail jump or falls off the end of the
    /// function's text: every register is observed there.
    exits: Vec<bool>,
    /// What `ret` reads: the return-value registers the prologue published
    /// (a0 and a1 when it said nothing), the link register and every
    /// register the caller expects back.
    ret_uses: u64,
    start: usize,
    end: usize,
}

impl RvCfg {
    /// `None` when the CFG is not resolvable (an indirect `jr`, a branch to a
    /// label this text does not define).
    fn build(lines: &[String], kinds: &[LineKind], start: usize, end: usize) -> Option<Self> {
        if start >= end {
            return None;
        }
        let transfers =
            |k: usize| matches!(kinds[k], LineKind::Branch | LineKind::Jump | LineKind::Ret);
        let mut starts: Vec<usize> = vec![start];
        for k in start..end {
            if kinds[k] == LineKind::Other && rv_is_unmodelled_transfer(&lines[k]) {
                return None;
            }
            if k > start && kinds[k] == LineKind::Label {
                starts.push(k);
            }
            if k + 1 < end && transfers(k) {
                starts.push(k + 1);
            }
        }
        starts.sort_unstable();
        starts.dedup();
        let nb = starts.len();
        let bend = |b: usize| if b + 1 < nb { starts[b + 1] } else { end };
        let mut label_block: FxHashMap<&str, usize> = FxHashMap::default();
        for (b, &s) in starts.iter().enumerate() {
            let mut k = s;
            while k < bend(b) && kinds[k] == LineKind::Label {
                if let Some(name) = label_name(&lines[k]) {
                    label_block.insert(name, b);
                }
                k += 1;
            }
        }
        // Lines from different blocks can carry labels mid-block only when a
        // label follows another label; record every label's block.
        for k in start..end {
            if kinds[k] == LineKind::Label
                && let Some(name) = label_name(&lines[k])
            {
                let b = starts.partition_point(|&s| s <= k) - 1;
                label_block.entry(name).or_insert(b);
            }
        }
        let mut succs: Vec<Vec<usize>> = Vec::with_capacity(nb);
        let mut exits: Vec<bool> = vec![false; nb];
        for b in 0..nb {
            let mut t = bend(b);
            while t > starts[b] && matches!(kinds[t - 1], LineKind::Nop | LineKind::Directive) {
                t -= 1;
            }
            let fall = if b + 1 < nb { vec![b + 1] } else { Vec::new() };
            // Falling off the end of the function's text -- a last block
            // that does not end in `ret` or an unconditional jump -- may
            // reach anything: assume the worst.
            if b + 1 == nb
                && (t == starts[b] || !matches!(kinds[t - 1], LineKind::Ret | LineKind::Jump))
            {
                exits[b] = true;
            }
            if t == starts[b] {
                succs.push(fall);
                continue;
            }
            let last = t - 1;
            match kinds[last] {
                LineKind::Ret => succs.push(Vec::new()),
                LineKind::Jump => {
                    let Some(target) = jump_target(&lines[last]) else {
                        return None; // `jr` / an indirect jump
                    };
                    match label_block.get(target) {
                        Some(&tb) => succs.push(vec![tb]),
                        // A jump to a symbol outside this function: a tail
                        // jump, which may read anything a call reads.
                        None if !target.starts_with(".L") => {
                            exits[b] = true;
                            succs.push(Vec::new());
                        }
                        None => return None,
                    }
                }
                LineKind::Branch => {
                    let (_, _, target) = branch_parts(&lines[last])?;
                    let tb = *label_block.get(target)?;
                    let mut s = vec![tb];
                    s.extend(fall);
                    succs.push(s);
                }
                _ => succs.push(fall),
            }
        }
        let mut ret_value = rv_bit(REG_A0) | rv_bit(REG_A1);
        if let Some(rest) = (start..end).find_map(|k| lines[k].trim().strip_prefix(RET_REGS_MARKER))
        {
            ret_value = 0;
            for tok in rest.split_whitespace() {
                match tok {
                    "a0" => ret_value |= rv_bit(REG_A0),
                    "a1" => ret_value |= rv_bit(REG_A1),
                    "-" => {}
                    _ => ret_value |= rv_bit(REG_A0) | rv_bit(REG_A1),
                }
            }
        }
        Some(RvCfg {
            starts,
            succs,
            exits,
            ret_uses: ret_value | rv_bit(REG_RA) | RV_CALLEE_SAVED,
            start,
            end,
        })
    }

    /// Solve a backward problem over the tracked registers to its least
    /// fixed point: `transfer(k, after)` is the set before line `k` given the
    /// set after it, and must be monotone.  Returns the set before every
    /// line of the function; exits start from every register.
    fn solve_backward(&self, transfer: impl FnMut(usize, u64) -> u64) -> Vec<u64> {
        self.solve_backward_from(RV_LIVE_ALL, transfer)
    }

    /// The set after each block-closing branch or jump of a solved backward
    /// problem (`before`, from `solve_backward_from` with `exit_set`): the
    /// union of its successors' entry sets, and `exit_set` where the block
    /// leaves the function.  The next line's set is only the fall-through
    /// successor's -- and a jump's textual successor is no successor at all
    /// -- so `before[k + 1]` would miss what the taken edge reads.  Sorted
    /// by line.
    fn transfer_outs(
        &self,
        kinds: &[LineKind],
        before: &[u64],
        exit_set: u64,
    ) -> Vec<(usize, u64)> {
        let nb = self.starts.len();
        let mut outs = Vec::new();
        for b in 0..nb {
            let end = if b + 1 < nb {
                self.starts[b + 1]
            } else {
                self.end
            };
            let mut t = end;
            while t > self.starts[b] && matches!(kinds[t - 1], LineKind::Nop | LineKind::Directive)
            {
                t -= 1;
            }
            if t == self.starts[b] || !matches!(kinds[t - 1], LineKind::Branch | LineKind::Jump) {
                continue;
            }
            let mut out = if self.exits[b] { exit_set } else { 0 };
            for &succ in &self.succs[b] {
                out |= before[self.starts[succ] - self.start];
            }
            outs.push((t - 1, out));
        }
        outs
    }

    /// `solve_backward` with `exit_set` observed where the function's text
    /// leaves (a tail jump, falling off the end) -- every register of the
    /// problem's own domain.
    fn solve_backward_from(
        &self,
        exit_set: u64,
        mut transfer: impl FnMut(usize, u64) -> u64,
    ) -> Vec<u64> {
        let nb = self.starts.len();
        let bend = |b: usize| {
            if b + 1 < nb {
                self.starts[b + 1]
            } else {
                self.end
            }
        };
        let mut before = vec![0u64; self.end - self.start];
        let mut block_in = vec![0u64; nb];
        loop {
            let mut changed = false;
            for b in (0..nb).rev() {
                let mut set = if self.exits[b] { exit_set } else { 0 };
                for &s in &self.succs[b] {
                    set |= block_in[s];
                }
                for k in (self.starts[b]..bend(b)).rev() {
                    set = transfer(k, set);
                    before[k - self.start] = set;
                }
                if set != block_in[b] {
                    block_in[b] = set;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        before
    }
}

/// The `RvCfg::transfer_outs` entry of line `k`, if it closes a block.
fn transfer_out_at(outs: &[(usize, u64)], k: usize) -> Option<u64> {
    let at = outs.binary_search_by_key(&k, |&(line, _)| line).ok()?;
    Some(outs[at].1)
}

/// Backward liveness of the tracked registers over one function's lines.
struct RvLiveness {
    /// `live_before[k - start]`: registers live immediately before line `k`.
    live_before: Vec<u64>,
    /// `RvCfg::transfer_outs`: what is live after each closing branch/jump.
    transfer_out: Vec<(usize, u64)>,
    start: usize,
    end: usize,
}

impl RvLiveness {
    /// `None` when the CFG is not resolvable (see `RvCfg::build`).
    fn build(lines: &[String], kinds: &[LineKind], start: usize, end: usize) -> Option<Self> {
        let cfg = RvCfg::build(lines, kinds, start, end)?;
        let ret_uses = cfg.ret_uses;
        let live_before = cfg.solve_backward(|k, live| {
            let (u, d) = if kinds[k] == LineKind::Ret {
                (ret_uses, 0)
            } else {
                rv_uses_defs(&lines[k], kinds[k])
            };
            (live & !d) | u
        });
        let transfer_out = cfg.transfer_outs(kinds, &live_before, RV_LIVE_ALL);
        Some(RvLiveness {
            live_before,
            transfer_out,
            start,
            end,
        })
    }

    /// Registers live immediately after line `k`: after a branch, on either
    /// edge.
    fn live_after(&self, k: usize) -> u64 {
        if let Some(set) = transfer_out_at(&self.transfer_out, k) {
            set
        } else if k + 1 >= self.end {
            RV_LIVE_ALL
        } else {
            self.live_before[k + 1 - self.start]
        }
    }
}

/// A control transfer `classify_line` leaves as `Other`: an indirect jump
/// (`jr`, `jalr zero, ...`, the compressed forms), a `tail` call, or any
/// unrecognised `b`-mnemonic.  Its successors are unknown, so a CFG built
/// around it would assume a fall-through that never happens.
fn rv_is_unmodelled_transfer(line: &str) -> bool {
    let t = line.trim();
    let m = t.split_whitespace().next().unwrap_or("");
    let m = m.strip_prefix("c.").unwrap_or(m);
    let zbs = matches!(
        m,
        "bset" | "bseti" | "bclr" | "bclri" | "binv" | "binvi" | "bext" | "bexti" | "brev8"
    );
    matches!(m, "jr" | "jalr" | "tail" | "j" | "jal" | "mret" | "sret")
        || (m.starts_with('b') && !zbs)
}

/// Function extents of the text: `[start, end)` from one column-0 symbol
/// label to the next.
fn rv_function_ranges(lines: &[String], kinds: &[LineKind], n: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    // Text before the first symbol (a bare instruction stream) is a range of
    // its own.
    let mut start: Option<usize> = (n > 0).then_some(0);
    for k in 0..n {
        if kinds[k] == LineKind::Label
            && lines[k].starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        {
            if let Some(s) = start
                && s < k
            {
                ranges.push((s, k));
            }
            start = Some(k);
        }
    }
    if let Some(s) = start {
        ranges.push((s, n));
    }
    ranges
}

// ── Callee-saved homes nobody reads ─────────────────────────────────────────
//
// In a function with calls the allocator homes values in s-registers; copy
// propagation then forwards most of their reads to the argument registers
// they came from (`int keep_none(int a, int b) { return take_two(a, b) * 2;
// }` homed a, b and the result in s1, s7, s8 and read none of them), and
// what is left is a write, a save and a restore per home.  When a function
// never READS sN -- every mention is the prologue save, an epilogue restore
// or a deletable definition -- sN keeps the caller's value throughout once
// the definitions go, so the save and the restores go too (the unwind row
// follows in `drop_orphaned_save_rows`).  Legality:
//   * no line reads sN (the global operand model, `rv_uses_defs`, counts an
//     unmodelled line as reading everything; a `ret`'s read of the callee-
//     saved set is exactly the caller's expectation this keeps);
//   * every other write is a pure single-destination definition or a load;
//   * exactly one prologue save `sd sN, OFF(base)`, and its address text
//     appears only there and in `ld sN` restores -- the slot serves nothing
//     else;
//   * the function holds no protected (inline assembly / volatile) text.
fn eliminate_unread_callee_saved_homes(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        if (start..end).any(|k| lines[k] == OPAQUE_LINE) {
            continue;
        }
        for reg in REG_S1..=REG_S11 {
            let name = reg_name(reg);
            let bit = rv_bit(reg);
            let mem_of = |k: usize, m: &str| -> Option<String> {
                let ops = rv_operands_of(&lines[k], m);
                match ops.as_slice() {
                    [r, addr]
                        if r == name && (addr.ends_with("(s0)") || addr.ends_with("(sp)")) =>
                    {
                        Some(addr.clone())
                    }
                    _ => None,
                }
            };
            let mut save: Option<(usize, String)> = None;
            let mut restores: Vec<usize> = Vec::new();
            let mut writes: Vec<usize> = Vec::new();
            let mut ok = true;
            for k in start..end {
                if matches!(
                    kinds[k],
                    LineKind::Nop | LineKind::Label | LineKind::Directive
                ) || kinds[k] == LineKind::Ret
                {
                    continue;
                }
                if let Some(addr) = mem_of(k, "sd") {
                    if save.is_some() {
                        ok = false;
                        break;
                    }
                    save = Some((k, addr));
                    continue;
                }
                if mem_of(k, "ld").is_some() {
                    restores.push(k);
                    continue;
                }
                let (uses, defs) = rv_uses_defs(&lines[k], kinds[k]);
                if uses & bit != 0 {
                    ok = false;
                    break;
                }
                if defs & bit != 0 {
                    let pure = match kinds[k] {
                        LineKind::Move { .. }
                        | LineKind::LoadImm { .. }
                        | LineKind::LoadAddr { .. }
                        | LineKind::SextW { .. }
                        | LineKind::LoadS0 { .. } => true,
                        LineKind::Alu => {
                            rv_is_pure_int_op(lines[k].split_whitespace().next().unwrap_or(""))
                                && parse_alu_dest(&lines[k]) == Some(reg)
                        }
                        LineKind::Other => rv_load_dest(&lines[k]) == Some(reg),
                        _ => false,
                    };
                    if !pure {
                        ok = false;
                        break;
                    }
                    writes.push(k);
                }
            }
            let Some((save_line, addr)) = save.filter(|_| ok) else {
                continue;
            };
            // The slot serves the save and its restores only.
            let elsewhere = (start..end).any(|k| {
                k != save_line
                    && !restores.contains(&k)
                    && kinds[k] != LineKind::Nop
                    && lines[k].contains(addr.as_str())
            });
            if elsewhere {
                continue;
            }
            for k in std::iter::once(save_line).chain(restores).chain(writes) {
                kinds[k] = LineKind::Nop;
            }
            changed = true;
        }
    }
    changed
}

/// Delete pure definitions of a temporary or argument register that no path
/// reads (global liveness).  Callee-saved registers are left alone: their
/// values are the caller's until the epilogue.
fn eliminate_dead_defs_global(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        let Some(live) = RvLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        for k in start..end {
            let dest = match kinds[k] {
                LineKind::Move { dst, .. }
                | LineKind::LoadImm { dst }
                | LineKind::LoadAddr { dst }
                | LineKind::SextW { dst, .. } => dst,
                LineKind::LoadS0 { reg, .. } => reg,
                LineKind::Alu => {
                    let m = lines[k].split_whitespace().next().unwrap_or("");
                    if !rv_is_pure_int_op(m) {
                        continue;
                    }
                    match parse_alu_dest(&lines[k]) {
                        Some(d) => d,
                        None => continue,
                    }
                }
                // A bit move out of an FP register: no flag, no trap (the
                // conversions and comparisons may raise FP exceptions).
                LineKind::Other
                    if matches!(
                        lines[k].split_whitespace().next(),
                        Some("fmv.x.d" | "fmv.x.w")
                    ) =>
                {
                    match rv_fp_gp_dest(&lines[k]) {
                        Some(d) => d,
                        None => continue,
                    }
                }
                _ => continue,
            };
            let temp_or_arg = dest <= REG_T6 || (REG_A0..=REG_A7).contains(&dest);
            if !temp_or_arg || lines[k] == OPAQUE_LINE {
                continue;
            }
            if live.live_after(k) & rv_bit(dest) == 0 {
                kinds[k] = LineKind::Nop;
                changed = true;
            }
        }
    }
    changed
}

// ── Extensions whose upper word nobody reads ─────────────────────────────────
//
// RV64 holds a 32-bit value in a 64-bit register, and LCCC normalises it where
// its internal form and the psABI's differ: an `unsigned` parameter is
// zero-extended on entry (`slli`/`srli` by 32), an `int` result is sign-
// extended again (`sext.w`) before it is returned, stored or compared.  Many
// of those extensions feed only instructions that read the low word -- the
// `*w` ALU family, `sw`/`sh`/`sb` data, shift amounts, `fcvt.*.w[u]` and
// `fmv.w.x` -- or carry-up and bit-local operations (`add`/`sub`/`mul`,
// `and`/`or`/`xor`, `addi`, `slli` below 32) whose own upper word nobody
// reads.  A backward "upper-word demand" problem over the function's CFG
// finds them: an extension whose result's upper word no path observes
// changes only bits nobody reads, so it becomes a copy, or nothing in its
// self form.  This is the RV64 counterpart of the demanded-bits reasoning
// GCC's `ext-dce` and LLVM's `RISCVOptWInstrs` apply.

/// How a line reads its general-register sources for the upper-word demand:
/// the register it writes (`dest`), the sources whose upper word it observes
/// only when its own result's upper word is demanded (`local`: carries and
/// bit-local logic flow upward only), and the sources whose upper word it
/// always observes (`full`).  A source in neither set is read in its low
/// word only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UpperRoles {
    dest: u64,
    local: u64,
    full: u64,
}

/// The roles of a line whose operands are all plain registers, immediates
/// and `off(base)` addresses of a known shape; `None` for anything else
/// (which the demand then models as reading every register it names in
/// full).
fn rv_upper_roles(line: &str, kind: LineKind) -> Option<UpperRoles> {
    let roles = |dest: u64, local: u64, full: u64| Some(UpperRoles { dest, local, full });
    match kind {
        LineKind::Move { dst, src } => return roles(rv_bit(dst), rv_bit(src), 0),
        LineKind::SextW { dst, .. } | LineKind::LoadImm { dst } => return roles(rv_bit(dst), 0, 0),
        LineKind::StoreS0 { reg, is_word, .. } => {
            let data = if is_word { 0 } else { rv_bit(reg) };
            return roles(0, 0, data | rv_bit(REG_S0));
        }
        LineKind::LoadS0 { reg, .. } => return roles(rv_bit(reg), 0, rv_bit(REG_S0)),
        LineKind::Alu | LineKind::Other => {}
        _ => return None,
    }
    if line == OPAQUE_LINE {
        return None;
    }
    let (m, rest) = line.trim().split_once(|c: char| c.is_ascii_whitespace())?;
    let ops: Vec<&str> = rest.split(',').map(str::trim).collect();
    // A register operand: a tracked register's bit, 0 for `zero`, `None` for
    // anything else (an untracked register, a symbol, a relocation).
    let reg = |s: &str| -> Option<u64> {
        if s == "zero" {
            return Some(0);
        }
        let id = parse_reg(s);
        (id != REG_NONE).then(|| rv_bit(id))
    };
    let imm = |s: &str| parse_asm_i64(s);
    match (m, ops.as_slice()) {
        // A narrow store reads its data's low byte, half or word, and its
        // base in full.
        ("sb" | "sh" | "sw", [data, addr]) => {
            reg(data)?;
            let (off, base) = addr.strip_suffix(')')?.split_once('(')?;
            if !off.trim().is_empty() {
                imm(off.trim())?;
            }
            roles(0, 0, reg(base.trim())?)
        }
        // The `*w` family reads the low word of every source (and `sllw`
        // five bits of its amount).
        (
            "addw" | "subw" | "mulw" | "divw" | "divuw" | "remw" | "remuw" | "sllw" | "srlw"
            | "sraw",
            [d, a, b],
        ) => {
            reg(a)?;
            reg(b)?;
            roles(reg(d)?, 0, 0)
        }
        ("addiw" | "slliw" | "srliw" | "sraiw", [d, a, k]) => {
            reg(a)?;
            imm(k)?;
            roles(reg(d)?, 0, 0)
        }
        ("negw", [d, a]) => {
            reg(a)?;
            roles(reg(d)?, 0, 0)
        }
        // Carries and bit-local logic flow upward only: the low word of the
        // result depends on the low words of the sources alone.
        ("add" | "sub" | "mul" | "and" | "or" | "xor", [d, a, b]) => {
            roles(reg(d)?, reg(a)? | reg(b)?, 0)
        }
        ("addi" | "ori" | "xori", [d, a, k]) => {
            imm(k)?;
            roles(reg(d)?, reg(a)?, 0)
        }
        ("neg" | "not", [d, a]) => roles(reg(d)?, reg(a)?, 0),
        // A nonnegative 12-bit mask clears the upper word whatever the
        // source held there; a negative one copies it.
        ("andi", [d, a, k]) => {
            let a = reg(a)?;
            let local = if imm(k)? >= 0 { 0 } else { a };
            roles(reg(d)?, local, 0)
        }
        // A shift by 32 or more moves the source's low word into the upper
        // one and reads nothing above it; a shorter one is bit-local.
        ("slli", [d, a, k]) => {
            let a = reg(a)?;
            let local = if (32..64).contains(&imm(k)?) { 0 } else { a };
            roles(reg(d)?, local, 0)
        }
        // A variable shift reads six bits of its amount; a left shift's low
        // word depends on the source's low word alone, a right shift's does
        // not.
        ("sll", [d, a, b]) => {
            reg(b)?;
            roles(reg(d)?, reg(a)?, 0)
        }
        ("srl" | "sra", [d, a, b]) => {
            reg(b)?;
            roles(reg(d)?, 0, reg(a)?)
        }
        // Integer-to-FP moves and conversions of a word read its low 32 bits;
        // the FP destination is not a tracked register.
        ("fmv.w.x" | "fcvt.s.w" | "fcvt.s.wu" | "fcvt.d.w" | "fcvt.d.wu", [fd, a, ..])
            if fd.starts_with('f') && ops.len() <= 3 =>
        {
            reg(a)?;
            roles(0, 0, 0)
        }
        _ => None,
    }
}

/// The upper-word demand before line `k`, given the demand after it: the
/// registers whose bits 63..32 some path from here may still observe.
fn rv_upper_demand_before(line: &str, kind: LineKind, after: u64, ret_uses: u64) -> u64 {
    if kind == LineKind::Ret {
        return after | ret_uses;
    }
    match rv_upper_roles(line, kind) {
        Some(r) => {
            let local = if after & r.dest != 0 { r.local } else { 0 };
            (after & !r.dest) | r.full | local
        }
        None => {
            let (uses, defs) = rv_uses_defs(line, kind);
            (after & !defs) | uses
        }
    }
}

/// `srli rD, rD, 32`: the second half of a zero extension, returning rD.
fn zext32_tail(line: &str) -> Option<u8> {
    let rest = line.trim().strip_prefix("srli ")?;
    let mut ops = rest.split(',').map(str::trim);
    let (d, s, k) = (ops.next()?, ops.next()?, ops.next()?);
    if ops.next().is_some() || d != s || parse_asm_i64(k) != Some(32) {
        return None;
    }
    let id = parse_reg(d);
    (id != REG_NONE).then_some(id)
}

/// `slli dst, rS, 32`: the first half of a zero extension into `dst`,
/// returning rS.
fn zext32_head(line: &str, dst: u8) -> Option<u8> {
    let rest = line.trim().strip_prefix("slli ")?;
    let mut ops = rest.split(',').map(str::trim);
    let (d, s, k) = (ops.next()?, ops.next()?, ops.next()?);
    if ops.next().is_some() || parse_reg(d) != dst || parse_asm_i64(k) != Some(32) {
        return None;
    }
    let id = parse_reg(s);
    (id != REG_NONE).then_some(id)
}

/// Turn every `sext.w` and `slli; srli` zero extension whose result's upper
/// word no path observes into a copy (deleted in the self form).
///
/// Each rewrite keeps the demand solution it was decided on: the extension
/// read only its source's low word, and the copy that replaces it reads the
/// source's upper word only where the result's is demanded, which it is
/// not.  So one solve serves every rewrite in the function.
fn eliminate_undemanded_extensions(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        let Some(cfg) = RvCfg::build(lines, kinds, start, end) else {
            continue;
        };
        let ret_uses = cfg.ret_uses;
        let before = cfg.solve_backward(|k, after| {
            rv_upper_demand_before(&lines[k], kinds[k], after, ret_uses)
        });
        // Every rewritten line is a straight-line instruction, so the
        // demand after it is the demand before the next line.
        let demanded_after = |k: usize| {
            if k + 1 >= end {
                RV_LIVE_ALL
            } else {
                before[k + 1 - start]
            }
        };
        for k in start..end {
            match kinds[k] {
                LineKind::SextW { dst, src } if demanded_after(k) & rv_bit(dst) == 0 => {
                    if dst == src {
                        kinds[k] = LineKind::Nop;
                    } else {
                        lines[k] = format!("    mv {}, {}", reg_name(dst), reg_name(src));
                        kinds[k] = LineKind::Move { dst, src };
                    }
                    changed = true;
                }
                LineKind::Alu => {
                    let Some(dst) = zext32_tail(&lines[k]) else {
                        continue;
                    };
                    if demanded_after(k) & rv_bit(dst) != 0 {
                        continue;
                    }
                    let Some(j) = previous_live_line(kinds, k).filter(|&j| j >= start) else {
                        continue;
                    };
                    let Some(src) = zext32_head(&lines[j], dst) else {
                        continue;
                    };
                    kinds[k] = LineKind::Nop;
                    if src == dst {
                        kinds[j] = LineKind::Nop;
                    } else {
                        lines[j] = format!("    mv {}, {}", reg_name(dst), reg_name(src));
                        kinds[j] = LineKind::Move { dst, src };
                    }
                    changed = true;
                }
                _ => {}
            }
        }
    }
    changed
}

/// Rename every read of `old` in `line` to `new`, or `None` when the line's
/// shape is not known well enough to do so.
fn rv_rename_reads(line: &str, kind: LineKind, old: u8, new: u8) -> Option<String> {
    let (old_name, new_name) = (reg_name(old), reg_name(new));
    let trimmed = line.trim();
    let (mnemonic, ops) = trimmed.split_once(|c: char| c.is_ascii_whitespace())?;
    let ops = ops.trim();
    let rename_all = |text: &str| -> String {
        text.split_inclusive(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map(|piece| {
                let word_len = piece
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(piece.len());
                if &piece[..word_len] == old_name {
                    format!("{new_name}{}", &piece[word_len..])
                } else {
                    piece.to_string()
                }
            })
            .collect()
    };
    match kind {
        LineKind::Branch => {
            // `b<cc> rs1, rs2, label` / `b<cc>z rs, label`: every operand but
            // the label is a source.
            let (regs, label) = ops.rsplit_once(',')?;
            Some(format!(
                "    {mnemonic} {}, {}",
                rename_all(regs),
                label.trim()
            ))
        }
        _ if rv_is_store(line) => Some(format!("    {mnemonic} {}", rename_all(ops))),
        LineKind::Move { dst, src } if src == old && dst != new => {
            Some(format!("    mv {}, {}", reg_name(dst), new_name))
        }
        LineKind::SextW { dst, src } if src == old => {
            Some(format!("    sext.w {}, {}", reg_name(dst), new_name))
        }
        LineKind::Alu | LineKind::StoreS0 { .. } => {
            replace_source_reg_in_instruction(line, old_name, new_name)
        }
        // A load reads only its address: `ld t0, 8(t1)`; an FP instruction
        // with general sources (`fmv.d.x ft0, t1`) reads them after its FP
        // destination.
        LineKind::Other if rv_load_dest(line).is_some() || rv_fp_reads_gp_sources(line) => {
            replace_source_reg_in_instruction(line, old_name, new_name)
        }
        _ => None,
    }
}

/// Split a memory instruction `op reg, off(base)` into its parts.
fn rv_mem_parts(line: &str) -> Option<(&str, &str, i64, &str)> {
    let t = line.trim();
    let (m, ops) = t.split_once(|c: char| c.is_ascii_whitespace())?;
    let (reg, addr) = ops.split_once(',')?;
    let addr = addr.trim();
    let open = addr.find('(')?;
    let base = addr[open + 1..].strip_suffix(')')?.trim();
    let off_text = addr[..open].trim();
    let off = if off_text.is_empty() {
        0
    } else {
        parse_asm_i64(off_text)?
    };
    Some((m, reg.trim(), off, base))
}

/// Fold an address computation or a redundant extension into the memory
/// access next to it, when the temporary it went through dies there:
///
/// ```text
///   addi t0, a0, 8              ld t1, 8(a0)
///   ld   t1, 0(t0)       ->
///
///   lw     t0, 4(a0)            lw a6, 4(a0)
///   sext.w a6, t0        ->     (lw/lh/lb already sign-extend on RV64;
///                                lwu becomes lw)
/// ```
///
/// The emitter computes every non-frame address into t0 and stages every
/// loaded word through it, so struct-field and array accesses paid one extra
/// instruction each (GCC and Clang address `off(base)` directly).  Both
/// lines must be adjacent, so liveness computed before a fold stays valid
/// for the next one: a fold only makes a register dead.
fn fold_into_memory_operands(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        let Some(live) = RvLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        for i in start..end {
            if kinds[i] == LineKind::Nop || lines[i] == OPAQUE_LINE {
                continue;
            }
            let mut j = i + 1;
            while j < end && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j >= end || lines[j] == OPAQUE_LINE {
                continue;
            }
            // addi rT, rB, imm ; <mem> rX, off(rT)
            if kinds[i] == LineKind::Alu {
                let t = lines[i].trim();
                let Some(rest) = t.strip_prefix("addi ") else {
                    continue;
                };
                let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
                let [rt, rb, imm] = parts.as_slice() else {
                    continue;
                };
                let (rt_id, rb_id) = (parse_reg(rt), parse_reg(rb));
                let Some(imm) = parse_asm_i64(imm) else {
                    continue;
                };
                if rt_id == REG_NONE
                    || rb_id == REG_NONE
                    || matches!(rt_id, REG_SP | REG_S0 | REG_RA)
                {
                    continue;
                }
                let Some((m, reg, off, base)) = rv_mem_parts(&lines[j]) else {
                    continue;
                };
                if base != *rt {
                    continue;
                }
                let is_load = rv_load_dest(&lines[j]).is_some() || matches!(m, "flw" | "fld");
                let is_store = rv_is_store(&lines[j]) && !m.starts_with("c.");
                if !(is_load || is_store) || m.starts_with("lr") {
                    continue;
                }
                // A store must not need rT's value as its data.
                if is_store && reg == *rt {
                    continue;
                }
                let folded = imm + off;
                if !(-2048..=2047).contains(&folded) {
                    continue;
                }
                let redefined = is_load && parse_reg(reg) == rt_id;
                if !redefined && live.live_after(j) & rv_bit(rt_id) != 0 {
                    continue;
                }
                let new_line = format!("    {m} {reg}, {folded}({rb})");
                kinds[i] = LineKind::Nop;
                kinds[j] = classify_line(&new_line);
                lines[j] = new_line;
                changed = true;
                continue;
            }
            // l{w,h,b}[u] rT, off(rB) ; sext.w rD, rT
            if let LineKind::SextW { dst, src } = kinds[j]
                && let Some(rt_id) = rv_load_dest(&lines[i])
                && rt_id == src
                && let Some((m, _, off, base)) = rv_mem_parts(&lines[i])
            {
                let new_m = match m {
                    "lw" | "lwu" => "lw",
                    "lh" | "lhu" | "lb" | "lbu" => m,
                    _ => continue,
                };
                if dst != src && live.live_after(j) & rv_bit(src) != 0 {
                    continue;
                }
                let new_line = format!("    {new_m} {}, {off}({base})", reg_name(dst));
                kinds[j] = LineKind::Nop;
                kinds[i] = classify_line(&new_line);
                lines[i] = new_line;
                changed = true;
            }
        }
    }
    changed
}

/// An FP computation, FP load or FP move that names no general register
/// other than a load's base (`fadd.d ft0, ft0, ft1`, `fld ft1, 8(a0)`,
/// `fmv.d.x ft1, t0`): it writes no general register and no memory.
fn rv_is_pure_fp_line(line: &str) -> bool {
    let t = line.trim();
    let Some((m, _)) = t.split_once(|c: char| c.is_ascii_whitespace()) else {
        return false;
    };
    if !m.starts_with('f') || m.starts_with("fs") || m == "fence" || m.starts_with("fence.") {
        return false; // FP stores, fences
    }
    if m.starts_with("fmv.x.") || rv_fp_gp_dest(line).is_some() {
        return false;
    }
    // frcsr/fscsr/frflags... touch CSRs.
    !matches!(
        m,
        "frcsr" | "fscsr" | "frrm" | "fsrm" | "frflags" | "fsflags" | "fsrmi" | "fsflagsi"
    )
}

/// Fold the general-register round trips the emitter puts around every FP
/// operation (FP values have no register home on RISC-V, so each operand is
/// fetched into ft0/ft1 through a GPR and each result leaves through one):
///
/// ```text
///   ld t0, 8(a0) ; ... ; fmv.d.x ft1, t0   ->  fld ft1, 8(a0)
///   ld t0, 8(a0) ; ... ; fmv.w.x ft1, t0   ->  flw ft1, 8(a0)
///   fmv.x.d t0, ft0 ; ... ; sd t0, 0(a0)   ->  fsd ft0, 0(a0)
///   fmv.x.d t0, ft0 ; ... ; fmv.d.x ft1, t0 -> fmv.d ft1, ft0
///   fmv.x.w t0, fa0 ; ... ; sd t0, -24(s0) ->  fsw fa0, -24(s0)
/// ```
///
/// The temporary must die at the consumer.  A load may be sunk past up to
/// three FP-only lines (no general write, no memory write, no label), which
/// is how the emitter interleaves the two operand fetches; `fmv.w.x` reads
/// only the low word of a doubleword reload, which is the word `flw` loads
/// (little-endian).  A bit move out of an FP register reaches its consumer
/// past lines that leave the temporary and the FP source alone, the consumer
/// staying where it is; an FP-to-FP move whose source is rewritten on the
/// way moves up to the bit move instead, when nothing in between names its
/// destination.  The emitter spills a single through `fmv.x.w` and a
/// doubleword `sd`, whose upper word is the sign copy of bit 31: it becomes
/// `fsw` when the function's frame census shows no read of that upper word.
/// `fmv.d` is `fsgnj.d`: a bit-exact copy, NaN payload included.
fn fold_fp_bounces(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    const WINDOW: usize = 8;
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        let Some(live) = RvLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        // Loads first: a doubleword reload folded into `flw` is what lets
        // the census below narrow the spill that feeds it.
        for i in start..end {
            if kinds[i] == LineKind::Nop || lines[i] == OPAQUE_LINE {
                continue;
            }
            // l{d,w,wu} tA, off(rB) ; [<= 3 FP-only lines] ; fmv.{d,w}.x fD, tA
            let Some(ta) = rv_load_dest(&lines[i]) else {
                continue;
            };
            let Some((m, _, off, base)) = rv_mem_parts(&lines[i]) else {
                continue;
            };
            if !matches!(m, "ld" | "lw" | "lwu") {
                continue;
            }
            let ta_name = reg_name(ta);
            if base == ta_name {
                continue;
            }
            let mut j = i + 1;
            let mut between = 0;
            let mut found = None;
            while j < end && between <= 3 {
                if kinds[j] == LineKind::Nop {
                    j += 1;
                    continue;
                }
                let t = lines[j].trim();
                // `fmv.d.x` reads all 64 bits: only a doubleword load feeds it.
                let consumer = t
                    .strip_prefix("fmv.w.x ")
                    .map(|ops| ("w", ops))
                    .or_else(|| {
                        t.strip_prefix("fmv.d.x ")
                            .filter(|_| m == "ld")
                            .map(|ops| ("d", ops))
                    });
                if let Some((width, ops)) = consumer
                    && let Some((fd, src)) = ops.split_once(',')
                    && src.trim() == ta_name
                {
                    found = Some((j, width, fd.trim().to_string()));
                    break;
                }
                // Only FP-only lines -- no general-register write, no
                // memory write -- that do not read tA may be stepped
                // over: the base cannot change under them.
                if kinds[j] != LineKind::Other
                    || !rv_is_pure_fp_line(&lines[j])
                    || rv_named_regs(&lines[j]).0 & rv_bit(ta) != 0
                {
                    break;
                }
                between += 1;
                j += 1;
            }
            let Some((j, width, fd)) = found else {
                continue;
            };
            if !fd.starts_with('f') || live.live_after(j) & rv_bit(ta) != 0 {
                continue;
            }
            let new_line = format!("    fl{width} {fd}, {off}({base})");
            kinds[i] = LineKind::Nop;
            kinds[j] = classify_line(&new_line);
            lines[j] = new_line;
            changed = true;
        }
        let frame_reads = rv_frame_reads(lines, kinds, start, end);
        for i in start..end {
            if kinds[i] == LineKind::Nop || lines[i] == OPAQUE_LINE {
                continue;
            }
            // fmv.x.{d,w} tA, fS ; ... ; (s{d,w} tA, off(rB) | fmv.{d,w}.x fD, tA)
            let Some(ta) = rv_fp_gp_dest(&lines[i]) else {
                continue;
            };
            let Some(rest) = lines[i].trim().strip_prefix("fmv.x.") else {
                continue;
            };
            let width = if rest.starts_with('d') {
                "d"
            } else if rest.starts_with('w') {
                "w"
            } else {
                continue;
            };
            let fs = rest[1..].split(',').nth(1).map(str::trim).unwrap_or("");
            let Some(fs_id) = rv_fp_reg(fs) else {
                continue;
            };
            let ta_name = reg_name(ta);
            let mut j = i + 1;
            let mut steps = 0;
            let mut found = None;
            // Whether a line in between rewrites the FP source, and the FP
            // registers the lines in between name.
            let mut fs_clobbered = false;
            let mut between_fp = 0u32;
            while j < end && steps < WINDOW {
                if kinds[j] == LineKind::Nop || kinds[j] == LineKind::Directive {
                    j += 1;
                    continue;
                }
                if rv_fp_barrier(&lines[j], kinds[j]) {
                    break;
                }
                let (named, unmodelled) = rv_named_regs(&lines[j]);
                if unmodelled {
                    break;
                }
                if named & rv_bit(ta) != 0 {
                    found = Some(j);
                    break;
                }
                let Some(roles) = rv_fp_roles(&lines[j]) else {
                    break;
                };
                fs_clobbered |= roles.def == Some(fs_id);
                between_fp |= roles.uses | roles.def_bit();
                steps += 1;
                j += 1;
            }
            let Some(j) = found else {
                continue;
            };
            if live.live_after(j) & rv_bit(ta) != 0 {
                continue;
            }
            let fs = rv_fp_name(fs_id);
            // The consumer keeps its place and reads the FP source there,
            // which must still hold the value -- or, for an FP-to-FP move
            // whose source is rewritten on the way, the move takes the bit
            // move's place when nothing in between names its destination.
            let mut at = j;
            let new_line = if fs_clobbered {
                lines[j]
                    .trim()
                    .strip_prefix(&format!("fmv.{width}.x "))
                    .and_then(|ops| ops.split_once(','))
                    .filter(|(fd, src)| {
                        src.trim() == ta_name
                            && rv_fp_reg(fd.trim()).is_some_and(|d| between_fp & rv_fp_bit(d) == 0)
                    })
                    .map(|(fd, _)| {
                        at = i;
                        let fmt = if width == "w" { "s" } else { "d" };
                        format!("    fmv.{fmt} {}, {fs}", fd.trim())
                    })
            } else if let Some((m, reg, off, base)) = rv_mem_parts(&lines[j])
                && reg == ta_name
                && base != ta_name
            {
                // `fmv.x.w; sd` spills the word with its sign copy above it:
                // narrow it when nothing in the function reads that upper word.
                let upper_unread = |off: i64| {
                    base == "s0"
                        && i32::try_from(off).is_ok_and(|off| {
                            frame_reads.as_ref().is_some_and(|reads| {
                                reads
                                    .iter()
                                    .all(|&(lo, sz)| !(lo < off + 8 && off + 4 < lo + sz))
                            })
                        })
                };
                match (width, m) {
                    ("d", "sd") | ("w", "sw") => Some(format!("    fs{width} {fs}, {off}({base})")),
                    ("w", "sd") if upper_unread(off) => {
                        Some(format!("    fsw {fs}, {off}({base})"))
                    }
                    _ => None,
                }
            } else if let Some(ops) = lines[j].trim().strip_prefix(&format!("fmv.{width}.x ")) {
                // The FP-to-FP move is `fmv.s`/`fmv.d` (fsgnj); `fmv.w`
                // names only the bit moves `fmv.w.x`/`fmv.x.w`.
                let fmt = if width == "w" { "s" } else { "d" };
                match ops.split_once(',') {
                    Some((fd, src)) if src.trim() == ta_name => {
                        Some(format!("    fmv.{fmt} {}, {fs}", fd.trim()))
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(new_line) = new_line {
                let gone = if at == i { j } else { i };
                kinds[gone] = LineKind::Nop;
                kinds[at] = classify_line(&new_line);
                lines[at] = new_line;
                changed = true;
            }
        }
    }
    changed
}

// ── FP register dataflow ─────────────────────────────────────────────────────
//
// FP values have no register home on RISC-V: the emitter fetches every
// operand into ft0/ft1 -- from the frame slot the prologue spilled an
// argument to, or through a general register -- and every result leaves
// through a copy.  `fold_fp_bounces` removes the general-register legs; the
// passes below remove the rest the way the integer passes treat the general
// registers: a slot reload becomes a copy of the register that was stored, a
// copy into a scratch is forwarded into its readers, a result computed into
// a scratch only to be copied out is computed into the copy's destination,
// and a frame nothing uses any more is not set up.
//
// Every rewrite relies on one invariant of the emitter's output: a
// single-precision value in an FP register is NaN-boxed.  The psABI requires
// it of arguments and results, and every instruction that writes a single
// (`.s` arithmetic, `flw`, `fmv.w.x`, `fcvt.s.*`) boxes it, so `fmv.s` copies
// a single bit for bit, exactly as `fmv.d` (`fsgnj.d`) copies a double.

/// FP temporaries ft0-ft7 (f0-f7) and ft8-ft11 (f28-f31): never an ABI value.
const RV_FP_TEMPS: u32 = 0xff | (0xf << 28);
/// FP argument registers fa0-fa7 (f10-f17).
const RV_FP_ARGS: u32 = 0xff << 10;
/// FP return-value registers fa0/fa1.
const RV_FP_RET: u32 = 0x3 << 10;
/// FP callee-saved registers fs0/fs1 (f8/f9) and fs2-fs11 (f18-f27).
const RV_FP_CALLEE_SAVED: u32 = (0x3 << 8) | (0x3ff << 18);

/// The architectural number of an FP register name: the ABI spellings
/// (`ft0`-`ft11`, `fs0`-`fs11`, `fa0`-`fa7`) and `f0`-`f31`.
fn rv_fp_reg(name: &str) -> Option<u8> {
    let num = |prefix: &str| -> Option<u8> {
        let digits = name.strip_prefix(prefix)?;
        let v = digits.parse::<u8>().ok()?;
        (v.to_string() == digits).then_some(v)
    };
    if let Some(v) = num("ft") {
        return match v {
            0..=7 => Some(v),
            8..=11 => Some(v + 20),
            _ => None,
        };
    }
    if let Some(v) = num("fs") {
        return match v {
            0 | 1 => Some(v + 8),
            2..=11 => Some(v + 16),
            _ => None,
        };
    }
    if let Some(v) = num("fa") {
        return (v <= 7).then_some(v + 10);
    }
    num("f").filter(|&v| v < 32)
}

/// The ABI name of FP register `id`, the spelling the emitter uses.
fn rv_fp_name(id: u8) -> &'static str {
    const NAMES: [&str; 32] = [
        "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1", "fa2",
        "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7", "fs8", "fs9",
        "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
    ];
    NAMES[usize::from(id & 31)]
}

fn rv_fp_bit(id: u8) -> u32 {
    1u32 << (id & 31)
}

/// The static rounding-mode operand of an FP computation or conversion.
fn rv_is_rounding_mode(op: &str) -> bool {
    matches!(op, "rne" | "rtz" | "rdn" | "rup" | "rmm" | "dyn")
}

/// How one instruction line touches the FP registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RvFpRoles {
    /// The FP register the line writes -- always its first operand.
    def: Option<u8>,
    /// Bit k: operand k is an FP register the line reads.
    src_ops: u8,
    /// The FP registers the line reads.
    uses: u32,
    /// What it writes is a single-precision value, NaN-boxed.
    single: bool,
}

impl RvFpRoles {
    fn def_bit(&self) -> u32 {
        self.def.map_or(0, rv_fp_bit)
    }
}

/// The FP roles of a line that names no FP register (`Some` with nothing
/// read or written), `None` when it does name one.
fn rv_fp_free(line: &str) -> Option<RvFpRoles> {
    let names_fp = line
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|tok| rv_fp_reg(tok).is_some());
    (!names_fp).then(RvFpRoles::default)
}

/// The FP roles of an instruction line, over the F and D forms the emitter
/// writes: arithmetic, sign injection, fused multiply-add, conversions,
/// comparisons, classification, bit moves, loads and stores.  `None` when the
/// line names an FP register in any other shape -- callers then assume it
/// reads and writes every FP register.
fn rv_fp_roles(line: &str) -> Option<RvFpRoles> {
    let t = line.trim();
    let (m, rest) = match t.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((m, rest)) => (m, rest.trim()),
        None => (t, ""),
    };
    let ops: Vec<&str> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split(',').map(str::trim).collect()
    };
    let fp = |k: usize| ops.get(k).and_then(|op| rv_fp_reg(op));
    // `arity` register operands, optionally followed by a rounding mode;
    // operand 0 is the FP destination when `def`, a general register
    // otherwise; `srcs` are the FP sources, every other operand past 0 a
    // general register.
    let shape = |def: bool, srcs: &[usize], arity: usize, rm: bool, single: bool| {
        let extra = ops.len().checked_sub(arity)?;
        if extra > usize::from(rm) || (extra == 1 && !rv_is_rounding_mode(ops[arity])) {
            return None;
        }
        let mut roles = RvFpRoles {
            def: None,
            single,
            ..RvFpRoles::default()
        };
        for k in 0..arity {
            match (k == 0 && def, srcs.contains(&k)) {
                (true, _) => roles.def = Some(fp(0)?),
                (false, true) => {
                    roles.uses |= rv_fp_bit(fp(k)?);
                    roles.src_ops |= 1 << k;
                }
                (false, false) => {
                    if fp(k).is_some() || (ops[k] != "zero" && parse_reg(ops[k]) == REG_NONE) {
                        return None;
                    }
                }
            }
        }
        Some(roles)
    };
    let mut parts = m.split('.');
    let base = parts.next().unwrap_or("");
    let (f1, f2) = (parts.next(), parts.next());
    if parts.next().is_some() {
        return rv_fp_free(t);
    }
    let real = |f: Option<&str>| matches!(f, Some("s" | "d"));
    let int = |f: Option<&str>| matches!(f, Some("w" | "wu" | "l" | "lu"));
    let single = f1 == Some("s");
    match (base, f2) {
        ("fadd" | "fsub" | "fmul" | "fdiv", None) if real(f1) => {
            shape(true, &[1, 2], 3, true, single)
        }
        ("fmin" | "fmax" | "fsgnj" | "fsgnjn" | "fsgnjx", None) if real(f1) => {
            shape(true, &[1, 2], 3, false, single)
        }
        ("fsqrt", None) if real(f1) => shape(true, &[1], 2, true, single),
        ("fmv" | "fneg" | "fabs", None) if real(f1) => shape(true, &[1], 2, false, single),
        ("fmadd" | "fmsub" | "fnmadd" | "fnmsub", None) if real(f1) => {
            shape(true, &[1, 2, 3], 4, true, single)
        }
        ("feq" | "flt" | "fle", None) if real(f1) => shape(false, &[1, 2], 3, false, false),
        ("fclass", None) if real(f1) => shape(false, &[1], 2, false, false),
        ("fcvt", Some(_)) if real(f1) && real(f2) && f1 != f2 => shape(true, &[1], 2, true, single),
        ("fcvt", Some(_)) if int(f1) && real(f2) => shape(false, &[1], 2, true, false),
        ("fcvt", Some(_)) if real(f1) && int(f2) => shape(true, &[], 2, true, single),
        ("fmv", Some("x")) if matches!(f1, Some("w" | "d")) => {
            shape(true, &[], 2, false, f1 == Some("w"))
        }
        ("fmv", Some("w" | "d")) if f1 == Some("x") => shape(false, &[1], 2, false, false),
        ("flw" | "fld" | "fsw" | "fsd", None) if f1.is_none() => {
            let (_, reg, _, mem_base) = rv_mem_parts(t)?;
            let reg = rv_fp_reg(reg)?;
            if parse_reg(mem_base) == REG_NONE || ops.len() != 2 {
                return None;
            }
            Some(if base.starts_with("fl") {
                RvFpRoles {
                    def: Some(reg),
                    single: base == "flw",
                    ..RvFpRoles::default()
                }
            } else {
                RvFpRoles {
                    src_ops: 1,
                    uses: rv_fp_bit(reg),
                    ..RvFpRoles::default()
                }
            })
        }
        _ => rv_fp_free(t),
    }
}

/// `line` with each FP register operand `k` in `ops_mask` that names `from`
/// renamed to `to`.
fn rv_rename_fp_operands(line: &str, ops_mask: u8, from: u8, to: u8) -> Option<String> {
    let t = line.trim();
    let (m, rest) = t.split_once(|c: char| c.is_ascii_whitespace())?;
    let indent = &line[..line.len() - line.trim_start().len()];
    let ops: Vec<&str> = rest
        .split(',')
        .map(str::trim)
        .enumerate()
        .map(|(k, op)| {
            if k < 8 && ops_mask & (1 << k) != 0 && rv_fp_reg(op) == Some(from) {
                rv_fp_name(to)
            } else {
                op
            }
        })
        .collect();
    Some(format!("{indent}{m} {}", ops.join(", ")))
}

/// A register-to-register FP copy `fmv.{s,d} fD, fS`: `(fD, fS, single)`.
fn rv_fp_copy(line: &str) -> Option<(u8, u8, bool)> {
    let t = line.trim();
    let (m, _) = t.split_once(|c: char| c.is_ascii_whitespace())?;
    if m != "fmv.d" && m != "fmv.s" {
        return None;
    }
    let roles = rv_fp_roles(t)?;
    if roles.uses.count_ones() != 1 {
        return None;
    }
    let src = u8::try_from(roles.uses.trailing_zeros()).ok()?;
    Some((roles.def?, src, m == "fmv.s"))
}

/// (uses, defs) of one line over the FP registers (bit k = fk), for backward
/// liveness: uses over-approximate, definitions under-approximate.
fn rv_fp_uses_defs(line: &str, kind: LineKind) -> (u32, u32) {
    match kind {
        LineKind::Nop | LineKind::Label | LineKind::Directive => (0, 0),
        // A callee may read every FP argument register and clobbers every
        // caller-saved one.
        LineKind::Call => (RV_FP_ARGS, RV_FP_TEMPS | RV_FP_ARGS),
        // The FP results and every register the caller expects back.
        LineKind::Ret => (RV_FP_RET | RV_FP_CALLEE_SAVED, 0),
        _ if line == OPAQUE_LINE => (u32::MAX, 0),
        _ => match rv_fp_roles(line) {
            Some(roles) => (roles.uses, roles.def_bit()),
            None => (u32::MAX, 0),
        },
    }
}

/// Backward liveness of the FP registers over one function's lines.
struct RvFpLiveness {
    /// `live_before[k - start]`: FP registers live immediately before line k.
    live_before: Vec<u64>,
    /// `RvCfg::transfer_outs`: what is live after each closing branch/jump.
    transfer_out: Vec<(usize, u64)>,
    start: usize,
    end: usize,
}

impl RvFpLiveness {
    /// `None` when the CFG is not resolvable (see `RvCfg::build`).
    fn build(lines: &[String], kinds: &[LineKind], start: usize, end: usize) -> Option<Self> {
        let cfg = RvCfg::build(lines, kinds, start, end)?;
        let live_before = cfg.solve_backward_from(u64::from(u32::MAX), |k, live| {
            let (uses, defs) = rv_fp_uses_defs(&lines[k], kinds[k]);
            (live & !u64::from(defs)) | u64::from(uses)
        });
        let transfer_out = cfg.transfer_outs(kinds, &live_before, u64::from(u32::MAX));
        Some(RvFpLiveness {
            live_before,
            transfer_out,
            start,
            end,
        })
    }

    /// FP registers live immediately before line `k`.
    fn live_before(&self, k: usize) -> u32 {
        // The solver only ever produces sets of the 32 FP bits.
        self.live_before
            .get(k.wrapping_sub(self.start))
            .map_or(u32::MAX, |&set| {
                u32::try_from(set & u64::from(u32::MAX)).unwrap_or(u32::MAX)
            })
    }

    /// FP registers live immediately after line `k`: after a branch, on
    /// either edge.
    fn live_after(&self, k: usize) -> u32 {
        if let Some(set) = transfer_out_at(&self.transfer_out, k) {
            // The solver only ever produces sets of the 32 FP bits.
            u32::try_from(set & u64::from(u32::MAX)).unwrap_or(u32::MAX)
        } else if k + 1 >= self.end {
            u32::MAX
        } else {
            self.live_before(k + 1)
        }
    }
}

/// A line FP facts do not survive: a join point, a transfer, a call, an
/// opaque body, a system call.
fn rv_fp_barrier(line: &str, kind: LineKind) -> bool {
    matches!(
        kind,
        LineKind::Label | LineKind::Branch | LineKind::Jump | LineKind::Ret | LineKind::Call
    ) || line == OPAQUE_LINE
        || (kind == LineKind::Other && rv_is_unmodelled_transfer(line))
        || matches!(
            line.split_whitespace().next(),
            Some("ecall" | "ebreak" | "call" | "tail" | "jal" | "jalr" | "jr")
        )
}

/// The byte width of a scalar load or store mnemonic, and whether it stores.
fn rv_access_width(m: &str) -> Option<(i32, bool)> {
    Some(match m {
        "ld" | "fld" => (8, false),
        "lw" | "lwu" | "flw" => (4, false),
        "lh" | "lhu" | "flh" => (2, false),
        "lb" | "lbu" => (1, false),
        "flq" => (16, false),
        "sd" | "fsd" => (8, true),
        "sw" | "fsw" => (4, true),
        "sh" | "fsh" => (2, true),
        "sb" => (1, true),
        "fsq" => (16, true),
        _ => return None,
    })
}

/// A store to a frame slot: `(offset, size)`.
fn rv_frame_store(line: &str) -> Option<(i32, i32)> {
    let (m, _, off, base) = rv_mem_parts(line)?;
    match rv_access_width(m)? {
        (size, true) if base == "s0" => Some((i32::try_from(off).ok()?, size)),
        _ => None,
    }
}

/// Forward a frame slot's FP value into the reloads its store reaches:
///
/// ```text
///   fsd fa0, -24(s0) ; ... ; fld ft0, -24(s0)   ->  fmv.d   ft0, fa0
///   fsd fa0, -24(s0) ; ... ; ld  t0, -24(s0)    ->  fmv.x.d t0, fa0
///   fsw fa0, -24(s0) ; ... ; flw ft0, -24(s0)   ->  fmv.s   ft0, fa0
///   fsw fa0, -24(s0) ; ... ; lw  t0, -24(s0)    ->  fmv.x.w t0, fa0
/// ```
///
/// `fmv.x.w` sign-extends the low word exactly as `lw` does.  A fact "slot
/// holds register r's bits" is born at an FP store and dies at a store
/// overlapping the slot, any `sp`-based store, a write to r, a call or an
/// opaque line; a forward dataflow over the function's CFG keeps the facts
/// every predecessor of a join agrees on, so an argument the prologue
/// spilled is still answered from its register after an `if`.  The frame
/// must be private -- its address never escapes and every slot access has
/// a known width (`rv_frame_reads`) -- so no store through another base can
/// reach a slot.
fn forward_fp_slot_reloads(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        if rv_frame_reads(lines, kinds, start, end).is_none() {
            continue;
        }
        let Some(cfg) = RvCfg::build(lines, kinds, start, end) else {
            continue;
        };
        let nb = cfg.starts.len();
        let bend = |b: usize| if b + 1 < nb { cfg.starts[b + 1] } else { end };
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
        for (b, succs) in cfg.succs.iter().enumerate() {
            for &s in succs {
                preds[s].push(b);
            }
        }
        // The facts at each block's exit; `None` until a path reaches it.
        let mut exits: Vec<Option<Vec<SlotFact>>> = vec![None; nb];
        let entry = |b: usize, exits: &[Option<Vec<SlotFact>>]| -> Option<Vec<SlotFact>> {
            if b == 0 {
                return Some(Vec::new());
            }
            let mut met: Option<Vec<SlotFact>> = None;
            for &p in &preds[b] {
                if let Some(out) = &exits[p] {
                    met = Some(match met {
                        None => out.clone(),
                        Some(acc) => acc.into_iter().filter(|f| out.contains(f)).collect(),
                    });
                }
            }
            met
        };
        loop {
            let mut moved = false;
            for b in 0..nb {
                let Some(mut facts) = entry(b, &exits) else {
                    continue;
                };
                for k in cfg.starts[b]..bend(b) {
                    slot_fact_step(lines, kinds, k, &mut facts, false);
                }
                facts.sort_unstable();
                if exits[b].as_ref() != Some(&facts) {
                    exits[b] = Some(facts);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        for b in 0..nb {
            let Some(mut facts) = entry(b, &exits) else {
                continue;
            };
            for k in cfg.starts[b]..bend(b) {
                changed |= slot_fact_step(lines, kinds, k, &mut facts, true);
            }
        }
    }
    changed
}

/// `(offset, size, FP register)`: the slot's bytes are the register's.
type SlotFact = (i32, i32, u8);

/// Advance the slot facts over line `k`; with `rewrite`, answer a reload
/// the facts know (returns whether it did).  The analysis and the rewrite
/// run this same step, so the facts each sees agree.
fn slot_fact_step(
    lines: &mut [String],
    kinds: &mut [LineKind],
    k: usize,
    facts: &mut Vec<SlotFact>,
    rewrite: bool,
) -> bool {
    // Transfers read general registers only (`jump` writes its t6 scratch):
    // the facts flow to every successor, and a label's facts are the meet
    // the dataflow computed for its block.
    if matches!(
        kinds[k],
        LineKind::Nop | LineKind::Directive | LineKind::Label | LineKind::Branch | LineKind::Jump
    ) {
        return false;
    }
    if rv_fp_barrier(&lines[k], kinds[k]) {
        facts.clear();
        return false;
    }
    let overlaps = |&(off, size, _): &SlotFact, lo: i32, sz: i32| off < lo + sz && lo < off + size;
    // A frame load the facts can answer.
    let load = match kinds[k] {
        LineKind::LoadS0 {
            reg,
            offset,
            is_word,
        } => Some((offset, if is_word { 4 } else { 8 }, None, Some(reg))),
        LineKind::Other => rv_mem_parts(&lines[k]).and_then(|(m, reg, off, base)| {
            let size = match m {
                "fld" => 8,
                "flw" => 4,
                _ => return None,
            };
            (base == "s0").then_some((i32::try_from(off).ok()?, size, Some(rv_fp_reg(reg)), None))
        }),
        _ => None,
    };
    if let Some((off, size, fp_dst, gp_dst)) = load
        && let Some(&(_, _, src)) = facts.iter().find(|&&(o, s, _)| o == off && s == size)
    {
        let fp_dst = match fp_dst {
            Some(Some(d)) => Some(d),
            Some(None) => {
                facts.clear(); // an FP load this pass cannot read
                return false;
            }
            None => None,
        };
        if let Some(d) = fp_dst
            && d != src
        {
            facts.retain(|&(_, _, r)| r != d);
        }
        if !rewrite {
            return false;
        }
        let new_line = match (fp_dst, gp_dst) {
            (Some(d), _) if d == src => None,
            (Some(d), _) => {
                let fmt = if size == 8 { 'd' } else { 's' };
                Some(format!(
                    "    fmv.{fmt} {}, {}",
                    rv_fp_name(d),
                    rv_fp_name(src)
                ))
            }
            (None, Some(g)) => {
                let fmt = if size == 8 { 'd' } else { 'w' };
                Some(format!(
                    "    fmv.x.{fmt} {}, {}",
                    reg_name(g),
                    rv_fp_name(src)
                ))
            }
            (None, None) => return false,
        };
        match new_line {
            Some(new_line) => {
                kinds[k] = classify_line(&new_line);
                lines[k] = new_line;
            }
            None => kinds[k] = LineKind::Nop,
        }
        return true;
    }
    // Stores: a frame store replaces the facts it overlaps, an FP one
    // records its own; an `sp`-based store may reach the frame's bottom, a
    // form this pass cannot read may reach anything, and any other base
    // cannot reach a private frame.
    let frame_store = match kinds[k] {
        LineKind::StoreS0 {
            offset, is_word, ..
        } => Some((offset, if is_word { 4 } else { 8 })),
        _ => rv_frame_store(&lines[k]),
    };
    if let Some((off, size)) = frame_store {
        facts.retain(|fact| !overlaps(fact, off, size));
        if let Some((m, reg, _, _)) = rv_mem_parts(&lines[k])
            && (m == "fsd" || m == "fsw")
            && let Some(r) = rv_fp_reg(reg)
        {
            facts.push((off, size, r));
        }
        return false;
    }
    if rv_is_store(&lines[k])
        && rv_mem_parts(&lines[k]).is_none_or(|(m, _, _, base)| {
            base == "sp" || base == "s0" || rv_access_width(m).is_none()
        })
    {
        facts.clear();
        return false;
    }
    match rv_fp_roles(&lines[k]) {
        Some(roles) => {
            if let Some(d) = roles.def {
                facts.retain(|&(_, _, r)| r != d);
            }
        }
        None => facts.clear(),
    }
    false
}

/// Serve the uses of a bit move's general register from the FP register it
/// mirrors:
///
/// ```text
///   fmv.x.d t0, ft0 ; ... ; fmv.d.x ft1, t0   ->  ... ; fmv.d ft1, ft0
///   fmv.x.d t0, ft0 ; ... ; sd t0, 8(a0)      ->  ... ; fsd ft0, 8(a0)
///   fmv.d.x ft0, t0 ; ... ; fmv.d.x ft1, t0   ->  ... ; fmv.d ft1, ft0
/// ```
///
/// A bit move in either direction leaves the two registers holding the same
/// bits -- all 64 for `.d`; for `.w` the low word, which is all `sw`/`fsw`
/// store and all `fmv.w.x` reads (`fmv.w.x` boxes the word it moves, so its
/// FP side is a boxed single) -- until either is rewritten.  The emitter
/// stages every FP value through a general register, so one value read
/// twice paid two cross-file moves, and a spill kept the general register
/// alive; once its uses move to the FP side the bit move dies
/// (`eliminate_dead_defs_global`).  The facts are block-local and flow into
/// a branch's fall-through.
fn forward_fp_bit_moves(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    /// A bit move: `(gp, fp, width, gp_is_written)` for `fmv.x.{d,w}`
    /// (`gp_is_written`) and `fmv.{d,w}.x`.
    fn bit_move(line: &str) -> Option<(u8, u8, char, bool)> {
        let t = line.trim();
        let (m, ops) = t.split_once(|c: char| c.is_ascii_whitespace())?;
        let (a, b) = ops.split_once(',')?;
        let (a, b) = (a.trim(), b.trim());
        let (gp_name, fp_name, width, to_gp) = match m {
            "fmv.x.d" => (a, b, 'd', true),
            "fmv.x.w" => (a, b, 'w', true),
            "fmv.d.x" => (b, a, 'd', false),
            "fmv.w.x" => (b, a, 'w', false),
            _ => return None,
        };
        let gp = parse_reg(gp_name);
        let fp = rv_fp_reg(fp_name)?;
        (gp != REG_NONE).then_some((gp, fp, width, to_gp))
    }
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        // (general register, FP register, width) holding the same bits.
        let mut facts: Vec<(u8, u8, char)> = Vec::new();
        for k in start..end {
            if matches!(
                kinds[k],
                LineKind::Nop | LineKind::Directive | LineKind::Branch
            ) {
                continue; // a branch reads general registers only
            }
            if rv_fp_barrier(&lines[k], kinds[k]) {
                facts.clear();
                continue;
            }
            let (named, unmodelled) = rv_named_regs(&lines[k]);
            if unmodelled {
                facts.clear();
                continue;
            }
            // A use the facts serve from the FP side.
            if let Some((gp, fd, width, false)) = bit_move(&lines[k])
                && let Some(&(_, fs, _)) = facts
                    .iter()
                    .find(|&&(g, f, w)| g == gp && w == width && f != fd)
            {
                let fmt = if width == 'd' { 'd' } else { 's' };
                let new_line = format!("    fmv.{fmt} {}, {}", rv_fp_name(fd), rv_fp_name(fs));
                kinds[k] = classify_line(&new_line);
                lines[k] = new_line;
                changed = true;
                facts.retain(|&(_, f, _)| f != fd);
                facts.push((gp, fd, width));
                continue;
            }
            if let Some((m, reg, off, base)) = rv_mem_parts(&lines[k])
                && rv_access_width(m).is_some_and(|(_, store)| store)
                && let width @ ('d' | 'w') = match m {
                    "sd" => 'd',
                    "sw" => 'w',
                    _ => ' ',
                }
                && base != reg
                && let Some(&(_, fs, _)) = facts
                    .iter()
                    .find(|&&(g, _, w)| reg_name(g) == reg && w == width)
            {
                let new_line = format!("    fs{width} {}, {off}({base})", rv_fp_name(fs));
                kinds[k] = classify_line(&new_line);
                lines[k] = new_line;
                changed = true;
                continue; // a store writes no register
            }
            // Retire every fact the line may break: a general register it
            // writes (any named register of a line that is not a store, a
            // pure FP line or a bit move into the FP side), an FP register
            // it writes.
            let fp_def = match rv_fp_roles(&lines[k]) {
                Some(roles) => roles.def,
                None => {
                    facts.clear();
                    continue;
                }
            };
            let gp_writes = match kinds[k] {
                LineKind::Move { dst, .. }
                | LineKind::LoadImm { dst }
                | LineKind::LoadAddr { dst }
                | LineKind::SextW { dst, .. }
                | LineKind::LoadS0 { reg: dst, .. } => rv_bit(dst),
                LineKind::StoreS0 { .. } => 0,
                LineKind::Alu => parse_alu_dest(&lines[k]).map_or(named, rv_bit),
                _ if rv_is_store(&lines[k]) => 0,
                LineKind::Other if rv_is_pure_fp_line(&lines[k]) => 0,
                _ => named,
            };
            facts.retain(|&(g, f, _)| gp_writes & rv_bit(g) == 0 && Some(f) != fp_def);
            if let Some((gp, fp, width, _)) = bit_move(&lines[k]) {
                facts.push((gp, fp, width));
            }
        }
    }
    changed
}

/// Keep an FP value that one straight-line stretch of code spills and
/// reloads in a free FP scratch instead of its frame slot:
///
/// ```text
///   fmul.d ft0, fa0, fa1          fmul.d ft0, fa0, fa1
///   fsd    ft0, -48(s0)           fmv.d  ft11, ft0
///   fmul.d ft0, fa2, fa0    ->    fmul.d ft0, fa2, fa0
///   fld    ft1, -48(s0)           fmv.d  ft1, ft11
/// ```
///
/// `forward_fp_slot_reloads` answers a reload from the register that was
/// stored only while that register is intact; the emitter reuses ft0/ft1 for
/// every operation, so a temporary that waits for another computation lost
/// its register.  The slot qualifies when every access to its bytes in the
/// whole function is an exact-width access of the same slot, at least one of
/// them an FP access, the first a store, and no label, transfer or call lies
/// between the first and the last (a branch's fall-through continues the
/// stretch; the taken path never reads the slot).  The scratch is a
/// temporary no line of the stretch names and the FP liveness proves dead
/// where it starts -- so it is dead on every path out of it as well.  Each
/// access becomes the copy or bit move with its exact semantics: `fsd`/`fld`
/// `fmv.d`, `fsw`/`flw` `fmv.s`, `sd`/`ld` `fmv.d.x`/`fmv.x.d`, `sw`
/// `fmv.w.x` (the low word, NaN-boxed) and `lw` `fmv.x.w` (sign-extended,
/// as `lw` loads it).  A callee-saved register's save slot is never
/// promoted: the unwinder reads it.
fn promote_fp_frame_slots(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        if rv_frame_reads(lines, kinds, start, end).is_none() {
            continue;
        }
        let Some(live) = RvFpLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        // Every frame access: (line, offset, size, mnemonic).
        let mut accesses: Vec<(usize, i32, i32, String)> = Vec::new();
        for k in start..end {
            if kinds[k] == LineKind::Nop || kinds[k] == LineKind::Directive {
                continue;
            }
            if let Some((m, _, off, base)) = rv_mem_parts(&lines[k])
                && base == "s0"
                && let Some((size, _)) = rv_access_width(m)
                && let Ok(off) = i32::try_from(off)
            {
                accesses.push((k, off, size, m.to_string()));
            }
        }
        let mut slots: Vec<(i32, i32)> = accesses
            .iter()
            .filter(|a| a.3.starts_with('f'))
            .map(|a| (a.1, a.2))
            .collect();
        slots.sort_unstable();
        slots.dedup();
        // The scratch registers this invocation already gave a stretch:
        // the liveness predates them.
        let mut taken: Vec<(usize, usize, u8)> = Vec::new();
        for (off, size) in slots {
            if size != 4 && size != 8 {
                continue;
            }
            let group: Vec<&(usize, i32, i32, String)> = accesses
                .iter()
                .filter(|a| a.1 < off + size && off < a.1 + a.2)
                .collect();
            let (Some(&&(first, ..)), Some(&&(last, ..))) = (group.first(), group.last()) else {
                continue;
            };
            let exact = group.iter().all(|a| a.1 == off && a.2 == size);
            let opens_with_store = rv_access_width(&group[0].3).is_some_and(|(_, st)| st);
            if !exact || !opens_with_store {
                continue;
            }
            let straight = (first..=last).all(|k| {
                kinds[k] == LineKind::Branch
                    || (!matches!(
                        kinds[k],
                        LineKind::Label | LineKind::Jump | LineKind::Ret | LineKind::Call
                    ) && !rv_fp_barrier(&lines[k], kinds[k]))
            });
            if !straight {
                continue;
            }
            // The FP registers the stretch names, and a free scratch.
            let mut named = 0u32;
            for k in first..=last {
                if kinds[k] == LineKind::Nop || kinds[k] == LineKind::Directive {
                    continue;
                }
                match rv_fp_roles(&lines[k]) {
                    Some(roles) => named |= roles.uses | roles.def_bit(),
                    None => named = u32::MAX,
                }
            }
            let mut free = RV_FP_TEMPS & !named & !live.live_before(first);
            for &(f, l, z) in &taken {
                if f <= last && first <= l {
                    free &= !rv_fp_bit(z);
                }
            }
            let Some(z) = (0..32u8).rev().find(|&r| free & rv_fp_bit(r) != 0) else {
                continue;
            };
            let zn = rv_fp_name(z);
            let rewritten: Option<Vec<(usize, String)>> = group
                .iter()
                .map(|&&(k, _, _, ref m)| {
                    let (_, reg, _, _) = rv_mem_parts(&lines[k])?;
                    let fp = rv_fp_reg(reg);
                    let gp = parse_reg(reg);
                    let callee_saved = fp.is_some_and(|r| rv_fp_bit(r) & RV_FP_CALLEE_SAVED != 0)
                        || (REG_S1..=REG_S11).contains(&gp)
                        || matches!(gp, REG_S0 | REG_SP | REG_RA);
                    if callee_saved || (fp.is_none() && gp == REG_NONE && reg != "zero") {
                        return None;
                    }
                    let fr = fp.map(rv_fp_name);
                    let line = match (m.as_str(), fr) {
                        ("fsd", Some(r)) => format!("    fmv.d {zn}, {r}"),
                        ("fsw", Some(r)) => format!("    fmv.s {zn}, {r}"),
                        ("fld", Some(r)) => format!("    fmv.d {r}, {zn}"),
                        ("flw", Some(r)) => format!("    fmv.s {r}, {zn}"),
                        ("sd", None) => format!("    fmv.d.x {zn}, {reg}"),
                        ("sw", None) => format!("    fmv.w.x {zn}, {reg}"),
                        ("ld", None) if gp != REG_NONE => format!("    fmv.x.d {reg}, {zn}"),
                        ("lw", None) if gp != REG_NONE => format!("    fmv.x.w {reg}, {zn}"),
                        _ => return None,
                    };
                    Some((k, line))
                })
                .collect();
            let Some(rewritten) = rewritten else {
                continue;
            };
            for (k, line) in rewritten {
                kinds[k] = classify_line(&line);
                lines[k] = line;
            }
            taken.push((first, last, z));
            changed = true;
        }
    }
    changed
}

/// Forward a copy into an FP scratch into the copy's readers:
///
/// ```text
///   fmv.d ft0, fa0 ; fadd.d ft0, ft0, ft1    ->   fadd.d ft0, fa0, ft1
/// ```
///
/// The scratch (ft0-ft11, never an ABI value) must be written again before
/// the next control transfer, or be dead there by the FP liveness; every
/// reader on the way is renamed, and the source must still hold its value at
/// each of them.  A read-modify-write reader (`fadd.d ft0, ft0, ft1`) renames
/// its read and ends the copy's life.  Copies of a register to itself go
/// (`fmv.d` is `fsgnj.d`: a bit copy), and so do dead copies, loads and bit
/// moves into a scratch or argument register -- the only FP definitions
/// that can neither trap nor set an exception flag.
fn forward_fp_copies(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    const WINDOW: usize = 64;
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        if (start..end).any(|k| lines[k] == OPAQUE_LINE) {
            continue;
        }
        let Some(live) = RvFpLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        for i in start..end {
            if kinds[i] != LineKind::Other {
                continue;
            }
            let Some((ft, fs, _)) = rv_fp_copy(&lines[i]) else {
                // A dead bit transfer into a scratch or argument register.
                let transfer = lines[i]
                    .trim()
                    .split_whitespace()
                    .next()
                    .is_some_and(|m| matches!(m, "fld" | "flw" | "fmv.d.x" | "fmv.w.x"));
                if transfer
                    && let Some(RvFpRoles { def: Some(d), .. }) = rv_fp_roles(&lines[i])
                    && rv_fp_bit(d) & (RV_FP_TEMPS | (RV_FP_ARGS & !RV_FP_RET)) != 0
                    && live.live_after(i) & rv_fp_bit(d) == 0
                {
                    kinds[i] = LineKind::Nop;
                    changed = true;
                }
                continue;
            };
            if ft == fs {
                if lines[i].trim().starts_with("fmv.d") {
                    kinds[i] = LineKind::Nop;
                    changed = true;
                }
                continue;
            }
            if rv_fp_bit(ft) & RV_FP_TEMPS == 0 {
                continue;
            }
            let mut readers: Vec<(usize, u8)> = Vec::new();
            let mut fs_clobbered = false;
            let mut ok = false;
            let mut steps = 0;
            let mut k = i + 1;
            while k < end && steps < WINDOW {
                if kinds[k] == LineKind::Nop || kinds[k] == LineKind::Directive {
                    k += 1;
                    continue;
                }
                if rv_fp_barrier(&lines[k], kinds[k]) {
                    ok = live.live_before(k) & rv_fp_bit(ft) == 0;
                    break;
                }
                let Some(roles) = rv_fp_roles(&lines[k]) else {
                    break;
                };
                if roles.uses & rv_fp_bit(ft) != 0 {
                    if fs_clobbered {
                        break;
                    }
                    readers.push((k, roles.src_ops));
                }
                if roles.def == Some(ft) {
                    ok = true;
                    break;
                }
                if roles.def == Some(fs) {
                    fs_clobbered = true;
                }
                steps += 1;
                k += 1;
            }
            if !ok {
                continue;
            }
            let renamed: Option<Vec<String>> = readers
                .iter()
                .map(|&(k, src_ops)| rv_rename_fp_operands(&lines[k], src_ops, ft, fs))
                .collect();
            let Some(renamed) = renamed else {
                continue;
            };
            for ((k, _), new_line) in readers.into_iter().zip(renamed) {
                kinds[k] = classify_line(&new_line);
                lines[k] = new_line;
            }
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

/// Compute an FP result straight into the register it is copied to:
///
/// ```text
///   fadd.d ft0, fa0, fa1 ; fmv.d fa0, ft0    ->   fadd.d fa0, fa0, fa1
/// ```
///
/// The copy must be the first line after the producer to name the scratch,
/// no line in between may name the copy's destination, and the scratch must
/// die at the copy.  A single-precision copy (`fmv.s`) takes only a producer
/// of a single.
fn retarget_fp_results(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    const WINDOW: usize = 16;
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        if (start..end).any(|k| lines[k] == OPAQUE_LINE) {
            continue;
        }
        let Some(live) = RvFpLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        for i in start..end {
            if kinds[i] != LineKind::Other {
                continue;
            }
            let Some(producer) = rv_fp_roles(&lines[i]) else {
                continue;
            };
            let Some(ft) = producer.def else {
                continue;
            };
            if rv_fp_bit(ft) & RV_FP_TEMPS == 0 {
                continue;
            }
            let mut between = 0u32;
            let mut found = None;
            let mut steps = 0;
            let mut j = i + 1;
            while j < end && steps < WINDOW {
                if kinds[j] == LineKind::Nop || kinds[j] == LineKind::Directive {
                    j += 1;
                    continue;
                }
                if rv_fp_barrier(&lines[j], kinds[j]) {
                    break;
                }
                let Some(roles) = rv_fp_roles(&lines[j]) else {
                    break;
                };
                if (roles.uses | roles.def_bit()) & rv_fp_bit(ft) != 0 {
                    found = Some(j);
                    break;
                }
                between |= roles.uses | roles.def_bit();
                steps += 1;
                j += 1;
            }
            let Some(j) = found else {
                continue;
            };
            let Some((fd, src, copy_single)) = rv_fp_copy(&lines[j]) else {
                continue;
            };
            if src != ft
                || fd == ft
                || between & rv_fp_bit(fd) != 0
                || (copy_single && !producer.single)
                || live.live_after(j) & rv_fp_bit(ft) != 0
            {
                continue;
            }
            let Some(new_line) = rv_rename_fp_operands(&lines[i], 1, ft, fd) else {
                continue;
            };
            kinds[i] = classify_line(&new_line);
            lines[i] = new_line;
            kinds[j] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

/// Delete the frame of a function whose body no longer uses it.
///
/// The emitter decides `frameless_leaf` before the passes run, so a leaf
/// whose frame slots the passes all forwarded and deleted still sets up and
/// tears down a frame nothing touches:
///
/// ```text
///   addi sp, sp, -32 ; sd ra, 24(sp) ; sd s0, 16(sp) ; addi s0, sp, 32
///   ... no sp, s0 or ra, no call ...
///   ld ra, 24(sp) ; ld s0, 16(sp) ; addi sp, sp, 32 ; ret
/// ```
///
/// Without a call `ra` keeps the return address, nothing writes `s0` and
/// `sp` never moves, so the caller sees the same registers and memory; the
/// CFA stays `sp + 0` with `ra` in `ra` -- the CIE's initial rule -- so the
/// prologue's CFI rows go with it.  Anything else -- a callee-saved save, a
/// stack-passed parameter, a variadic save area, `__builtin_frame_address`,
/// a dynamic allocation, the large-frame forms -- names `s0` or `sp` in the
/// body or changes the prologue's shape, and keeps the frame.
fn eliminate_vestigial_frames(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        if let Some(frame) = rv_vestigial_frame(lines, kinds, start, end) {
            for k in frame {
                kinds[k] = LineKind::Nop;
            }
            changed = true;
        }
    }
    changed
}

/// The prologue, CFI and epilogue lines of a function whose frame its body
/// never uses (see `eliminate_vestigial_frames`), or `None`.
fn rv_vestigial_frame(
    lines: &[String],
    kinds: &[LineKind],
    start: usize,
    end: usize,
) -> Option<Vec<usize>> {
    let code: Vec<usize> = (start + 1..end)
        .filter(|&k| kinds[k] != LineKind::Nop)
        .collect();
    if code.iter().any(|&k| lines[k] == OPAQUE_LINE) {
        return None;
    }
    let text = |k: usize| lines[k].trim();
    let is_cfi = |k: usize| kinds[k] == LineKind::Directive && text(k).starts_with(".cfi_");
    let mut pos = 0;
    // Directives ahead of the prologue: only `.cfi_startproc` among the CFI.
    while pos < code.len() && kinds[code[pos]] == LineKind::Directive {
        if is_cfi(code[pos]) && text(code[pos]) != ".cfi_startproc" {
            return None;
        }
        pos += 1;
    }
    let size: i64 = text(*code.get(pos)?)
        .strip_prefix("addi sp, sp, -")?
        .parse()
        .ok()
        .filter(|&s: &i64| s >= 16 && s % 16 == 0 && s <= 2032)?;
    let mut frame = vec![code[pos]];
    pos += 1;
    let mut expect = |pos: &mut usize, want: &str, optional: bool| -> Option<()> {
        match code.get(*pos) {
            Some(&k) if text(k) == want => {
                frame.push(k);
                *pos += 1;
                Some(())
            }
            _ if optional => Some(()),
            _ => None,
        }
    };
    expect(&mut pos, &format!(".cfi_def_cfa_offset {size}"), true)?;
    expect(&mut pos, &format!("sd ra, {}(sp)", size - 8), false)?;
    expect(&mut pos, &format!("sd s0, {}(sp)", size - 16), false)?;
    expect(&mut pos, &format!("addi s0, sp, {size}"), false)?;
    expect(&mut pos, ".cfi_def_cfa s0, 0", true)?;
    expect(&mut pos, ".cfi_offset ra, -8", true)?;
    expect(&mut pos, ".cfi_offset s0, -16", true)?;
    let epilogue = [
        format!("ld ra, {}(sp)", size - 8),
        format!("ld s0, {}(sp)", size - 16),
        format!("addi sp, sp, {size}"),
    ];
    // Every return comes right after the three-line epilogue; the lines in
    // between are the body, which may name neither sp, s0 nor ra.
    let mut epilogue_lines = Vec::new();
    for (idx, &k) in code.iter().enumerate().skip(pos) {
        if kinds[k] == LineKind::Ret {
            let tail = code.get(idx.checked_sub(3)?..idx)?;
            if idx < pos + 3 || tail.iter().zip(&epilogue).any(|(&e, want)| text(e) != want) {
                return None;
            }
            epilogue_lines.extend_from_slice(tail);
        }
    }
    let frame_bits = rv_bit(REG_SP) | rv_bit(REG_S0) | rv_bit(REG_RA);
    for &k in &code[pos..] {
        if epilogue_lines.binary_search(&k).is_ok() {
            continue;
        }
        match kinds[k] {
            LineKind::Label | LineKind::Ret => continue,
            LineKind::Directive => {
                if is_cfi(k) && text(k) != ".cfi_endproc" {
                    return None;
                }
                continue;
            }
            LineKind::Call => return None,
            LineKind::Jump if !jump_target(&lines[k]).is_some_and(|t| t.starts_with(".L")) => {
                return None;
            }
            _ => {}
        }
        let (named, unmodelled) = rv_named_regs(&lines[k]);
        let (ireads, iwrites) = classify_implicit_operands_rv(&lines[k]);
        if unmodelled
            || (named | ireads | iwrites) & frame_bits != 0
            || (kinds[k] == LineKind::Other && rv_is_unmodelled_transfer(&lines[k]))
            || matches!(
                text(k).split_whitespace().next(),
                Some("ecall" | "ebreak" | "call" | "tail" | "jal" | "jalr" | "jr")
            )
        {
            return None;
        }
    }
    if epilogue_lines.is_empty() {
        return None;
    }
    frame.extend(epilogue_lines);
    Some(frame)
}

/// Reads of a register just set to 0 read `zero` instead: `li t2, 0;
/// slt t0, a0, t2` -> `slt t0, a0, zero`.  x0 is a valid source in every
/// base-ISA operand position, so the rename is shape-free for the line kinds
/// `rv_rename_reads` knows; the `li` then dies (global liveness).  The
/// emitter materialises every zero comparand and zero store through a
/// temporary.
fn propagate_zero_constants(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        let LineKind::LoadImm { dst } = kinds[i] else {
            continue;
        };
        let imm = lines[i]
            .trim()
            .strip_prefix("li ")
            .and_then(|r| r.split_once(','));
        if !imm.is_some_and(|(_, v)| parse_asm_i64(v) == Some(0)) || dst == REG_NONE {
            continue;
        }
        let name = reg_name(dst);
        let mut j = i + 1;
        while j < n {
            match kinds[j] {
                LineKind::Nop | LineKind::Directive => {
                    j += 1;
                    continue;
                }
                LineKind::Label | LineKind::Ret | LineKind::Call | LineKind::Jump => break,
                _ => {}
            }
            if lines[j] == OPAQUE_LINE {
                break;
            }
            let (uses, _) = rv_uses_defs(&lines[j], kinds[j]);
            let writes = named_gp_dest(&lines[j], kinds[j]);
            if uses & rv_bit(dst) != 0 {
                let renamed = if matches!(kinds[j], LineKind::Alu | LineKind::Branch) {
                    let t = lines[j].trim();
                    t.split_once(|c: char| c.is_ascii_whitespace())
                        .and_then(|(m, ops)| {
                            let mut parts: Vec<String> =
                                ops.split(',').map(|p| p.trim().to_string()).collect();
                            // Sources: every operand of a branch but its label;
                            // every operand after the destination of an ALU op.
                            let (from, to) = if kinds[j] == LineKind::Branch {
                                (0, parts.len().saturating_sub(1))
                            } else {
                                (1, parts.len())
                            };
                            let mut hit = false;
                            for part in parts.iter_mut().take(to).skip(from) {
                                if part == name {
                                    *part = "zero".to_string();
                                    hit = true;
                                }
                            }
                            hit.then(|| format!("    {m} {}", parts.join(", ")))
                        })
                } else {
                    None
                };
                match renamed {
                    Some(new_line)
                        if rv_uses_defs(&new_line, classify_line(&new_line)).0 & rv_bit(dst)
                            == 0 =>
                    {
                        kinds[j] = classify_line(&new_line);
                        lines[j] = new_line;
                        changed = true;
                    }
                    _ => break,
                }
            }
            if writes == Some(dst) || kinds[j] == LineKind::Branch || kinds[j] == LineKind::Other {
                break;
            }
            j += 1;
        }
    }
    changed
}

/// Forward copy propagation inside each block: after `mv d, s`, reads of `d`
/// read `s` instead until either register is written.  The copy is left in
/// place; when no read remains, `eliminate_dead_defs_global` removes it.
fn propagate_copies_within_blocks(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        let LineKind::Move { dst, src } = kinds[i] else {
            continue;
        };
        if dst == src
            || [dst, src]
                .iter()
                .any(|&r| matches!(r, REG_S0 | REG_SP | REG_RA | REG_NONE))
            || lines[i] == OPAQUE_LINE
        {
            continue;
        }
        let mut j = i + 1;
        while j < n {
            match kinds[j] {
                LineKind::Nop | LineKind::Directive => {
                    j += 1;
                    continue;
                }
                LineKind::Label | LineKind::Ret | LineKind::Call | LineKind::Jump => break,
                _ => {}
            }
            if lines[j] == OPAQUE_LINE {
                break;
            }
            let (uses, _) = rv_uses_defs(&lines[j], kinds[j]);
            let writes = named_gp_dest(&lines[j], kinds[j]);
            let implicit_writes = classify_implicit_operands_rv(&lines[j]).1;
            if implicit_writes & (rv_bit(dst) | rv_bit(src)) != 0 {
                break;
            }
            if uses & rv_bit(dst) != 0 {
                match rv_rename_reads(&lines[j], kinds[j], dst, src) {
                    Some(new_line) => {
                        let new_kind = classify_line(&new_line);
                        // The rewrite must not have touched anything else.
                        if rv_uses_defs(&new_line, new_kind).0 & rv_bit(dst) != 0
                            && writes != Some(dst)
                        {
                            break;
                        }
                        lines[j] = new_line;
                        kinds[j] = new_kind;
                        changed = true;
                    }
                    None => break,
                }
            }
            if writes == Some(dst) || writes == Some(src) || kinds[j] == LineKind::Branch {
                break;
            }
            if kinds[j] == LineKind::Other
                && rv_load_dest(&lines[j]).is_none()
                && rv_fp_gp_dest(&lines[j]).is_none()
                && !rv_is_pure_fp_line(&lines[j])
            {
                break;
            }
            j += 1;
        }
    }
    changed
}

// Shared peephole string utilities -- see backend/peephole_common.rs
#[cfg(test)]
use crate::backend::peephole_common::replace_whole_word;
use crate::backend::peephole_common::{has_whole_word, replace_source_reg_in_instruction};
use crate::common::fx_hash::{FxHashMap, FxHashSet};

// ── Global dead store elimination ────────────────────────────────────────────
//
// Scans the entire function to find stack slot offsets that are never loaded.
// Stores to such slots are dead and can be eliminated.
// This runs after global store forwarding, which may have converted loads
// to register moves, leaving the original stores dead.

fn global_dead_store_elimination(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    // Frame slots belong to one function: its `s0` names its own frame, and
    // any other way in -- a pointer to a slot handed to a callee or kept in
    // a register -- starts with that function taking the slot's address.
    // The census is therefore per function.  It used to span the whole
    // translation unit, so a slot offset that ANY function loaded kept every
    // function's stores to that offset alive, and one `addi tX, s0, N`
    // anywhere disabled the pass for the whole file: the dead FP spills in
    // a float loop survived because a neighbouring function read the same
    // offset.  A function with an opaque (inline-asm or volatile) body is
    // skipped whole, which also lets the pass run in units that have one.
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        changed |= dead_stores_in_function(lines, kinds, start, end);
    }
    changed
}

fn dead_stores_in_function(
    lines: &[String],
    kinds: &mut [LineKind],
    start: usize,
    end: usize,
) -> bool {
    let Some(loaded_ranges) = rv_frame_reads(lines, kinds, start, end) else {
        return false;
    };
    // Remove stores whose byte range overlaps no load range: the integer
    // `sd`/`sw` the classifier tracks, and the FP and narrow stores
    // (`fsd`/`fsw`/`sh`/`sb`) a forwarded slot leaves behind.
    let mut changed = false;
    for i in start..end {
        let (offset, store_size) = match kinds[i] {
            LineKind::StoreS0 {
                offset, is_word, ..
            } => (offset, if is_word { 4 } else { 8 }),
            LineKind::Other => match rv_frame_store(&lines[i]) {
                Some(store) => store,
                None => continue,
            },
            _ => continue,
        };
        let overlaps_any_load = loaded_ranges.iter().any(|&(load_off, load_sz)| {
            // Two ranges [a, a+as) and [b, b+bs) overlap iff a < b+bs && b < a+as
            offset < load_off + load_sz && load_off < offset + store_size
        });
        if !overlaps_any_load {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

/// The frame bytes one function reads, as `(offset from s0, size)` ranges --
/// or `None` when that census cannot be complete: a line is opaque, the
/// frame address escapes (offset into or copied to another register, read
/// as a value), or a line accesses a slot in a form whose width is unknown.
/// Frame slots are reachable only through `s0` otherwise, so a slot outside
/// every range is never read.
fn rv_frame_reads(
    lines: &[String],
    kinds: &[LineKind],
    start: usize,
    end: usize,
) -> Option<Vec<(i32, i32)>> {
    // Safety: if any instruction takes the address of s0 or uses s0 in a way
    // that could create pointers to stack slots (e.g., `addi xN, s0, off`
    // where xN is not s0 itself), we bail out to avoid eliminating stores
    // that might be accessed through pointers.
    for i in start..end {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        if lines[i] == OPAQUE_LINE {
            return None;
        }
        let trimmed = lines[i].trim();
        // Check for address-of-frame-pointer patterns
        if trimmed.starts_with("addi ") && trimmed.contains(", s0,") {
            // `addi s0, sp, N` is frame pointer setup — that's fine.
            // But `addi tX, s0, N` takes address of a stack slot.
            if let Some(dest) = parse_alu_dest(trimmed) {
                if dest != REG_S0 {
                    return None;
                }
            }
        }
        if trimmed.starts_with("mv ") && trimmed.contains(", s0") {
            if let Some(rest) = trimmed.strip_prefix("mv ") {
                if let Some((_, src)) = rest.split_once(", ") {
                    if src.trim() == "s0" {
                        // mv xN, s0 — copying frame pointer
                        return None;
                    }
                }
            }
        }
        // Any other line that reads s0 as a value (not as a base) lets the
        // frame address escape.
        if !trimmed.contains("(s0)")
            && kinds[i] != LineKind::Label
            && kinds[i] != LineKind::Directive
            && trimmed
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .skip(1)
                .any(|tok| tok == "s0" || tok == "fp")
            && !trimmed.starts_with("addi s0, sp,")
            && !trimmed.starts_with("ld s0,")
            && !trimmed.starts_with("sd s0,")
        {
            return None;
        }
    }

    // Collect all (offset, size) byte ranges that are loaded from.
    // We must use byte-range overlap (not exact offset match) because a wide
    // store (e.g. `sd` at offset -24, 8 bytes) can be partially read by a
    // narrower load at a different offset (e.g. `lwu` at offset -20, 4 bytes).
    let mut loaded_ranges: Vec<(i32, i32)> = Vec::new(); // (offset, size)
    for i in start..end {
        match kinds[i] {
            LineKind::LoadS0 {
                offset, is_word, ..
            } => {
                let size = if is_word { 4 } else { 8 };
                loaded_ranges.push((offset, size));
            }
            LineKind::Nop => {}
            _ => {
                // Every other line that names a frame slot.  The census must
                // be complete: a store whose only reader it misses is deleted.
                // The FP loads were missing, so `sd t0, -72(s0); flw fa1,
                // -72(s0)` -- a `__complex__ float` result staged through an
                // integer register -- lost its store at -O2 (gcc.c-torture
                // execute/pr38969.c).
                let trimmed = lines[i].trim();
                if !trimmed.contains("(s0)") {
                    continue;
                }
                // An atomic, a vector access or anything else unsized: no
                // census.  A store writes the slot; it keeps no store alive.
                let (size, is_store) =
                    rv_access_width(trimmed.split_whitespace().next().unwrap_or(""))?;
                if !is_store {
                    loaded_ranges.push((extract_s0_offset_from_line(trimmed)?, size));
                }
            }
        }
    }
    Some(loaded_ranges)
}

/// Extract s0 offset from any instruction line containing `off(s0)`.
fn extract_s0_offset_from_line(line: &str) -> Option<i32> {
    if let Some(paren_pos) = line.find("(s0)") {
        // Walk backwards from paren_pos to find the start of the offset number
        let before = &line[..paren_pos];
        // Find the last comma or space before the offset
        let start = before.rfind([',', ' ']).map(|p| p + 1).unwrap_or(0);
        let offset_str = before[start..].trim();
        return offset_str.parse::<i32>().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_function_quarantine_preserves_code_rows_and_label_references() {
        let asm = ".type v, @function\nv:\n.cfi_startproc\n.loc 1 5 0\n    li t0, 7\n    sw t0, -24(s0)\n    # LCCC_VOLATILE_BEGIN\n    lw t0, -24(s0)\n    # LCCC_VOLATILE_END\n.loc 1 6 0\n    # LCCC_VOLATILE_BEGIN\n    lw t0, -24(s0)\n    # LCCC_VOLATILE_END\n    lla a0, .Ldup\n    mv t4, t4\n    ret\n.cfi_endproc\n.size v, .-v\nplain:\n.cfi_startproc\n    mv t3, t3\n    j .Lkeep\n.Lkeep:\n    j .Ldone\n.Ldup:\n    j .Ldone\n.Ldone:\n    ret\n.cfi_endproc\n.size plain, .-plain\n";
        let out = peephole_optimize(asm.into());
        let expected = ".cfi_startproc\n.loc 1 5 0\n    li t0, 7\n    sw t0, -24(s0)\n    lw t0, -24(s0)\n.loc 1 6 0\n    lw t0, -24(s0)\n    lla a0, .Ldup\n    mv t4, t4\n    ret\n.cfi_endproc";
        assert!(out.contains(expected), "protected body changed: {out}");
        assert!(out.contains(".Ldup:"), "hidden label reference lost: {out}");
        assert!(
            !out.contains("mv t3, t3"),
            "unrelated function lost local optimization: {out}"
        );
        assert!(
            !out.contains("LCCC_VOLATILE_"),
            "internal marker leaked: {out}"
        );
    }

    #[test]
    fn volatile_quarantine_falls_back_on_unproven_boundaries() {
        for asm in [
            "v:\n    # LCCC_VOLATILE_BEGIN\n    lw t0, -24(s0)\n    # LCCC_VOLATILE_END\n    mv t0, t0\n    ret\n",
            "v:\n.cfi_startproc\n    # LCCC_VOLATILE_END\n    mv t0, t0\n.cfi_endproc\n",
            "v:\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    mv t0, t0\n.cfi_endproc\n",
            "v:\n.cfi_startproc\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    lw t0, -24(s0)\n    # LCCC_VOLATILE_END\n.cfi_endproc\n.cfi_endproc\n",
            // CFI alone does not prove the preceding instructions are inside
            // the protected function; the codegen's type/label/size must agree.
            ".type v, @function\nv:\n    mv t0, t0\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    lw t0, 0(t0)\n    # LCCC_VOLATILE_END\n.cfi_endproc\n.size v, .-v\n",
            ".type v, @function\nv:\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    lw t0, 0(t0)\n    # LCCC_VOLATILE_END\n.cfi_endproc\n.size other, .-other\n    mv t0, t0\n",
            ".type v, @function\nv:\n.cfi_startproc simple\n    # LCCC_VOLATILE_BEGIN\n    lw t0, 0(t0)\n    # LCCC_VOLATILE_END\n.cfi_endproc\n.size v, .-v\n",
        ] {
            let expected = crate::backend::peephole_common::strip_volatile_markers(asm.into());
            assert_eq!(peephole_optimize(asm.into()), expected);
        }
        // Corrupt the real marker rather than inventing a second knob-looking
        // identifier in the source: it remains a malformed-input test only.
        let marker = "# LCCC_VOLATILE_BEGIN".trim_end_matches('N');
        let asm = format!(
            ".type v, @function\nv:\n.cfi_startproc\n    {marker}\n    mv t0, t0\n.cfi_endproc\n.size v, .-v\n"
        );
        let expected = crate::backend::peephole_common::strip_volatile_markers(asm.clone());
        assert_eq!(peephole_optimize(asm), expected);
    }

    #[test]
    fn volatile_quarantine_handles_nested_markers() {
        let asm = ".type v, @function\nv:\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    # LCCC_VOLATILE_BEGIN\n    lw t0, -24(s0)\n    # LCCC_VOLATILE_END\n    # LCCC_VOLATILE_END\n    ret\n.cfi_endproc\n.size v, .-v\n";
        let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let ranges = volatile_function_ranges(&lines).unwrap();
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], 2..10);
        let out = peephole_optimize(asm.into());
        assert!(out.contains("    lw t0, -24(s0)"), "{out}");
        assert!(!out.contains("LCCC_VOLATILE_"), "{out}");
    }

    #[test]
    fn near_jump_does_not_cross_space_or_alignment_directives() {
        for directive in [".space 1048576", ".p2align 21", ".balign 2097152"] {
            let asm = format!("f:\n    jump .Ltarget, t6\n    {directive}\n.Ltarget:\n    ret\n");
            let out = peephole_optimize(asm);
            assert!(out.contains("    jump .Ltarget, t6"), "{directive}: {out}");
        }
    }

    #[test]
    fn near_jump_budget_accounts_for_pseudo_expansion() {
        let mut lines = vec!["f:".to_string(), "    jump .Ltarget, t6".to_string()];
        lines.extend((0..25_001).map(|_| "    li t0, 0x123456789abcdef".to_string()));
        lines.push(".Ltarget:".to_string());
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!rewrite_far_jumps_to_near(&mut lines, &mut kinds, n));
    }

    #[test]
    fn volatile_opaque_ranges_are_barriers_for_range_rewrites() {
        // The opaque line may hide an arbitrarily large `.space` directive.
        let asm = format!(
            "f:\n    beq t0, t1, .Lskip\n    jump .Ltarget, t6\n.Lskip:\n{OPAQUE_LINE}\n.Ltarget:\n    ret\n"
        );
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!fuse_branch_over_jump(&mut lines, &mut kinds, n));
        assert!(!rewrite_far_jumps_to_near(&mut lines, &mut kinds, n));
    }

    #[test]
    fn volatile_does_not_disable_unrelated_local_branch_rewrites() {
        let asm = ".type v, @function\nv:\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    lw t0, 0(a0)\n    # LCCC_VOLATILE_END\n    ret\n.cfi_endproc\n.size v, .-v\n.type plain, @function\nplain:\n.cfi_startproc\n    beq t0, t1, .Lskip\n    jump .Ltarget, t6\n.Lskip:\n    li a0, 1\n.Ltarget:\n    ret\n.cfi_endproc\n.size plain, .-plain\n";
        let out = peephole_optimize(asm.into());
        assert!(out.contains("    lw t0, 0(a0)"), "{out}");
        assert!(out.contains("    bne t0, t1, .Ltarget"), "{out}");
        assert!(!out.contains("    jump .Ltarget"), "{out}");
    }

    #[test]
    fn volatile_local_symbol_blocks_cross_boundary_entry_coalescing() {
        // The coalescer's function-start census intentionally ignores .L
        // symbols. Its window must not rename through the hidden read of t1.
        let asm = "outer:\n    mv t1, t2\n.type .Llocal, @function\n.Llocal:\n.cfi_startproc\n    # LCCC_VOLATILE_BEGIN\n    lw a0, 0(t1)\n    # LCCC_VOLATILE_END\n    ret\n.cfi_endproc\n.size .Llocal, .-.Llocal\n";
        let out = peephole_optimize(asm.into());
        assert!(
            out.contains("    mv t1, t2"),
            "hidden reader lost its definition: {out}"
        );
        assert!(out.contains("    lw a0, 0(t1)"), "{out}");
        assert!(!out.contains(OPAQUE_LINE), "placeholder leaked: {out}");
    }

    /// A parameter copied into a callee-saved home that the entry run already
    /// used as a temporary: renaming the copy also renamed the epilogue
    /// restore of s1, and s1 returned holding the first parameter.
    #[test]
    fn entry_coalescing_keeps_the_restore_of_a_rewritten_callee_saved_dst() {
        let asm = "f:\n    addi sp, sp, -32\n    sd ra, 24(sp)\n    sd s1, 16(sp)\n    sd s2, 8(sp)\n    mv s1, a0\n    mv s2, s1\n    mv s1, a1\n    add a0, s1, s2\n    ld s1, 16(sp)\n    ld s2, 8(sp)\n    ld ra, 24(sp)\n    addi sp, sp, 32\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        assert!(
            matches!(
                kinds[7],
                LineKind::Move {
                    dst: REG_S1,
                    src: REG_A1
                }
            ),
            "fixture premise: the copy is a coalescing candidate"
        );
        while coalesce_entry_copies(&mut lines, &mut kinds, n) {}
        let live: Vec<&str> = lines
            .iter()
            .zip(&kinds)
            .filter(|(_, k)| **k != LineKind::Nop)
            .map(|(l, _)| l.trim())
            .collect();
        assert!(live.contains(&"ld s1, 16(sp)"), "{live:#?}");
        assert!(!live.iter().any(|l| l.starts_with("ld a1")), "{live:#?}");
    }

    /// A global named like a register keeps its name through the rename.
    #[test]
    fn entry_coalescing_does_not_rename_register_named_symbols() {
        let asm = "f:\n    mv t3, a1\n    lga t0, t3\n    ld t0, 0(t0)\n    add a0, t3, t0\n    call t3\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        while coalesce_entry_copies(&mut lines, &mut kinds, n) {}
        for (l, k) in lines.iter().zip(&kinds) {
            if *k != LineKind::Nop && (l.contains("lga") || l.contains("call")) {
                assert!(l.trim().ends_with("t3"), "symbol renamed: {l}");
            }
        }
    }

    fn live_lines(lines: &[String], kinds: &[LineKind]) -> Vec<String> {
        lines
            .iter()
            .zip(kinds)
            .filter(|(_, k)| **k != LineKind::Nop)
            .map(|(l, _)| l.trim().to_string())
            .collect()
    }

    /// gcc.c-torture/execute/pr40022.c: `foo(x) { asm("" :: "g"(x)); return
    /// x; }` at -O2, as the emitter writes it (x homed in s1 because the asm is
    /// a call point).  The local passes reduce the body to `mv s1, a0` before
    /// the epilogue; coalescing it renamed the restore `ld s1` into `ld a0` and
    /// foo returned the caller's s1.  a0 must reach `ret` unwritten.
    #[test]
    fn entry_coalescing_keeps_a_returned_argument_register_intact() {
        let asm = "foo:\n    addi sp, sp, -32\n    sd ra, 24(sp)\n    sd s0, 16(sp)\n    addi s0, sp, 32\n    sd s1, -32(s0)\n    mv s1, a0\n    mv t0, s1\n    mv t0, s1\n    mv a0, t0\n    ld s1, -32(s0)\n    ld ra, 24(sp)\n    ld s0, 16(sp)\n    addi sp, sp, 32\n    ret\n";
        let out = peephole_optimize(asm.into());
        for line in out.lines() {
            let kind = classify_line(line);
            assert_ne!(
                named_gp_dest(line, kind),
                Some(REG_A0),
                "a0 written before ret:\n{out}"
            );
            assert_ne!(
                named_gp_dest(line, kind),
                Some(REG_S1),
                "s1 written without its restore:\n{out}"
            );
        }
        assert!(out.contains("ret"), "{out}");
    }

    /// The whole chain: the coalescer deletes the restore of a coalesced
    /// callee-saved home, the dead-store pass deletes its save, and the save's
    /// CFI row goes with it (s1 keeps the caller's value: "same value").
    #[test]
    fn coalesced_callee_saved_home_loses_its_save_row() {
        let asm = ".type f, @function\nf:\n.cfi_startproc\n    addi sp, sp, -32\n    .cfi_def_cfa_offset 32\n    sd ra, 24(sp)\n    sd s0, 16(sp)\n    addi s0, sp, 32\n    .cfi_def_cfa s0, 0\n    .cfi_offset ra, -8\n    .cfi_offset s0, -16\n    sd s1, -32(s0)\n    .cfi_offset s1, -32\n    mv s1, a2\n    add t0, s1, s1\n    sd t0, 0(t5)\n    ld s1, -32(s0)\n    ld ra, 24(sp)\n    ld s0, 16(sp)\n    addi sp, sp, 32\n    ret\n.cfi_endproc\n.size f, .-f\n";
        let out = peephole_optimize(asm.to_string());
        assert!(!out.contains("sd s1"), "{out}");
        assert!(!out.contains(".cfi_offset s1"), "{out}");
        // Nothing else uses the frame any more: it goes, rows and all.
        assert!(
            !out.contains("addi sp") && !out.contains(".cfi_offset"),
            "{out}"
        );
        // A body that still reads the frame (a stack-passed argument) keeps
        // it and the frame record's rows; only the dead save's row goes.
        let framed = peephole_optimize(asm.replace(
            "    sd t0, 0(t5)\n",
            "    sd t0, 0(t5)\n    ld t1, 0(s0)\n    sd t1, 8(t5)\n",
        ));
        assert!(!framed.contains("sd s1"), "{framed}");
        assert!(!framed.contains(".cfi_offset s1"), "{framed}");
        assert!(framed.contains(".cfi_offset ra, -8"), "{framed}");
        assert!(framed.contains(".cfi_offset s0, -16"), "{framed}");
        // With the restore in place the save and its row both stay.
        let kept = peephole_optimize(asm.replace("    mv s1, a2\n", "    mv s1, a2\n    call g\n"));
        assert!(kept.contains("sd s1, -32(s0)"), "{kept}");
        assert!(kept.contains(".cfi_offset s1, -32"), "{kept}");
    }

    /// A coalesced callee-saved home is never written again, so its epilogue
    /// restore is deleted -- not renamed into a write of the source register.
    #[test]
    fn entry_coalescing_deletes_the_restore_of_a_coalesced_callee_saved_dst() {
        let asm = "f:\n    sd s1, -32(s0)\n    mv s1, a2\n    add t0, s1, s1\n    sd t0, 0(t5)\n    ld s1, -32(s0)\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        while coalesce_entry_copies(&mut lines, &mut kinds, n) {}
        let live = live_lines(&lines, &kinds);
        assert_eq!(
            live,
            [
                "f:",
                "sd s1, -32(s0)",
                "add t0, a2, a2",
                "sd t0, 0(t5)",
                "ret"
            ],
            "{live:#?}"
        );
    }

    /// Any other write of the home lands in the source after the rename; with
    /// an argument-register source that `ret` (a0/a1) or a call (a0-a7) reads,
    /// the copy stays.
    #[test]
    fn entry_coalescing_refuses_to_write_an_implicitly_read_argument_register() {
        for (src, reader) in [("a0", "ret"), ("a1", "ret"), ("a5", "call g\n    ret")] {
            let asm = format!(
                "f:\n    sd s1, -32(s0)\n    mv s1, {src}\n    sd s1, 0(t5)\n    li s1, 7\n    sd s1, 8(t5)\n    ld s1, -32(s0)\n    {reader}\n"
            );
            let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
            let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
            let n = lines.len();
            assert!(
                !coalesce_entry_copies(&mut lines, &mut kinds, n),
                "{src}: {:#?}",
                live_lines(&lines, &kinds)
            );
        }
        // a2 is read by calls only: a leaf function may reuse it freely.
        let asm = "f:\n    sd s1, -32(s0)\n    mv s1, a2\n    sd s1, 0(t5)\n    li s1, 7\n    sd s1, 8(t5)\n    ld s1, -32(s0)\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        while coalesce_entry_copies(&mut lines, &mut kinds, n) {}
        let live = live_lines(&lines, &kinds);
        assert!(live.contains(&"li a2, 7".to_string()), "{live:#?}");
        assert!(!live.iter().any(|l| l.starts_with("ld ")), "{live:#?}");
    }

    /// `long f(long (*fp)(void), long x) { return fp() + x; }` at -O2: x homed
    /// in s1 across the indirect call.  `jalr ra, t2, 0` was not a call to the
    /// coalescer, which renamed s1 onto a1 -- destroyed by the call.
    #[test]
    fn entry_coalescing_does_not_carry_a_value_across_an_indirect_call() {
        let asm = "f:\n    sd s1, -48(s0)\n    sd s7, -40(s0)\n    mv t0, a0\n    mv s1, a1\n    mv t2, t0\n    jalr ra, t2, 0\n    mv s7, a0\n    add t0, s7, s1\n    mv a0, t0\n    ld s1, -48(s0)\n    ld s7, -40(s0)\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        while coalesce_entry_copies(&mut lines, &mut kinds, n) {}
        let live = live_lines(&lines, &kinds);
        assert!(live.contains(&"mv s1, a1".to_string()), "{live:#?}");
        assert!(live.contains(&"add t0, s7, s1".to_string()), "{live:#?}");
    }

    /// A use above the call inside a loop is reached again after the call: a
    /// caller-saved source must not carry the value even though no line after
    /// the call names it (restores aside).
    #[test]
    fn entry_coalescing_respects_back_edges_around_a_call() {
        let asm = "f:\n    sd s1, -48(s0)\n    mv s1, a1\n.LBB1:\n    add t0, t0, s1\n    call g\n    bnez t0, .LBB1\n    ld s1, -48(s0)\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        assert!(
            !coalesce_entry_copies(&mut lines, &mut kinds, n),
            "{:#?}",
            live_lines(&lines, &kinds)
        );
    }

    fn code_lines(out: &str) -> Vec<String> {
        out.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('.') && !l.ends_with(':'))
            .map(str::to_string)
            .collect()
    }

    /// The emitter's loop shape for `for (i...) s += a[i]`: the copies it
    /// stages through t0-t2 before each branch are dead across the back edge
    /// and the exit, and the accumulator copies forward into the add.
    #[test]
    fn global_liveness_cleans_the_accumulator_loop() {
        let asm = "sum:\n    li a2, 0\n    li a3, 0\n.LBB1:\n    mv t1, a3\n    mv t0, a1\n    mv t2, a1\n    blt t1, a1, .LBB2\n    j .LBB3\n.LBB2:\n    mv t5, a0\n    ld t0, 0(a0)\n    mv a5, t0\n    mv t1, a2\n    add a2, t1, a5\n    addi a3, a3, 1\n    addi a0, a0, 8\n    mv t1, a3\n    mv t0, a1\n    mv t2, a1\n    blt t1, a1, .LBB2\n.LBB3:\n    mv a0, a2\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(
            code_lines(&out),
            [
                "li a2, 0",
                "li a3, 0",
                "bge a3, a1, .LBB3",
                // The load writes the copy's destination directly.
                "ld a5, 0(a0)",
                "add a2, a2, a5",
                "addi a3, a3, 1",
                "addi a0, a0, 8",
                "blt a3, a1, .LBB2",
                "mv a0, a2",
                "ret"
            ],
            "{out}"
        );
    }

    #[test]
    fn global_liveness_keeps_values_a_successor_reads() {
        // t1 is read in .L2, reached through the branch: not dead.
        let asm = "f:\n    li t1, 9\n    beqz a0, .L2\n    li a0, 1\n    ret\n.L2:\n    mv a0, t1\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(code_lines(&out).contains(&"li t1, 9".to_string()), "{out}");
        // Only a0/a1 are read by `ret`: a3 is dead, a1 is not.
        let asm = "f:\n    li a3, 5\n    li a1, 6\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["li a1, 6", "ret"], "{out}");
    }

    #[test]
    fn global_liveness_respects_calls_and_unknown_control_flow() {
        // A call may read any argument or scratch register (t2: the static
        // chain of a nested function).
        let asm = "f:\n    li t2, 5\n    call g\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(code_lines(&out).contains(&"li t2, 5".to_string()), "{out}");
        // An indirect jump has unknown successors: nothing is provably dead.
        let asm = "f:\n    li t3, 4\n    jr t0\n.L1:\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(code_lines(&out).contains(&"li t3, 4".to_string()), "{out}");
        // A tail jump to another function behaves like a call.
        let asm = "f:\n    li a2, 4\n    j other\n";
        let out = peephole_optimize(asm.into());
        assert!(code_lines(&out).contains(&"li a2, 4".to_string()), "{out}");
    }

    #[test]
    fn address_and_extension_fold_into_the_access() {
        let asm = "f:\n    addi t0, a0, 4\n    lw t0, 0(t0)\n    sext.w a5, t0\n    addi t1, a0, 8\n    sd a5, 0(t1)\n    mv a0, a5\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(
            code_lines(&out),
            ["lw a5, 4(a0)", "sd a5, 8(a0)", "mv a0, a5", "ret"],
            "{out}"
        );
        // The temporary is read again later: no fold.
        let asm = "f:\n    addi t0, a0, 4\n    lw t1, 0(t0)\n    add a0, t0, t1\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(
            code_lines(&out).contains(&"addi t0, a0, 4".to_string()),
            "{out}"
        );
        // Out of the 12-bit range: no fold.
        let asm = "f:\n    addi t0, a0, 2047\n    lw a0, 8(t0)\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(
            code_lines(&out).contains(&"addi t0, a0, 2047".to_string()),
            "{out}"
        );
        // lwu then sext.w is lw.
        let asm = "f:\n    lwu t0, 0(a1)\n    sext.w a0, t0\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["lw a0, 0(a1)", "ret"], "{out}");
    }

    /// FP values reach and leave ft0/ft1 through general registers; the
    /// round trips fold into FP loads, stores and moves when the temporary
    /// dies at the consumer.
    #[test]
    fn fp_round_trips_fold_into_fp_memory_operations() {
        let asm = "f:\n    ld t0, 0(a1)\n    ld t1, 8(a1)\n    mv t2, t0\n    fmv.d.x ft0, t1\n    fmv.d.x ft1, t2\n    fmul.d ft0, ft0, ft1\n    fmv.x.d t0, ft0\n    sd t0, 0(a0)\n    fmv.x.d t1, ft1\n    fmv.d.x fa0, t1\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(
            code_lines(&out),
            [
                "fld ft0, 8(a1)",
                "fld ft1, 0(a1)",
                "fmul.d ft0, ft0, ft1",
                "fsd ft0, 0(a0)",
                "fmv.d fa0, ft1",
                "ret"
            ],
            "{out}"
        );
        // Single precision: the FP-to-FP move is `fmv.s`, the memory forms
        // `flw`/`fsw`.
        let asm = "f:\n    lw t0, 4(a1)\n    fmv.w.x ft0, t0\n    fmv.x.w t1, ft0\n    fmv.w.x fa0, t1\n    fmv.x.w t2, ft0\n    sw t2, 0(a0)\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(
            code_lines(&out),
            ["flw ft0, 4(a1)", "fmv.s fa0, ft0", "fsw ft0, 0(a0)", "ret"],
            "{out}"
        );
        // The temporary is read again: no fold.
        let asm = "f:\n    fmv.x.d t0, ft0\n    sd t0, 0(a0)\n    mv a0, t0\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(
            code_lines(&out).contains(&"fmv.x.d t0, ft0".to_string()),
            "{out}"
        );
        // A store between the load and its FP move blocks the sink.
        let asm = "f:\n    ld t0, 0(a1)\n    fsd ft2, 0(a1)\n    fmv.d.x ft1, t0\n    fmv.x.d a0, ft1\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(
            code_lines(&out).contains(&"ld t0, 0(a1)".to_string()),
            "{out}"
        );
    }

    fn run_fp_pass(
        input: &str,
        pass: fn(&mut [String], &mut [LineKind], usize) -> bool,
    ) -> Vec<String> {
        let mut lines: Vec<String> = input.lines().map(str::to_string).collect();
        let n = lines.len();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let _ = pass(&mut lines, &mut kinds, n);
        live_lines(&lines, &kinds)
    }

    /// The FP role table: every register spelling, the emitted F/D shapes,
    /// and `None` for a shape it does not model.
    #[test]
    fn fp_roles_cover_the_emitted_shapes() {
        for id in 0..32u8 {
            assert_eq!(rv_fp_reg(rv_fp_name(id)), Some(id));
            assert_eq!(rv_fp_reg(&format!("f{id}")), Some(id));
        }
        for bad in ["ft12", "fs12", "fa8", "f32", "f01", "fp", "fa", "x5"] {
            assert_eq!(rv_fp_reg(bad), None, "{bad}");
        }
        let fa = |k: u8| rv_fp_bit(10 + k);
        assert_eq!(
            rv_fp_roles("    fadd.d ft0, fa0, fa1"),
            Some(RvFpRoles {
                def: Some(0),
                src_ops: 0b110,
                uses: fa(0) | fa(1),
                single: false,
            })
        );
        let shape = |l: &str| rv_fp_roles(l).map(|r| (r.def, r.src_ops, r.uses, r.single));
        assert_eq!(
            shape("fmul.s ft1, ft1, fa0, rne"),
            Some((Some(1), 0b110, 2 | fa(0), true))
        );
        assert_eq!(
            shape("fmadd.d fa0, fa1, fa2, fa3"),
            Some((Some(10), 0b1110, fa(1) | fa(2) | fa(3), false))
        );
        assert_eq!(shape("fneg.d ft0, ft0"), Some((Some(0), 0b10, 1, false)));
        assert_eq!(shape("fcvt.l.d t0, ft0, rtz"), Some((None, 0b10, 1, false)));
        assert_eq!(shape("fcvt.s.l ft2, a0"), Some((Some(2), 0, 0, true)));
        assert_eq!(
            shape("fcvt.d.s ft0, fa0"),
            Some((Some(0), 0b10, fa(0), false))
        );
        assert_eq!(shape("fmv.w.x ft0, zero"), Some((Some(0), 0, 0, true)));
        assert_eq!(
            shape("fmv.x.d a0, fs11"),
            Some((None, 0b10, rv_fp_bit(27), false))
        );
        assert_eq!(shape("flt.d t0, ft0, ft1"), Some((None, 0b110, 3, false)));
        assert_eq!(
            shape("fclass.s t0, ft8"),
            Some((None, 0b10, rv_fp_bit(28), false))
        );
        assert_eq!(shape("fld ft11, 8(a0)"), Some((Some(31), 0, 0, false)));
        assert_eq!(shape("flw fs0, -4(s0)"), Some((Some(8), 0, 0, true)));
        assert_eq!(
            shape("fsd fs2, -8(s0)"),
            Some((None, 1, rv_fp_bit(18), false))
        );
        // Lines without FP registers have no FP roles.
        assert_eq!(shape("add a0, a1, a2"), Some((None, 0, 0, false)));
        // Unknown shapes, a general register where an FP source belongs, an
        // FP register where a general one belongs: unknown.
        for line in [
            "fli.d ft0, 1.0",
            "fadd.d ft0, fa0, t1",
            "fcvt.l.d ft0, ft1, rtz",
            "fadd.d ft0, fa0, fa1, ft2",
            "fsd ft0, 0(ft1)",
            "vfmv.f.s fa0, v1",
        ] {
            assert_eq!(rv_fp_roles(line), None, "{line}");
        }
    }

    /// A frame slot's FP value answers the reloads that follow its store, in
    /// both register files; an overlapping or `sp`-based store, a write to
    /// the stored register, a join point or a call retires the fact, and an
    /// escaping frame address disables the pass.
    #[test]
    fn fp_slot_reloads_forward_from_the_stored_register() {
        let out = run_fp_pass(
            "f:\n    fsd fa0, -24(s0)\n    fsw fa1, -32(s0)\n    fld ft0, -24(s0)\n    flw ft1, -32(s0)\n    ld t0, -24(s0)\n    lw t1, -32(s0)\n    fld fa0, -24(s0)\n    ret\n",
            forward_fp_slot_reloads,
        );
        assert_eq!(
            out,
            [
                "f:",
                "fsd fa0, -24(s0)",
                "fsw fa1, -32(s0)",
                "fmv.d ft0, fa0",
                "fmv.s ft1, fa1",
                "fmv.x.d t0, fa0",
                "fmv.x.w t1, fa1",
                "ret"
            ]
        );
        for (between, why) in [
            ("    sw t3, -20(s0)\n", "overlapping store"),
            ("    fadd.d fa0, fa0, fa1\n", "stored register rewritten"),
            ("    call g\n", "call"),
            ("    sd t3, 8(sp)\n", "sp-based store"),
        ] {
            let asm = format!("f:\n    fsd fa0, -24(s0)\n{between}    fld ft0, -24(s0)\n    ret\n");
            let out = run_fp_pass(&asm, forward_fp_slot_reloads);
            assert!(
                out.contains(&"fld ft0, -24(s0)".to_string()),
                "{why}: {out:?}"
            );
        }
        // A store through another base cannot reach a private frame, and a
        // branch's fall-through keeps the fact.  At a join the facts every
        // predecessor agrees on survive: an argument spilled at entry is
        // answered from its register after an `if`, unless one path wrote
        // the register.
        let join = "f:\n    fsd fa0, -24(s0)\n    beqz a1, .L1\n    fadd.d fa1, fa1, fa1\n.L1:\n    fld ft0, -24(s0)\n    ret\n";
        let out = run_fp_pass(join, forward_fp_slot_reloads);
        assert!(out.contains(&"fmv.d ft0, fa0".to_string()), "{out:?}");
        let out = run_fp_pass(
            &join.replace("fadd.d fa1, fa1, fa1", "fadd.d fa0, fa1, fa1"),
            forward_fp_slot_reloads,
        );
        assert!(out.contains(&"fld ft0, -24(s0)".to_string()), "{out:?}");
        // An unconditional jump carries the facts to its target, where they
        // meet the other predecessor's.
        let jumped = "f:\n    fsd fa0, -24(s0)\n    beqz a1, .L1\n    j .L2\n.L1:\n    fsd fa1, -24(s0)\n.L2:\n    fld ft0, -24(s0)\n    ret\n";
        let out = run_fp_pass(jumped, forward_fp_slot_reloads);
        assert!(out.contains(&"fld ft0, -24(s0)".to_string()), "{out:?}");
        let out = run_fp_pass(
            &jumped.replace("fsd fa1, -24(s0)", "fsd fa0, -24(s0)"),
            forward_fp_slot_reloads,
        );
        assert!(out.contains(&"fmv.d ft0, fa0".to_string()), "{out:?}");
        // A loop: the back edge brings a store of another register.
        let out = run_fp_pass(
            "f:\n    fsd fa0, -24(s0)\n.L1:\n    fld ft0, -24(s0)\n    fsd ft1, -24(s0)\n    bnez a1, .L1\n    ret\n",
            forward_fp_slot_reloads,
        );
        assert!(out.contains(&"fld ft0, -24(s0)".to_string()), "{out:?}");
        let out = run_fp_pass(
            "f:\n    fsd fa0, -24(s0)\n    sd t3, 0(a0)\n    beqz a1, .L1\n    fld ft0, -24(s0)\n.L1:\n    ret\n",
            forward_fp_slot_reloads,
        );
        assert!(out.contains(&"fmv.d ft0, fa0".to_string()), "{out:?}");
        // The frame address escapes: no forwarding.
        let out = run_fp_pass(
            "f:\n    addi a1, s0, -24\n    fsd fa0, -24(s0)\n    fld ft0, -24(s0)\n    ret\n",
            forward_fp_slot_reloads,
        );
        assert!(out.contains(&"fld ft0, -24(s0)".to_string()), "{out:?}");
        // Widths must match: a word reload of a doubleword store reads half.
        let out = run_fp_pass(
            "f:\n    fsd fa0, -24(s0)\n    flw ft0, -24(s0)\n    lw t0, -24(s0)\n    ret\n",
            forward_fp_slot_reloads,
        );
        assert!(out.contains(&"flw ft0, -24(s0)".to_string()), "{out:?}");
        assert!(out.contains(&"lw t0, -24(s0)".to_string()), "{out:?}");
    }

    /// A bit move leaves both registers with the same bits: the general
    /// register's FP uses and stores read the FP side, and the bit move
    /// then dies; a rewrite of either register ends the fact.
    #[test]
    fn fp_bit_moves_serve_their_uses_from_the_fp_side() {
        let out = run_fp_pass(
            "f:\n    fmv.x.d t0, fa0\n    fmv.d.x ft8, t0\n    fmv.d.x ft0, t0\n    sd t0, 0(a0)\n    ld t1, 8(a0)\n    fmv.d.x ft1, t1\n    fmv.d.x ft2, t1\n    fmv.x.w t2, fa1\n    sw t2, 16(a0)\n    fmv.w.x ft3, t2\n    ret\n",
            forward_fp_bit_moves,
        );
        assert_eq!(
            out,
            [
                "f:",
                "fmv.x.d t0, fa0",
                "fmv.d ft8, fa0",
                "fmv.d ft0, fa0",
                "fsd fa0, 0(a0)",
                "ld t1, 8(a0)",
                "fmv.d.x ft1, t1",
                "fmv.d ft2, ft1",
                "fmv.x.w t2, fa1",
                "fsw fa1, 16(a0)",
                "fmv.s ft3, fa1",
                "ret"
            ]
        );
        // The full pipeline then deletes the dead bit move.
        let out = peephole_optimize(
            "f:\n    fmv.x.d t0, fa0\n    fmv.d.x ft0, t0\n    fadd.d fa0, ft0, ft0\n    ret\n"
                .into(),
        );
        assert_eq!(code_lines(&out), ["fadd.d fa0, fa0, fa0", "ret"], "{out}");
        for (between, why) in [
            ("    addi t0, t0, 1\n", "general register rewritten"),
            ("    fneg.d fa0, fa0\n", "FP register rewritten"),
            (".L1:\n", "join point"),
            ("    call g\n", "call"),
        ] {
            let asm = format!("f:\n    fmv.x.d t0, fa0\n{between}    fmv.d.x ft1, t0\n    ret\n");
            let out = run_fp_pass(&asm, forward_fp_bit_moves);
            assert!(
                out.contains(&"fmv.d.x ft1, t0".to_string()),
                "{why}: {out:?}"
            );
        }
        // Widths must agree: the doubleword fact serves no word move (fa0's
        // low word is not a boxed single); the word move then mirrors t0's
        // low word in ft1, which is all `sw` stores.
        let out = run_fp_pass(
            "f:\n    fmv.x.d t0, fa0\n    fmv.w.x ft1, t0\n    sw t0, 0(a0)\n    ret\n",
            forward_fp_bit_moves,
        );
        assert_eq!(
            out,
            [
                "f:",
                "fmv.x.d t0, fa0",
                "fmv.w.x ft1, t0",
                "fsw ft1, 0(a0)",
                "ret"
            ]
        );
    }

    /// A copy into an FP scratch is forwarded into its readers when the
    /// scratch is rewritten or dead before the next transfer and the source
    /// holds its value at every reader.
    #[test]
    fn fp_scratch_copies_forward_into_their_readers() {
        let out = run_fp_pass(
            "f:\n    fmv.d ft0, fa0\n    fmv.d ft1, fa1\n    fadd.d ft0, ft0, ft1\n    fmv.d fa0, ft0\n    ret\n",
            forward_fp_copies,
        );
        assert_eq!(out, ["f:", "fadd.d ft0, fa0, fa1", "fmv.d fa0, ft0", "ret"]);
        // The source is rewritten before the second reader.
        let out = run_fp_pass(
            "f:\n    fmv.d ft0, fa0\n    fadd.d ft1, ft0, fa1\n    fmv.d fa0, fa2\n    fadd.d fa1, ft0, ft1\n    ret\n",
            forward_fp_copies,
        );
        assert!(out.contains(&"fmv.d ft0, fa0".to_string()), "{out:?}");
        // The scratch is live into a successor.
        let out = run_fp_pass(
            "f:\n    fmv.d ft0, fa0\n    beqz a0, .L1\n    fmv.d fa0, fa1\n.L1:\n    fadd.d fa0, fa0, ft0\n    ret\n",
            forward_fp_copies,
        );
        assert!(out.contains(&"fmv.d ft0, fa0".to_string()), "{out:?}");
        // A double self copy and a dead load into a scratch go; a single
        // self copy is a NaN-boxing check and stays.
        let out = run_fp_pass(
            "f:\n    fmv.d ft0, ft0\n    fmv.s ft2, ft2\n    fld ft1, 0(a0)\n    fmv.d fa0, ft0\n    ret\n",
            forward_fp_copies,
        );
        assert_eq!(out, ["f:", "fmv.s ft2, ft2", "fmv.d fa0, ft0", "ret"]);
    }

    /// A result computed into a scratch only to be copied out is computed
    /// into the copy's destination.
    #[test]
    fn fp_results_compute_into_their_copy() {
        let out = run_fp_pass(
            "f:\n    fadd.d ft0, fa0, fa1\n    fmv.d fa0, ft0\n    ret\n",
            retarget_fp_results,
        );
        assert_eq!(out, ["f:", "fadd.d fa0, fa0, fa1", "ret"]);
        for (asm, why) in [
            (
                "f:\n    fadd.d ft0, fa0, fa1\n    fmv.d fa0, ft0\n    fsd ft0, 0(a0)\n    ret\n",
                "scratch read after the copy",
            ),
            (
                "f:\n    fadd.d ft0, fa1, fa2\n    fsd fa0, 0(a0)\n    fmv.d fa0, ft0\n    ret\n",
                "destination read in between",
            ),
            (
                "f:\n    fadd.d ft0, fa0, fa1\n    fmv.s fa0, ft0\n    ret\n",
                "single copy of a double",
            ),
            (
                "f:\n    fadd.d ft0, fa0, fa1\n.L1:\n    fmv.d fa0, ft0\n    ret\n",
                "join point in between",
            ),
        ] {
            let out = run_fp_pass(asm, retarget_fp_results);
            assert!(out.iter().any(|l| l.starts_with("fmv.")), "{why}: {out:?}");
        }
        let out = run_fp_pass(
            "f:\n    fmul.s ft0, fa0, fa1\n    fsw fa2, 0(a0)\n    fmv.s fa0, ft0\n    ret\n",
            retarget_fp_results,
        );
        assert_eq!(out, ["f:", "fmul.s fa0, fa0, fa1", "fsw fa2, 0(a0)", "ret"]);
    }

    /// FP and narrow frame stores nothing reads are dead; a store an integer
    /// load reads back is not.
    #[test]
    fn dead_fp_and_narrow_frame_stores_are_deleted() {
        let asm = "f:\n    fsd fa0, -24(s0)\n    fsw fa1, -32(s0)\n    sh a0, -40(s0)\n    sb a1, -48(s0)\n    fsd fa2, -56(s0)\n    ld a0, -56(s0)\n    ret\n";
        let lines: Vec<String> = asm.lines().map(str::to_string).collect();
        let n = lines.len();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        assert!(global_dead_store_elimination(&lines, &mut kinds, n));
        assert_eq!(
            live_lines(&lines, &kinds),
            ["f:", "fsd fa2, -56(s0)", "ld a0, -56(s0)", "ret"]
        );
    }

    /// The emitter spills a single through `fmv.x.w` and a doubleword `sd`:
    /// when every reader of the slot reads the low word only, the spill
    /// becomes `fsw` and a doubleword reload feeding `fmv.w.x` becomes `flw`.
    #[test]
    fn single_spills_narrow_to_fsw_when_the_upper_word_is_unread() {
        let asm = "f:\n    fmv.x.w t0, fa0\n    sd t0, -24(s0)\n    fmv.x.w t0, fa1\n    ld t1, -24(s0)\n    fmv.w.x ft0, t1\n    fmv.w.x ft1, t0\n    fadd.s ft0, ft0, ft1\n    fmv.s fa0, ft0\n    ret\n";
        let out = run_fp_pass(asm, fold_fp_bounces);
        assert_eq!(
            out,
            [
                "f:",
                "fsw fa0, -24(s0)",
                "flw ft0, -24(s0)",
                "fmv.s ft1, fa1",
                "fadd.s ft0, ft0, ft1",
                "fmv.s fa0, ft0",
                "ret"
            ]
        );
        // A doubleword reader of the slot keeps the doubleword spill.
        let out = run_fp_pass(
            &asm.replace("    ret\n", "    ld a0, -24(s0)\n    ret\n"),
            fold_fp_bounces,
        );
        assert!(out.contains(&"sd t0, -24(s0)".to_string()), "{out:?}");
        // The FP source is rewritten before the consumer: the FP move takes
        // the bit move's place -- unless a line in between names its
        // destination, or the consumer is a store, which keeps its place.
        let out = run_fp_pass(
            "f:\n    fmv.x.d t0, fa1\n    fadd.d fa1, fa1, fa1\n    fmv.d.x ft1, t0\n    fsd ft1, 0(a0)\n    ret\n",
            fold_fp_bounces,
        );
        assert_eq!(
            out,
            [
                "f:",
                "fmv.d ft1, fa1",
                "fadd.d fa1, fa1, fa1",
                "fsd ft1, 0(a0)",
                "ret"
            ]
        );
        for asm in [
            "f:\n    fmv.x.d t0, fa1\n    fadd.d fa1, fa1, ft1\n    fmv.d.x ft1, t0\n    fsd ft1, 0(a0)\n    ret\n",
            "f:\n    fmv.x.d t0, fa1\n    fadd.d fa1, fa1, fa1\n    sd t0, 0(a0)\n    ret\n",
        ] {
            let out = run_fp_pass(asm, fold_fp_bounces);
            assert!(out.contains(&"fmv.x.d t0, fa1".to_string()), "{out:?}");
        }
    }

    /// The emitter's `double addd(double a, double b) { return a + b; }` at
    /// -O2, before the passes: every slot and copy goes, then the frame
    /// nothing touches any more -- Clang's two instructions.
    #[test]
    fn fp_leaf_functions_drop_their_vestigial_frame() {
        let asm = ".globl addd\n.type addd, @function\naddd:\n.cfi_startproc\n    # lccc-ret-regs -\n    addi sp, sp, -48\n    .cfi_def_cfa_offset 48\n    sd ra, 40(sp)\n    sd s0, 32(sp)\n    addi s0, sp, 48\n    .cfi_def_cfa s0, 0\n    .cfi_offset ra, -8\n    .cfi_offset s0, -16\n    fmv.x.d t0, fa0\n    sd t0, -24(s0)\n    fmv.x.d t0, fa1\n    sd t0, -32(s0)\n    ld t0, -24(s0)\n    mv t1, t0\n    ld t0, -32(s0)\n    mv t2, t0\n    fmv.d.x ft0, t1\n    fmv.d.x ft1, t2\n    fadd.d ft0, ft0, ft1\n    fmv.x.d t0, ft0\n    sd t0, -40(s0)\n    fmv.d.x fa0, t0\n    ld ra, 40(sp)\n    ld s0, 32(sp)\n    addi sp, sp, 48\n    ret\n.cfi_endproc\n.size addd, .-addd\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["fadd.d fa0, fa0, fa1", "ret"], "{out}");
        assert!(
            !out.contains(".cfi_def_cfa") && !out.contains(".cfi_offset"),
            "{out}"
        );
        assert!(
            out.contains(".cfi_startproc") && out.contains(".cfi_endproc"),
            "{out}"
        );
        // A call needs ra saved; a stack-passed argument read needs s0.
        for (extra, why) in [
            ("    call g\n", "call"),
            ("    ld t3, 0(s0)\n    sd t3, 0(a0)\n", "frame read"),
        ] {
            let framed = peephole_optimize(asm.replace(
                "    ld ra, 40(sp)\n",
                &format!("{extra}    ld ra, 40(sp)\n"),
            ));
            assert!(framed.contains("addi sp, sp, -48"), "{why}: {framed}");
            assert!(framed.contains("ld ra, 40(sp)"), "{why}: {framed}");
        }
    }

    /// FP results in a general register: the word conversions, `fmv.x.w`,
    /// the comparisons and `fclass` are sign-extended producers, and FP lines
    /// in between keep the proof; the doubleword forms are not.
    #[test]
    fn fp_word_results_are_sext32_producers() {
        for producer in [
            "fcvt.w.d t0, ft0, rtz",
            "fcvt.wu.s t0, ft0, rtz",
            "fmv.x.w t0, ft0",
            "flt.d t0, ft0, ft1",
            "fclass.d t0, ft0",
        ] {
            let asm = format!(
                "f:\n    {producer}\n    fadd.d ft2, ft2, ft3\n    sext.w t0, t0\n    sd t0, 0(a0)\n    ret\n"
            );
            let out = peephole_optimize(asm);
            assert!(!out.contains("sext.w"), "{producer}: {out}");
        }
        for producer in [
            "fcvt.l.d t0, ft0, rtz",
            "fcvt.lu.d t0, ft0, rtz",
            "fmv.x.d t0, ft0",
        ] {
            let asm = format!("f:\n    {producer}\n    sext.w t0, t0\n    sd t0, 0(a0)\n    ret\n");
            let out = peephole_optimize(asm);
            assert!(out.contains("sext.w"), "{producer}: {out}");
        }
    }

    #[test]
    fn retarget_fp_result_in_a_general_register_writes_the_home() {
        let out = run_retarget("f:\n    fcvt.l.d t0, ft0, rtz\n    mv a0, t0\n    ret\n");
        assert_eq!(out, "f:\n    fcvt.l.d a0, ft0, rtz\n    ret");
        let out = run_retarget("f:\n    flt.d t1, fa0, fa1\n    mv a0, t1\n    ret\n");
        assert_eq!(out, "f:\n    flt.d a0, fa0, fa1\n    ret");
    }

    /// Frame slots are per function: another function reading the same
    /// offset keeps nothing alive here, and a frame address that escapes in
    /// one function does not disable the census in the others.
    #[test]
    fn dead_frame_stores_are_judged_per_function() {
        let asm = "f:\n    sd a0, -32(s0)\n    li a0, 1\n    ret\ng:\n    ld a0, -32(s0)\n    addi t0, s0, -40\n    sd t0, 0(a1)\n    ret\nh:\n    sd a0, -24(s0)\n    ret\n";
        let out = peephole_optimize(asm.into());
        let f = out.split("g:").next().unwrap_or("");
        assert!(!f.contains("sd a0, -32(s0)"), "{out}");
        let h = out.rsplit("h:").next().unwrap_or("");
        assert!(!h.contains("sd a0, -24(s0)"), "{out}");
        // g takes a frame address: its stores stay.
        assert!(out.contains("sd t0, 0(a1)"), "{out}");
    }

    #[test]
    fn zero_constants_read_as_the_zero_register() {
        let asm = "f:\n    li t2, 0\n    slt t0, a0, t2\n    blt a1, t2, .L1\n    mv a0, t0\n    ret\n.L1:\n    li a0, 7\n    ret\n";
        let out = peephole_optimize(asm.into());
        let code = code_lines(&out);
        assert!(code.contains(&"slt t0, a0, zero".to_string()), "{out}");
        assert!(code.contains(&"blt a1, zero, .L1".to_string()), "{out}");
        assert!(!code.contains(&"li t2, 0".to_string()), "{out}");
    }

    /// The prologue publishes which registers `ret` hands back; a value left
    /// in a1 by a single-register function is a dead write, while an
    /// unannotated function keeps the conservative a0/a1.
    #[test]
    fn published_return_registers_bound_the_ret_reads() {
        let asm = "f:\n    # lccc-ret-regs a0\n    mv a1, t0\n    sext.w a0, t0\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["sext.w a0, t0", "ret"], "{out}");
        assert!(!out.contains("lccc-ret-regs"), "{out}");
        let asm = "f:\n    # lccc-ret-regs -\n    li a0, 3\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["ret"], "{out}");
        let asm = "f:\n    mv a1, t0\n    sext.w a0, t0\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(code_lines(&out).contains(&"mv a1, t0".to_string()), "{out}");
    }

    /// The annotation is internal on every exit: a function the passes keep
    /// opaque (inline assembly) and a bail-out on malformed volatile markers
    /// both drop it.
    #[test]
    fn return_register_annotation_never_reaches_the_output() {
        let opaque = ".type f, @function\nf:\n.cfi_startproc\n    # lccc-ret-regs a0\n#APP\n    nop\n#NO_APP\n    ret\n.cfi_endproc\n.size f, .-f\n";
        let out = peephole_optimize(opaque.into());
        assert!(!out.contains("lccc-ret-regs"), "{out}");
        assert!(out.contains("#APP\n    nop\n#NO_APP"), "{out}");
        let malformed = "f:\n    # lccc-ret-regs a0\n    # LCCC_VOLATILE_END\n    ret\n";
        let out = peephole_optimize(malformed.into());
        assert!(!out.contains("lccc-ret-regs"), "{out}");
        assert!(out.contains("    ret"), "{out}");
    }

    #[test]
    fn branch_pseudoinstructions_classify_as_branches() {
        for line in [
            "    ble t5, t6, .L1",
            "    bgt t1, t2, .L1",
            "    bgez a0, .L1",
            "    bltz a0, .L1",
        ] {
            assert_eq!(classify_line(line), LineKind::Branch, "{line}");
        }
        assert_eq!(invert_branch_mnemonic("ble"), Some("bgt"));
        assert_eq!(invert_branch_mnemonic("bgtz"), Some("blez"));
    }

    #[test]
    fn loop_alignment_does_not_block_near_branches() {
        assert_eq!(directive_max_bytes("    .p2align 4,,10"), Some(10));
        assert_eq!(directive_max_bytes("    .p2align 3"), Some(7));
        assert_eq!(directive_max_bytes("    .balign 16"), Some(15));
        assert_eq!(directive_max_bytes("    .cfi_def_cfa_offset 16"), Some(0));
        assert_eq!(directive_max_bytes("    .space 64"), None);
        assert_eq!(directive_max_bytes("    .section .rodata"), None);
        let asm = "f:\n    bge t1, a1, .Lskip\n    jump .Lbody, t6\n.Lskip:\n    jump .Lexit, t6\n    .p2align 4,,10\n    .p2align 3\n.Lbody:\n    addi a0, a0, 1\n    blt a0, a1, .Lbody\n.Lexit:\n    ret\n";
        let out = peephole_optimize(asm.into());
        // Both far jumps go: the guard branches straight to the exit and the
        // loop is entered by falling into its aligned header.
        assert!(!out.contains("jump ") && !out.contains("j .L"), "{out}");
        assert!(out.contains("bge t1, a1, .Lexit"), "{out}");
    }

    #[test]
    fn linking_jumps_classify_as_calls() {
        for line in [
            "    call foo",
            "    jal ra, foo",
            "    jal foo",
            "    jalr ra, t2, 0",
            "    jalr ra, 0(t2)",
            "    jalr t2",
            "    jalr t2, 8",
            "    jalr t1, t2",
        ] {
            assert_eq!(classify_line(line), LineKind::Call, "{line}");
        }
        for line in [
            "    jalr zero, 0(t0)",
            "    jalr x0, t0, 0",
            "    jal zero, foo",
        ] {
            assert_ne!(classify_line(line), LineKind::Call, "{line}");
        }
    }

    #[test]
    fn identical_blocks_with_different_fallthroughs_do_not_merge() {
        let asm = "f:\n    beq a0, zero, .Ldup\n.Lkeep:\n    li t0, 1\n.Lkeep_next:\n    li t1, 11\n    ret\n.Ldup:\n    li t0, 1\n.Ldup_next:\n    li t1, 22\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!merge_identical_blocks(&mut lines, &mut kinds, n));
        assert_eq!(kinds[7], LineKind::Label, "duplicate block was deleted");
    }

    #[test]
    fn identical_blocks_preserve_jump_table_data_references() {
        let asm = "f:\n    j .Lkeep\n.section .rodata\n.Ljt:\n    .word .Lkeep - .Ljt\n    .word .Ldup - .Ljt\n.section .text\n.Lkeep:\n    j .Ldone\n.Ldup:\n    j .Ldone\n.Ldone:\n    li a0, 42\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!merge_identical_blocks(&mut lines, &mut kinds, n));
        let dup = lines.iter().position(|line| line == ".Ldup:").unwrap();
        assert_eq!(kinds[dup], LineKind::Label, "jump target was deleted");
    }

    #[test]
    fn identical_blocks_still_merge_without_data_references() {
        let asm = "f:\n    j .Lkeep\n.Lkeep:\n    j .Ldone\n.Ldup:\n    j .Ldone\n.Ldone:\n    li a0, 42\n    ret\n";
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(merge_identical_blocks(&mut lines, &mut kinds, n));
        let dup = lines.iter().position(|line| line == ".Ldup:").unwrap();
        assert_eq!(kinds[dup], LineKind::Nop, "plain duplicate was kept");
    }

    #[test]
    fn branch_over_jump_fuses_only_to_in_range_target() {
        let input = "f:\n    beq t0, t1, .Lskip\n    jump .Ltarget, t6\n.Lskip:\n    li t2, 3\n.Ltarget:\n    ret\n";
        let out = peephole_optimize(input.to_string());
        assert!(out.contains("bne t0, t1, .Ltarget"), "{out}");
        assert!(!out.contains("jump .Ltarget"), "{out}");
    }

    /// `-g` rows are hidden from every pass: a row between the branch and
    /// its target is no directive the fusion must fail closed on (the
    /// `bge .Lskip; j` pair survived `-g` in k_hashmix.c at -O0).
    #[test]
    fn debug_rows_change_no_code() {
        let plain = "f:\n    beq t0, t1, .Lskip\n    jump .Ltarget, t6\n.Lskip:\n    li a0, 3\n.Ltarget:\n    ret\n";
        let rows = "f:\n.loc 1 2 3\n    beq t0, t1, .Lskip\n.loc 1 3 1\n    jump .Ltarget, t6\n.Lskip:\n.loc 1 4 5\n    li a0, 3\n.Ltarget:\n.loc 1 5 1\n    ret\n";
        let (plain, debug) = (
            peephole_optimize(plain.into()),
            peephole_optimize(rows.into()),
        );
        let code = |s: &str| {
            s.lines()
                .filter(|l| !l.starts_with(".loc"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(code(&debug), code(&plain));
        assert!(plain.contains("bne t0, t1, .Ltarget"), "{plain}");
        // Each row survives, directly before the line it described.
        assert!(
            debug.contains(".loc 1 2 3\n    bne t0, t1, .Ltarget"),
            "{debug}"
        );
        assert!(
            debug.contains(".loc 1 4 5\n    li a0, 3") && debug.contains(".loc 1 5 1\n    ret"),
            "{debug}"
        );
    }

    #[test]
    fn branch_over_jump_keeps_out_of_range_target() {
        let mut input =
            String::from("f:\n    beq t0, t1, .Lskip\n    jump .Ltarget, t6\n.Lskip:\n");
        for _ in 0..100 {
            input.push_str("    add t2, t2, t3\n");
        }
        input.push_str(".Ltarget:\n    ret\n");
        let out = peephole_optimize(input);
        assert!(out.contains("beq t0, t1, .Lskip"), "{out}");
    }

    #[test]
    fn test_classify_store() {
        assert!(matches!(
            classify_line("    sd t0, -24(s0)"),
            LineKind::StoreS0 {
                reg: REG_T0,
                offset: -24,
                is_word: false
            }
        ));
        assert!(matches!(
            classify_line("    sw a0, -32(s0)"),
            LineKind::StoreS0 {
                reg: REG_A0,
                offset: -32,
                is_word: true
            }
        ));
    }

    #[test]
    fn test_classify_load() {
        assert!(matches!(
            classify_line("    ld t0, -24(s0)"),
            LineKind::LoadS0 {
                reg: REG_T0,
                offset: -24,
                is_word: false
            }
        ));
    }

    #[test]
    fn test_classify_move() {
        assert!(matches!(
            classify_line("    mv t1, t0"),
            LineKind::Move {
                dst: REG_T1,
                src: REG_T0
            }
        ));
    }

    #[test]
    fn test_classify_li() {
        assert!(matches!(
            classify_line("    li t0, 0"),
            LineKind::LoadImm { dst: REG_T0 }
        ));
        assert!(matches!(
            classify_line("    li a0, 42"),
            LineKind::LoadImm { dst: REG_A0 }
        ));
    }

    #[test]
    fn test_classify_sext_w() {
        assert!(matches!(
            classify_line("    sext.w t0, t0"),
            LineKind::SextW {
                dst: REG_T0,
                src: REG_T0
            }
        ));
    }

    #[test]
    fn test_classify_jump() {
        assert_eq!(classify_line("    jump .LBB3, t6"), LineKind::Jump);
        assert_eq!(classify_line("    j .LBB3"), LineKind::Jump);
    }

    #[test]
    fn test_classify_label() {
        assert_eq!(classify_line(".LBB3:"), LineKind::Label);
        assert_eq!(classify_line("main:"), LineKind::Label);
    }

    #[test]
    fn test_classify_ret() {
        assert_eq!(classify_line("    ret"), LineKind::Ret);
    }

    #[test]
    fn test_classify_branch() {
        assert_eq!(classify_line("    beq t1, t2, .LBB4"), LineKind::Branch);
        assert_eq!(classify_line("    bge t1, t2, .Lskip_0"), LineKind::Branch);
    }

    #[test]
    fn test_classify_alu() {
        assert_eq!(classify_line("    addw t0, t1, t2"), LineKind::Alu);
        assert_eq!(classify_line("    slli t0, t1, 2"), LineKind::Alu);
        assert_eq!(classify_line("    addi t0, t0, 4"), LineKind::Alu);
    }

    #[test]
    fn test_adjacent_store_load_elimination() {
        // The reload is forwarded from the store.  Nothing reads the slot
        // afterwards, so the global dead-store census may drop the store too.
        let input = "    sd t0, -24(s0)\n    ld t0, -24(s0)\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ld t0, -24(s0)"));
        // Once the slot's address reaches a callee, the store must stay.
        let input =
            "    sd t0, -24(s0)\n    ld t0, -24(s0)\n    addi a0, s0, -24\n    call g\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("sd t0, -24(s0)"));
        assert!(!result.contains("ld t0, -24(s0)"));
    }

    #[test]
    fn test_redundant_jump_elimination() {
        let input = "    jump .LBB3, t6\n.LBB3:\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("jump .LBB3"));
        assert!(result.contains(".LBB3:"));
    }

    #[test]
    fn test_self_move_elimination() {
        let input = "    mv t0, t0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("mv t0, t0"));
    }

    #[test]
    fn test_li_mv_chain() {
        let input = "    li t0, 42\n    mv s1, t0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("li s1, 42"));
    }

    #[test]
    fn test_gsf_same_reg_elimination() {
        // Store t0 then load t0 from same slot (non-adjacent) — load is dead.
        // The slot's address escapes to a callee afterwards, so the store
        // itself must survive the dead-store census.
        let input = "\
    sd t0, -24(s0)\n\
    addi t1, t1, 4\n\
    ld t0, -24(s0)\n\
    addi a0, s0, -24\n\
    call g\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("sd t0, -24(s0)"));
        assert!(!result.contains("ld t0, -24(s0)"));
    }

    #[test]
    fn test_gsf_different_reg_forwarding() {
        // Store t0 then load a0 from same slot — replace load with mv.  The
        // destination is the return register: a temporary loaded and never
        // read before `ret` would be deleted outright as a dead move.
        let input = "\
    sd t0, -24(s0)\n\
    addi t2, t2, 4\n\
    ld a0, -24(s0)\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ld a0, -24(s0)"));
        assert!(result.contains("mv a0, t0"));
    }

    #[test]
    fn test_gsf_invalidation_on_reg_overwrite() {
        // After t0 is overwritten, the mapping slot → t0 is stale
        // Both the overwritten t0 and the reloaded value stay live.
        let input = "\
    sd t0, -24(s0)\n\
    li t0, 42\n\
    ld t1, -24(s0)\n\
    add a0, t1, t0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // The load should NOT be forwarded since t0 was overwritten
        assert!(result.contains("ld t1, -24(s0)"), "{result}");
        assert!(!result.contains("mv t1, t0"), "{result}");
    }

    #[test]
    fn test_gsf_invalidation_at_label() {
        // Mappings are invalidated at labels
        let input = "\
    sd t0, -24(s0)\n\
    jump .LBB1, t6\n\
.LBB1:\n\
    ld t0, -24(s0)\n\
    mv a0, t0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // Label invalidates mappings, so load should remain
        assert!(
            result.contains("ld t0, -24(s0)") || result.contains("ld a0, -24(s0)"),
            "{result}"
        );
    }

    #[test]
    fn test_global_dse() {
        // Store to a slot that is never loaded — dead store
        let input = "\
    sd t0, -24(s0)\n\
    li t0, 0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("sd t0, -24(s0)"));
    }

    #[test]
    fn test_global_dse_with_addr_taken() {
        // When frame pointer address is taken, no stores eliminated
        let input = "\
    sd t0, -24(s0)\n\
    addi t1, s0, -24\n\
    sd t1, 0(a1)\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // Store must be preserved
        assert!(result.contains("sd t0, -24(s0)"));
    }

    // ── Dying-accumulator producer retarget ─────────────────────────────────
    //
    // These drive the pass directly instead of the whole peephole pipeline:
    // the guards are what this pass owns, and no other pass may decide whether
    // they hold.

    /// Run only the retarget pass over `input` and render the surviving lines.
    fn run_retarget(input: &str) -> String {
        let mut lines: Vec<String> = input.lines().map(str::to_string).collect();
        let n = lines.len();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let _ = retarget_dying_accumulator_moves(&mut lines, &mut kinds, n);
        lines
            .iter()
            .zip(kinds.iter())
            .filter(|(_, k)| **k != LineKind::Nop)
            .map(|(l, _)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn retarget_alu_producer_writes_the_home() {
        let out = run_retarget("f:\n    addw t0, t1, a1\n    mv a0, t0\n    ret\n");
        assert!(out.contains("addw a0, t1, a1"), "{out}");
        assert!(!out.contains("mv a0, t0"), "{out}");
    }

    #[test]
    fn retarget_immediate_producer_writes_the_home() {
        let out = run_retarget("f:\n    li t0, 42\n    mv a0, t0\n    ret\n");
        assert!(out.contains("li a0, 42"), "{out}");
        assert!(!out.contains("li t0, 42"), "{out}");
    }

    #[test]
    fn retarget_frame_load_producer_writes_the_home() {
        let out = run_retarget("f:\n    lw t0, 8(s0)\n    mv a1, t0\n    ret\n");
        assert!(out.contains("lw a1, 8(s0)"), "{out}");
    }

    /// A plain load staged through a scratch loads straight into the copy's
    /// destination (`load_zext8`: `lbu t0, 0(t0); mv a0, t0`), even when the
    /// scratch is the load's own base; a reserved load is no plain load.
    #[test]
    fn retarget_plain_load_producer_writes_the_home() {
        let out =
            run_retarget("f:\n    add t0, a0, a1\n    lbu t0, 0(t0)\n    mv a0, t0\n    ret\n");
        assert!(out.contains("lbu a0, 0(t0)"), "{out}");
        assert!(!out.contains("mv a0, t0"), "{out}");
        for m in ["lb", "lh", "lhu", "lw", "lwu", "ld"] {
            let out = run_retarget(&format!("f:\n    {m} t1, 8(a2)\n    mv a1, t1\n    ret\n"));
            assert!(out.contains(&format!("{m} a1, 8(a2)")), "{out}");
        }
        let out = run_retarget("f:\n    lr.w t0, (a0)\n    mv a1, t0\n    ret\n");
        assert!(out.contains("mv a1, t0"), "{out}");
        // The scratch is read again: the load keeps it.
        let out =
            run_retarget("f:\n    lw t0, 0(a0)\n    mv a1, t0\n    add a0, t0, a1\n    ret\n");
        assert!(out.contains("mv a1, t0"), "{out}");
    }

    #[test]
    fn retarget_self_move_drops_both_lines() {
        let out = run_retarget("f:\n    sext.w a0, a0\n    mv t0, a0\n    mv a0, t0\n    ret\n");
        assert!(!out.contains("mv a0, t0"), "{out}");
        assert!(!out.contains("mv t0, a0"), "{out}");
        assert!(out.contains("sext.w a0, a0"), "{out}");
    }

    #[test]
    fn retarget_refuses_a_live_scratch() {
        // `t0` is read again, so the copy is the only definition of `a0`.
        // The reader has to name `t0` in an operand slot: `addw t0, t6, t6`
        // only overwrites it and therefore leaves it dead.
        let out =
            run_retarget("f:\n    addw t0, t1, a1\n    mv a0, t0\n    addw t6, t0, t0\n    ret\n");
        assert!(out.contains("mv a0, t0"), "{out}");
        assert!(out.contains("addw t0, t1, a1"), "{out}");
    }

    /// Past a label the function's liveness decides: the scratch read
    /// after the join keeps the move, a scratch every path rewrites first
    /// does not.
    #[test]
    fn retarget_consults_the_liveness_across_a_label() {
        let out = run_retarget(
            "f:\n    addw t0, t1, a1\n    mv a0, t0\n.LBB9:\n    addw t0, t0, t6\n    ret\n",
        );
        assert!(out.contains("mv a0, t0"), "{out}");
        let out = run_retarget(
            "f:\n    addw t0, t1, a1\n    mv a0, t0\n    beqz a2, .LBB9\n    addw t0, t6, t6\n.LBB9:\n    addw t0, t0, t6\n    ret\n",
        );
        assert!(out.contains("mv a0, t0"), "{out}");
        let out = run_retarget(
            "f:\n    addw t0, t1, a1\n    mv a0, t0\n.LBB9:\n    addw t0, t6, t6\n    ret\n",
        );
        assert_eq!(
            out,
            "f:\n    addw a0, t1, a1\n.LBB9:\n    addw t0, t6, t6\n    ret"
        );
    }

    #[test]
    fn retarget_refuses_when_the_scratch_is_read_again() {
        // `sext.w t0, t0` reads t0: the retarget would leave the pre-shift
        // value in t0 and sign-extend that instead of the shift result
        // (check_cross_alu_abs_runtime caught exactly this).
        let out = run_retarget(
            "f:\n    srli t0, t0, 3\n    mv a0, t0\n    sext.w t0, t0\n    mv a1, t0\n    ret\n",
        );
        assert!(out.contains("mv a0, t0"), "{out}");
        assert!(out.contains("srli t0, t0, 3"), "{out}");
    }

    #[test]
    fn retarget_refuses_through_a_store() {
        // A store reads its source; `t0` is live at the next line.
        let out =
            run_retarget("f:\n    addw t0, t1, a1\n    mv a0, t0\n    sw t0, 8(s0)\n    ret\n");
        assert!(out.contains("mv a0, t0"), "{out}");
    }

    #[test]
    fn retarget_leaves_value_registers_alone() {
        // `s1` is a value home, not an accumulator: nothing to retarget.
        let out = run_retarget("f:\n    lw s1, 8(s0)\n    mv a0, s1\n    ret\n");
        assert!(out.contains("mv a0, s1"), "{out}");
        assert!(out.contains("lw s1, 8(s0)"), "{out}");
    }

    #[test]
    fn retarget_refuses_a_frame_pointer_home() {
        let out = run_retarget("f:\n    addw t0, t1, a1\n    mv sp, t0\n    ret\n");
        assert!(out.contains("mv sp, t0"), "{out}");
    }

    #[test]
    fn retarget_tracks_a_dead_scratch_past_the_return() {
        // `ret` ends the search successfully: no scratch is a return register.
        let out = run_retarget(
            "f:\n    addw t0, t1, a1\n    mv a0, t0\n    ld ra, 8(s0)\n    addi sp, sp, 16\n    ret\n",
        );
        assert!(out.contains("addw a0, t1, a1"), "{out}");
        assert!(!out.contains("mv a0, t0"), "{out}");
    }

    #[test]
    fn test_copy_prop_word_boundary() {
        assert_eq!(replace_whole_word("t10, t1", "t1", "s1"), "t10, s1");
    }

    #[test]
    fn test_copy_prop_no_false_match() {
        assert_eq!(replace_whole_word("s10", "s1", "s5"), "s10");
    }

    #[test]
    fn test_copy_prop_no_symbol_corruption() {
        // Register name inside symbol names must not be replaced
        assert_eq!(replace_whole_word(" main.s1.0", "s1", "t0"), " main.s1.0");
        assert_eq!(replace_whole_word(" _s1_var", "s1", "t0"), " _s1_var");
    }

    #[test]
    fn test_lla_not_corrupted_by_copy_prop() {
        // Regression test: copy propagation must not corrupt symbol names in lla
        let input = "\
    mv t0, s1\n\
    lla t0, main.s1.0\n\
    mv a0, t0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("main.s1.0"),
            "Symbol name corrupted, got:\n{}",
            result
        );
        assert!(
            !result.contains("main.t0.0"),
            "Symbol name corrupted to main.t0.0"
        );
    }

    #[test]
    fn test_copy_prop_basic() {
        // mv t0, s1 ; addw t2, t0, t1 → addw t2, s1, t1
        let input = "\
    mv t0, s1\n\
    addw a0, t0, t1\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("addw a0, s1, t1"), "{result}");
    }

    #[test]
    fn test_dead_reg_move() {
        // mv t1, t0 ; li t1, 42 → (t1 overwritten, first mv is dead)
        let input = "\
    mv t1, t0\n\
    li t1, 42\n\
    mv a0, t1\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mv t1, t0"),
            "Expected dead move eliminated, got:\n{}",
            result
        );
        // The constant still reaches the return register.
        assert!(
            result.contains("li a0, 42") || result.contains("li t1, 42"),
            "{result}"
        );
    }

    #[test]
    fn test_full_bb_copy_prop() {
        // mv t0, s1 ; mv t1, t0 ; add a0, t1, t0
        // Copy propagation is single-step lookahead:
        // 1. mv t0, s1 → propagates into next: mv t1, t0 → mv t1, s1
        // 2. mv t1, s1 → propagates into next: add a0, t1, t0 → add a0, s1, t0
        // after which `mv t1, s1` has no reader before `ret` and is deleted
        // (t0 is still read by the add, so its copy stays).
        let input = "\
    mv t0, s1\n\
    mv t1, t0\n\
    add a0, t1, t0\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("add a0, s1,"),
            "Expected t1 = s1 propagated through the chain into the add, got:\n{}",
            result
        );
        assert!(
            !result.contains("mv t1,"),
            "Expected the propagated copy to be dead at ret, got:\n{}",
            result
        );
    }
}

#[cfg(test)]
mod jump_near_tests {
    use super::*;

    #[test]
    fn near_jump_rewrite_same_function() {
        let input = "f:\n    li t0, 5\n    jump .L1, t6\n    mv a0, t0\n.L1:\n    ret\n";
        let out = peephole_optimize(input.to_string());
        assert!(
            out.contains("    j .L1") && !out.contains("jump .L1, t6"),
            "expected near jump:\n{}",
            out
        );
    }

    #[test]
    fn far_jump_kept_when_target_is_external() {
        // Tail-call / cross-function target: no local label — must stay `jump`.
        let input = "f:\n    jump some_function, t6\n    ret\n";
        let out = peephole_optimize(input.to_string());
        assert!(out.contains("jump some_function, t6"), "\n{}", out);
    }

    #[test]
    fn word_zext_pair_removed_after_lwu_and_copy() {
        let input = concat!(
            "f:\n",
            "    lwu t0, 0(t2)\n",
            "    mv a0, t0\n",
            "    slli a0, a0, 32\n",
            "    srli a0, a0, 32\n",
            "    ret\n",
        );
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[3], LineKind::Nop);
        assert_eq!(kinds[4], LineKind::Nop);
    }

    #[test]
    fn word_zext_cross_register_pair_becomes_move() {
        let input = "f:\n    lhu t0, 0(t2)\n    slli a0, t0, 32\n    srli a0, a0, 32\n    add a1, a0, t3\n    ret\n";
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(lines[2].trim(), "mv a0, t0");
        assert_eq!(kinds[3], LineKind::Nop);
    }

    #[test]
    fn narrow_logic_and_shift_producers_feed_word_zext() {
        let input = concat!(
            "f:\n",
            "    lbu t0, 0(t2)\n",
            "    li t1, 255\n",
            "    xor t0, t0, t1\n",
            "    srli t0, t0, 1\n",
            "    slli t0, t0, 32\n",
            "    srli t0, t0, 32\n",
            "    ret\n",
        );
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[5], LineKind::Nop);
        assert_eq!(kinds[6], LineKind::Nop);
    }

    #[test]
    fn narrow_producer_lattice_covers_comparisons_masks_and_logic() {
        let cases = [
            "    seqz t0, t1\n",
            "    sltu t0, t1, t2\n",
            "    andi t0, t1, 1023\n",
            "    lhu t0, 0(t2)\n    ld t1, 8(t2)\n    and t0, t0, t1\n",
            "    lbu t0, 0(t2)\n    li t1, 3\n    or t0, t0, t1\n",
            "    lbu t0, 0(t2)\n    slli t0, t0, 4\n",
        ];
        for producer in cases {
            let input =
                format!("f:\n{producer}    slli t0, t0, 32\n    srli t0, t0, 32\n    ret\n");
            let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
            let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
            let n = lines.len();
            assert!(
                eliminate_redundant_zext_byte(&mut lines, &mut kinds, n),
                "producer did not establish a narrow value:\n{input}"
            );
            assert!(
                !lines
                    .iter()
                    .zip(&kinds)
                    .any(|(line, kind)| *kind != LineKind::Nop && line.trim() == "slli t0, t0, 32"),
                "zext pair survived:\n{input}"
            );
        }
    }

    #[test]
    fn wide_or_operand_prevents_false_narrow_proof() {
        let input = concat!(
            "f:\n",
            "    lbu t0, 0(t2)\n",
            "    ld t1, 8(t2)\n",
            "    or t0, t0, t1\n",
            "    slli t0, t0, 32\n",
            "    srli t0, t0, 32\n",
            "    ret\n",
        );
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[4], LineKind::Alu);
        assert_eq!(kinds[5], LineKind::Alu);
    }

    #[test]
    fn word_zext_pair_kept_without_upper_bit_proof() {
        let input = "f:\n    ld t0, 0(t2)\n    slli t0, t0, 32\n    srli t0, t0, 32\n    ret\n";
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[2], LineKind::Alu);
        assert_eq!(kinds[3], LineKind::Alu);
    }

    /// Run `eliminate_redundant_sign_extensions` on `input`.
    fn sign_ext(input: &str) -> (bool, Vec<String>, Vec<LineKind>) {
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        let changed = eliminate_redundant_sign_extensions(&mut lines, &mut kinds, n);
        (changed, lines, kinds)
    }

    /// `lh`/`lb` already sign-extend: the compare staging's shift pair is a
    /// move (or nothing, in place). One bit too wide keeps it.
    #[test]
    fn narrow_signed_loads_need_no_shift_pair() {
        let (changed, lines, kinds) = sign_ext(
            "f:\n    lh t0, 0(a0)\n    slli t1, t0, 48\n    srai t1, t1, 48\n    slti a0, t1, 8\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[2].trim(), "mv t1, t0");
        assert_eq!(kinds[3], LineKind::Nop);
        let (changed, _, kinds) =
            sign_ext("f:\n    lb t0, 0(a0)\n    slli t0, t0, 56\n    srai t0, t0, 56\n    ret\n");
        assert!(changed);
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        // An `lbu` is one bit too wide for the pair to be redundant, but the
        // pair extends the load: it loads signed instead.
        let (changed, lines, kinds) =
            sign_ext("f:\n    lbu t0, 0(a0)\n    slli t0, t0, 56\n    srai t0, t0, 56\n    ret\n");
        assert!(changed);
        assert_eq!(lines[1].trim(), "lb t0, 0(a0)");
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        for wider in [
            "f:\n    lh t0, 0(a0)\n    slli t0, t0, 56\n    srai t0, t0, 56\n    ret\n",
            "f:\n    lh t0, 0(a0)\n.L1:\n    slli t1, t0, 48\n    srai t1, t1, 48\n    ret\n",
            "f:\n    lh t0, 0(a0)\n    call g\n    slli t1, t0, 48\n    srai t1, t1, 48\n    ret\n",
        ] {
            assert!(!sign_ext(wider).0, "{wider}");
        }
    }

    /// A non-negative value under 2^31 is its own sign-extended word.
    #[test]
    fn a_narrow_nonnegative_value_needs_no_sext_w() {
        let (changed, lines, _) =
            sign_ext("f:\n    andi t0, t0, 0xff\n    sext.w t1, t0\n    ret\n");
        assert!(changed);
        assert_eq!(lines[2].trim(), "mv t1, t0");
        for kept in [
            // bit 31 may be set after a 32-bit shift down.
            "f:\n    srli t0, t0, 32\n    sext.w t1, t0\n    ret\n",
            "f:\n    andi t0, t0, -16\n    sext.w t1, t0\n    ret\n",
            "f:\n    ld t0, 0(a0)\n    sext.w t1, t0\n    ret\n",
        ] {
            assert!(!sign_ext(kept).0, "{kept}");
        }
    }

    /// `(unsigned)(c - '0') < 10`: a sign-extended word compares against a
    /// small constant exactly like its zero-extension (Clang 23.1.5:
    /// `addiw; sltiu`), and so does an equality zero test.  The zero-extended
    /// value is non-negative, so a signed `slti` of it is the same compare.
    #[test]
    fn a_sign_extended_word_needs_no_zext_for_a_small_unsigned_compare() {
        let (changed, lines, kinds) = sign_ext(
            "f:\n    addiw t0, a0, -48\n    slli t0, t0, 32\n    srli t0, t0, 32\n    sltiu a0, t0, 10\n    ret\n",
        );
        assert!(changed);
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        assert_eq!(lines[4].trim(), "sltiu a0, t0, 10");
        let (changed, lines, kinds) = sign_ext(
            "f:\n    addiw t1, a0, -5\n    slli t2, t1, 32\n    srli t2, t2, 32\n    xori t0, t2, 7\n    seqz t0, t0\n    mv a0, t0\n    ret\n",
        );
        assert!(changed);
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        assert_eq!(lines[4].trim(), "xori t0, t1, 7");
        let (changed, lines, _) = sign_ext(
            "f:\n    addiw t0, a0, -48\n    slli t0, t0, 32\n    srli t0, t0, 32\n    slti a0, t0, 10\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[4].trim(), "sltiu a0, t0, 10");
        for kept in [
            // Not a sign-extended word.
            "f:\n    ld t0, 0(a1)\n    slli t0, t0, 32\n    srli t0, t0, 32\n    sltiu a0, t0, 10\n    ret\n",
            // The zero-extended copy is read again.
            "f:\n    addiw t0, a0, -48\n    slli t1, t0, 32\n    srli t1, t1, 32\n    sltiu a0, t1, 10\n    mv a1, t1\n    ret\n",
            // The xori result itself is observed, not just tested for zero.
            "f:\n    addiw t1, a0, -5\n    slli t1, t1, 32\n    srli t1, t1, 32\n    xori t0, t1, 7\n    mv a0, t0\n    ret\n",
            // A negative bound compares against the top of the range.
            "f:\n    addiw t0, a0, -48\n    slli t0, t0, 32\n    srli t0, t0, 32\n    sltiu a0, t0, -1\n    ret\n",
        ] {
            assert!(!sign_ext(kept).0, "{kept}");
        }
    }

    /// `(unsigned char)(p[i] - 'a') <= 25`: the byte mask cannot change the
    /// compare of a value in [-97, 158], so the compare reads it unmasked
    /// and unsigned (Clang 23.1.5: `addi; sltiu`), the mask disappears, and
    /// the same holds for an equality test.  The mask stays where some value
    /// of the range would answer differently, or where it is read again.
    #[test]
    fn a_mask_that_cannot_change_the_comparison_is_dropped() {
        let (changed, lines, kinds) = sign_ext(concat!(
            "f:\n",
            "    lbu t0, 0(t0)\n",
            "    addiw t0, t0, -97\n",
            "    andi t0, t0, 0xff\n",
            "    slti a7, t0, 26\n",
            "    addw a3, a3, a7\n",
            "    ret\n",
        ));
        assert!(changed);
        assert_eq!(kinds[3], LineKind::Nop);
        assert_eq!(lines[4].trim(), "sltiu a7, t0, 26");
        let (changed, lines, kinds) = sign_ext(concat!(
            "f:\n",
            "    lbu t0, 0(a0)\n",
            "    addiw t0, t0, -97\n",
            "    andi t1, t0, 255\n",
            "    xori t1, t1, 3\n",
            "    seqz a0, t1\n",
            "    ret\n",
        ));
        assert!(changed);
        assert_eq!(kinds[3], LineKind::Nop);
        assert_eq!(lines[4].trim(), "xori t1, t0, 3");
        for kept in [
            // [-97, 65438]: 65439 - 97 masks to 2 and compares below 26.
            "f:\n    lhu t0, 0(a0)\n    addiw t0, t0, -97\n    andi t0, t0, 0xff\n    slti a0, t0, 26\n    ret\n",
            // [-240, 15]: -235 masks to 21 but is no unsigned value below 26.
            "f:\n    lbu t0, 0(a0)\n    addiw t0, t0, -240\n    andi t0, t0, 0xff\n    sltiu a0, t0, 26\n    ret\n",
            // [200, 455]: 259 masks to 3 but is not 3.
            "f:\n    lbu t0, 0(a0)\n    addi t0, t0, 200\n    andi t0, t0, 0xff\n    xori t0, t0, 3\n    seqz a0, t0\n    ret\n",
            // The masked value is read again.
            "f:\n    lbu t0, 0(a0)\n    addiw t0, t0, -97\n    andi t1, t0, 0xff\n    sltiu a0, t1, 26\n    mv a1, t1\n    ret\n",
            // Not a low-bit mask.
            "f:\n    lbu t0, 0(a0)\n    addiw t0, t0, -97\n    andi t0, t0, 0xfe\n    sltiu a0, t0, 26\n    ret\n",
            // A join forgets the range.
            "f:\n    lbu t0, 0(a0)\n    addiw t0, t0, -97\n.L1:\n    andi t0, t0, 0xff\n    sltiu a0, t0, 26\n    ret\n",
        ] {
            assert!(!sign_ext(kept).0, "{kept}");
        }
    }

    /// One decoded line of the straight-line subset `run_rv` executes.
    #[derive(Clone, Copy)]
    struct RvOp {
        mnem: &'static str,
        rd: usize,
        rs: usize,
        imm: i64,
    }

    /// Decode the live lines of `lines`; every load reads the run's input.
    fn decode_rv(lines: &[String], kinds: &[LineKind]) -> Vec<RvOp> {
        const MNEMONICS: [&str; 20] = [
            "lbu", "lb", "lhu", "lh", "lw", "lwu", "ld", "mv", "addiw", "addi", "andi", "xori",
            "slli", "srli", "srai", "sltiu", "slti", "seqz", "neg", "negw",
        ];
        let mut ops = Vec::new();
        for (line, kind) in lines.iter().zip(kinds) {
            if matches!(kind, LineKind::Nop | LineKind::Label | LineKind::Ret) {
                continue;
            }
            let t = line.trim();
            let (m, rest) = t.split_once(' ').unwrap_or((t, ""));
            let o: Vec<&str> = rest.split(',').map(str::trim).collect();
            let mnem = MNEMONICS
                .iter()
                .copied()
                .find(|&x| x == m)
                .unwrap_or_else(|| panic!("unmodelled `{t}`"));
            let rs = o.get(1).map_or(REG_NONE, |r| parse_reg(r));
            ops.push(RvOp {
                mnem,
                rd: parse_reg(o[0]) as usize,
                rs: rs as usize,
                imm: o.get(2).and_then(|v| parse_asm_i64(v)).unwrap_or(0),
            });
        }
        ops
    }

    /// Execute `ops` with every load producing `input`; the final `a0`.
    fn run_rv(ops: &[RvOp], input: i64) -> i64 {
        let mut x = [0i64; NUM_REGS + 1];
        for op in ops {
            let v = x[op.rs.min(NUM_REGS)];
            x[op.rd.min(NUM_REGS)] = match op.mnem {
                // `input` is the memory's content, read at the load's width
                // and extended by its signedness.
                "lb" => i64::from(input as i8),
                "lbu" => i64::from(input as u8),
                "lh" => i64::from(input as i16),
                "lhu" => i64::from(input as u16),
                "lw" => i64::from(input as i32),
                "lwu" => i64::from(input as u32),
                "ld" => input,
                "neg" => v.wrapping_neg(),
                "negw" => i64::from((v as i32).wrapping_neg()),
                "mv" => v,
                "addiw" => i64::from(v.wrapping_add(op.imm) as i32),
                "addi" => v.wrapping_add(op.imm),
                "andi" => v & op.imm,
                "xori" => v ^ op.imm,
                "slli" => v << op.imm,
                "srli" => ((v as u64) >> op.imm) as i64,
                "srai" => v >> op.imm,
                "sltiu" => i64::from((v as u64) < (op.imm as u64)),
                "slti" => i64::from(v < op.imm),
                _ => i64::from(v == 0), // seqz
            };
        }
        x[REG_A0 as usize]
    }

    /// Every fold of the range pass, against an interpreter: each load
    /// type, an offset either side of every bound, each extension spelling
    /// (in place and into a second register) and each compare form, over
    /// every value the load can produce.  The rewritten function must return
    /// what the original returns for all of them, and the folds must fire
    /// (the exact count pins that the oracle is not vacuous).
    #[test]
    fn range_folds_preserve_every_comparison_exhaustively() {
        let loads: [(&str, Vec<i64>); 4] = [
            ("lbu t0, 0(a0)", (0..=255).collect()),
            ("lb t0, 0(a0)", (-128..=127).collect()),
            (
                "lw t0, 0(a0)\n    andi t0, t0, 1023",
                (-1100..=1100).collect(),
            ),
            (
                "lhu t0, 0(a0)",
                (0..=65535)
                    .step_by(59)
                    .chain([255, 256, 257, 65534, 65535])
                    .collect(),
            ),
        ];
        let extensions = [
            ("andi t0, t0, 0xff", "t0"),
            ("andi t1, t0, 255", "t1"),
            ("andi t0, t0, 2047", "t0"),
            ("slli t1, t0, 56\n    srli t1, t1, 56", "t1"),
            ("slli t0, t0, 48\n    srli t0, t0, 48", "t0"),
            ("slli t0, t0, 32\n    srli t0, t0, 32", "t0"),
        ];
        let mut folds = 0;
        for (load, inputs) in &loads {
            for off in [-2048i64, -300, -231, -230, -129, -97, -1, 0, 1, 200, 2047] {
                for (ext, t) in extensions {
                    for k in [0i64, 1, 3, 25, 26, 255, 256, 2047] {
                        for cmp in ["sltiu", "slti", "xori"] {
                            let tail = if cmp == "xori" {
                                format!("    xori a1, {t}, {k}\n    seqz a1, a1\n")
                            } else {
                                format!("    {cmp} a1, {t}, {k}\n")
                            };
                            let asm = format!(
                                "f:\n    {load}\n    addiw t0, t0, {off}\n    {ext}\n{tail}    mv a0, a1\n    ret\n"
                            );
                            let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
                            let kinds: Vec<LineKind> =
                                lines.iter().map(|l| classify_line(l)).collect();
                            let before = decode_rv(&lines, &kinds);
                            let (changed, new_lines, new_kinds) = sign_ext(&asm);
                            if !changed {
                                continue;
                            }
                            folds += 1;
                            let after = decode_rv(&new_lines, &new_kinds);
                            for &v in inputs {
                                assert_eq!(
                                    run_rv(&before, v),
                                    run_rv(&after, v),
                                    "input {v}:\n{asm}=>\n{}",
                                    new_lines.join("\n")
                                );
                            }
                        }
                    }
                }
            }
        }
        // 3131 of the 6336 functions fold: the count pins the coverage, so
        // a pass that stops (or starts) firing on any shape shows up here.
        assert_eq!(folds, 3131, "folds fired");
    }

    /// What is live after a branch is what its successors read: the taken
    /// edge's reads as well as the fall-through's after a conditional
    /// branch, and the target's alone after `j` (whose next line is no
    /// successor) -- for the general and the FP registers alike.
    #[test]
    fn liveness_after_a_branch_is_every_successor_live_in() {
        let text = |asm: &str| {
            let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
            let kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
            (lines, kinds)
        };
        let (lines, kinds) = text(
            "f:\n    beqz a0, .L1\n    mv a0, t2\n    j .L2\n.L1:\n    mv a0, t1\n    ret\n.L2:\n    add a0, a0, t3\n    ret\n",
        );
        let live = RvLiveness::build(&lines, &kinds, 0, lines.len()).expect("resolvable");
        let [t1, t2, t3, t4] = ["t1", "t2", "t3", "t4"].map(parse_reg);
        // After `beqz`: t1 on the taken edge, t2 and t3 on the fall-through.
        for reg in [t1, t2, t3] {
            assert_ne!(live.live_after(1) & rv_bit(reg), 0, "{}", reg_name(reg));
        }
        assert_eq!(live.live_after(1) & rv_bit(t4), 0);
        // After `j .L2`: t3 only -- not t1, which `.L1` (the next line) reads.
        assert_ne!(live.live_after(3) & rv_bit(t3), 0);
        assert_eq!(live.live_after(3) & (rv_bit(t1) | rv_bit(t2)), 0);
        let mut lazy = RvLazyLiveness::new(&lines, &kinds, lines.len());
        assert!(!lazy.dead_after(&lines, &kinds, 1, t1));
        assert!(lazy.dead_after(&lines, &kinds, 3, t1));
        // Without a resolvable CFG (`jr`), the block-local proof sees no
        // taken edge, so it answers nothing for a branch.
        let (lines, kinds) =
            text("f:\n    bltz a0, .L1\n    li t1, 0\n    jr a1\n.L1:\n    mv a0, t1\n    ret\n");
        let mut lazy = RvLazyLiveness::new(&lines, &kinds, lines.len());
        assert!(!lazy.dead_after(&lines, &kinds, 1, t1));
        let (lines, kinds) = text(
            "f:\n    beqz a0, .L1\n    fmv.d fa0, ft2\n    j .L2\n.L1:\n    fmv.d fa0, ft1\n    ret\n.L2:\n    fadd.d fa0, fa0, ft3\n    ret\n",
        );
        let live = RvFpLiveness::build(&lines, &kinds, 0, lines.len()).expect("resolvable");
        let [ft1, ft2, ft3, ft4] = ["ft1", "ft2", "ft3", "ft4"].map(|r| rv_fp_reg(r).expect("fp"));
        for reg in [ft1, ft2, ft3] {
            assert_ne!(live.live_after(1) & rv_fp_bit(reg), 0, "ft{reg}");
        }
        assert_eq!(live.live_after(1) & rv_fp_bit(ft4), 0);
        assert_ne!(live.live_after(3) & rv_fp_bit(ft3), 0);
        assert_eq!(live.live_after(3) & (rv_fp_bit(ft1) | rv_fp_bit(ft2)), 0);
    }

    /// `(signed char)x >= 0` staged as `slli; srai; slti 0`: the arithmetic
    /// shift does not move the sign bit, so a test that reads only the sign
    /// needs no `srai`, whether it is a value (`slti`/`slt`/`srli 63`) or a
    /// branch.  Anything else reading the extended value keeps it.
    #[test]
    fn a_sign_test_needs_no_arithmetic_shift() {
        for reader in [
            "slti a0, t1, 0",
            "slt a0, t1, zero",
            "srli a0, t1, 63",
            "slti t1, t1, 0\n    mv a0, t1",
        ] {
            let asm =
                format!("f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    {reader}\n    ret\n");
            let (changed, lines, kinds) = sign_ext(&asm);
            assert!(changed, "{asm}");
            assert_eq!(lines[1].trim(), "slli t1, a0, 56", "{asm}");
            assert_eq!(kinds[2], LineKind::Nop, "{asm}");
        }
        for branch in [
            "blt t1, zero, .L1",
            "bge t1, zero, .L1",
            "bltz t1, .L1",
            "bgez t1, .L1",
            "bgt zero, t1, .L1",
            "ble zero, t1, .L1",
        ] {
            let asm = format!(
                "f:\n    slli t1, a0, 48\n    srai t1, t1, 48\n    {branch}\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n"
            );
            let (changed, _, kinds) = sign_ext(&asm);
            assert!(changed, "{asm}");
            assert_eq!(kinds[2], LineKind::Nop, "{asm}");
        }
        for kept in [
            // The extended value is read again after the test.
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    slti a1, t1, 0\n    add a0, a1, t1\n    ret\n",
            // ... or on the taken edge of the branch.
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    bltz t1, .L1\n    li a0, 0\n    ret\n.L1:\n    mv a0, t1\n    ret\n",
            // Not sign tests: `t < 1`, bit 62, a zero test.
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    slti a0, t1, 1\n    ret\n",
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    srli a0, t1, 62\n    ret\n",
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    beqz t1, .L1\n.L1:\n    ret\n",
            // Not an extension pair, or not adjacent.
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 48\n    slti a0, t1, 0\n    ret\n",
            "f:\n    slli t1, a0, 56\n.L1:\n    srai t1, t1, 56\n    slti a0, t1, 0\n    ret\n",
            "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n.L1:\n    slti a0, t1, 0\n    ret\n",
        ] {
            assert!(!sign_ext(kept).0, "{kept}");
        }
    }

    /// The sign of a zero-extended byte's sign extension is the byte's top
    /// bit: one `srli 7` of the byte.  A source wider than the extension
    /// only loses the `srai`, and a narrow load right before the pair takes
    /// the extension instead (`lb`, which the sign test then reads).
    #[test]
    fn a_sign_test_of_a_zero_extended_value_is_its_top_bit() {
        let (changed, lines, kinds) = sign_ext(
            "f:\n    andi t0, a2, 255\n    slli t1, t0, 56\n    srai t1, t1, 56\n    slti t1, t1, 0\n    addw a0, a1, t1\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[2].trim(), "srli t1, t0, 7");
        assert_eq!((kinds[3], kinds[4]), (LineKind::Nop, LineKind::Nop));
        let (changed, lines, _) = sign_ext(
            "f:\n    srli t0, a2, 48\n    slli t0, t0, 48\n    srai t0, t0, 48\n    srli a0, t0, 63\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[2].trim(), "srli a0, t0, 15");
        let (changed, lines, kinds) = sign_ext(
            "f:\n    lhu t0, 0(a0)\n    slli t1, t0, 56\n    srai t1, t1, 56\n    slti a0, t1, 0\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[2].trim(), "slli t1, t0, 56");
        assert_eq!(
            (kinds[3], lines[4].trim()),
            (LineKind::Nop, "slti a0, t1, 0")
        );
        let (changed, lines, kinds) = sign_ext(
            "f:\n    lbu t0, 0(a0)\n    slli t1, t0, 56\n    srai t1, t1, 56\n    slti t1, t1, 0\n    addw a0, a1, t1\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[1].trim(), "lb t1, 0(a0)");
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        assert_eq!(lines[4].trim(), "slti t1, t1, 0");
    }

    /// `x >= 0` on a byte is `andi 128; seqz` (Clang 23.1.5); a short's bit
    /// 15 does not fit `andi`, so only the `srai` goes.
    #[test]
    fn a_negated_byte_sign_test_is_an_andi_and_seqz() {
        let (changed, lines, kinds) = sign_ext(
            "f:\n    slli t0, a0, 56\n    srai t0, t0, 56\n    slti t0, t0, 0\n    xori a0, t0, 1\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[1].trim(), "andi t0, a0, 128");
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        assert_eq!(lines[4].trim(), "seqz a0, t0");
        let (changed, lines, kinds) = sign_ext(
            "f:\n    slli t0, a0, 48\n    srai t0, t0, 48\n    slti t0, t0, 0\n    xori a0, t0, 1\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[1].trim(), "slli t0, a0, 48");
        assert_eq!(kinds[2], LineKind::Nop);
        assert_eq!(
            (lines[3].trim(), lines[4].trim()),
            ("slti t0, t0, 0", "xori a0, t0, 1")
        );
        // The test's result read after the negation keeps the `slti`.
        let kept = "f:\n    slli t1, a0, 56\n    srai t1, t1, 56\n    slti t0, t1, 0\n    xori a1, t0, 1\n    add a0, a1, t0\n    ret\n";
        let (_, lines, _) = sign_ext(kept);
        assert!(
            !lines.iter().any(|l| l.trim().starts_with("andi")),
            "{lines:?}"
        );
    }

    /// `signed char`/`short` arguments arrive sign-extended from bit 7/15
    /// (psABI), so re-extending one is a move; a byte view of a short still
    /// needs its pair, minus the `srai` when only the sign is read.
    #[test]
    fn signed_narrow_arguments_arrive_extended() {
        let (changed, lines, kinds) = sign_ext(
            "f:\n    # lccc-arg-ext a0/s8\n    slli t1, a0, 56\n    srai t1, t1, 56\n    slti t0, t1, 0\n    xori a0, t0, 1\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[2].trim(), "mv t1, a0");
        assert_eq!(kinds[3], LineKind::Nop);
        let (changed, lines, kinds) = sign_ext(
            "f:\n    # lccc-arg-ext a0/s16\n    slli t1, a0, 48\n    srai t1, t1, 48\n    ret\n",
        );
        assert!(changed);
        assert_eq!((lines[2].trim(), kinds[3]), ("mv t1, a0", LineKind::Nop));
        let (changed, lines, kinds) = sign_ext(
            "f:\n    # lccc-arg-ext a0/s16\n    slli t1, a0, 56\n    srai t1, t1, 56\n    slti a0, t1, 0\n    ret\n",
        );
        assert!(changed);
        assert_eq!(
            (lines[2].trim(), kinds[3]),
            ("slli t1, a0, 56", LineKind::Nop)
        );
        // A write ends the entry fact.
        let asm = "f:\n    # lccc-arg-ext a0/s8\n    addi a0, a0, 1\n    slli t1, a0, 56\n    srai t1, t1, 56\n    ret\n";
        assert!(!sign_ext(asm).0, "{asm}");
    }

    /// Every sign-test fold against the interpreter: each load, offsets on
    /// both sides of the narrow bounds, each shift amount in place and into a
    /// second register, and each reader (plain, in place, negated), over
    /// every value the load produces.
    #[test]
    fn sign_tests_of_extensions_preserve_every_value() {
        // A stride through [lo, hi] plus its edges and the values around
        // zero the load can produce (an unsigned load never yields -1).
        let sampled = |lo: i64, hi: i64, step: usize| -> Vec<i64> {
            (lo..=hi)
                .step_by(step)
                .chain(
                    [lo, lo + 1, -1, 0, 1, hi - 1, hi]
                        .into_iter()
                        .filter(|v| (lo..=hi).contains(v)),
                )
                .collect()
        };
        let loads: [(&str, Vec<i64>); 5] = [
            ("lbu t0, 0(a0)", (0..=255).collect()),
            ("lb t0, 0(a0)", (-128..=127).collect()),
            ("lhu t0, 0(a0)", sampled(0, 65535, 37)),
            ("lh t0, 0(a0)", sampled(-32768, 32767, 37)),
            (
                "lw t0, 0(a0)",
                sampled(i64::from(i32::MIN), i64::from(i32::MAX), 9_999_991),
            ),
        ];
        let readers = [
            ("slti a1, {t}, 0", "a1"),
            ("srli a1, {t}, 63", "a1"),
            ("slti {t}, {t}, 0", "{t}"),
            ("slti a2, {t}, 0\n    xori a1, a2, 1", "a1"),
            ("slti {t}, {t}, 0\n    xori {t}, {t}, 1", "{t}"),
        ];
        // Folds by the shape the shift became: `andi` (and `seqz`), `srli`
        // of the source, or still `slli` (the `srai` dropped).
        let mut shapes = [0usize; 3];
        for (load, inputs) in &loads {
            for off in [0i64, -1, 1, -128, 127, 128, 255, 256] {
                for k in [56i64, 48, 32, 60, 40] {
                    for t in ["t0", "t1"] {
                        for (reader, result) in readers {
                            let reader = reader.replace("{t}", t);
                            let result = result.replace("{t}", t);
                            let asm = format!(
                                "f:\n    {load}\n    addiw t0, t0, {off}\n    slli {t}, t0, {k}\n    srai {t}, {t}, {k}\n    {reader}\n    mv a0, {result}\n    ret\n"
                            );
                            let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
                            let kinds: Vec<LineKind> =
                                lines.iter().map(|l| classify_line(l)).collect();
                            let before = decode_rv(&lines, &kinds);
                            let (changed, new_lines, new_kinds) = sign_ext(&asm);
                            if !changed {
                                continue;
                            }
                            shapes[match new_lines[3].trim().split(' ').next() {
                                Some("andi") => 0,
                                Some("srli") => 1,
                                _ => 2,
                            }] += 1;
                            let after = decode_rv(&new_lines, &new_kinds);
                            assert!(after.len() < before.len(), "{asm}");
                            for &v in inputs {
                                assert_eq!(
                                    run_rv(&before, v),
                                    run_rv(&after, v),
                                    "input {v}:\n{asm}=>\n{}",
                                    new_lines.join("\n")
                                );
                            }
                        }
                    }
                }
            }
        }
        // Every one of the 2000 functions folds.  A pair whose source
        // already fits in 64 - K bits is redundant first (every K = 32 pair,
        // as `addiw` leaves a word; the narrow loads at K = 40 and K = 48;
        // `lb + 0` and `lbu - 128` at K = 56).  Of the rest, the 2 negated
        // readers of bit 7 or bit 3 (K = 56, 60) become `andi; seqz`: 5
        // loads x 8 offsets x 2 shifts x 2 registers x 2 readers = 320, less
        // the 8 redundant ones = 312.  A source in [0, 2^(64-K)) becomes one
        // `srli` -- `lbu + 0` and `lb + 128` at K = 56 under the 3 plain
        // readers (2 x 2 x 3 = 12), `lhu + 0` at K = 48 under all 5 (2 x 5 =
        // 10): 22.  The other 1666 drop the `srai` or the redundant pair.
        assert_eq!(shapes, [312, 22, 1666], "folds by shape");
    }

    /// `-(x < 0)` is the sign broadcast, `srai d, x, 63`: each negation
    /// spelling, in place and into another register, against the
    /// interpreter over values at every sign and width boundary.
    #[test]
    fn a_negated_sign_test_is_an_arithmetic_shift() {
        let fold = |asm: &str| {
            let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
            let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
            let n = lines.len();
            let changed = fold_sign_masks(&mut lines, &mut kinds, n);
            (changed, lines, kinds)
        };
        let values = [
            0i64,
            1,
            -1,
            2,
            i64::MAX,
            i64::MIN,
            0x7fff_ffff,
            -0x8000_0000,
            0x8000_0000,
            0xffff_ffff,
            -0x1_0000_0000,
            0x1234_5678_9abc_def0,
        ];
        let mut folds = 0;
        for neg in ["neg {d}, {t}", "negw {d}, {t}"] {
            for (t, d) in [("t1", "t1"), ("t1", "a3"), ("t0", "t0"), ("t0", "a0")] {
                let neg = neg.replace("{d}", d).replace("{t}", t);
                let asm = format!(
                    "f:\n    ld t0, 0(a0)\n    slti {t}, t0, 0\n    {neg}\n    mv a0, {d}\n    ret\n"
                );
                let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
                let kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
                let before = decode_rv(&lines, &kinds);
                let (changed, new_lines, new_kinds) = fold(&asm);
                assert!(changed, "{asm}");
                assert_eq!(new_lines[2].trim(), format!("srai {d}, t0, 63"));
                folds += 1;
                let after = decode_rv(&new_lines, &new_kinds);
                for &v in &values {
                    assert_eq!(run_rv(&before, v), run_rv(&after, v), "input {v}:\n{asm}");
                }
            }
        }
        assert_eq!(folds, 8);
        for sub in ["sub a0, zero, t1", "subw a0, x0, t1"] {
            let (changed, lines, kinds) =
                fold(&format!("f:\n    slti t1, a1, 0\n    {sub}\n    ret\n"));
            assert!(changed, "{sub}");
            assert_eq!(
                (lines[1].trim(), kinds[2]),
                ("srai a0, a1, 63", LineKind::Nop)
            );
        }
        for kept in [
            // The test's result is read again.
            "f:\n    slti t1, a1, 0\n    neg a0, t1\n    add a0, a0, t1\n    ret\n",
            // Not a sign test, or not its negation.
            "f:\n    slti t1, a1, 1\n    neg a0, t1\n    ret\n",
            "f:\n    slti t1, a1, 0\n    neg a0, t2\n    ret\n",
            "f:\n    slti t1, a1, 0\n    sub a0, a2, t1\n    ret\n",
            // Not the next line.
            "f:\n    slti t1, a1, 0\n    addi a2, a2, 1\n    neg a0, t1\n    ret\n",
            "f:\n    slti t1, a1, 0\n.L1:\n    neg a0, t1\n    ret\n",
        ] {
            assert!(!fold(kept).0, "{kept}");
        }
    }

    /// A narrow load extended the other way round is the load of that
    /// signedness (`lbu; slli 56; srai 56` is `lb`, `lw; slli 32; srli 32`
    /// is `lwu`): every load, both extensions at the load's width and one bit
    /// either side (which must leave the load alone -- one bit wider, a
    /// sign-extension is redundant instead), with the shifts in place, into
    /// a second register, and the right shift into a third (`slli t0, t0,
    /// 56; srai a5, t0, 56`, as the emitter stages a byte sum), over every
    /// value the memory can hold.
    #[test]
    fn extended_loads_take_the_extension_for_every_value() {
        let memory = |width: u32| -> Vec<i64> {
            match width {
                8 => (0..=255).collect(),
                16 => (0..=65535)
                    .step_by(13)
                    .chain([32767, 32768, 65535])
                    .collect(),
                _ => (0..=u32::MAX)
                    .step_by(9_999_991)
                    .map(i64::from)
                    .chain([0x7fff_ffff, 0x8000_0000, 0xffff_ffff])
                    .collect(),
            }
        };
        let mut folds = 0;
        for (load, width) in [
            ("lbu", 8u32),
            ("lb", 8),
            ("lhu", 16),
            ("lh", 16),
            ("lw", 32),
            ("lwu", 32),
        ] {
            let exact = 64 - width;
            for (back, k) in [
                ("srai", exact),
                ("srli", exact),
                ("srai", exact - 1),
                ("srli", exact + 1),
            ] {
                for (t, d) in [("t0", "t0"), ("t1", "t1"), ("t0", "a3"), ("t1", "a3")] {
                    let asm = format!(
                        "f:\n    {load} t0, 4(a0)\n    slli {t}, t0, {k}\n    {back} {d}, {t}, {k}\n    mv a0, {d}\n    ret\n"
                    );
                    let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
                    let kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
                    let before = decode_rv(&lines, &kinds);
                    let (changed, new_lines, new_kinds) = sign_ext(&asm);
                    if k != exact {
                        assert_eq!(new_lines[1], lines[1], "{asm}");
                    }
                    if !changed {
                        continue;
                    }
                    folds += 1;
                    let after = decode_rv(&new_lines, &new_kinds);
                    assert!(after.len() < before.len(), "{asm}");
                    for v in memory(width) {
                        assert_eq!(
                            run_rv(&before, v),
                            run_rv(&after, v),
                            "memory {v:#x}:\n{asm}=>\n{}",
                            new_lines.join("\n")
                        );
                    }
                }
            }
        }
        // All 48 at the load's width (a retype, or the redundant `lb`/`lh`/
        // `lw` sign-extension) and the 24 one bit wider sign-extensions.
        assert_eq!(folds, 72, "folds fired");
        // The retyped load, and what keeps the staging.
        let (changed, lines, kinds) = sign_ext(
            "f:\n    lbu t0, %lo(sym)(a5)\n    slli t1, t0, 56\n    srai t1, t1, 56\n    bltz t1, .L1\n    ret\n.L1:\n    li a0, 1\n    ret\n",
        );
        assert!(changed);
        assert_eq!(lines[1].trim(), "lb t1, %lo(sym)(a5)");
        assert_eq!((kinds[2], kinds[3]), (LineKind::Nop, LineKind::Nop));
        for kept in [
            // The zero-extended byte is read again.
            "f:\n    lbu t0, 0(a0)\n    slli t1, t0, 56\n    srai t1, t1, 56\n    add a0, t1, t0\n    ret\n",
            // Not the previous line, or across a label.
            "f:\n    lbu t0, 0(a0)\n    addi t2, t2, 1\n    slli t1, t0, 56\n    srai t1, t1, 56\n    mv a0, t1\n    ret\n",
            "f:\n    lbu t0, 0(a0)\n.L1:\n    slli t1, t0, 56\n    srai t1, t1, 56\n    mv a0, t1\n    ret\n",
            // A doubleword load, or another register's extension.
            "f:\n    ld t0, 0(a0)\n    slli t1, t0, 56\n    srai t1, t1, 56\n    mv a0, t1\n    ret\n",
            "f:\n    lbu t0, 0(a0)\n    slli t1, t2, 56\n    srai t1, t1, 56\n    mv a0, t1\n    ret\n",
            // The left shift's result is read after the right shift.
            "f:\n    lbu t0, 0(a0)\n    slli t1, t0, 56\n    srai a3, t1, 56\n    add a0, a3, t1\n    ret\n",
        ] {
            let (changed, lines, _) = sign_ext(kept);
            assert!(
                !changed || !lines.iter().any(|l| l.trim().starts_with("lb ")),
                "{kept}"
            );
        }
    }

    #[test]
    fn byte_zext_tracks_lbu_through_moves() {
        let input = "f:\n    lbu t0, 0(t2)\n    mv a0, t0\n    andi t0, t0, 0xff\n    ret\n";
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[3], LineKind::Nop);
    }

    #[test]
    fn byte_zext_cross_register_form_keeps_definition() {
        let input = "f:\n    lbu t0, 0(t2)\n    andi a0, t0, 0xff\n    add a1, a0, t3\n    ret\n";
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(lines[2].trim(), "mv a0, t0");
    }

    #[test]
    fn byte_zext_respects_calls_and_abi_register_classes() {
        let input = concat!(
            "f:\n",
            "    lbu s1, 0(t2)\n",
            "    lbu a0, 1(t2)\n",
            "    call helper\n",
            "    andi s1, s1, 0xff\n",
            "    andi a0, a0, 0xff\n",
            "    ret\n",
        );
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[4], LineKind::Nop, "s1 is callee-saved");
        assert_eq!(kinds[5], LineKind::Alu, "a0 is caller-clobbered");
    }

    #[test]
    fn byte_zext_label_kills_path_specific_proof() {
        let input = "f:\n    lbu t0, 0(t2)\n.Ljoin:\n    andi t0, t0, 0xff\n    ret\n";
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[3], LineKind::Alu);
    }

    #[test]
    fn byte_zext_unknown_instruction_kills_proof() {
        let input =
            "f:\n    lbu t0, 0(t2)\n    csrrw t3, mstatus, t4\n    andi t0, t0, 0xff\n    ret\n";
        let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!eliminate_redundant_zext_byte(&mut lines, &mut kinds, n));
        assert_eq!(kinds[3], LineKind::Alu);
    }

    #[test]
    fn sext_self_form_deleted_only_when_sext_wide() {
        // li produces a sign-extended-32 value -> `sext.w t0, t0` is a no-op.
        let input = "f:\n    li t0, 7\n    sext.w t0, t0\n    mv t1, t0\n    ret\n";
        let out = peephole_optimize(input.to_string());
        assert!(!out.contains("sext.w t0, t0"), "\n{}", out);
    }

    /// A `sext.w <dst>, <src>` line survives and still reads `src`.
    ///
    /// The accumulator retarget may move the destination of the sext into the
    /// home it is copied to (`sext.w t0, t0` + `mv t1, t0` becomes
    /// `sext.w t1, t0`), so these tests assert that the sext is still there and
    /// still consumes the loaded value, not that its destination is `t0`.
    fn sext_from(out: &str, src: &str) -> bool {
        out.lines()
            .any(|line| match line.trim().strip_prefix("sext.w ") {
                Some(rest) => rest.split_once(", ").is_some_and(|(_dst, s)| s == src),
                None => false,
            })
    }

    /// A loop constant hoisted into a register (`li a3, 50` before the loop)
    /// is a sign-extended word wherever the function reads it, so the `int`
    /// compare inside the loop no longer re-extends it per iteration; each
    /// way the fact can fail keeps the extension.
    #[test]
    fn invariantly_wide_registers_cross_joins() {
        let shape = |pre: &str, body: &str| {
            format!(
                "f:\n{pre}    li a4, 0\n.L1:\n    lw t1, 0(a0)\n    sext.w t2, a3\n    slt a5, t1, t2\n    addw a4, a4, a5\n{body}    addi a0, a0, 4\n    addi a1, a1, -1\n    bnez a1, .L1\n    mv a0, a4\n    ret\n"
            )
        };
        let extends = |out: &str| out.lines().any(|l| l.trim().starts_with("sext.w"));
        // The fact holds: a3's only write is the word constant.
        let out = peephole_optimize(shape("    li a3, 50\n", ""));
        assert!(!extends(&out), "\n{out}");
        // Through a copy of an invariantly wide register.
        let out = peephole_optimize(shape("    li a6, -7\n    mv a3, a6\n", ""));
        assert!(!extends(&out), "\n{out}");
        // A constant beyond the word range.
        let out = peephole_optimize(shape("    li a3, 4294967296\n", ""));
        assert!(extends(&out), "\n{out}");
        // Another write that is not a word: a doubleword load in the loop.
        let out = peephole_optimize(shape("    li a3, 50\n", "    ld a3, 8(a0)\n"));
        assert!(extends(&out), "\n{out}");
        // Live on entry: the argument register itself.
        let out = peephole_optimize(shape("", ""));
        assert!(extends(&out), "\n{out}");
        // A call clobbers the caller-saved register.
        let out = peephole_optimize(shape("    li a3, 50\n", "    call g\n"));
        assert!(extends(&out), "\n{out}");
        // A write this does not model.
        let out = peephole_optimize(shape("    li a3, 50\n", "    csrr a3, cycle\n"));
        assert!(extends(&out), "\n{out}");
    }

    #[test]
    fn invariant_word_facts_need_every_write_and_no_entry_value() {
        let facts = |asm: &str| {
            let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
            let kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
            rv_invariantly_wide(&lines, &kinds, 0, lines.len())
        };
        let a3 = rv_bit(REG_A3);
        let a4 = rv_bit(REG_A4);
        assert_eq!(
            facts("f:\n    li a3, 1\n    lw a4, 0(a0)\n    ret\n") & (a3 | a4),
            a3 | a4
        );
        // Copies of each other only: a4 is read before any write (live on entry).
        assert_eq!(
            facts("f:\n    mv a3, a4\n    beqz a0, .L1\n    mv a4, a3\n.L1:\n    ret\n")
                & (a3 | a4),
            0
        );
        // `lwu` and `ld` are not words; `lhu`/`lbu` are.
        assert_eq!(facts("f:\n    lwu a3, 0(a0)\n    ret\n") & a3, 0);
        assert_eq!(
            facts("f:\n    lbu a3, 0(a0)\n    lhu a4, 2(a0)\n    ret\n") & (a3 | a4),
            a3 | a4
        );
        // An unresolvable transfer gives no facts at all.
        assert_eq!(facts("f:\n    li a3, 1\n    jr a0\n"), 0);
    }

    /// `li` of a constant outside the signed 32-bit range is no sign-extended
    /// word: `(int)0x100000001L` at -O0 is `li t0, 4294967297; sext.w a0, t0`,
    /// and dropping the extension returned 0x100000001 for the int 1 (the
    /// RV64 psABI hands an int back sign-extended, and callers compare the
    /// whole register).  The signed 32-bit range itself is still a producer.
    #[test]
    fn li_beyond_the_word_range_is_not_a_sext32_producer() {
        for (imm, word) in [
            ("4294967297", 1i64),
            ("2147483648", -2147483648),
            ("-2147483649", 2147483647),
            ("0x100000000", 0),
        ] {
            let input = format!("f:\n    li t0, {imm}\n    sext.w a0, t0\n    ret\n");
            let out = peephole_optimize(input.clone());
            // The extension stays, or a0 receives the extended word itself.
            assert!(
                sext_from(&out, "t0") || out.contains(&format!("li a0, {word}\n")),
                "li {imm}\n{out}"
            );
            assert!(!out.contains(&format!("li a0, {imm}")), "li {imm}\n{out}");
        }
        for imm in ["2147483647", "-2147483648", "-1", "0x7fffffff"] {
            let input =
                format!("f:\n    li t0, {imm}\n    sext.w t0, t0\n    mv a0, t0\n    ret\n");
            let out = peephole_optimize(input.clone());
            assert!(!out.contains("sext.w"), "li {imm}\n{out}");
        }
    }

    #[test]
    fn sext_self_form_kept_when_not_sext_wide() {
        // ld does NOT produce a sext-wide value; the sext.w must survive.
        let input = "f:\n    ld t0, 0(s0)\n    sext.w t0, t0\n    mv a0, t0\n    ret\n";
        let out = peephole_optimize(input.to_string());
        assert!(sext_from(&out, "t0"), "\n{}", out);
    }

    #[test]
    fn a_short_srai_is_not_a_sext32_producer() {
        // `(int)(x >> 3)` for a `long x`: srai by 3 of 0x10_0000_0000 is
        // 0x2_0000_0000, whose `sext.w` is 0 -- the truncation is real code.
        for k in [1, 3, 31] {
            let input = format!(
                "f:\n    ld t0, 0(s0)\n    srai t0, t0, {k}\n    sext.w t0, t0\n    mv a0, t0\n    ret\n"
            );
            let out = peephole_optimize(input.clone());
            assert!(sext_from(&out, "t0"), "k={k}\n{out}");
        }
    }

    #[test]
    fn a_long_srai_and_the_w_immediates_are_sext32_producers() {
        for producer in [
            "srai t0, t0, 32",
            "srai t0, t0, 63",
            "slliw t0, t0, 3",
            "srliw t0, t0, 1",
            "sraiw t0, t0, 31",
            "addiw t0, t0, -1",
        ] {
            let input = format!(
                "f:\n    ld t0, 0(s0)\n    {producer}\n    sext.w t0, t0\n    mv a0, t0\n    ret\n"
            );
            let out = peephole_optimize(input.clone());
            assert!(!out.contains("sext.w t0, t0"), "{producer}\n{out}");
        }
    }

    #[test]
    fn sext_cross_register_form_never_deleted() {
        // Even a value no-op keeps its definition: the instruction DEFINES
        // t2, and `bne t2, t1` below reads it (cfg_phi_web miscompile class).
        // A provably sign-extended source turns it into the copy it amounts
        // to (here folded further into `li t2, 7`), never into nothing.
        let input = "f:\n    li t0, 7\n    sext.w t2, t0\n    li t1, 0\n    bne t2, t1, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n";
        let out = peephole_optimize(input.to_string());
        let rows: Vec<&str> = out.lines().map(str::trim).collect();
        let branch = rows
            .iter()
            .position(|l| l.starts_with("bne t2,"))
            .expect("the branch survives");
        assert!(
            rows[..branch].iter().any(|l| l
                .split_once(' ')
                .is_some_and(|(_, ops)| ops.starts_with("t2,"))),
            "\n{}",
            out
        );
        // An unproven source keeps the sign extension itself.
        let input = input.replace("li t0, 7", "ld t0, 0(a0)");
        let out = peephole_optimize(input);
        assert!(out.contains("sext.w t2, t0"), "\n{}", out);
    }
    // ── Tests: the implicit-operand oracle ───────────────────────────────

    #[test]
    fn oracle_link_register_contract() {
        let ra = 1u64 << REG_RA;
        // `ret` jumps through ra: an implicit read.
        assert_eq!(classify_implicit_operands_rv("ret"), (ra, 0));
        // `call sym` and the bare `jal target` pseudo write ra.
        assert_eq!(classify_implicit_operands_rv("call printf"), (0, ra));
        assert_eq!(classify_implicit_operands_rv("jal .Ltail"), (0, ra));
        // With a named rd nothing is implicit (`jal x0` is a plain jump).
        assert_eq!(classify_implicit_operands_rv("jal ra, .Ltail"), (0, 0));
        assert_eq!(classify_implicit_operands_rv("jal x0, .LBB2"), (0, 0));
    }

    #[test]
    fn oracle_indirect_jump_forms() {
        let ra = 1u64 << REG_RA;
        let t2 = 1u64 << REG_T2;
        // `jr rs1` reads the target, writes nothing.
        assert_eq!(classify_implicit_operands_rv("jr t2"), (t2, 0));
        // `jalr rs1` — one operand: reads the target AND writes ra.
        assert_eq!(classify_implicit_operands_rv("jalr t2"), (t2, ra));
        // `jalr rs1, imm` — two operands: reads the target, writes ra.
        assert_eq!(classify_implicit_operands_rv("jalr t2, 0"), (t2, ra));
        // `jalr rd, rs1, imm` — the form the emitter produces: fully
        // explicit, nothing implicit (the copy-propagation guard must not
        // over-bail on it).
        assert_eq!(classify_implicit_operands_rv("jalr ra, t2, 0"), (0, 0));
    }

    #[test]
    fn oracle_non_instructions_name_nothing() {
        assert_eq!(classify_implicit_operands_rv(".LBB1:"), (0, 0));
        assert_eq!(classify_implicit_operands_rv(".cfi_startproc"), (0, 0));
        assert_eq!(classify_implicit_operands_rv(""), (0, 0));
        assert_eq!(classify_implicit_operands_rv("ecall"), (0, 0));
        assert_eq!(classify_implicit_operands_rv("fence iorw, iorw"), (0, 0));
        assert_eq!(classify_implicit_operands_rv("add t0, t1, t2"), (0, 0));
    }

    #[test]
    fn implicitly_writes_rv_projection() {
        let ra = 1u64 << REG_RA;
        assert!(implicitly_writes_rv("call printf", REG_RA));
        assert!(implicitly_writes_rv("jal .Ltail", REG_RA));
        assert!(!implicitly_writes_rv("jal ra, .Ltail", REG_RA));
        assert!(!implicitly_writes_rv("ret", REG_RA));
        assert_eq!(classify_implicit_operands_rv("ret").0, ra);
    }

    #[test]
    fn redundant_conditional_jump_to_its_own_fallthrough_is_removed() {
        // `bnez t0, .Lnext` immediately followed by `.Lnext:` reaches the same
        // instruction whether or not it is taken.
        let asm = "f:\n    bnez t0, .Lnext\n.Lnext:\n    ret\n";
        let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(eliminate_redundant_jumps(&lines, &mut kinds, n));
        assert_eq!(kinds[0], LineKind::Label, "labels are never touched");
        assert_eq!(kinds[1], LineKind::Nop);

        // The same shape with a different target must survive: the branch is
        // the only thing selecting between the two labels.
        let asm = "f:\n    bnez t0, .Lother\n.Lnext:\n    ret\n";
        let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!eliminate_redundant_jumps(&lines, &mut kinds, n));
        assert_eq!(kinds[1], LineKind::Branch);
    }

    #[test]
    fn trampolines_are_threaded_and_cycles_are_left_alone() {
        // A two-hop chain: the entry jump must reach `.Lreal` in one step.
        let asm = concat!(
            "f:\n",
            "    jump .Ltramp, t6\n",
            "    ret\n",
            ".Ltramp:\n",
            "    jump .Lhop, t6\n",
            ".Lother:\n",
            "    li a0, 9\n",
            "    ret\n",
            ".Lhop:\n",
            "    jump .Lreal, t6\n",
            ".Lrest:\n",
            "    li a0, 7\n",
            "    ret\n",
            ".Lreal:\n",
            "    li a0, 3\n",
            "    ret\n",
        );
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(thread_trampolines(&mut lines, &mut kinds, n));
        assert_eq!(lines[1], "    jump .Lreal, t6");
        assert_eq!(lines[3], ".Ltramp:", "the label itself is never removed");

        // A cyclic chain would loop forever whichever hop is taken; leave the
        // text exactly as it was instead of picking a direction.
        let asm = concat!(
            "f:\n",
            "    jump .La, t6\n",
            "    ret\n",
            ".La:\n",
            "    jump .Lb, t6\n",
            ".Lmid:\n",
            "    li a0, 1\n",
            "    ret\n",
            ".Lb:\n",
            "    jump .La, t6\n",
            ".Lrest:\n",
            "    li a0, 2\n",
            "    ret\n",
        );
        let original: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut lines = original.clone();
        let mut kinds: Vec<LineKind> = lines.iter().map(|line| classify_line(line)).collect();
        let n = lines.len();
        assert!(!thread_trampolines(&mut lines, &mut kinds, n));
        assert_eq!(lines, original);
    }
}

#[cfg(test)]
mod inline_asm_and_return_tests {
    use super::*;

    fn opt(asm: &str) -> String {
        peephole_optimize(asm.to_string())
    }

    /// The instruction lines of `out`, trimmed (labels and directives kept).
    fn rows(out: &str) -> Vec<String> {
        out.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_naked_function_body_is_left_as_written() {
        // A custom convention may hand t0 back to an asm caller: the user's
        // `mv t0, a0` before `ret` is not a dead temporary.
        let asm = concat!(
            ".type f, @function\n",
            "f:\n",
            ".cfi_startproc\n",
            "#APP\n",
            "    mv t0, a0\n",
            "    ret\n",
            "#NO_APP\n",
            ".cfi_endproc\n",
            ".size f, .-f\n",
            ".type g, @function\n",
            "g:\n",
            ".cfi_startproc\n",
            "    mv t0, a0\n",
            "    sext.w a0, a0\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size g, .-g\n",
        );
        let out = opt(asm);
        let r = rows(&out);
        let f = r.iter().position(|l| l == "f:").unwrap();
        assert_eq!(
            r[f + 1..f + 6],
            [".cfi_startproc", "#APP", "mv t0, a0", "ret", "#NO_APP"],
            "{out}"
        );
        let g = r.iter().position(|l| l == "g:").unwrap();
        assert_eq!(
            r[g + 1..g + 4],
            [".cfi_startproc", "sext.w a0, a0", "ret"],
            "{out}"
        );
    }

    #[test]
    fn inline_asm_operands_staged_outside_the_region_survive() {
        // `li a7, 64` is read by the `ecall` the text does not connect to it,
        // and the later `li a7, 93` would otherwise look like its overwrite.
        let asm = concat!(
            ".type s, @function\n",
            "s:\n",
            ".cfi_startproc\n",
            "    li a7, 64\n",
            "    mv t1, a0\n",
            "    mv t1, a1\n",
            "#APP\n",
            "    ecall\n",
            "#NO_APP\n",
            "    li a7, 93\n",
            "    mv a0, a0\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size s, .-s\n",
        );
        assert_eq!(opt(asm), asm, "a function holding inline asm changed");
    }

    #[test]
    fn a_function_with_both_volatile_access_and_asm_is_restored_once() {
        let asm = concat!(
            ".type v, @function\n",
            "v:\n",
            ".cfi_startproc\n",
            "    # LCCC_VOLATILE_BEGIN\n",
            "    lw t0, -24(s0)\n",
            "    # LCCC_VOLATILE_END\n",
            "#APP\n",
            "    mv t2, t2\n",
            "#NO_APP\n",
            "    mv t0, t0\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size v, .-v\n",
        );
        let expected = crate::backend::peephole_common::strip_volatile_markers(asm.into());
        assert_eq!(opt(asm), expected);
    }

    #[test]
    fn top_level_asm_is_left_alone_and_neighbours_still_optimize() {
        let asm = concat!(
            "#APP\n",
            ".globl _start\n",
            "_start:\n",
            "    mv a0, a0\n",
            "    mv t0, a1\n",
            "    ret\n",
            "#NO_APP\n",
            ".type h, @function\n",
            "h:\n",
            ".cfi_startproc\n",
            "    mv t3, t3\n",
            "    ret\n",
            ".cfi_endproc\n",
            ".size h, .-h\n",
        );
        let out = opt(asm);
        assert!(
            out.starts_with(
                "#APP\n.globl _start\n_start:\n    mv a0, a0\n    mv t0, a1\n    ret\n#NO_APP\n"
            ),
            "{out}"
        );
        assert!(
            !out.contains("mv t3, t3"),
            "the next function lost its peephole: {out}"
        );
    }

    #[test]
    fn unbalanced_asm_markers_leave_the_unit_unchanged() {
        let asm = ".type f, @function\nf:\n#APP\n    mv t0, t0\n    ret\n.size f, .-f\n";
        assert_eq!(opt(asm), asm);
        let asm = "#NO_APP\nf:\n    mv t0, t0\n    ret\n";
        assert_eq!(opt(asm), asm);
    }

    #[test]
    fn a_temporary_written_on_the_way_to_ret_is_dead() {
        let out = opt("f:\n    mv t0, a0\n    sext.w a0, a0\n    ret\n");
        assert_eq!(rows(&out), ["f:", "sext.w a0, a0", "ret"], "{out}");
        // Return registers, callee-saved registers, reads on the way and
        // other exits keep the move.
        for (keep, mv) in [
            ("f:\n    mv a1, a2\n    ret\n", "mv a1, a2"),
            ("f:\n    call g\n    mv s1, a0\n    ret\n", "mv s1, a0"),
            (
                "f:\n    mv t0, a0\n    li a0, 1\n    addw a0, t0, a0\n    ret\n",
                "mv t0, a0",
            ),
            ("f:\n    mv t0, a0\n    jr t0\n", "mv t0, a0"),
            (
                "f:\n    mv t0, a0\n    beqz a1, .L1\n    ret\n.L1:\n    mv a0, t0\n    ret\n",
                "mv t0, a0",
            ),
        ] {
            let out = opt(keep);
            assert!(rows(&out).iter().any(|l| l == mv), "{keep} -> {out}");
        }
    }

    #[test]
    fn a_zero_extension_read_only_by_sext_w_folds_away() {
        let out = opt("f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n    sext.w a0, a0\n    ret\n");
        assert_eq!(rows(&out), ["f:", "sext.w a0, a0", "ret"], "{out}");
        // The returned-parameter shape the code generator emits.
        let out = opt(
            "f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n    mv t0, a0\n    sext.w a0, t0\n    ret\n",
        );
        assert_eq!(rows(&out), ["f:", "sext.w a0, a0", "ret"], "{out}");
        // The cross-register pair is the copy it amounts to, which folds
        // into the sign extension.
        let out = opt("f:\n    slli a0, a1, 32\n    srli a0, a0, 32\n    sext.w a0, a0\n    ret\n");
        assert_eq!(rows(&out), ["f:", "sext.w a0, a1", "ret"], "{out}");
        // A label between the pair and its reader: the upper-word demand
        // sees every predecessor of the join in the function's CFG, and none
        // observes a0's upper word.
        let out =
            opt("f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n.L1:\n    sext.w a0, a0\n    ret\n");
        assert_eq!(rows(&out), ["f:", ".L1:", "sext.w a0, a0", "ret"], "{out}");
        for keep in [
            // A path from the pair that stores the whole register.
            "f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n    beqz a1, .L2\n.L1:\n    sext.w a0, a0\n    ret\n.L2:\n    sd a0, 0(a2)\n    ret\n",
            // The zero-extended value lives on in a0.
            "f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n    sext.w a1, a0\n    ret\n",
            // Not a word zero extension.
            "f:\n    slli a0, a0, 31\n    srli a0, a0, 31\n    sext.w a0, a0\n    ret\n",
            "f:\n    slli a0, a0, 32\n    srai a0, a0, 32\n    sext.w a0, a0\n    ret\n",
        ] {
            let out = opt(keep);
            assert!(
                rows(&out)
                    .iter()
                    .any(|l| l.starts_with("srli") || l.starts_with("srai")),
                "{keep} -> {out}"
            );
        }
    }
}

#[cfg(test)]
mod frame_store_census_tests {
    use super::peephole_optimize;

    fn code_lines(asm: &str) -> Vec<String> {
        asm.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    /// The global dead-store census counts every reader of a frame slot.
    /// An FP load is a reader: a `__complex__ float` result staged through
    /// an integer register (gcc.c-torture execute/pr38969.c) lost the `sd`
    /// before the `flw` at -O2.
    #[test]
    fn a_slot_read_only_by_an_fp_load_keeps_its_store() {
        // The label keeps the load out of reach of every forwarding pass:
        // only the census decides.
        for (load, width) in [("flw fa1", 4), ("fld fa1", 8), ("flh fa1", 2)] {
            let asm = format!(
                "f:\n    ld t0, -64(s0)\n    sd t0, -72(s0)\n.L1:\n    {load}, -72(s0)\n    ret\n"
            );
            let out = code_lines(&peephole_optimize(asm.clone()));
            assert!(
                out.iter().any(|l| l == "sd t0, -72(s0)"),
                "{width}-byte FP reader lost its store:\n{asm} -> {out:?}"
            );
        }
        // In straight-line code the doubleword slot is a register: the
        // reload reads the source slot directly.
        let asm = "f:\n    ld t0, -64(s0)\n    sd t0, -72(s0)\n    fld fa1, -72(s0)\n    ret\n";
        let out = code_lines(&peephole_optimize(asm.to_string()));
        assert_eq!(out, ["f:", "fld fa1, -64(s0)", "ret"], "{out:?}");
    }

    /// A slot named by an instruction the census cannot size (an atomic, a
    /// vector access) keeps every store in the unit.
    #[test]
    fn an_unsized_frame_access_disables_the_census() {
        let asm =
            "f:\n    li t0, 5\n    sd t0, -24(s0)\n    lr.d t1, (s0)\n    mv a0, t1\n    ret\n";
        let out = code_lines(&peephole_optimize(asm.to_string()));
        assert!(out.iter().any(|l| l == "sd t0, -24(s0)"), "{out:?}");
    }

    /// The census still deletes a store nothing reads, FP loads of other
    /// slots notwithstanding.
    #[test]
    fn a_store_no_load_reaches_is_still_deleted() {
        let asm = "f:\n    li t0, 5\n    sd t0, -24(s0)\n    flw fa0, -40(s0)\n    ret\n";
        let out = code_lines(&peephole_optimize(asm.to_string()));
        assert!(!out.iter().any(|l| l == "sd t0, -24(s0)"), "{out:?}");
        assert!(out.iter().any(|l| l == "flw fa0, -40(s0)"), "{out:?}");
    }
}

#[cfg(test)]
mod word_reload_tests {
    use super::peephole_optimize;

    fn code_lines(asm: &str) -> Vec<String> {
        asm.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn opt(asm: &str) -> Vec<String> {
        code_lines(&peephole_optimize(asm.to_string()))
    }

    /// `sw` keeps the low word and `lw` returns it sign-extended, so a reload
    /// forwarded from a word store is a sign extension, never a plain copy.
    /// The boundary witness `integer_pointer_width.c` caught the copy form:
    /// `(void *)(int32_t)u` at -O0 returned 0x8000_0000 for 0x8000_0000.
    #[test]
    fn an_adjacent_word_reload_is_a_sign_extension() {
        let out = opt("f:\n    sw t0, -24(s0)\n    lw t0, -24(s0)\n    mv a0, t0\n    ret\n");
        assert!(
            out.iter()
                .any(|l| l == "sext.w a0, t0" || l == "sext.w t0, t0"),
            "{out:?}"
        );
        assert!(!out.iter().any(|l| l.starts_with("lw ")), "{out:?}");
        let out = opt("f:\n    sw t0, -24(s0)\n    lw a0, -24(s0)\n    ret\n");
        assert!(out.iter().any(|l| l == "sext.w a0, t0"), "{out:?}");
        assert!(!out.iter().any(|l| l == "mv a0, t0"), "{out:?}");
    }

    /// The same rule across unrelated code (`global_store_forwarding`).
    #[test]
    fn a_forwarded_word_reload_is_a_sign_extension() {
        let out = opt(
            "f:\n    sw t0, -24(s0)\n    addi t2, t3, 1\n    lw t1, -24(s0)\n    \
             add a0, t1, t2\n    ret\n",
        );
        assert!(
            out.iter()
                .any(|l| l.starts_with("sext.w ") && l.ends_with(", t0")),
            "{out:?}"
        );
        assert!(!out.iter().any(|l| l == "mv t1, t0"), "{out:?}");
    }

    /// A doubleword reload is still a copy, and a word whose producer already
    /// sign-extends (`addw`) needs no extension at all: the store in between
    /// defines no register, so the producer's proof survives it.
    #[test]
    fn sign_extended_words_and_doublewords_forward_for_free() {
        let out = opt(
            "f:\n    addw t0, a0, a1\n    sw t0, -24(s0)\n    lw t0, -24(s0)\n    \
             mv a0, t0\n    ret\n",
        );
        assert!(!out.iter().any(|l| l.starts_with("sext.w")), "{out:?}");
        assert!(!out.iter().any(|l| l.starts_with("lw ")), "{out:?}");
        let out = opt("f:\n    sd t0, -24(s0)\n    ld a0, -24(s0)\n    ret\n");
        assert!(out.iter().any(|l| l == "mv a0, t0"), "{out:?}");
    }

    /// The -O0 shape of `(int32_t)u` end to end: the zero-extended unsigned
    /// value is staged, reloaded unsigned and sign-extended; folding the
    /// `lwu; sext.w` pair into `lw` must not let the word store swallow it.
    #[test]
    fn an_unsigned_to_int_cast_through_the_frame_keeps_its_extension() {
        let out = opt(
            "s32:\n    slli t0, a0, 32\n    srli t0, t0, 32\n    sd t0, -24(s0)\n    \
             lwu t0, -24(s0)\n    sd t0, -24(s0)\n    ld t0, -24(s0)\n    sw t0, -24(s0)\n    \
             lwu t0, -24(s0)\n    sd t0, -32(s0)\n    sext.w t0, t0\n    sd t0, -40(s0)\n    \
             sext.w t0, t0\n    sd t0, -48(s0)\n    mv a0, t0\n    ret\n",
        );
        let ret = out.iter().position(|l| l == "ret").expect("ret");
        assert!(
            out[..ret]
                .iter()
                .any(|l| l.starts_with("sext.w") || l.starts_with("lw ")),
            "{out:?}"
        );
    }
}

#[cfg(test)]
mod upper_demand_tests {
    use super::*;

    /// The upper-word demand pass alone, to a fixed point; the live lines,
    /// trimmed.
    fn run(asm: &str) -> Vec<String> {
        let mut lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        while eliminate_undemanded_extensions(&mut lines, &mut kinds, n) {}
        (0..n)
            .filter(|&i| kinds[i] != LineKind::Nop)
            .map(|i| lines[i].trim().to_string())
            .collect()
    }

    /// `unsigned bset(unsigned x, unsigned n) { return x | (1u << n); }`: the
    /// entry zero extensions feed `sllw` (five bits of the amount) and an
    /// `or` whose result only `sext.w` reads, so no upper word is observed.
    #[test]
    fn entry_zero_extensions_feeding_word_consumers_go() {
        let asm = "bset:\n    # lccc-ret-regs a0\n    slli a0, a0, 32\n    srli a0, a0, 32\n    slli a1, a1, 32\n    srli a1, a1, 32\n    li t0, 1\n    sllw t0, t0, a1\n    or t0, a0, t0\n    sext.w a0, t0\n    ret\n";
        assert_eq!(
            run(asm)[1..],
            [
                "li t0, 1",
                "sllw t0, t0, a1",
                "or t0, a0, t0",
                "sext.w a0, t0",
                "ret"
            ]
        );
        // Without the prologue's annotation `ret` may hand a1 back as the
        // upper half of a pair, so a1's extension stays.
        let unannotated = asm.replace("    # lccc-ret-regs a0\n", "");
        assert!(run(&unannotated).iter().any(|l| l == "srli a1, a1, 32"));
        assert!(!run(&unannotated).iter().any(|l| l == "srli a0, a0, 32"));
    }

    /// The cross-register form of the pair becomes the copy it amounts to.
    #[test]
    fn cross_register_zero_extension_becomes_a_copy() {
        let asm = "f:\n    slli t1, a0, 32\n    srli t1, t1, 32\n    addw a0, t1, a1\n    ret\n";
        assert_eq!(run(asm), ["f:", "mv t1, a0", "addw a0, t1, a1", "ret"]);
    }

    /// Every consumer that reads the upper word keeps the extension: an
    /// unsigned compare, a right shift, a 64-bit store, an address, the
    /// return value, a call argument, a branch, and a bit-local `add` whose
    /// own result is observed in full.
    #[test]
    fn observed_upper_words_keep_their_extensions() {
        let zext = "f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n";
        for tail in [
            "    sltu a0, a0, a1\n    ret\n",
            "    srli a0, a0, 3\n    sext.w a0, a0\n    ret\n",
            "    sd a0, 0(a1)\n    ret\n",
            "    add t0, a1, a0\n    ld a0, 0(t0)\n    ret\n",
            "    ret\n",
            "    call g\n    ret\n",
            "    bltu a0, a1, .L1\n.L1:\n    li a0, 0\n    ret\n",
            "    mv t1, a0\n    add t1, t1, t1\n    sd t1, 0(a2)\n    ret\n",
            "    srl t1, a0, a1\n    sw t1, 0(a2)\n    ret\n",
            "    andi t1, a0, -16\n    sd t1, 0(a2)\n    ret\n",
            "    slli t1, a0, 3\n    sd t1, 0(a2)\n    ret\n",
            "    tail g\n",
        ] {
            let asm = format!("{zext}{tail}");
            let out = run(&asm);
            assert!(
                out.iter().any(|l| l == "slli a0, a0, 32")
                    && out.iter().any(|l| l == "srli a0, a0, 32"),
                "{asm}\n{out:#?}"
            );
        }
    }

    /// Low-word consumers, directly or through bit-local operations whose
    /// own upper word is not observed, let the sign extension go -- deleted
    /// in its self form.
    #[test]
    fn sign_extensions_read_only_in_the_low_word_go() {
        let sext = "f:\n    sext.w t0, t0\n";
        for tail in [
            "    sw t0, 0(a1)\n    ret\n",
            "    sw t0, -20(s0)\n    ret\n",
            "    addw a0, t0, t0\n    ret\n",
            "    fcvt.d.w fa0, t0\n    ret\n",
            "    fmv.w.x fa0, t0\n    ret\n",
            "    add t1, t0, a1\n    sw t1, 0(a2)\n    ret\n",
            "    slli t1, t0, 32\n    sd t1, 0(a2)\n    ret\n",
            "    andi t1, t0, 255\n    sd t1, 0(a2)\n    ret\n",
            "    sll t1, a1, t0\n    sd t1, 0(a2)\n    ret\n",
            "    srl t1, a1, t0\n    sd t1, 0(a2)\n    ret\n",
            "    li t0, 7\n    sd t0, 0(a2)\n    ret\n",
        ] {
            let asm = format!("{sext}{tail}");
            let out = run(&asm);
            assert!(
                !out.iter().any(|l| l.starts_with("sext.w")),
                "{asm}\n{out:#?}"
            );
        }
        // The cross-register form defines its destination: it becomes a copy.
        assert_eq!(
            run("f:\n    sext.w t1, t0\n    sw t1, 0(a1)\n    ret\n"),
            ["f:", "mv t1, t0", "sw t1, 0(a1)", "ret"]
        );
    }

    /// ... and every observed one stays, including the psABI's sign-extended
    /// `int` return value and a 64-bit frame store.
    #[test]
    fn observed_sign_extensions_stay() {
        for asm in [
            "f:\n    sext.w a0, t0\n    ret\n",
            "f:\n    sext.w t0, t0\n    sd t0, -24(s0)\n    ret\n",
            "f:\n    sext.w t0, t0\n    bnez t0, .L1\n    li a0, 1\n.L1:\n    ret\n",
            "f:\n    sext.w t0, t0\n    add t1, t0, a1\n    sd t1, 0(a2)\n    ret\n",
            "f:\n    sext.w t0, t0\n    srl t1, t0, a1\n    sd t1, 0(a2)\n    ret\n",
            "f:\n    sext.w t0, t0\n    sw t0, 0(t0)\n    ret\n",
            "f:\n    sext.w a0, a0\n    call g\n    ret\n",
        ] {
            let out = run(asm);
            assert!(
                out.iter().any(|l| l.starts_with("sext.w")),
                "{asm}\n{out:#?}"
            );
        }
    }

    /// The demand is a may-problem over the CFG: one path that observes the
    /// upper word keeps the extension; when no path does, it goes, across
    /// joins and around loops.
    #[test]
    fn demand_follows_every_path() {
        // The taken path stores the whole register.
        let observed = "f:\n    sext.w t0, t0\n    beqz a1, .L1\n    sw t0, 0(a2)\n    ret\n.L1:\n    sd t0, 0(a2)\n    ret\n";
        assert!(run(observed).iter().any(|l| l == "sext.w t0, t0"));
        // Both paths store a word.
        let low = observed.replace("sd t0", "sw t0");
        assert!(!run(&low).iter().any(|l| l == "sext.w t0, t0"));
        // A loop whose body reads only the low word, and whose exit returns
        // a different register.
        let looped = "f:\n    sext.w t0, t0\n.L1:\n    addw a1, a1, t0\n    addi a2, a2, -1\n    bnez a2, .L1\n    mv a0, a1\n    ret\n";
        assert!(!run(looped).iter().any(|l| l == "sext.w t0, t0"));
        // The same loop handing the extended value back: kept.
        let returned = looped.replace("mv a0, a1", "mv a0, t0");
        assert!(run(&returned).iter().any(|l| l == "sext.w t0, t0"));
    }

    /// A function whose CFG cannot be resolved (an indirect jump), and text
    /// that ends without a return, are left alone or assumed fully observed.
    #[test]
    fn unresolvable_control_flow_is_left_alone() {
        let indirect = "f:\n    sext.w t0, t0\n    sw t0, 0(a1)\n    jr a2\n";
        assert!(run(indirect).iter().any(|l| l == "sext.w t0, t0"));
        let falls_off = "f:\n    sext.w t0, t0\n    sw t0, 0(a1)\n";
        assert!(run(falls_off).iter().any(|l| l == "sext.w t0, t0"));
    }

    /// Operands the role table does not know are read in full: an untracked
    /// register name, a relocation, an unknown mnemonic.
    #[test]
    fn unknown_operand_shapes_read_in_full() {
        for consumer in [
            "    addi a0, t0, %lo(sym)\n",
            "    addw a0, t0, x5\n",
            "    clzw a0, t0\n",
            "    sw t0, %lo(sym)(a1)\n",
        ] {
            let asm = format!("f:\n    sext.w t0, t0\n{consumer}    sd a0, 0(a2)\n    ret\n");
            assert!(run(&asm).iter().any(|l| l == "sext.w t0, t0"), "{asm}");
        }
    }

    /// The role table itself, for the shapes the emitter writes.
    #[test]
    fn role_table() {
        let roles = |l: &str| rv_upper_roles(l, classify_line(l));
        let b = rv_bit;
        assert_eq!(
            roles("    addw t0, t1, t2"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: 0,
                full: 0
            })
        );
        assert_eq!(
            roles("    add t0, t1, t2"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: b(REG_T1) | b(REG_T2),
                full: 0
            })
        );
        assert_eq!(
            roles("    sll t0, t1, t2"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: b(REG_T1),
                full: 0
            })
        );
        assert_eq!(
            roles("    sra t0, t1, t2"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: 0,
                full: b(REG_T1)
            })
        );
        assert_eq!(
            roles("    slli t0, t1, 32"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: 0,
                full: 0
            })
        );
        assert_eq!(
            roles("    slli t0, t1, 31"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: b(REG_T1),
                full: 0
            })
        );
        assert_eq!(
            roles("    andi t0, t1, 2047"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: 0,
                full: 0
            })
        );
        assert_eq!(
            roles("    andi t0, t1, -1"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: b(REG_T1),
                full: 0
            })
        );
        assert_eq!(
            roles("    sw t0, 8(a1)"),
            Some(UpperRoles {
                dest: 0,
                local: 0,
                full: b(REG_A1)
            })
        );
        assert_eq!(
            roles("    sd t0, -24(s0)"),
            Some(UpperRoles {
                dest: 0,
                local: 0,
                full: b(REG_T0) | b(REG_S0)
            })
        );
        assert_eq!(
            roles("    sw t0, -20(s0)"),
            Some(UpperRoles {
                dest: 0,
                local: 0,
                full: b(REG_S0)
            })
        );
        assert_eq!(
            roles("    mv t0, t1"),
            Some(UpperRoles {
                dest: b(REG_T0),
                local: b(REG_T1),
                full: 0
            })
        );
        assert_eq!(roles("    srli t0, t1, 3"), None);
        assert_eq!(roles("    sltu t0, t1, t2"), None);
        assert_eq!(roles("    ld t0, 0(a1)"), None);
    }
}

#[cfg(test)]
mod arg_extension_marker_tests {
    use super::*;

    fn code(out: &str) -> Vec<String> {
        out.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('.') && !l.ends_with(':'))
            .map(str::to_owned)
            .collect()
    }

    /// A narrow argument arrives sign-extended from bit 31 (`unsigned int`
    /// included): the cross-register `sext.w` the parity lowering reads it
    /// through is a copy, and the copy folds into its readers.
    #[test]
    fn sign_extended_arguments_need_no_sext_w() {
        let asm = "f:\n    # lccc-ret-regs a0\n    # lccc-arg-ext a0\n    sext.w t0, a0\n    srli t1, t0, 16\n    xor a0, t0, t1\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(
            code(&out),
            ["srli t1, a0, 16", "xor a0, a0, t1", "ret"],
            "{out}"
        );
        // No fact, no rewrite.
        let bare = asm.replace("    # lccc-arg-ext a0\n", "");
        let out = peephole_optimize(bare);
        assert!(code(&out).iter().any(|l| l.starts_with("sext.w")), "{out}");
        // The fact names a0 only.
        let other = asm.replace("# lccc-arg-ext a0", "# lccc-arg-ext a1");
        let out = peephole_optimize(other);
        assert!(code(&out).iter().any(|l| l.starts_with("sext.w")), "{out}");
    }

    /// `unsigned short`/`unsigned char` arguments arrive zero-extended: the
    /// halfword and byte re-normalisations of the incoming value go, as
    /// shift pairs and as `andi 0xff`.
    #[test]
    fn zero_extended_arguments_need_no_renormalisation() {
        let asm = "f:\n    # lccc-ret-regs a0\n    # lccc-arg-ext a0/z16 a1/z8\n    slli t0, a0, 48\n    srli t0, t0, 48\n    andi t1, a1, 0xff\n    slli t2, a1, 56\n    srli t2, t2, 56\n    add t0, t0, t1\n    add a0, t0, t2\n    ret\n";
        let out = peephole_optimize(asm.into());
        let rows = code(&out);
        assert!(
            !rows
                .iter()
                .any(|l| l.starts_with("slli") || l.starts_with("srli") || l.starts_with("andi")),
            "{out}"
        );
        // Only sign-extended: a halfword zero extension is real code.
        let signed = asm.replace("a0/z16", "a0");
        let out = peephole_optimize(signed);
        assert!(code(&out).iter().any(|l| l == "srli t0, t0, 48"), "{out}");
        // A halfword fact does not cover a byte (a byte one covers both).
        let wide = asm.replace("a0/z16 a1/z8", "a0/z8 a1/z16");
        let out = peephole_optimize(wide);
        let rows = code(&out);
        assert!(!rows.iter().any(|l| l == "srli t0, t0, 48"), "{out}");
        assert!(rows.iter().any(|l| l.starts_with("andi")), "{out}");
        assert!(rows.iter().any(|l| l == "srli t2, t2, 56"), "{out}");
    }

    /// The facts describe the entry value: a write to the register ends them
    /// (an unknown 64-bit sum keeps both extensions).  A write the range
    /// analysis models derives the new value's fact from the old one instead:
    /// `a0 + 1` of a 16-bit `a0` lies in [1, 65536], so its 16-bit
    /// zero-extension is real code (65536 wraps to 0) while its `sext.w` is
    /// not.
    #[test]
    fn a_write_ends_the_entry_fact() {
        let srli_48 = |rows: &[String]| {
            rows.iter()
                .any(|l| l.ends_with(", 48") && l.starts_with("srli"))
        };
        let asm = "f:\n    # lccc-ret-regs a0\n    # lccc-arg-ext a0/z16\n    add a0, a0, a1\n    slli t0, a0, 48\n    srli t0, t0, 48\n    sext.w t1, a0\n    add a0, t0, t1\n    ret\n";
        let out = peephole_optimize(asm.into());
        let rows = code(&out);
        assert!(srli_48(&rows), "{out}");
        assert!(rows.iter().any(|l| l.starts_with("sext.w")), "{out}");
        let asm = "f:\n    # lccc-ret-regs a0\n    # lccc-arg-ext a0/z16\n    addi a0, a0, 1\n    slli t0, a0, 48\n    srli t0, t0, 48\n    sext.w t1, a0\n    add a0, t0, t1\n    ret\n";
        let out = peephole_optimize(asm.into());
        let rows = code(&out);
        assert!(srli_48(&rows), "{out}");
        assert!(!rows.iter().any(|l| l.starts_with("sext.w")), "{out}");
    }

    /// Malformed tags state nothing, and the line itself never reaches the
    /// output.
    #[test]
    fn the_marker_is_internal_and_strict() {
        assert_eq!(
            arg_ext_facts("# lccc-arg-ext a0 a1/z16 a2/z8 a3/s8 a4/s16").collect::<Vec<_>>(),
            [
                (REG_A0, ArgExt::Word),
                (REG_A0 + 1, ArgExt::Zero(16)),
                (REG_A0 + 2, ArgExt::Zero(8)),
                (REG_A0 + 3, ArgExt::Sign(8)),
                (REG_A0 + 4, ArgExt::Sign(16)),
            ]
        );
        assert_eq!(
            arg_ext_facts("# lccc-arg-ext a0/z32 a1/s32 t0 s1 a9").count(),
            0
        );
        let asm = "f:\n    # lccc-arg-ext a0/z16\n    ret\n";
        assert!(!peephole_optimize(asm.into()).contains("lccc-arg-ext"));
        assert!(!strip_internal_markers(asm.into()).contains("lccc-arg-ext"));
    }
}

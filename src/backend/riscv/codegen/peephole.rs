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
    // The prologue's return-register contract: read by the liveness, never
    // emitted.
    if trimmed.starts_with(RET_REGS_MARKER) {
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
        return crate::backend::peephole_common::strip_volatile_markers(asm);
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
            None => return crate::backend::peephole_common::strip_volatile_markers(asm),
        }
    } else {
        Vec::new()
    };
    // Functions holding inline assembly are opaque for the same reason: the
    // statement's operands are bound by constraints the text does not show.
    if has_inline_asm {
        match crate::backend::peephole_common::inline_asm_protected_ranges(&lines) {
            Some(asm_ranges) => ranges.extend(asm_ranges),
            None => return crate::backend::peephole_common::strip_volatile_markers(asm),
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
        changed |= eliminate_redundant_sext_w(&lines, &mut kinds, n);
        changed |= fold_zext32_into_sext_w(&lines, &mut kinds, n);
        changed |= eliminate_redundant_zext_byte(&mut lines, &mut kinds, n);
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
    global_changed |= fold_into_memory_operands(&mut lines, &mut kinds, n);
    global_changed |= fold_fp_bounces(&mut lines, &mut kinds, n);
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
            changed2 |= eliminate_redundant_sext_w(&lines, &mut kinds, n);
            changed2 |= fold_zext32_into_sext_w(&lines, &mut kinds, n);
            changed2 |= eliminate_redundant_zext_byte(&mut lines, &mut kinds, n);
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
            changed2 |= fold_into_memory_operands(&mut lines, &mut kinds, n);
            changed2 |= fold_fp_bounces(&mut lines, &mut kinds, n);
            rounds2 += 1;
        }
    }

    // Restore protected functions byte-for-byte, including their original -g
    // rows and CFI. No placeholder is permitted to survive the optimizer.
    for (i, original) in protected {
        lines[i] = original;
        kinds[i] = LineKind::Other;
    }
    let optimized = crate::backend::peephole_common::join_with_debug_rows(&lines[..n], |i| {
        kinds[i] != LineKind::Nop
    });
    crate::backend::peephole_common::strip_volatile_markers(optimized)
}

// ── Pass 1: Adjacent store/load elimination ──────────────────────────────────
//
// Pattern: sd/sw tX, off(s0)  ;  ld/lw tX, off(s0)  (same reg, same offset)
// The load is redundant because the value is already in the register.
// Also handles: sd rX, off(s0)  ;  ld rY, off(s0)  → mv rY, rX

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
                        if store_reg == load_reg {
                            // Same register: eliminate the load
                            kinds[j] = LineKind::Nop;
                            changed = true;
                        } else {
                            // Different register: replace load with mv
                            lines[j] =
                                format!("    mv {}, {}", reg_name(load_reg), reg_name(store_reg));
                            kinds[j] = LineKind::Move {
                                dst: load_reg,
                                src: store_reg,
                            };
                            changed = true;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    changed
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
        LineKind::LoadImm { dst } => Some((dst, true)), // li: sign-extended imm
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
        _ => None,
    }
}

fn eliminate_redundant_sext_w(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_SEXT_ELIM").is_ok() {
        return false;
    }
    let mut changed = false;
    // sext-wide set: bit per register id (0..=31 GPRs).
    let mut wide: u64 = 0;
    let mut i = 0;
    while i < n {
        match kinds[i] {
            LineKind::Label | LineKind::Branch | LineKind::Ret | LineKind::Jump => {
                wide = 0; // join point / boundary
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
            LineKind::Nop | LineKind::Directive => {
                i += 1;
                continue;
            }
            _ => {}
        }
        // sext.w candidate — ONLY the self form `sext.w rX, rX` may be
        // deleted outright: even when the operation is a value no-op, the
        // instruction DEFINES rD, and cross-register consumers (`bne t2, t1`
        // after `sext.w t2, t0`) read that definition — deleting the
        // cross-register form leaves them with a stale register
        // (cfg_phi_web miscompile class). Self-form deletion requires rX to
        // be provably sext-wide; the result is unchanged either way.
        if let LineKind::SextW { dst, src } = kinds[i] {
            if dst == src && wide & (1u64 << src) != 0 {
                kinds[i] = LineKind::Nop;
                changed = true;
                i += 1;
                continue;
            }
            wide |= 1u64 << dst; // sext.w always produces a sext-wide dest
            i += 1;
            continue;
        }
        // Def tracking.
        let (dst, producer_sext) = match sext_producing_def(&lines[i], kinds[i]) {
            Some(x) => x,
            None => {
                // Unknown line: clear nothing specific, be safe — clear all.
                wide = 0;
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

        // Canonical RV64 zero-extension: slli d,s,32; srli d,d,32.  If s is
        // already u32-or-narrower, preserve the destination definition with a
        // move (or delete both for the self form).
        if mnem == "slli" && ops.len() == 3 && parse_asm_i64(ops[2]) == Some(32) {
            let dst = parse_reg(ops[0]);
            let src = parse_reg(ops[1]);
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if dst != REG_NONE
                && src != REG_NONE
                && widths[src as usize] != 0
                && widths[src as usize] <= 32
                && j < n
            {
                let jt = lines[j].trim();
                let jops: Vec<&str> = jt
                    .strip_prefix("srli ")
                    .map(|s| s.split(',').map(str::trim).collect())
                    .unwrap_or_default();
                if jops.len() == 3
                    && parse_reg(jops[0]) == dst
                    && parse_reg(jops[1]) == dst
                    && parse_asm_i64(jops[2]) == Some(32)
                {
                    if dst == src {
                        kinds[i] = LineKind::Nop;
                    } else {
                        lines[i] = format!("    mv {}, {}", reg_name(dst), reg_name(src));
                        kinds[i] = LineKind::Move { dst, src };
                    }
                    kinds[j] = LineKind::Nop;
                    let src_width = widths[src as usize];
                    set_width(&mut widths, dst, src_width);
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
            if src != tmp
                || dst == tmp
                || dst == REG_S0
                || dst == REG_SP
                || dst == REG_RA
                || !scratch_dead_after(lines, kinds, j + 1, tmp, fend)
            {
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
                    if load_reg == mapping.reg {
                        // Same register: eliminate the redundant load
                        kinds[i] = LineKind::Nop;
                        changed = true;
                    } else {
                        // Different register: replace load with mv
                        lines[i] =
                            format!("    mv {}, {}", reg_name(load_reg), reg_name(mapping.reg));
                        kinds[i] = LineKind::Move {
                            dst: load_reg,
                            src: mapping.reg,
                        };
                        changed = true;
                    }
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

/// The prologue's `# LCCC_RET_REGS a0 a1|a0|-` line: the general registers
/// the function's `ret` hands back as its value.
const RET_REGS_MARKER: &str = "# LCCC_RET_REGS";

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
        // and the stack, and clobbers every caller-saved register.
        LineKind::Call => (
            (named & !rv_bit(REG_RA)) | RV_CALLER_SAVED & !rv_bit(REG_RA) | rv_bit(REG_SP),
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

/// Backward liveness of the tracked registers over one function's lines.
/// `None` when the CFG is not resolvable (an indirect `jr`, a branch to a
/// label this text does not define).
struct RvLiveness {
    /// `live_before[k - start]`: registers live immediately before line `k`.
    live_before: Vec<u64>,
    start: usize,
    end: usize,
}

impl RvLiveness {
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
        // What `ret` reads: the return-value registers the prologue
        // published (a0 and a1 when it said nothing), the link register and
        // every register the caller expects back.
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
        let ret_uses = ret_value | rv_bit(REG_RA) | RV_CALLEE_SAVED;
        let width = end - start;
        let mut live_before = vec![0u64; width];
        let mut live_in = vec![0u64; nb];
        loop {
            let mut changed = false;
            for b in (0..nb).rev() {
                let mut live = if exits[b] { RV_LIVE_ALL } else { 0 };
                for &s in &succs[b] {
                    live |= live_in[s];
                }
                // Falling off the end of the function's text: assume the
                // worst.
                if succs[b].is_empty() && !exits[b] && b + 1 == nb {
                    let mut t = bend(b);
                    while t > starts[b]
                        && matches!(kinds[t - 1], LineKind::Nop | LineKind::Directive)
                    {
                        t -= 1;
                    }
                    if t == starts[b] || kinds[t - 1] != LineKind::Ret {
                        live = RV_LIVE_ALL;
                    }
                }
                for k in (starts[b]..bend(b)).rev() {
                    let (u, d) = if kinds[k] == LineKind::Ret {
                        (ret_uses, 0)
                    } else {
                        rv_uses_defs(&lines[k], kinds[k])
                    };
                    live = (live & !d) | u;
                    live_before[k - start] = live;
                }
                if live != live_in[b] {
                    live_in[b] = live;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Some(RvLiveness {
            live_before,
            start,
            end,
        })
    }

    /// Registers live immediately after line `k`.
    fn live_after(&self, k: usize) -> u64 {
        if k + 1 >= self.end {
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
///   fmv.x.d t0, ft0 ; sd t0, 0(a0)         ->  fsd ft0, 0(a0)
///   fmv.x.d t0, ft0 ; fmv.d.x ft1, t0      ->  fmv.d ft1, ft0
/// ```
///
/// The temporary must die at the consumer.  A load may be sunk past up to
/// three FP-only lines (no general write, no memory write, no label), which
/// is how the emitter interleaves the two operand fetches.  `fmv.d` is
/// `fsgnj.d`: a bit-exact copy, NaN payload included.
fn fold_fp_bounces(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for (start, end) in rv_function_ranges(lines, kinds, n) {
        let Some(live) = RvLiveness::build(lines, kinds, start, end) else {
            continue;
        };
        for i in start..end {
            if kinds[i] == LineKind::Nop || lines[i] == OPAQUE_LINE {
                continue;
            }
            // fmv.x.{d,w} tA, fS ; (sd/sw tA, off(rB) | fmv.{d,w}.x fD, tA)
            if let Some(ta) = rv_fp_gp_dest(&lines[i])
                && let Some(rest) = lines[i].trim().strip_prefix("fmv.x.")
            {
                let width = &rest[..1];
                let fs = rest[1..].split(',').nth(1).map(str::trim).unwrap_or("");
                if !matches!(width, "d" | "w") || !fs.starts_with('f') {
                    continue;
                }
                let mut j = i + 1;
                while j < end && kinds[j] == LineKind::Nop {
                    j += 1;
                }
                if j >= end || lines[j] == OPAQUE_LINE {
                    continue;
                }
                if live.live_after(j) & rv_bit(ta) != 0 {
                    continue;
                }
                let ta_name = reg_name(ta);
                let store = match width {
                    "d" => "sd",
                    _ => "sw",
                };
                let new_line = if let Some((m, reg, off, base)) = rv_mem_parts(&lines[j])
                    && m == store
                    && reg == ta_name
                    && base != ta_name
                {
                    Some(format!("    fs{width} {fs}, {off}({base})"))
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
                    kinds[i] = LineKind::Nop;
                    kinds[j] = classify_line(&new_line);
                    lines[j] = new_line;
                    changed = true;
                }
                continue;
            }
            // l{d,w} tA, off(rB) ; [<= 3 FP-only lines] ; fmv.{d,w}.x fD, tA
            if let Some(ta) = rv_load_dest(&lines[i])
                && let Some((m, _, off, base)) = rv_mem_parts(&lines[i])
                && matches!(m, "ld" | "lw")
            {
                let width = if m == "ld" { "d" } else { "w" };
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
                    if let Some(ops) = t.strip_prefix(&format!("fmv.{width}.x "))
                        && let Some((fd, src)) = ops.split_once(',')
                        && src.trim() == ta_name
                    {
                        found = Some((j, fd.trim().to_string()));
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
                let Some((j, fd)) = found else {
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
        }
    }
    changed
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
    // Safety: if any instruction takes the address of s0 or uses s0 in a way
    // that could create pointers to stack slots (e.g., `addi xN, s0, off`
    // where xN is not s0 itself), we bail out to avoid eliminating stores
    // that might be accessed through pointers.
    for i in start..end {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        if lines[i] == OPAQUE_LINE {
            return false;
        }
        let trimmed = lines[i].trim();
        // Check for address-of-frame-pointer patterns
        if trimmed.starts_with("addi ") && trimmed.contains(", s0,") {
            // `addi s0, sp, N` is frame pointer setup — that's fine.
            // But `addi tX, s0, N` takes address of a stack slot.
            if let Some(dest) = parse_alu_dest(trimmed) {
                if dest != REG_S0 {
                    return false;
                }
            }
        }
        if trimmed.starts_with("mv ") && trimmed.contains(", s0") {
            if let Some(rest) = trimmed.strip_prefix("mv ") {
                if let Some((_, src)) = rest.split_once(", ") {
                    if src.trim() == "s0" {
                        // mv xN, s0 — copying frame pointer
                        return false;
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
            return false;
        }
    }

    // Phase 1: Collect all (offset, size) byte ranges that are loaded from.
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
                let load_size = match trimmed.split_whitespace().next().unwrap_or("") {
                    "ld" | "fld" => Some(8),
                    "lw" | "lwu" | "flw" => Some(4),
                    "lh" | "lhu" | "flh" => Some(2),
                    "lb" | "lbu" => Some(1),
                    "flq" => Some(16),
                    // A store writes the slot; it keeps no other store alive.
                    "sd" | "sw" | "sh" | "sb" | "fsd" | "fsw" | "fsh" | "fsq" => None,
                    // An atomic, a vector access or anything else unsized:
                    // keep every store.
                    _ => return false,
                };
                if let Some(sz) = load_size {
                    match extract_s0_offset_from_line(trimmed) {
                        Some(off) => loaded_ranges.push((off, sz)),
                        None => return false,
                    }
                }
            }
        }
    }

    // Phase 2: Remove stores whose byte range does not overlap any load range
    let mut changed = false;
    for i in start..end {
        if let LineKind::StoreS0 {
            offset, is_word, ..
        } = kinds[i]
        {
            let store_size = if is_word { 4 } else { 8 };
            let overlaps_any_load = loaded_ranges.iter().any(|&(load_off, load_sz)| {
                // Two ranges [a, a+as) and [b, b+bs) overlap iff a < b+bs && b < a+as
                offset < load_off + load_sz && load_off < offset + store_size
            });
            if !overlaps_any_load {
                kinds[i] = LineKind::Nop;
                changed = true;
            }
        }
    }
    changed
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
                "ld t0, 0(a0)",
                "add a2, a2, t0",
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
        let asm = "f:\n    # LCCC_RET_REGS a0\n    mv a1, t0\n    sext.w a0, t0\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["sext.w a0, t0", "ret"], "{out}");
        assert!(!out.contains("LCCC_RET_REGS"), "{out}");
        let asm = "f:\n    # LCCC_RET_REGS -\n    li a0, 3\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert_eq!(code_lines(&out), ["ret"], "{out}");
        let asm = "f:\n    mv a1, t0\n    sext.w a0, t0\n    ret\n";
        let out = peephole_optimize(asm.into());
        assert!(code_lines(&out).contains(&"mv a1, t0".to_string()), "{out}");
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

    #[test]
    fn retarget_refuses_across_a_label() {
        let out = run_retarget(
            "f:\n    addw t0, t1, a1\n    mv a0, t0\n.LBB9:\n    addw t0, t6, t6\n    ret\n",
        );
        assert!(out.contains("mv a0, t0"), "{out}");
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
        // Even a value no-op must survive: the instruction DEFINES t2, and
        // `bne t2, t1` below reads it (cfg_phi_web miscompile class).
        let input = "f:\n    li t0, 7\n    sext.w t2, t0\n    li t1, 0\n    bne t2, t1, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n";
        let out = peephole_optimize(input.to_string());
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
        for keep in [
            // A join between the pair and its reader.
            "f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n.L1:\n    sext.w a0, a0\n    ret\n",
            // The zero-extended value lives on in a0.
            "f:\n    slli a0, a0, 32\n    srli a0, a0, 32\n    sext.w a1, a0\n    ret\n",
            // Not a word zero extension.
            "f:\n    slli a0, a0, 31\n    srli a0, a0, 31\n    sext.w a0, a0\n    ret\n",
            "f:\n    slli a0, a1, 32\n    srli a0, a0, 32\n    sext.w a0, a0\n    ret\n",
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
        for (load, width) in [("flw fa1", 4), ("fld fa1", 8), ("flh fa1", 2)] {
            let asm = format!(
                "f:\n    ld t0, -64(s0)\n    sd t0, -72(s0)\n    {load}, -72(s0)\n    ret\n"
            );
            let out = code_lines(&peephole_optimize(asm.clone()));
            assert!(
                out.iter().any(|l| l == "sd t0, -72(s0)"),
                "{width}-byte FP reader lost its store:\n{asm} -> {out:?}"
            );
        }
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

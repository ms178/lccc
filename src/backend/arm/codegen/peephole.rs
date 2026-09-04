//! AArch64 peephole optimizer for assembly text.
//!
//! Operates on generated assembly text to eliminate redundant patterns from the
//! stack-based codegen. Lines are pre-parsed into `LineKind` enums so hot-path
//! pattern matching uses integer/enum comparisons instead of string parsing.
//!
//! ## Pass structure
//!
//! **Local passes** (iterative, up to 8 rounds): store/load elimination,
//! redundant branch removal, self-move elimination, move chain optimization,
//! branch-over-branch fusion, and move-immediate chain optimization.
//!
//! ## Optimizations
//!
//! 1. **Adjacent store/load elimination**: `str xN, [sp, #off]` followed by
//!    `ldr xN, [sp, #off]` — the load is redundant since the value is
//!    already in the register.
//!
//! 2. **Redundant branch elimination**: `b .LBBN` where `.LBBN:` is the
//!    immediately next non-empty line — falls through naturally.
//!
//! 3. **Self-move elimination**: `mov xN, xN` (64-bit) is a no-op.
//!    Note: `mov wN, wN` (32-bit) zeros upper 32 bits and is NOT eliminated.
//!
//! 4. **Move chain optimization**: `mov A, B; mov C, A` → `mov C, B`,
//!    enabling the first mov to become dead if A is unused.
//!
//! 5. **Branch-over-branch fusion**: `b.cc .Lskip; b .target; .Lskip:`
//!    → `b.!cc .target` (invert condition, eliminate skip label).
//!
//! 6. **Move-immediate chain**: `mov xN, #imm; mov xM, xN` where xN is a
//!    scratch register (x0-x15) → `mov xM, #imm` when safe.

// ── Line classification types ────────────────────────────────────────────────

// AUDITED ADOPTION of levkropp ARM peephole passes (session 30, commits
// aadab341/e20cdf6e/de451e57/b712c79c/89111a1f/8a052b9a/339cdb8b/69bbce24):
// forward_fp_slot_loads, fuse_fp_adjacent_pairs, eliminate_repeated_slot_loads,
// eliminate_overwritten_moves, fold_zext_move_chains, eliminate_redundant_sxtb,
// eliminate_overwritten_stores, sink_loop_carried_stores, and the GDSE rewrite
// (escape analysis for sp-derived bases + loop-variant base -> whole-frame
// reads, which also fixes the e03da2f iteration-0-offset unsoundness class).
// Every pass individually audited: movk excluded from MoveWide overwrites,
// store/branch/compare excluded from register-write helpers, gap hazards in
// ldp/stp fusion checked (partner-reg mention, dest write, base write), and
// store sinking requires call-free single-exit label-free regions with the
// stored register surviving to the backedge. Per-pass env gates retained for
// bisection: CCC_NO_FP_PAIR, CCC_NO_SLOT_LOAD_DEDUP, CCC_NO_FP_SLOT_FWD,
// CCC_NO_STORE_DSE, CCC_NO_STORE_SINK, CCC_NO_GDSE, CCC_NO_REUSE_STACK_LOADS.
use crate::common::fx_hash::{FxHashMap, FxHashSet};

/// Compact classification of an assembly line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    /// Deleted / blank
    Nop,
    /// `str xN/wN, [sp, #off]` — store to stack (via sp)
    StoreSp { reg: u8, offset: i32, is_word: bool },
    /// `ldr xN/wN, [sp, #off]` — load from stack (via sp)
    LoadSp { reg: u8, offset: i32, is_word: bool },
    /// `ldrsw xN, [sp, #off]` — load signed word from stack
    LoadswSp { reg: u8, offset: i32 },
    /// `ldrsb xN, [xM]` — load signed byte (general)
    LoadsbReg,
    /// `stp xN, xM, [sp, #off]` — store pair to stack
    StorePairSp,
    /// `ldp xN, xM, [sp, #off]` — load pair from stack
    LoadPairSp,
    /// `mov xN, xM` — register-to-register move.
    /// `is_32bit` indicates whether this is a w-register (32-bit) move.
    /// On AArch64, `mov wN, wM` zeros the upper 32 bits of the destination,
    /// so it is NOT equivalent to `mov xN, xM`.
    Move { dst: u8, src: u8, is_32bit: bool },
    /// `mov xN, #imm` — move immediate to register
    MoveImm { dst: u8 },
    /// `movz/movn xN, #imm` — move wide immediate
    MoveWide { dst: u8 },
    /// `sxtw xN, wM` — sign-extend word to doubleword
    Sxtw { dst: u8, src: u8 },
    /// `b .label` — unconditional branch
    Branch,
    /// `b.cc .label` — conditional branch
    CondBranch,
    /// `cbz/cbnz xN, .label` — compare and branch on (non-)zero
    CmpBranch,
    /// Label (`.LBBx:` etc.)
    Label,
    /// `ret`
    Ret,
    /// `bl func` — branch with link (function call)
    Call,
    /// `cmp` or `cmn` instruction
    Compare,
    /// `add`, `sub`, or other ALU instruction
    Alu,
    /// `str`/`ldr` to non-sp addresses (e.g., `str w1, [x9]`)
    MemOther,
    /// Assembler directive (`.section`, `.globl`, etc.)
    Directive,
    /// Any other instruction
    Other,
}

/// AArch64 register IDs for pattern matching.
/// We map x0-x30, sp, and w0-w30 to the same set (register number).
const REG_NONE: u8 = 255;

/// Parse an AArch64 register name to an internal ID (0-30, or special).
/// x0/w0 → 0, x1/w1 → 1, ..., x30/w30 → 30, sp → 31, xzr/wzr → 32.
fn parse_reg(name: &str) -> u8 {
    let name = name.trim();
    if let Some(n) = name.strip_prefix('x').or_else(|| name.strip_prefix('w')) {
        if let Ok(num) = n.parse::<u8>() {
            if num <= 30 {
                return num;
            }
        }
        if n == "zr" {
            return 32; // zero register
        }
    }
    if name == "sp" {
        return 31;
    }
    REG_NONE
}

// ── Implicit register operands (central oracle) ─────────────────────────────

/// One A64 instruction's implicit (reads, writes) over the family numbering
/// of `parse_reg` (0..=30 = x0..x30, 31 = sp).
///
/// AArch64 names almost every register it touches; exactly three effects are
/// invisible to a textual scan:
///   * `bl sym` links through x30 — a WRITE of family 30 with no `x30`
///     token anywhere in the text (`blr xN` likewise links x30, while
///     reading the branch-target register xN it names).
///   * `ret` returns through x30 — an implicit READ of family 30 (or of the
///     register the `ret xN` form names explicitly).
///   * pre-indexed (`[xN, #imm]!`) and post-indexed (`[xN], #imm`) memory
///     forms WRITE their base register after the access — the emitter
///     produces them (`strb w12, [x9], #1` in the byte-copy loops,
///     `ldp x29, x30, [sp], #frame` in every epilogue), but the base token
///     sits inside the address operand where every destination-oriented
///     scan ignores it.  The base read (addressing) is already visible as
///     an explicit mention; the WRITE is what passes miss.
/// Everything else (`svc`/`dmb`/`dsb`/`isb`/`msr`/`nop`/hint) touches no GP
/// register.  Unknown mnemonics yield (0, 0) — callers keep their own
/// conservative fallbacks for those, exactly as the x86-64 oracle's
/// consumers do.
fn classify_implicit_operands_a64(line: &str) -> (u64, u64) {
    const X30: u64 = 1 << 30;
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
        "bl" => (0, X30),
        "blr" => {
            let r = parse_reg(rest);
            let reads = if r <= 30 { 1u64 << r } else { 0 };
            (reads, X30)
        }
        "br" => {
            let r = parse_reg(rest);
            (if r <= 30 { 1u64 << r } else { 0 }, 0)
        }
        "ret" => {
            // Bare `ret` returns through the link register; `ret xN` names
            // its target explicitly (and is then not an implicit read).
            if rest.is_empty() {
                (X30, 0)
            } else {
                (0, 0)
            }
        }
        _ => {
            let writes = match writeback_base(rest) {
                Some(b) if b <= 31 => 1u64 << b,
                _ => 0,
            };
            (0, writes)
        }
    }
}

/// The base register of a pre-indexed (`…]!`) or post-indexed (`…], #imm`)
/// memory operand in the operand text `rest`, if any.
///
/// The closing bracket decides the form: `!` right after it is a pre-index,
/// a `,` right after it is a post-index; anything else leaves the base
/// untouched.  Only the FIRST bracketed address is examined — A64 addressing
/// modes never contain a second bracket pair, and `.cfi`/data directives do
/// not reach this parser (their lines are filtered by mnemonic above).
fn writeback_base(rest: &str) -> Option<u8> {
    let open = rest.find('[')?;
    let close = open + rest[open..].find(']')?;
    let after = rest[close + 1..].trim_start();
    if !(after.starts_with('!') || after.starts_with(',')) {
        return None;
    }
    let inner = &rest[open + 1..close];
    let base = inner.split(',').next()?.trim();
    match parse_reg(base) {
        REG_NONE => None,
        r => Some(r),
    }
}

/// True when the line implicitly writes family `num` (the write half of
/// [`classify_implicit_operands_a64`]).
fn implicitly_writes_a64(line: &str, num: u8) -> bool {
    num <= 31 && (classify_implicit_operands_a64(line).1 >> num) & 1 != 0
}

/// Return the x-register name for a given ID.
fn xreg_name(id: u8) -> &'static str {
    match id {
        0 => "x0",
        1 => "x1",
        2 => "x2",
        3 => "x3",
        4 => "x4",
        5 => "x5",
        6 => "x6",
        7 => "x7",
        8 => "x8",
        9 => "x9",
        10 => "x10",
        11 => "x11",
        12 => "x12",
        13 => "x13",
        14 => "x14",
        15 => "x15",
        16 => "x16",
        17 => "x17",
        18 => "x18",
        19 => "x19",
        20 => "x20",
        21 => "x21",
        22 => "x22",
        23 => "x23",
        24 => "x24",
        25 => "x25",
        26 => "x26",
        27 => "x27",
        28 => "x28",
        29 => "x29",
        30 => "x30",
        31 => "sp",
        32 => "xzr",
        _ => "??",
    }
}

/// Return the w-register name for a given ID.
fn wreg_name(id: u8) -> &'static str {
    match id {
        0 => "w0",
        1 => "w1",
        2 => "w2",
        3 => "w3",
        4 => "w4",
        5 => "w5",
        6 => "w6",
        7 => "w7",
        8 => "w8",
        9 => "w9",
        10 => "w10",
        11 => "w11",
        12 => "w12",
        13 => "w13",
        14 => "w14",
        15 => "w15",
        16 => "w16",
        17 => "w17",
        18 => "w18",
        19 => "w19",
        20 => "w20",
        21 => "w21",
        22 => "w22",
        23 => "w23",
        24 => "w24",
        25 => "w25",
        26 => "w26",
        27 => "w27",
        28 => "w28",
        29 => "w29",
        30 => "w30",
        32 => "wzr",
        _ => "??",
    }
}

// ── Line classification ──────────────────────────────────────────────────────

/// Classify a single assembly line into a LineKind.
fn classify_line(line: &str) -> LineKind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineKind::Nop;
    }

    // Labels end with ':'
    if trimmed.ends_with(':') {
        return LineKind::Label;
    }

    // Directives start with '.'
    if trimmed.starts_with('.') {
        return LineKind::Directive;
    }

    // str xN/wN, [sp, #off]
    if let Some(rest) = trimmed.strip_prefix("str ") {
        if let Some(info) = parse_sp_mem_op(rest) {
            return LineKind::StoreSp {
                reg: info.0,
                offset: info.1,
                is_word: info.2,
            };
        }
        return LineKind::MemOther;
    }

    // strb wN, [xM]  — byte store
    if trimmed.starts_with("strb ") || trimmed.starts_with("strh ") {
        return LineKind::MemOther;
    }

    // ldr xN/wN, [sp, #off]
    if let Some(rest) = trimmed.strip_prefix("ldr ") {
        if let Some(info) = parse_sp_mem_op(rest) {
            return LineKind::LoadSp {
                reg: info.0,
                offset: info.1,
                is_word: info.2,
            };
        }
        return LineKind::MemOther;
    }

    // ldrsw xN, [sp, #off]
    if let Some(rest) = trimmed.strip_prefix("ldrsw ") {
        if let Some((reg_str, addr)) = rest.split_once(", ") {
            let reg = parse_reg(reg_str.trim());
            if reg != REG_NONE {
                if let Some(offset) = parse_sp_offset(addr.trim()) {
                    return LineKind::LoadswSp { reg, offset };
                }
            }
        }
        return LineKind::MemOther;
    }

    // ldrsb
    if trimmed.starts_with("ldrsb ")
        || trimmed.starts_with("ldrsh ")
        || trimmed.starts_with("ldrb ")
        || trimmed.starts_with("ldrh ")
    {
        return LineKind::LoadsbReg;
    }

    // stp (store pair)
    if trimmed.starts_with("stp ") {
        if trimmed.contains("[sp") {
            return LineKind::StorePairSp;
        }
        return LineKind::MemOther;
    }

    // ldp (load pair)
    if trimmed.starts_with("ldp ") {
        if trimmed.contains("[sp") {
            return LineKind::LoadPairSp;
        }
        return LineKind::MemOther;
    }

    // mov xN, xM  or  mov xN, #imm  or  mov xN, :lo12:sym etc.
    if let Some(rest) = trimmed.strip_prefix("mov ") {
        // Avoid matching movz, movn, movk
        if !trimmed.starts_with("movz")
            && !trimmed.starts_with("movn")
            && !trimmed.starts_with("movk")
        {
            if let Some((dst_str, src_str)) = rest.split_once(", ") {
                let dst_trimmed = dst_str.trim();
                let dst = parse_reg(dst_trimmed);
                if dst != REG_NONE {
                    let src_trimmed = src_str.trim();
                    if src_trimmed.starts_with('#') || src_trimmed.starts_with('-') {
                        return LineKind::MoveImm { dst };
                    }
                    let src = parse_reg(src_trimmed);
                    if src != REG_NONE {
                        let is_32bit = dst_trimmed.starts_with('w');
                        return LineKind::Move { dst, src, is_32bit };
                    }
                    // mov xN, :lo12:symbol etc.
                    return LineKind::MoveImm { dst };
                }
            }
        }
    }

    // movz / movn / movk
    if trimmed.starts_with("movz ") || trimmed.starts_with("movn ") {
        if let Some((_, rest)) = trimmed.split_once(' ') {
            if let Some((dst_str, _)) = rest.split_once(", ") {
                let dst = parse_reg(dst_str.trim());
                if dst != REG_NONE {
                    return LineKind::MoveWide { dst };
                }
            }
        }
        return LineKind::Other;
    }

    // movk is an update, not a fresh definition
    if trimmed.starts_with("movk ") {
        return LineKind::Other;
    }

    // sxtw xN, wM
    if let Some(rest) = trimmed.strip_prefix("sxtw ") {
        if let Some((dst_str, src_str)) = rest.split_once(", ") {
            let dst = parse_reg(dst_str.trim());
            let src = parse_reg(src_str.trim());
            if dst != REG_NONE && src != REG_NONE {
                return LineKind::Sxtw { dst, src };
            }
        }
    }

    // Unconditional branch: b .label (but not bl, b.cc)
    if trimmed.starts_with("b ") && !trimmed.starts_with("bl ") && !trimmed.starts_with("b.") {
        return LineKind::Branch;
    }

    // Conditional branch: b.eq, b.ne, b.lt, b.ge, b.gt, b.le, b.hi, b.ls, b.cs, b.cc, etc.
    if trimmed.starts_with("b.") {
        return LineKind::CondBranch;
    }

    // cbz/cbnz/tbz/tbnz
    if trimmed.starts_with("cbz ")
        || trimmed.starts_with("cbnz ")
        || trimmed.starts_with("tbz ")
        || trimmed.starts_with("tbnz ")
    {
        return LineKind::CmpBranch;
    }

    // ret
    if trimmed == "ret" {
        return LineKind::Ret;
    }

    // bl (branch and link = call)
    if trimmed.starts_with("bl ") || trimmed.starts_with("blr ") {
        return LineKind::Call;
    }

    // br xN (indirect branch = control flow barrier)
    if trimmed.starts_with("br ") {
        return LineKind::Branch;
    }

    // cmp/cmn
    if trimmed.starts_with("cmp ") || trimmed.starts_with("cmn ") {
        return LineKind::Compare;
    }

    // ALU: add, sub, and, orr, eor, lsl, lsr, asr, mul, etc.
    if trimmed.starts_with("add ")
        || trimmed.starts_with("sub ")
        || trimmed.starts_with("and ")
        || trimmed.starts_with("orr ")
        || trimmed.starts_with("eor ")
        || trimmed.starts_with("mul ")
        || trimmed.starts_with("neg ")
        || trimmed.starts_with("mvn ")
        || trimmed.starts_with("lsl ")
        || trimmed.starts_with("lsr ")
        || trimmed.starts_with("asr ")
        || trimmed.starts_with("madd ")
        || trimmed.starts_with("msub ")
        || trimmed.starts_with("sdiv ")
        || trimmed.starts_with("udiv ")
        || trimmed.starts_with("adds ")
        || trimmed.starts_with("subs ")
    {
        return LineKind::Alu;
    }

    LineKind::Other
}

/// Parse `xN/wN, [sp, #off]` and return (reg_id, offset, is_word).
fn parse_sp_mem_op(rest: &str) -> Option<(u8, i32, bool)> {
    let (reg_str, addr) = rest.split_once(", ")?;
    let reg_str = reg_str.trim();
    let is_word = reg_str.starts_with('w');
    let reg = parse_reg(reg_str);
    if reg == REG_NONE {
        return None;
    }
    let offset = parse_sp_offset(addr.trim())?;
    Some((reg, offset, is_word))
}

/// Parse `[sp, #off]` or `[sp]` and return the offset.
fn parse_sp_offset(addr: &str) -> Option<i32> {
    // [sp] — zero offset
    if addr == "[sp]" {
        return Some(0);
    }
    // [sp, #N] or [sp, #-N]
    if addr.starts_with("[sp, #") && addr.ends_with(']') {
        let inner = &addr[6..addr.len() - 1]; // strip "[sp, #" and "]"
        return inner.parse::<i32>().ok();
    }
    // [sp, #N]! (pre-index) — not a simple stack slot access
    None
}

/// Extract branch target from `b .label` or `b label`.
fn branch_target(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    // Match "b .label" but not "br xN" (indirect branch)
    if let Some(rest) = trimmed.strip_prefix("b ") {
        let target = rest.trim();
        // Must start with '.' (a label), not a register
        if target.starts_with('.') {
            return Some(target);
        }
    }
    None
}

/// Extract the condition code and target from a conditional branch.
/// `b.eq .label` → Some(("eq", ".label"))
fn cond_branch_parts(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("b.") {
        if let Some((cc, target)) = rest.split_once(' ') {
            return Some((cc, target.trim()));
        }
    }
    None
}

/// Invert a condition code.
fn invert_condition(cc: &str) -> Option<&'static str> {
    match cc {
        "eq" => Some("ne"),
        "ne" => Some("eq"),
        "lt" => Some("ge"),
        "ge" => Some("lt"),
        "gt" => Some("le"),
        "le" => Some("gt"),
        "hi" => Some("ls"),
        "ls" => Some("hi"),
        "hs" | "cs" => Some("lo"),
        "lo" | "cc" => Some("hs"),
        "mi" => Some("pl"),
        "pl" => Some("mi"),
        "vs" => Some("vc"),
        "vc" => Some("vs"),
        _ => None,
    }
}

/// Extract label name from a label line (strip trailing `:`)
fn label_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed.strip_suffix(':')
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Run peephole optimization on AArch64 assembly text.
/// Returns the optimized assembly string.
pub fn peephole_optimize(asm: String) -> String {
    if std::env::var("CCC_NO_PEEPHOLE").is_ok() {
        return asm;
    }
    let mut lines: Vec<String> = asm.lines().map(String::from).collect();
    let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
    let n = lines.len();

    if n == 0 {
        return asm;
    }

    // Phase 1: Iterative local passes (up to 8 rounds)
    let rotate = std::env::var("CCC_NO_LOOP_ROTATE").is_err();
    let mut changed = true;
    let mut rounds = 0;
    while changed && rounds < 8 {
        changed = false;
        changed |= eliminate_adjacent_store_load(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_branches(&lines, &mut kinds, n);
        changed |= eliminate_self_moves(&mut kinds, n);
        changed |= eliminate_redundant_sxtw(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_sxtb(&mut lines, &mut kinds, n);
        changed |= eliminate_redundant_zext_and(&mut lines, &mut kinds, n);
        changed |= fuse_and_cmp_zero(&mut lines, &mut kinds, n);
        changed |= eliminate_dead_pre_ret_moves(&mut lines, &mut kinds, n);
        changed |= eliminate_move_chains(&mut lines, &mut kinds, n);
        changed |= fold_zext_move_chains(&mut lines, &mut kinds, n);
        changed |= eliminate_unused_x9_address_moves(&lines, &mut kinds, n);
        changed |= propagate_address_aliases(&mut lines, &mut kinds, n);
        changed |= forward_fp_slot_loads(&mut lines, &mut kinds, n);
        changed |= fuse_fp_adjacent_pairs(&mut lines, &mut kinds, n);
        changed |= eliminate_repeated_slot_loads(&mut lines, &mut kinds, n);
        changed |= thread_spill_slots(&mut lines, &mut kinds, n);
        changed |= hoist_loop_invariant_remats(&mut lines, &mut kinds, n);
        let n = lines.len();
        changed |= cse_adrp_remat_pairs(&mut lines, &mut kinds, n);
        changed |= fold_destructive_pointer_updates(&mut lines, &mut kinds, n);
        changed |= fold_zero_stores(&mut lines, &mut kinds, n);
        changed |= fuse_branch_over_branch(&mut lines, &mut kinds, n);
        // After the folds: dead staging movs (their patterns must win first).
        changed |= eliminate_overwritten_moves(&lines, &mut kinds, n);
        changed |= eliminate_overwritten_stores(&lines, &mut kinds, n);
        if rotate {
            changed |= rotate_simple_loops(&mut lines, &mut kinds, n);
        }
        rounds += 1;
    }

    // Phase 2: Global passes
    //
    // Global store forwarding is disabled: same-register NOP elimination has a
    // remaining correctness bug in complex float-array code (test 0036_0041).
    // The root cause is not yet identified — it appears correct within a basic
    // block but produces wrong output. Until this is fixed, GSF is skipped.
    //
    // Copy propagation and dead store elimination are independent and safe.
    propagate_register_copies(&mut lines, &mut kinds, n);
    reuse_stack_loads_within_blocks(&mut lines, &mut kinds, n);
    propagate_register_copies(&mut lines, &mut kinds, n);
    eliminate_dead_call_staging_moves(&lines, &mut kinds, n);
    if std::env::var("CCC_NO_GDSE").is_err() {
        global_dead_store_elimination(&lines, &mut kinds, n);
    }
    // After rotation (phase 1) and dead-store cleanup: sink loop-carried
    // slot-home stores out of bottom-tested loops.
    sink_loop_carried_stores(&mut lines, &mut kinds, n);

    // Phase 3: Local cleanup after global passes (up to 4 rounds)
    {
        let mut changed2 = true;
        let mut rounds2 = 0;
        while changed2 && rounds2 < 4 {
            changed2 = false;
            changed2 |= eliminate_adjacent_store_load(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_branches(&lines, &mut kinds, n);
            changed2 |= eliminate_self_moves(&mut kinds, n);
            changed2 |= eliminate_redundant_sxtw(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_sxtb(&mut lines, &mut kinds, n);
            changed2 |= eliminate_redundant_zext_and(&mut lines, &mut kinds, n);
            changed2 |= fuse_and_cmp_zero(&mut lines, &mut kinds, n);
            changed2 |= eliminate_dead_pre_ret_moves(&mut lines, &mut kinds, n);
            changed2 |= eliminate_move_chains(&mut lines, &mut kinds, n);
            changed2 |= fold_zext_move_chains(&mut lines, &mut kinds, n);
            changed2 |= eliminate_unused_x9_address_moves(&lines, &mut kinds, n);
            changed2 |= propagate_address_aliases(&mut lines, &mut kinds, n);
            changed2 |= forward_fp_slot_loads(&mut lines, &mut kinds, n);
            changed2 |= fuse_fp_adjacent_pairs(&mut lines, &mut kinds, n);
            changed2 |= eliminate_repeated_slot_loads(&mut lines, &mut kinds, n);
            changed2 |= thread_spill_slots(&mut lines, &mut kinds, n);
            changed2 |= hoist_loop_invariant_remats(&mut lines, &mut kinds, n);
            let n = lines.len();
            changed2 |= cse_adrp_remat_pairs(&mut lines, &mut kinds, n);
            changed2 |= fold_destructive_pointer_updates(&mut lines, &mut kinds, n);
            changed2 |= fold_zero_stores(&mut lines, &mut kinds, n);
            changed2 |= eliminate_overwritten_moves(&lines, &mut kinds, n);
            changed2 |= eliminate_overwritten_stores(&lines, &mut kinds, n);
            rounds2 += 1;
        }
    }

    // Build result, filtering out Nop lines
    let mut result = String::with_capacity(asm.len());
    for i in 0..n {
        if kinds[i] != LineKind::Nop {
            result.push_str(&lines[i]);
            result.push('\n');
        }
    }
    result
}

/// Propagate a register copy used only as a memory-address alias within one
/// basic block. This handles vector intrinsics where an SSA Copy gives the C
/// pointer a short-lived register solely for `[xN]` loads/stores.
///
/// Soundness: the defining `mov` may only be deleted when the copy's value is
/// provably dead after the rewritten window — every mention of `dst` from here
/// to the end of the function's text must be accounted for. An earlier version
/// stopped the scan at the first block boundary and deleted the mov anyway,
/// which silently dropped the initializer of any alias used in a later block
/// (exposed when global-address CSE made such aliases multi-use). We therefore
/// scan to the end of the function and commit only on proof of death:
/// overwrite of `dst` by an instruction that does not read it, or the end of
/// the function's text. Anything unrecognized (other memory ops, unknown
/// instruction kinds, calls) blocks the transform.
/// Column-0 identifier label: the next function or data object, i.e. the
/// end of the current function's text. Block labels start with '.', and
/// inline-asm numeric local labels (`1:`) start with a digit — neither
/// terminates the function's text.
fn is_function_boundary(line: &str) -> bool {
    line.ends_with(':') && line.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
}

fn propagate_address_aliases(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    fn mentions(line: &str, reg: u8) -> bool {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|tok| tok == xreg_name(reg) || tok == wreg_name(reg))
    }
    fn mention_count(line: &str, reg: u8) -> usize {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|tok| *tok == xreg_name(reg) || *tok == wreg_name(reg))
            .count()
    }
    let mut changed = false;
    for i in 0..n {
        let LineKind::Move {
            dst,
            src,
            is_32bit: false,
        } = kinds[i]
        else {
            continue;
        };
        if dst == src || dst >= 29 || src >= 29 {
            continue;
        }
        let mut uses = Vec::new();
        let mut valid = true;
        let mut provably_dead = false;
        let mut src_alive = true;
        let mut j = i + 1;
        while j < n {
            if is_function_boundary(&lines[j]) {
                provably_dead = true;
                break;
            }
            match kinds[j] {
                // No runtime effect. A return ends this path; textually later
                // lines belong to other paths, so keep scanning.
                //
                // Note: an epilogue's `ldp xD, ...` callee-save restore is NOT
                // treated as proof of death here — it ends the value only on
                // that return path, and textually later blocks (other paths)
                // may still use it, so it falls through to the plain-mention
                // handling and blocks the transform.
                LineKind::Nop | LineKind::Directive | LineKind::Label | LineKind::Ret => {}
                // Calls clobber src (x0) and all caller-saved registers. Rather
                // than reason about save/restore pairs, stop without committing.
                LineKind::Call => {
                    valid = false;
                    break;
                }
                _ => {
                    // Kinds whose destination register written_gp_register
                    // models precisely.
                    let trusted = matches!(
                        kinds[j],
                        LineKind::Move { .. }
                            | LineKind::MoveImm { .. }
                            | LineKind::MoveWide { .. }
                            | LineKind::Sxtw { .. }
                            | LineKind::LoadSp { .. }
                            | LineKind::LoadswSp { .. }
                            | LineKind::Alu
                            | LineKind::Compare
                            | LineKind::Branch
                            | LineKind::CondBranch
                            | LineKind::CmpBranch
                            | LineKind::StoreSp { .. }
                    );
                    let written = written_gp_register(&lines[j], kinds[j]);
                    if (trusted && written == Some(src)) || (!trusted && mentions(&lines[j], src)) {
                        src_alive = false;
                    }
                    if mentions(&lines[j], dst) {
                        if trusted && written == Some(dst) && mention_count(&lines[j], dst) == 1 {
                            // dst overwritten without being read: the mov's
                            // value is dead past this instruction (register
                            // assignments are per-value with disjoint live
                            // intervals, so no later text can use the old value).
                            provably_dead = true;
                            break;
                        }
                        // A pre/post-indexed writeback WRITES the base:
                        // renaming this address use onto src would redirect
                        // the post-increment onto the alias source and
                        // corrupt it (`mov x1,x5; str x9,[x1],#8` must not
                        // become `str x9,[x5],#8`).
                        if implicitly_writes_a64(&lines[j], dst) {
                            valid = false;
                            break;
                        }
                        let address = format!("[{}", xreg_name(dst));
                        if src_alive
                            && mention_count(&lines[j], dst) == 1
                            && lines[j].contains(&address)
                        {
                            // dst used purely as the address base of a
                            // load/store while src is alive: rewritable.
                            uses.push(j);
                        } else {
                            // Non-address use, an unmodelled effect, or an
                            // address use after src died: the mov must stay.
                            valid = false;
                            break;
                        }
                    }
                }
            }
            j += 1;
        }
        if j == n {
            provably_dead = true; // scanned to the end of the file
        }
        if valid && provably_dead && !uses.is_empty() {
            for use_idx in uses {
                lines[use_idx] =
                    replace_whole_word(&lines[use_idx], xreg_name(dst), xreg_name(src));
                kinds[use_idx] = classify_line(&lines[use_idx]);
            }
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

/// Fold an SSA pointer update that is copied back to its loop-carried register:
/// `mov x0,xS; add x0,xS,#K; mov xT,x0; ...; mov xS,xT` → `add xS,xS,#K`.
/// The scan proves neither xS nor xT is referenced in the intervening region.
fn fold_destructive_pointer_updates(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    fn mentions(line: &str, reg: u8) -> bool {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|tok| tok == xreg_name(reg) || tok == wreg_name(reg))
    }
    let mut changed = false;
    for i in 0..n.saturating_sub(3) {
        let LineKind::Move {
            dst: 0,
            src,
            is_32bit: false,
        } = kinds[i]
        else {
            continue;
        };
        if src == 0 || src >= 29 {
            continue;
        }
        let mut j = i + 1;
        while j < n && kinds[j] == LineKind::Nop {
            j += 1;
        }
        if j >= n || kinds[j] != LineKind::Alu {
            continue;
        }
        let expected = format!("add x0, {}, #", xreg_name(src));
        let trimmed = lines[j].trim();
        if !trimmed.starts_with(&expected) {
            continue;
        }
        let immediate = match trimmed
            .rsplit_once('#')
            .and_then(|(_, v)| v.parse::<i64>().ok())
        {
            Some(v) if (0..=4095).contains(&v) => v,
            _ => continue,
        };
        let mut k = j + 1;
        while k < n && kinds[k] == LineKind::Nop {
            k += 1;
        }
        let temp = match kinds.get(k).copied() {
            Some(LineKind::Move {
                dst,
                src: 0,
                is_32bit: false,
            }) if dst != src => dst,
            _ => continue,
        };
        let mut end = k + 1;
        let mut valid = true;
        let mut found = None;
        while end < n && end <= k + 16 {
            if matches!(
                kinds[end],
                LineKind::Label
                    | LineKind::Branch
                    | LineKind::CondBranch
                    | LineKind::CmpBranch
                    | LineKind::Call
                    | LineKind::Ret
            ) {
                break;
            }
            if let LineKind::Move {
                dst,
                src: move_src,
                is_32bit: false,
            } = kinds[end]
            {
                if dst == src && move_src == temp {
                    found = Some(end);
                    break;
                }
            }
            if mentions(&lines[end], src) || mentions(&lines[end], temp) {
                valid = false;
                break;
            }
            end += 1;
        }
        let Some(end) = found else { continue };
        if !valid {
            continue;
        }
        lines[i] = format!(
            "    add {}, {}, #{}",
            xreg_name(src),
            xreg_name(src),
            immediate
        );
        kinds[i] = LineKind::Alu;
        kinds[j] = LineKind::Nop;
        kinds[k] = LineKind::Nop;
        kinds[end] = LineKind::Nop;
        changed = true;
    }
    changed
}

/// Reuse a value already copied out of a stack slot inside one basic block.
///
/// Register-pressure-heavy code commonly contains
/// `ldr x0, [sp,#N]; mov wD,w0` several times for the same spill.  Once the
/// first copy is in a register, later reloads can source that register until
/// either it or the slot is overwritten.  Keep this deliberately local and
/// clear state for instruction classes whose register effects are ambiguous.
fn reuse_stack_loads_within_blocks(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_REUSE_STACK_LOADS").is_ok() {
        return false;
    }
    let mut cached: Vec<(i32, bool, u8, bool)> = Vec::new();
    let mut changed = false;
    let mut i = 0;
    let mut block_safe = false;

    while i < n {
        if kinds[i] == LineKind::Label {
            let mut j = i + 1;
            block_safe = true;
            while j < n && kinds[j] != LineKind::Label {
                if matches!(
                    kinds[j],
                    LineKind::MemOther
                        | LineKind::Other
                        | LineKind::StorePairSp
                        | LineKind::LoadPairSp
                        | LineKind::LoadsbReg
                        | LineKind::Call
                ) {
                    block_safe = false;
                    break;
                }
                j += 1;
            }
        }
        match kinds[i] {
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret
            | LineKind::MemOther
            | LineKind::Other
            | LineKind::StorePairSp
            | LineKind::LoadPairSp
            | LineKind::LoadsbReg => cached.clear(),
            LineKind::StoreSp { offset, .. } => cached.retain(|(off, _, _, _)| *off != offset),
            _ => {}
        }

        if !block_safe {
            i += 1;
            continue;
        }

        let (load_reg, offset, is_word) = match kinds[i] {
            LineKind::LoadSp {
                reg,
                offset,
                is_word,
            } => (reg, offset, is_word),
            _ => {
                if let Some(dst) = written_gp_register(&lines[i], kinds[i]) {
                    cached.retain(|(_, _, reg, _)| *reg != dst);
                }
                i += 1;
                continue;
            }
        };

        let old = cached
            .iter()
            .find(|(off, word, reg, _)| *off == offset && *word == is_word && *reg != load_reg)
            .copied();
        cached.retain(|(_, _, reg, _)| *reg != load_reg);

        let mut j = i + 1;
        while j < n && kinds[j] == LineKind::Nop {
            j += 1;
        }
        if j < n {
            if let LineKind::Move { dst, src, is_32bit } = kinds[j] {
                if src == load_reg {
                    if let Some((_, _, source, source_word)) =
                        old.filter(|entry| entry.3 == is_32bit)
                    {
                        // NOPing the load is only sound if its register has no
                        // other readers before it is next fully overwritten —
                        // copy propagation (which runs BEFORE this pass) can
                        // move an instruction's reference onto the load's own
                        // register (rewriting `add x1, x5, x2` to read the
                        // copy source), and deleting the load would leave
                        // that read with stale contents. Audited adoption of
                        // levkropp f958b6a fix (e) (strlen_bench segfault on
                        // his tree; same pass order hazard exists here).
                        let xn = xreg_name(load_reg);
                        let wn = wreg_name(load_reg);
                        let mut load_dead_after = false;
                        let mut k = j + 1;
                        while k < n {
                            if kinds[k] == LineKind::Nop {
                                k += 1;
                                continue;
                            }
                            let t = &lines[k];
                            let mentions = t
                                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                                .any(|tok| tok == xn || tok == wn);
                            if mentions {
                                // A pure overwrite ends the hazard; anything
                                // else reads the (soon deleted) register.
                                load_dead_after = match kinds[k] {
                                    LineKind::Move { dst, src, .. } => {
                                        dst == load_reg && src != load_reg
                                    }
                                    LineKind::MoveImm { dst } | LineKind::MoveWide { dst } => {
                                        dst == load_reg
                                    }
                                    LineKind::LoadSp { reg, .. }
                                    | LineKind::LoadswSp { reg, .. } => reg == load_reg,
                                    _ => false,
                                };
                                break;
                            }
                            match kinds[k] {
                                LineKind::Label
                                | LineKind::Branch
                                | LineKind::CondBranch
                                | LineKind::CmpBranch
                                | LineKind::Call
                                | LineKind::Ret
                                | LineKind::Directive => break,
                                _ => {}
                            }
                            k += 1;
                        }
                        if !load_dead_after {
                            i += 1;
                            continue;
                        }
                        kinds[i] = LineKind::Nop;
                        lines[j] = format!(
                            "    mov {}, {}",
                            if is_32bit {
                                wreg_name(dst)
                            } else {
                                xreg_name(dst)
                            },
                            if source_word {
                                wreg_name(source)
                            } else {
                                xreg_name(source)
                            }
                        );
                        kinds[j] = LineKind::Move {
                            dst,
                            src: source,
                            is_32bit,
                        };
                        changed = true;
                    }
                    cached.retain(|(_, _, reg, _)| *reg != dst);
                    cached.retain(|(off, word, _, _)| *off != offset || *word != is_word);
                    cached.push((offset, is_word, dst, is_32bit));
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    changed
}

/// Return the GP register written by instruction classes whose first operand
/// is unambiguously a destination. Unknown classes are handled by clearing the
/// cache in the caller.
fn written_gp_register(line: &str, kind: LineKind) -> Option<u8> {
    match kind {
        LineKind::Move { dst, .. }
        | LineKind::MoveImm { dst }
        | LineKind::MoveWide { dst }
        | LineKind::Sxtw { dst, .. }
        | LineKind::LoadSp { reg: dst, .. }
        | LineKind::LoadswSp { reg: dst, .. } => Some(dst),
        LineKind::Alu => {
            let operands = line.trim().split_once(' ')?.1;
            let dst = operands.split(',').next()?;
            let reg = parse_reg(dst);
            (reg != REG_NONE).then_some(reg)
        }
        _ => None,
    }
}

/// Remove `mov x9, xN` when x9's staged address is provably dead after the
/// mov. Regalloc-aware loads and copy propagation can make the legacy x9
/// setup dead.
///
/// Soundness requires a FULL dead-value scan, not a single-instruction
/// lookahead: `propagate_register_copies` rewrites only the FIRST use of the
/// copy (`ldr x0, [x9]` -> `ldr x0, [x0]`), so the instruction after the mov
/// can look clean while a later instruction (`ldr x1, [x9, #8]` — the high
/// half of a 16-byte _Float128-carrier global load) still reads x9. Deleting
/// the mov on the one-line view left a stale-x9 SIGSEGV (f128_softfloat).
/// The scan stops at the first x9 read (mov stays), the first pure x9
/// redefinition (mov is dead past it), a control-flow boundary, a call (x9
/// is caller-saved: a later read would need a fresh definition), or the
/// end of the function.
fn eliminate_unused_x9_address_moves(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    fn mentions_x9(line: &str) -> bool {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|tok| tok == "x9" || tok == "w9")
    }
    fn mentions_x9_count(line: &str) -> usize {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|&tok| tok == "x9" || tok == "w9")
            .count()
    }
    let mut changed = false;
    for i in 0..n {
        let LineKind::Move {
            dst: 9,
            is_32bit: false,
            ..
        } = kinds[i]
        else {
            continue;
        };
        let mut dead = true;
        let mut j = i + 1;
        while j < n {
            if is_function_boundary(&lines[j]) {
                break;
            }
            match kinds[j] {
                LineKind::Nop | LineKind::Directive => {}
                // Path ends: textually later lines belong to other paths, and
                // a label can only be joined from before the mov (skipping it).
                LineKind::Label
                | LineKind::Branch
                | LineKind::CondBranch
                | LineKind::CmpBranch
                | LineKind::Ret
                | LineKind::Call => break,
                _ => {
                    if written_gp_register(&lines[j], kinds[j]) == Some(9)
                        && mentions_x9_count(&lines[j]) == 1
                    {
                        // Pure redefinition of x9: the staged value is dead
                        // past this instruction.
                        break;
                    }
                    if mentions_x9(&lines[j]) {
                        // A read (or read-modify-write) of x9: the mov is live.
                        dead = false;
                        break;
                    }
                }
            }
            j += 1;
        }
        if dead {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

/// Remove a move into an AArch64 caller-saved staging register when the value
/// is never referenced before the next call clobbers it. Copy propagation
/// commonly turns
/// `mov x9, x24; mov x0, x9; bl f` into
/// `mov x9, x24; mov x0, x24; bl f`, leaving the first move dead.
fn eliminate_dead_call_staging_moves(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    fn mentions_reg(line: &str, reg: u8) -> bool {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|token| token == xreg_name(reg) || token == wreg_name(reg))
    }

    let mut changed = false;
    for i in 0..n {
        let dst = match kinds[i] {
            LineKind::Move { dst, .. } if (9..=15).contains(&dst) => dst,
            _ => continue,
        };
        let mut j = i + 1;
        let mut used = false;
        let mut reaches_call = false;
        while j < n {
            match kinds[j] {
                LineKind::Nop | LineKind::Directive => {}
                LineKind::Call => {
                    reaches_call = true;
                    break;
                }
                LineKind::Label
                | LineKind::Branch
                | LineKind::CondBranch
                | LineKind::CmpBranch
                | LineKind::Ret => break,
                _ => {
                    if mentions_reg(&lines[j], dst) {
                        used = true;
                        break;
                    }
                }
            }
            j += 1;
        }
        if reaches_call && !used {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

// ── Pass 1: Adjacent store/load elimination ──────────────────────────────────
//
// Pattern: str xN, [sp, #off]  →  ldr xN, [sp, #off]  (same reg, same offset)
// The load is redundant since the value is already in the register.
// Also: str xN, [sp, #off]  →  ldr xM, [sp, #off]  → replace load with mov xM, xN
// Also handles: str wN, [sp, #off]  →  ldrsw xN, [sp, #off]

// ── FP spill slot load forwarding ────────────────────────────────────────────
//
// Pattern: `str dS, [sp, #off]` (or via a materialized `sp`-derived base
// register) followed by `ldr dD, [sp, #off]` in the same block. The slot-home
// emission produces these store/reload round-trips for unallocated FP
// temporaries; the load is replaceable with `fmov dD, dS` (or deleted when
// dD == dS) as long as neither the slot nor dS is clobbered in between.
//
// Tracking is deliberately conservative: any store through an unknown base,
// any call, and any control-flow boundary drops all state, and any write of
// a source register drops the entries that forward from it.

/// Parse an FP register name token (`d7`, `s3`, `q16`, `v2`, `v2.2d`) to its
/// register number.
fn fp_token_reg(tok: &str) -> Option<u8> {
    let tok = tok.trim();
    let digits = match tok.as_bytes().first() {
        Some(b'd') | Some(b's') | Some(b'q') | Some(b'v') => {
            let mut end = 1;
            while end < tok.len() && tok.as_bytes()[end].is_ascii_digit() {
                end += 1;
            }
            if end == 1 {
                return None;
            }
            &tok[1..end]
        }
        _ => return None,
    };
    digits.parse::<u8>().ok().filter(|&r| r <= 31)
}

/// An FP memory operand location: direct `[sp, #off]` slot, or a register
/// base with a byte offset (`[xN]` / `[xN, #k]`).
#[derive(Clone, Copy)]
enum FpAddr {
    Sp(i32),
    Base(u8, i32),
}

/// Parse `str dS, ...` / `ldr dD, ...` (d/s/q widths) into (reg, FpAddr).
fn parse_fp_mem(line: &str) -> Option<(bool, u8, FpAddr, u8)> {
    let t = line.trim();
    let (is_store, rest) = if let Some(r) = t.strip_prefix("str ") {
        (true, r)
    } else if let Some(r) = t.strip_prefix("ldr ") {
        (false, r)
    } else {
        return None;
    };
    let (reg_str, addr) = rest.split_once(", ")?;
    let reg_str = reg_str.trim();
    let width = reg_str.as_bytes().first().copied()?;
    if !matches!(width, b'd' | b's' | b'q') {
        return None;
    }
    let reg = fp_token_reg(reg_str)?;
    let addr = addr.trim();
    if let Some(off) = parse_sp_offset(addr) {
        return Some((is_store, reg, FpAddr::Sp(off), width));
    }
    if addr.starts_with("[x") && addr.ends_with(']') && !addr.contains('!') {
        let inner = &addr[1..addr.len() - 1];
        let (base, off) = match inner.split_once(", ") {
            Some((b, o)) => (b.trim(), o.trim().strip_prefix('#')?.parse::<i32>().ok()?),
            None => (inner.trim(), 0),
        };
        let base_reg = parse_reg(base);
        if base_reg < 29 {
            return Some((is_store, reg, FpAddr::Base(base_reg, off), width));
        }
    }
    None
}

/// Width in bytes of an FP spill slot access kind.
fn fp_access_width(width: u8) -> i32 {
    match width {
        b's' => 4,
        b'q' => 16,
        _ => 8, // d
    }
}

/// Does this line write FP register `num` (any d/s/q/v name form) as its
/// destination? Store/branch/compare mnemonics never write a register.
fn fp_reg_written(line: &str, num: u8) -> bool {
    let t = line.trim();
    let Some((mnem, rest)) = t.split_once(' ') else {
        return false;
    };
    if matches!(
        mnem,
        "str"
            | "stp"
            | "strb"
            | "strh"
            | "stur"
            | "sturb"
            | "sturh"
            | "stlr"
            | "stlrb"
            | "stlrh"
            | "fcmp"
            | "fcmpe"
            | "b"
            | "ret"
            | "cbz"
            | "cbnz"
            | "tbz"
            | "tbnz"
            | "cmp"
            | "cmn"
    ) || mnem.starts_with("b.")
    {
        return false;
    }
    rest.split(',')
        .next()
        .is_some_and(|first| fp_token_reg(first) == Some(num))
        // Writeback bases are GP registers, but the family numbering is
        // shared with the v-registers: a false positive here only drops
        // forwarding state (the documented, sound direction).
        || implicitly_writes_a64(t, num)
}

/// Does this line write GP register `num` (x or w form)? Checks the first two
/// operand positions (pair loads write the second). False positives are fine
/// (callers only use this to drop tracking state, never to enable a rewrite);
/// false negatives are not, so store/branch/compare mnemonics are excluded and
/// anything unrecognized counts as a write when the register appears early.
fn gp_reg_written_broad(line: &str, num: u8) -> bool {
    let t = line.trim();
    let Some((mnem, rest)) = t.split_once(' ') else {
        return false;
    };
    if matches!(
        mnem,
        "str"
            | "stp"
            | "strb"
            | "strh"
            | "stur"
            | "sturb"
            | "sturh"
            | "stlr"
            | "stlrb"
            | "stlrh"
            | "fcmp"
            | "fcmpe"
            | "b"
            | "ret"
            | "cbz"
            | "cbnz"
            | "tbz"
            | "tbnz"
            | "cmp"
            | "cmn"
    ) || mnem.starts_with("b.")
    {
        return false;
    }
    rest.split(',').take(2).any(|op| {
        let op = op.trim();
        op == xreg_name(num) || op == wreg_name(num)
    }) || implicitly_writes_a64(t, num)
}

// ── Adjacent FP field pair fusion (ldp/stp) ─────────────────────────────────
//
// `ldr dA, [xB]` + `ldr dC, [xB, #8]` (within a small window) → `ldp dA, dC,
// [xB]`, and likewise `str` pairs → `stp`. Struct-of-double loops (nbody,
// spectral_norm, struct_copy) load/store adjacent fields in pairs; fusing
// halves the memory-op issue slots. Bit-identical: ldp/stp move the same
// bytes as the two scalar accesses. CCC_NO_FP_PAIR disables.
//
// Soundness: the second access moves up to the first's position, so nothing
// in the gap may write memory (for loads), touch memory at all (for stores),
// or mention either destination register or the base.
fn fuse_fp_adjacent_pairs(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_FP_PAIR").is_ok() {
        return false;
    }
    let mention = |line: &str, reg: u8, is_fp: bool| {
        let (a, b) = if is_fp {
            (format!("d{}", reg), format!("q{}", reg))
        } else {
            (xreg_name(reg).to_string(), wreg_name(reg).to_string())
        };
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|tok| tok == a || tok == b)
    };
    let mut changed = false;
    for i in 0..n {
        // Skip lines already Nop'd by earlier fusions/passes: their text is
        // stale and must not be re-paired.
        if kinds[i] == LineKind::Nop {
            continue;
        }
        let Some((is_store, ra, addra, b'd')) = parse_fp_mem(&lines[i]) else {
            continue;
        };
        let base_reg = match addra {
            FpAddr::Sp(_) => u8::MAX, // sp is never rewritten mid-body
            FpAddr::Base(b, _) => b,
        };
        // Find a mergeable partner within a 4-instruction window.
        let mut partner = None;
        let mut j = i + 1;
        while j < n && j <= i + 4 {
            if kinds[j] == LineKind::Nop {
                j += 1;
                continue;
            }
            match kinds[j] {
                LineKind::Label
                | LineKind::Branch
                | LineKind::CondBranch
                | LineKind::CmpBranch
                | LineKind::Call
                | LineKind::Ret
                | LineKind::Directive => break,
                _ => {}
            }
            let tj = lines[j].trim();
            // A valid merge partner is not a hazard to itself — check for it
            // before the generic memory-hazard break.
            if let Some((is_store2, rc, addrc, b'd')) = parse_fp_mem(&lines[j]) {
                if is_store2 == is_store && rc != ra {
                    let same_base = match (addra, addrc) {
                        (FpAddr::Sp(a), FpAddr::Sp(b)) => Some((a, b)),
                        (FpAddr::Base(x, a), FpAddr::Base(y, b)) if x == y => Some((a, b)),
                        _ => None,
                    };
                    if let Some((oa, ob)) = same_base {
                        if (oa - ob).abs() == 8 {
                            // The partner's load/store moves up to position i:
                            // no gap line may mention rc (its physical register
                            // could carry a live older value), write ra, or
                            // write the base.
                            let mut ok = true;
                            for k in (i + 1)..j {
                                if kinds[k] == LineKind::Nop {
                                    continue;
                                }
                                if mention(&lines[k], rc, true)
                                    || fp_reg_written(&lines[k], ra)
                                    || (base_reg != u8::MAX
                                        && gp_reg_written_broad(&lines[k], base_reg))
                                {
                                    ok = false;
                                    break;
                                }
                            }
                            if ok {
                                partner = Some((j, rc, ob));
                            }
                            break;
                        }
                    }
                }
            }
            let is_mem = parse_fp_mem(&lines[j]).is_some()
                || matches!(
                    kinds[j],
                    LineKind::StoreSp { .. }
                        | LineKind::LoadSp { .. }
                        | LineKind::LoadswSp { .. }
                        | LineKind::MemOther
                        | LineKind::StorePairSp
                        | LineKind::LoadPairSp
                        | LineKind::LoadsbReg
                );
            // A store in the gap could alias the pending load; for store
            // fusion any memory access is a hazard.
            if is_mem && (is_store || tj.starts_with("st")) {
                break;
            }
            j += 1;
        }
        let Some((pj, rc, ob)) = partner else {
            continue;
        };
        let oa = match addra {
            FpAddr::Sp(o) => o,
            FpAddr::Base(_, o) => o,
        };
        let (lo_reg, hi_reg, lo) = if oa <= ob { (ra, rc, oa) } else { (rc, ra, ob) };
        // LDP/STP immediate offset must be in [-512, 504] and multiple of 8.
        // Outside that range gas 2.47 rejects the encoding.
        if !(-512..=504).contains(&lo) || lo % 8 != 0 {
            continue;
        }
        let base_str = match addra {
            FpAddr::Sp(_) => "sp".to_string(),
            FpAddr::Base(b, _) => xreg_name(b).to_string(),
        };
        let mnem = if is_store { "stp" } else { "ldp" };
        lines[i] = if lo == 0 {
            format!("    {} d{}, d{}, [{}]", mnem, lo_reg, hi_reg, base_str)
        } else {
            format!(
                "    {} d{}, d{}, [{}, #{}]",
                mnem, lo_reg, hi_reg, base_str, lo
            )
        };
        kinds[i] = classify_line(&lines[i]);
        kinds[pj] = LineKind::Nop;
        changed = true;
    }
    changed
}

// ── Repeated GP slot load elimination ───────────────────────────────────────
//
// `ldr x9, [sp, #off]` ... `ldr x9, [sp, #off]` (same slot, no clobber):
// spill-heavy code reloads slot-homed pointers once per field access. The
// second load is redundant (or becomes a `mov` when the destination differs).
// Same tracking discipline as forward_fp_slot_loads: stores through unknown
// bases, calls, and control-flow boundaries drop all state; a write of the
// cached register drops the entry.
fn eliminate_repeated_slot_loads(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_SLOT_LOAD_DEDUP").is_ok() {
        return false;
    }
    // (frame byte, is_word) -> register holding the slot's content.
    // Keys are ENTRY-sp-relative frame bytes (raw offset + sp displacement):
    // inside a `sub sp` staging window the same raw offset names a different
    // byte, and forwarding across that boundary would alias call arguments
    // with frame slots (see sp_displacements).
    let Some(disps) = sp_displacements(lines, kinds, n) else {
        return false;
    };
    let mut cached: FxHashMap<(i64, bool), u8> = FxHashMap::default();
    let mut changed = false;
    for i in 0..n {
        match kinds[i] {
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret => {
                cached.clear();
                continue;
            }
            LineKind::Directive | LineKind::Nop => continue,
            LineKind::StoreSp {
                offset, is_word, ..
            } => {
                // Invalidate entries overlapping the stored range.
                let lo = offset as i64 + disps[i];
                let hi = lo + if is_word { 4 } else { 8 };
                cached.retain(|&(off, word), _| {
                    let (l2, h2) = (off, off + if word { 4 } else { 8 });
                    h2 <= lo || l2 >= hi
                });
                continue;
            }
            LineKind::LoadSp {
                reg,
                offset,
                is_word,
            } => {
                let key = (offset as i64 + disps[i], is_word);
                if let Some(&prev) = cached.get(&key) {
                    if prev == reg {
                        kinds[i] = LineKind::Nop;
                        changed = true;
                        continue;
                    }
                    lines[i] = format!(
                        "    mov {}, {}",
                        if is_word {
                            wreg_name(reg)
                        } else {
                            xreg_name(reg)
                        },
                        if is_word {
                            wreg_name(prev)
                        } else {
                            xreg_name(prev)
                        }
                    );
                    kinds[i] = LineKind::Move {
                        dst: reg,
                        src: prev,
                        is_32bit: is_word,
                    };
                    changed = true;
                }
                // The load (or its mov replacement) writes `reg`: any entry
                // cached under it is stale. Then record the new mapping.
                cached.retain(|_, &mut r| r != reg);
                cached.insert(key, reg);
                continue;
            }
            _ => {}
        }
        let t = lines[i].trim();
        // Stores through untracked bases may hit the frame: drop everything.
        if t.starts_with("st") {
            cached.clear();
            continue;
        }
        // Loads and other instructions only invalidate via their destination.
        if let Some(w) = written_gp_register(&lines[i], kinds[i]) {
            cached.retain(|_, &mut r| r != w);
        } else {
            // Unrecognized kinds: check destination-position mentions broadly.
            let mut dead = Vec::new();
            for &r in cached.values() {
                if gp_reg_written_broad(&lines[i], r) {
                    dead.push(r);
                }
            }
            for r in dead {
                cached.retain(|_, &mut v| v != r);
            }
        }
    }
    changed
}

/// Strict single-destination form: only the first operand position, and only
/// when it is the line's sole mention of the register (a read-write like
/// `add x0, x0, #1` is NOT a pure overwrite).
fn gp_reg_write_only(line: &str, num: u8) -> bool {
    let t = line.trim();
    let Some((mnem, rest)) = t.split_once(' ') else {
        return false;
    };
    if matches!(
        mnem,
        "str"
            | "stp"
            | "strb"
            | "strh"
            | "stur"
            | "sturb"
            | "sturh"
            | "stlr"
            | "stlrb"
            | "stlrh"
            | "fcmp"
            | "fcmpe"
            | "b"
            | "bl"
            | "ret"
            | "cbz"
            | "cbnz"
            | "tbz"
            | "tbnz"
            | "cmp"
            | "cmn"
    ) || mnem.starts_with("b.")
    {
        return false;
    }
    let (xa, wa) = (xreg_name(num), wreg_name(num));
    let first = rest.split(',').next().map(str::trim);
    if first != Some(xa) && first != Some(wa) {
        return false;
    }
    t.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|tok| *tok == xa || *tok == wa)
        .count()
        == 1
}

fn forward_fp_slot_loads(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_FP_SLOT_FWD").is_ok() {
        return false;
    }
    // slot (offset, width) -> source FP register number
    let mut slot_src: FxHashMap<(i32, u8), u8> = FxHashMap::default();
    // sp-derived base register -> sp offset (`add xN, sp, #off` / `mov xN, sp`)
    let mut base_off: FxHashMap<u8, i32> = FxHashMap::default();
    let mut changed = false;

    let mut i = 0;
    while i < n {
        match kinds[i] {
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret => {
                slot_src.clear();
                base_off.clear();
                i += 1;
                continue;
            }
            LineKind::Directive | LineKind::Nop => {
                i += 1;
                continue;
            }
            _ => {}
        }

        let t = lines[i].trim().to_string();

        // Track sp-derived base registers: `add xN, sp, #off`, `mov xN, sp`.
        if let Some(rest) = t.strip_prefix("add ") {
            if let Some((dst_s, srcs)) = rest.split_once(", ") {
                if let Some((src_s, imm_s)) = srcs.split_once(", ") {
                    if src_s.trim() == "sp" {
                        let dst = parse_reg(dst_s.trim());
                        if dst < 29 {
                            if let Some(imm) = imm_s
                                .trim()
                                .strip_prefix('#')
                                .and_then(|v| v.parse::<i32>().ok())
                            {
                                base_off.insert(dst, imm);
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
            }
        } else if let Some(rest) = t.strip_prefix("mov ") {
            if let Some((dst_s, src_s)) = rest.split_once(", ") {
                if src_s.trim() == "sp" {
                    let dst = parse_reg(dst_s.trim());
                    if dst < 29 {
                        base_off.insert(dst, 0);
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Any write of a tracked base register invalidates its mapping.
        // (Broad check: loads through untracked bases and pair loads can also
        // rewrite the register; over-dropping only loses optimization.)
        {
            let mut dead_bases = Vec::new();
            for &b in base_off.keys() {
                if written_gp_register(&lines[i], kinds[i]) == Some(b)
                    || gp_reg_written_broad(&lines[i], b)
                {
                    dead_bases.push(b);
                }
            }
            for b in dead_bases {
                base_off.remove(&b);
            }
        }

        if let Some((is_store, reg, addr, width)) = parse_fp_mem(&lines[i]) {
            let slot_off = match addr {
                FpAddr::Sp(off) => Some(off),
                FpAddr::Base(b, k) => base_off.get(&b).map(|&base| base + k),
            };
            let Some(slot_off) = slot_off else {
                // Unknown base: a store through it may alias anything tracked.
                if is_store {
                    slot_src.clear();
                    base_off.clear();
                } else {
                    // Unknown-base load still clobbers its destination.
                    let mut to_drop = Vec::new();
                    for (&key, &s) in slot_src.iter() {
                        if s == reg {
                            to_drop.push(key);
                        }
                    }
                    for key in to_drop {
                        slot_src.remove(&key);
                    }
                }
                i += 1;
                continue;
            };
            if is_store {
                let (lo, hi) = (slot_off, slot_off + fp_access_width(width));
                slot_src.retain(|&(off, w), _| {
                    let (l2, h2) = (off, off + fp_access_width(w));
                    h2 <= lo || l2 >= hi
                });
                slot_src.insert((slot_off, width), reg);
            } else {
                if let Some(&src) = slot_src.get(&(slot_off, width)) {
                    if src == reg {
                        // Loading a slot into the register that already holds
                        // its content: redundant.
                        kinds[i] = LineKind::Nop;
                        changed = true;
                        i += 1;
                        continue;
                    }
                    lines[i] = if width == b'q' {
                        format!("    mov v{}.16b, v{}.16b", reg, src)
                    } else {
                        format!(
                            "    fmov {}{}, {}{}",
                            width as char, reg, width as char, src
                        )
                    };
                    kinds[i] = classify_line(&lines[i]);
                    changed = true;
                }
                // The load writes its destination register, whether or not a
                // forward happened: entries forwarding FROM that register are
                // now stale. (Missing this clobbered a still-tracked entry
                // whose source was silently reloaded between store and use.)
                slot_src.retain(|_, &mut s| s != reg);
            }
            i += 1;
            continue;
        }

        // GP stack stores invalidate overlapping FP entries.
        if let LineKind::StoreSp {
            offset, is_word, ..
        } = kinds[i]
        {
            let (lo, hi) = (offset, offset + if is_word { 4 } else { 8 });
            slot_src.retain(|&(off, w), _| {
                let (l2, h2) = (off, off + fp_access_width(w));
                h2 <= lo || l2 >= hi
            });
            i += 1;
            continue;
        }

        match kinds[i] {
            // Untracked store forms may write anywhere: drop all state.
            LineKind::MemOther | LineKind::StorePairSp if t.starts_with("st") => {
                slot_src.clear();
                base_off.clear();
            }
            // Pair loads write two registers: drop entries sourcing either.
            LineKind::LoadPairSp => {
                let mut ops = t
                    .trim_start_matches("ldp ")
                    .split(',')
                    .take(2)
                    .filter_map(|tok| fp_token_reg(tok));
                let (a, b) = (ops.next(), ops.next());
                slot_src.retain(|_, &mut s| Some(s) != a && Some(s) != b);
            }
            // Other instructions: drop entries whose source register they write.
            _ => {
                let mut to_drop = Vec::new();
                for (&key, &s) in slot_src.iter() {
                    if fp_reg_written(&lines[i], s) {
                        to_drop.push(key);
                    }
                }
                for key in to_drop {
                    slot_src.remove(&key);
                }
            }
        }
        i += 1;
    }
    changed
}

fn eliminate_adjacent_store_load(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    // Pair store→load by FRAME BYTE (raw offset + sp displacement): the
    // outgoing-args staging window reuses raw offsets for different bytes
    // (see sp_displacements).
    let Some(disps) = sp_displacements(lines, kinds, n) else {
        return false;
    };
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        if let LineKind::StoreSp {
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
                match kinds[j] {
                    LineKind::LoadSp {
                        reg: load_reg,
                        offset: load_off,
                        is_word: load_word,
                    } if store_off as i64 + disps[i] == load_off as i64 + disps[j]
                        && store_word == load_word =>
                    {
                        if store_reg == load_reg {
                            // Same register: eliminate the load entirely
                            kinds[j] = LineKind::Nop;
                            changed = true;
                        } else {
                            // Different register: replace load with mov
                            let reg_fmt = if store_word { wreg_name } else { xreg_name };
                            lines[j] =
                                format!("    mov {}, {}", reg_fmt(load_reg), reg_fmt(store_reg));
                            kinds[j] = LineKind::Move {
                                dst: load_reg,
                                src: store_reg,
                                is_32bit: store_word,
                            };
                            changed = true;
                        }
                    }
                    // str wN, [sp, #off] followed by ldrsw xM, [sp, #off]
                    // The value was just stored as a word; sign-extending load can be replaced
                    // with sxtw xM, wN (or eliminated if same reg).
                    LineKind::LoadswSp {
                        reg: load_reg,
                        offset: load_off,
                    } if store_off as i64 + disps[i] == load_off as i64 + disps[j]
                        && store_word =>
                    {
                        lines[j] =
                            format!("    sxtw {}, {}", xreg_name(load_reg), wreg_name(store_reg));
                        kinds[j] = LineKind::Sxtw {
                            dst: load_reg,
                            src: store_reg,
                        };
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
    changed
}

// ── Pass 2: Redundant branch elimination ─────────────────────────────────────
//
// Pattern: b .LBBN  ;  .LBBN:  (branch to immediately next label)
// Falls through naturally, so the branch is redundant.

fn eliminate_redundant_branches(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        if kinds[i] == LineKind::Branch {
            if let Some(target) = branch_target(&lines[i]) {
                // Find next non-Nop line
                let mut j = i + 1;
                while j < n && kinds[j] == LineKind::Nop {
                    j += 1;
                }
                if j < n && kinds[j] == LineKind::Label {
                    if let Some(lbl) = label_name(&lines[j]) {
                        if target == lbl {
                            kinds[i] = LineKind::Nop;
                            changed = true;
                        }
                    }
                }
            }
        }
    }
    changed
}

// ── Pass 3: Self-move elimination ────────────────────────────────────────────
//
// Pattern: mov xN, xN — no-op

fn eliminate_self_moves(kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        if let LineKind::Move { dst, src, is_32bit } = kinds[i] {
            if dst == src && !is_32bit {
                // Only eliminate 64-bit self-moves (mov xN, xN).
                // On AArch64, `mov wN, wN` zeros the upper 32 bits of xN,
                // so it is NOT a true no-op and must be preserved.
                kinds[i] = LineKind::Nop;
                changed = true;
            }
        }
    }
    changed
}

// ── Pass 3b: Redundant sign-extension elimination ───────────────────────────
//
// Pattern: `sxtw xD, wS` where xS already holds a sign-extended value (written
// by `ldrsw`, an earlier `sxtw`, or a 64-bit mov of an extended register).
// The sxtw recomputes what is already true — replace with `mov xD, xS`
// (or drop entirely when D == S). Common after `ldrsw x0, [..]; mov xR, x0`
// feeding an I32→I64 cast (e.g. dot_product's per-load sxtw).
//
// A register stops being known-extended on any write: w-writes zero the upper
// bits, x-writes of unknown provenance clobber conservatively. All tracking is
// cleared at labels, branches, and calls.

fn eliminate_redundant_sxtw(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut extended = [false; 33];
    extended[32] = true; // xzr is always "extended" (sxtw xD, wzr ≡ mov xD, xzr)
    let mut changed = false;
    for i in 0..n {
        match kinds[i] {
            // Control-flow barriers: nothing is known across them.
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret => {
                extended = [false; 33];
                extended[32] = true;
            }
            // Unknown/memory kinds may clobber registers — except ldrsw with
            // non-sp addressing (classifies as MemOther), which CREATES a
            // sign-extended register.
            LineKind::MemOther | LineKind::Other | LineKind::LoadsbReg | LineKind::LoadPairSp => {
                let t = lines[i].trim();
                let mut ldrsw_dst: Option<u8> = None;
                if let Some(rest) = t.strip_prefix("ldrsw ") {
                    if let Some((reg_str, _)) = rest.split_once(',') {
                        let r = parse_reg(reg_str.trim());
                        if r != REG_NONE && r < 32 {
                            ldrsw_dst = Some(r);
                        }
                    }
                }
                extended = [false; 33];
                extended[32] = true;
                if let Some(r) = ldrsw_dst {
                    extended[r as usize] = true;
                }
            }
            LineKind::LoadswSp { reg, .. } => {
                if reg < 32 {
                    extended[reg as usize] = true;
                }
            }
            LineKind::Sxtw { dst, src } => {
                if extended[src as usize] {
                    if dst == src {
                        kinds[i] = LineKind::Nop;
                    } else {
                        lines[i] = format!("    mov {}, {}", xreg_name(dst), xreg_name(src));
                        kinds[i] = LineKind::Move {
                            dst,
                            src,
                            is_32bit: false,
                        };
                    }
                    changed = true;
                }
                // Whether redundant or not, the sxtw result is sign-extended.
                if dst < 32 {
                    extended[dst as usize] = true;
                }
            }
            LineKind::Move { dst, src, is_32bit } => {
                if is_32bit {
                    // mov wN, wM zero-extends into xN — NOT a sign extension.
                    if dst < 32 {
                        extended[dst as usize] = false;
                    }
                } else if dst < 32 {
                    extended[dst as usize] = extended[src as usize];
                }
            }
            _ => {
                if let Some(dst) = written_gp_register(&lines[i], kinds[i]) {
                    if dst < 32 {
                        extended[dst as usize] = false;
                    }
                }
            }
        }
    }
    changed
}

// ── Pass 3b: Redundant byte sign-extension elimination ──────────────────────
//
// Mirror of eliminate_redundant_sxtw for byte-level extensions: `ldrsb`
// produces a value whose bits 7..63 are uniform (a sign-extended byte), so a
// later `sxtb xD, wS` on it is redundant. Fires on char-byte loops like
// sieve's count loop (`ldrsb x0, [p]` then `sxtb` before the zero test).

fn eliminate_redundant_sxtb(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut extb = [false; 33];
    extb[32] = true; // the zero register is "byte-extended"
    let mut changed = false;
    for i in 0..n {
        match kinds[i] {
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret => {
                extb = [false; 33];
                extb[32] = true;
            }
            LineKind::Move { dst, src, is_32bit } => {
                if is_32bit {
                    // w-mov zeroes bits 32-63, breaking byte-extension of negatives.
                    if dst < 32 {
                        extb[dst as usize] = false;
                    }
                } else if dst < 32 {
                    extb[dst as usize] = extb[src as usize];
                }
            }
            _ => {
                let t = lines[i].trim().to_string();
                // sxtb xD, wS
                if let Some(rest) = t.strip_prefix("sxtb ") {
                    if let Some((d, s)) = rest.split_once(", ") {
                        let dst = parse_reg(d.trim());
                        let src = parse_reg(s.trim());
                        if dst != REG_NONE && src != REG_NONE && dst < 32 && src < 32 {
                            if extb[src as usize] {
                                if dst == src {
                                    kinds[i] = LineKind::Nop;
                                } else {
                                    lines[i] =
                                        format!("    mov {}, {}", xreg_name(dst), xreg_name(src));
                                    kinds[i] = LineKind::Move {
                                        dst,
                                        src,
                                        is_32bit: false,
                                    };
                                }
                                changed = true;
                            }
                            // The sxtb result is byte-extended either way.
                            extb[dst as usize] = true;
                            continue;
                        }
                    }
                }
                // ldrsb xD/wD, [...] creates a byte-sign-extended value.
                if let Some(rest) = t.strip_prefix("ldrsb ") {
                    if let Some((d, _)) = rest.split_once(',') {
                        let dst = parse_reg(d.trim());
                        if dst != REG_NONE && dst < 32 {
                            extb[dst as usize] = true;
                        }
                        continue;
                    }
                }
                // Any other write clobbers the tracking.
                if let Some(dst) = written_gp_register(&t, kinds[i]) {
                    if dst < 32 {
                        extb[dst as usize] = false;
                    }
                }
            }
        }
    }
    changed
}

// ── Redundant zero-extension mask elimination ────────────────────────────────
//
// The cast emitters stage through x0 and mask every widening/narrowing cast
// (`and x0, x0, #0xff` for U8, #0xffff for U16), even when the value already
// satisfies the mask because it came from an `ldrb`/`ldrh` (zero-extending
// loads), a narrower mask, or a small immediate. AArch64 has no need for the
// re-mask; GCC drops it (zext_chain/load_cmp compile the mask away entirely).
//
// Forward knowledge tracking, one pass per basic-block-shaped region: for
// every x-register we track a WIDTH bound w meaning value(x-reg) < 2^w
// (all bits at index >= w are zero). Producers: zero-extending loads
// (w=8/16), 32-bit writes (w=32), a previous mask (its width), small
// immediates, ubfx #0,#w, uxtb/uxth, xzr (w=0), and register moves (the
// bound transfers; a w-form move clamps to 32). Everything else that
// writes a register widens the bound to its form (32 for w-form writes,
// 64 = no information for x-form writes): precisely for the classified
// kinds, and CONSERVATIVELY (full reset to 64) for the unclassified
// Other/MemOther kinds -- matching the soundness stance of
// eliminate_redundant_sxtw above. Labels, branches, calls and returns are
// barriers (no cross-block knowledge).
//
// Fire condition: value(s) < 2^mask_width, i.e. bound(s) <= mask_width.
// Note the direction -- a 32-bit bound says NOTHING about bits 31:8, so a
// generic w-form write never fires an #0xff mask; a tight 8-bit bound from
// ldrb fires both #0xff and #0xffff masks.
//
// Rewrite: `and d, s, #mask` with knowledge(s) >= mask width becomes
// `mov d, s` — value-identical — and the existing move-chain/self-move
// passes clean the copy up. `and` does not read flags and the rewrite does
// not write any, so no flag state is disturbed.
//
// Example (zext_chain, per iteration):
//   ldrb w5, [x19, x1]     ldrb w5, [x19, x1]
//   and  w4, w5, #0xff  →  mov  w4, w5     (then propagated away)
//   mov  x0, x4            mov  x0, x5

fn zext_mask_bits(imm: &str) -> Option<u8> {
    match imm.trim() {
        "#0xff" => Some(8),
        "#0xffff" => Some(16),
        _ => None,
    }
}

/// Width bound contributed by a destination's REGISTER FORM: a w-form
/// write produces a value < 2^32 (bits 63:32 zeroed); an x-form write
/// carries no bound. Used as the base rule for classified writes.
const ZEXT_NO_INFO: u8 = 64;

fn form_zero_width(op: &str) -> u8 {
    if op.starts_with('w') {
        32
    } else {
        ZEXT_NO_INFO
    }
}

/// Destination register of an instruction whose first comma-operand is the
/// written register (all AArch64 ALU/move/load forms), or REG_NONE.
fn first_operand_reg(line: &str) -> u8 {
    let Some(rest) = line.trim().split_once(' ').map(|(_, r)| r) else {
        return REG_NONE;
    };
    let Some((d, _)) = rest.split_once(',') else {
        return REG_NONE;
    };
    parse_reg(d.trim())
}

/// Kill knowledge for a post-indexed writeback base (`..., [xN], #imm`).
fn kill_writeback_base(line: &str, zext: &mut [u8; 33]) {
    let t = line.trim();
    let Some(bracket) = t.find('[') else { return };
    let Some(close) = t[bracket..].find(']') else {
        return;
    };
    let inner = &t[bracket + 1..bracket + close];
    // Post-index (`[xN], #imm`) or pre-index (`[xN, #imm]!`) writeback both
    // redefine the base register.
    if let Some((_, after)) = t.split_once(']') {
        let after = after.trim_start();
        if after.starts_with('!') || after.starts_with('#') || after.starts_with(", #") {
            // Writeback base becomes base+imm: arbitrary 64-bit.
            if let Some((base, _)) = inner.split_once(',') {
                let r = parse_reg(base.trim());
                if r < 31 {
                    zext[r as usize] = ZEXT_NO_INFO;
                }
            } else if !inner.is_empty() {
                let r = parse_reg(inner.trim());
                if r < 31 {
                    zext[r as usize] = ZEXT_NO_INFO;
                }
            }
        }
    }
}

fn eliminate_redundant_zext_and(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_ZEXT_MASK_FOLD").is_ok() {
        return false;
    }
    let mut zext = [ZEXT_NO_INFO; 33];
    zext[32] = 0; // xzr/wzr is all-zero and unwritable
    let mut changed = false;
    for i in 0..n {
        match kinds[i] {
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret => {
                zext = [ZEXT_NO_INFO; 33];
                zext[32] = 0;
                continue;
            }
            LineKind::Directive | LineKind::Nop => continue,
            _ => {}
        }
        let t = lines[i].trim();
        let Some(sp) = t.find(' ') else { continue };
        let mnemonic = &t[..sp];
        let rest = &t[sp + 1..];
        match mnemonic {
            // Zero-extending loads: bits above the access size are zero.
            "ldrb" | "ldurb" | "ldarb" | "ldaprb" | "ldtrb" | "ldrab" => {
                if let Some((d, _)) = rest.split_once(',') {
                    let dst = parse_reg(d.trim());
                    if dst < 32 {
                        zext[dst as usize] = 8;
                    }
                }
                kill_writeback_base(t, &mut zext);
            }
            "ldrh" | "ldurh" | "ldarh" | "ldaprh" | "ldtrh" => {
                if let Some((d, _)) = rest.split_once(',') {
                    let dst = parse_reg(d.trim());
                    if dst < 32 {
                        zext[dst as usize] = 16;
                    }
                }
                kill_writeback_base(t, &mut zext);
            }
            // Masks produce exactly their own width of known zeros. NOTE:
            // `ands` also SETS FLAGS — its value knowledge is tracked, but
            // it is never rewritten (a `mov` replacement would drop the
            // flag definitions).
            "and" | "ands" => {
                let is_ands = mnemonic == "ands";
                let mut ops = rest.split(',');
                let dst = ops.next().map(|o| parse_reg(o.trim())).unwrap_or(REG_NONE);
                let src = ops.next().map(|o| parse_reg(o.trim())).unwrap_or(REG_NONE);
                let third = ops.next();
                // Capture the source bound BEFORE writing the destination:
                // for the in-place form `and x0, x0, #mask` (dst == src) the
                // destination update below would otherwise overwrite the
                // very bound the redundancy check reads — the mask would
                // then validate itself (e.g. an #0xffff truncation after
                // `sxth` saw its own 16-bit result instead of the 64-bit
                // sign-extended input and deleted itself — cast_chain_fold,
                // sqlite_yy_shift miscompiles).
                let src_bound = if src < 32 {
                    zext[src as usize]
                } else {
                    ZEXT_NO_INFO
                };
                if dst < 32 {
                    let w = third.and_then(zext_mask_bits).unwrap_or_else(|| {
                        let dform = rest.split(',').next().unwrap_or("").trim();
                        form_zero_width(dform)
                    });
                    zext[dst as usize] = w;
                }
                // A plain-AND mask whose source is already wider-known is
                // redundant: value-identical `mov`, flags untouched (`and`
                // reads no flags and writes none).
                if !is_ands {
                    if let Some(bits) = third.and_then(zext_mask_bits) {
                        if src < 32 && src_bound <= bits {
                            let dform = rest.split(',').next().unwrap_or("").trim();
                            let sform = rest.split(',').nth(1).unwrap_or("").trim().to_string();
                            let is32 = dform.starts_with('w');
                            let new = format!("    mov {}, {}", dform, sform);
                            kinds[i] = LineKind::Move {
                                dst,
                                src,
                                is_32bit: is32,
                            };
                            lines[i] = new;
                            changed = true;
                        }
                    }
                }
            }
            // ubfx wD, ..., #0, #w yields w known-zero bits when lsb == 0.
            "ubfx" => {
                // Operands: dst, src, #lsb, #width.
                let mut ops = rest.split(',');
                let dst = ops.next().map(|o| parse_reg(o.trim())).unwrap_or(REG_NONE);
                let lsb: Option<u32> = ops
                    .nth(2)
                    .and_then(|o| o.trim().strip_prefix('#'))
                    .and_then(|o| o.trim().parse().ok());
                let width: Option<u32> = ops
                    .next()
                    .and_then(|o| o.trim().strip_prefix('#'))
                    .and_then(|o| o.trim().parse().ok());
                if dst < 32 {
                    zext[dst as usize] = if lsb == Some(0) {
                        width.map(|w| w.clamp(0, 63) as u8).unwrap_or(0)
                    } else {
                        form_zero_width(rest.split(',').next().unwrap_or("").trim())
                    };
                }
            }
            "uxtb" | "uxth" => {
                if let Some((d, _)) = rest.split_once(',') {
                    let dst = parse_reg(d.trim());
                    if dst < 32 {
                        zext[dst as usize] = if mnemonic == "uxtb" { 8 } else { 16 };
                    }
                }
            }
            // adrp: page-aligned (bits 11:0 zero) is an ALIGNMENT fact,
            // not a width bound -- value(adrp) is generally >= 2^12. No
            // width information here; the x-form base rule applies.
            "adrp" => {}
            "mov" | "movz" => {
                let mut ops = rest.split(',');
                let dform = ops.next().unwrap_or("").trim().to_string();
                let srcop = ops.next().unwrap_or("").trim().to_string();
                let dst = parse_reg(&dform);
                // `movz xD, #imm, lsl #n` shifts — the immediate's width no
                // longer bounds the register's zero bits (e.g. lsl #48 puts
                // non-zero bits high). The register IS written, so the old
                // bound must not survive: fall back to the form rule.
                let shifted = rest.contains(", lsl") || rest.contains(", lsl ");
                if dst < 32 && shifted {
                    zext[dst as usize] = form_zero_width(&dform);
                }
                if dst < 32 && !shifted {
                    if let Some(immv) = srcop.strip_prefix('#') {
                        // mov/movz with a non-negative immediate: the value's
                        // known zero width is the immediate's significant
                        // width rounded up to 8/16/32.
                        if let Ok(v) = u64::from_str_radix(
                            immv.trim_start_matches("0x"),
                            if immv.starts_with("0x") { 16 } else { 10 },
                        ) {
                            zext[dst as usize] = if v <= 0xff {
                                8
                            } else if v <= 0xffff {
                                16
                            } else {
                                32
                            };
                        } else {
                            zext[dst as usize] = form_zero_width(&dform);
                        }
                    } else {
                        let src = parse_reg(&srcop);
                        if src < 32 {
                            // Copy: the written value keeps the source's
                            // width bound; a w-form write additionally
                            // zeroes bits 63:32, clamping the bound at 32.
                            zext[dst as usize] = if dform.starts_with('w') {
                                zext[src as usize].min(32)
                            } else {
                                zext[src as usize]
                            };
                        } else {
                            zext[dst as usize] = form_zero_width(&dform);
                        }
                    }
                }
            }
            // Everything else with a classified single destination: the form
            // of the destination fixes the base knowledge.
            _ => match kinds[i] {
                LineKind::Alu => {
                    let dst = first_operand_reg(t);
                    if dst < 32 {
                        let dform = t
                            .split_once(' ')
                            .and_then(|(_, r)| r.split(',').next())
                            .unwrap_or("")
                            .trim();
                        zext[dst as usize] = form_zero_width(dform);
                    }
                }
                // Move-family mnemonics movn/movk reach here: movn writes a
                // complemented immediate (unknown zeros — form rule), movk
                // patches mid-bits (form rule). Both are captured by the
                // destination-form base rule below.
                LineKind::Move { dst, src, is_32bit } => {
                    if dst < 32 {
                        // Reached only for forms the mov/movz mnemonic arm
                        // does not cover (movn/movk); transfer conservatively.
                        let _ = src;
                        zext[dst as usize] = if is_32bit { 32 } else { ZEXT_NO_INFO };
                    }
                }
                LineKind::MoveImm { dst } | LineKind::MoveWide { dst } => {
                    if dst < 32 {
                        let dform = t
                            .split_once(' ')
                            .and_then(|(_, r)| r.split(',').next())
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        zext[dst as usize] = form_zero_width(&dform);
                    }
                }
                LineKind::LoadSp { reg, is_word, .. } => {
                    if reg < 32 {
                        // w-form: value < 2^32; x-form: arbitrary 64-bit.
                        zext[reg as usize] = if is_word { 32 } else { ZEXT_NO_INFO };
                    }
                }
                LineKind::LoadswSp { reg, .. } => {
                    if reg < 32 {
                        // ldrsw sign-extends: bits 63:32 may be all ones.
                        zext[reg as usize] = ZEXT_NO_INFO;
                    }
                }
                LineKind::LoadPairSp => {
                    // ldp wA, wB / ldp xA, xB — both destinations.
                    let mut ops = rest.split(',');
                    if let Some(a) = ops.next() {
                        let r = parse_reg(a.trim());
                        if r < 32 {
                            zext[r as usize] = form_zero_width(a.trim());
                        }
                    }
                    if let Some(b) = ops.next() {
                        let r = parse_reg(b.trim());
                        if r < 32 {
                            zext[r as usize] = form_zero_width(b.trim());
                        }
                    }
                }
                // cmp/cmn write no GP registers; stores write none (sp-based
                // forms carry no writeback in this backend); sxtw/ldrsb
                // produce sign-extended values (base form rule below).
                LineKind::Compare | LineKind::StoreSp { .. } | LineKind::StorePairSp => {}
                LineKind::Sxtw { dst, .. } => {
                    if dst < 32 {
                        // Sign-extended: bits 63:32 copy bit 31 (may be set).
                        zext[dst as usize] = ZEXT_NO_INFO;
                    }
                }
                LineKind::LoadsbReg => {
                    let d = first_operand_reg(t);
                    if d < 32 {
                        // ldrsb sign-extends: no width bound.
                        zext[d as usize] = ZEXT_NO_INFO;
                    }
                }
                // Unknown shape: reset everything (conservative).
                _ => {
                    zext = [ZEXT_NO_INFO; 33];
                    zext[32] = 0;
                }
            },
        }
    }
    changed
}

// ── AND + CMP #0 fusion (AArch64 TST) ────────────────────────────────────────
//
// `and wA, wB, C` immediately followed by `cmp wA, #0` and an N/Z/V-based
// flag consumer burns an instruction: ANDS computes the same value, writes
// the same destination register, and sets N/Z/V from the identical result.
// Fuse to `ands wA, wB, C` and delete the compare:
//
//   and  w5, w19, w20        ands w5, w19, w20
//   cmp  w5, #0          →   cset x0, ne
//   cset x0, ne
//
// Exactness (ARM ARM): for `cmp x, #0` N = bit31(x), Z = (x == 0), C = 1,
// V = 0. For `ands` N and Z are identical (same value); V = 0 always
// (logical ops). C differs (shifter carry), so C-consuming conditions
// (hs/lo/cs/cc/hi/ls) are excluded. Allowed set uses only N/Z/V:
// eq/ne/mi/pl/ge/lt/gt/le/vs/vc (for ge/lt/gt/le note V is 0 on BOTH sides,
// so N-vs-V relations are identical).
//
// The ANDS keeps writing wA, so no liveness is required; the immediate form
// shares AND's encoder, so any valid `and #imm` is a valid `ands #imm`.

fn nzv_condition(cc: &str) -> bool {
    matches!(
        cc,
        "eq" | "ne" | "mi" | "pl" | "ge" | "lt" | "gt" | "le" | "vs" | "vc"
    )
}

fn fuse_and_cmp_zero(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_AND_TST_FUSION").is_ok() {
        return false;
    }
    let mut changed = false;
    for i in 0..n.saturating_sub(2) {
        if kinds[i] != LineKind::Alu {
            continue;
        }
        let t = lines[i].trim();
        if !t.starts_with("and ") {
            continue;
        }
        let mut ops = t[4..].splitn(3, ',');
        let dform = ops.next().unwrap_or("").trim().to_string();
        let sform = ops.next().unwrap_or("").trim().to_string();
        // Tail kept VERBATIM (may contain shifted-register operands such as
        // `x2, lsl #3` — ands accepts the identical operand encoding).
        let cform = ops.next().unwrap_or("").trim().to_string();
        let dst = parse_reg(&dform);
        if dst == REG_NONE || dst >= 32 {
            continue;
        }
        // i+1: cmp <same reg>, #0 (or xzr/wzr)
        let t1 = lines[i + 1].trim();
        if kinds[i + 1] != LineKind::Compare || !t1.starts_with("cmp ") {
            continue;
        }
        let mut cops = t1[4..].split(',');
        let creg = cops.next().unwrap_or("").trim();
        let cimm = cops.next().unwrap_or("").trim();
        if creg != dform || !(cimm == "#0" || cimm == "xzr" || cimm == "wzr") {
            continue;
        }
        // i+2: an N/Z/V-only flag consumer.
        let t2 = lines[i + 2].trim();
        let cc_ok = if let Some(rest) = t2.strip_prefix("b.") {
            rest.split_once(' ')
                .map(|(cc, _)| nzv_condition(cc))
                .unwrap_or(false)
        } else if t2.starts_with("cset ") || t2.starts_with("csetm ") {
            t2.split_once(' ')
                .and_then(|(_, tail)| tail.split(',').nth(1))
                .map(|cc| nzv_condition(cc.trim()))
                .unwrap_or(false)
        } else {
            false
        };
        if !cc_ok {
            continue;
        }
        lines[i] = format!("    ands {}, {}, {}", dform, sform, cform);
        kinds[i] = LineKind::Alu;
        lines[i + 1] = String::new();
        kinds[i + 1] = LineKind::Nop;
        changed = true;
    }
    changed
}

// ── Dead move before return ──────────────────────────────────────────────────
//
// Return staging can leave `mov xD, xS` writes whose destination is the
// value's register-allocation home rather than the ABI return register x0 —
// e.g. bool-gate-shaped functions end
//   cset x0, ne ; sxtw x0, w0 ; mov x5, x0 ; <epilogue restores> ; ret
// where x5 is never read again. `ret` reads no general-purpose register
// except x30 (lr), so a move whose destination is not mentioned again
// before the ret (a later pure redefinition also proves death) is dead.
//
// The scan is the eliminate_dead_call_staging_moves shape, tightened for
// the no-call path: every instruction between the move and the ret must
// either not mention the destination, or provably only WRITE it (Move from
// a different source, MoveImm, LoadSp, Sxtw from a different source, Alu
// writing a different register). Anything ambiguous — unknown kinds, memory
// ops, calls, control flow — aborts the scan (move stays). Destinations
// x29 (fp), x30 (lr: `ret` reads it), sp and xzr are excluded.
fn eliminate_dead_pre_ret_moves(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_DEAD_PRE_RET").is_ok() {
        return false;
    }
    fn mentions_reg(line: &str, reg: u8) -> bool {
        line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|token| token == xreg_name(reg) || token == wreg_name(reg))
    }
    /// True when the line writes `reg` and provably does not read it.
    fn pure_write(line: &str, kind: &LineKind, reg: u8) -> bool {
        match kind {
            LineKind::Move { dst, src, .. } => *dst == reg && *src != reg,
            LineKind::MoveImm { dst } | LineKind::MoveWide { dst } => *dst == reg,
            LineKind::LoadSp { reg: d, .. } | LineKind::LoadswSp { reg: d, .. } => *d == reg,
            LineKind::LoadPairSp => {
                // ldp d, other — both destinations; reg must be one and the
                // other must differ.
                let rest = line.trim().split_once(' ').map(|(_, r)| r).unwrap_or("");
                let mut ops = rest.split(',');
                let a = ops.next().map(|o| parse_reg(o.trim())).unwrap_or(REG_NONE);
                let b = ops.next().map(|o| parse_reg(o.trim())).unwrap_or(REG_NONE);
                (a == reg) != (b == reg)
            }
            LineKind::Sxtw { dst, src } => *dst == reg && *src != reg,
            LineKind::Alu => {
                // Alu writes its first operand and reads the rest (for
                // read-modify forms the source operands may name the same
                // register — those are NOT pure writes). Memory operands
                // like `[x19, x1]` never parse to `reg` unless reg is
                // genuinely named, which the all-check treats as a read.
                let d = first_operand_reg(line);
                if d != reg {
                    return false;
                }
                let rest = line.trim().split_once(' ').map(|(_, r)| r).unwrap_or("");
                rest.split(',').skip(1).all(|o| parse_reg(o.trim()) != reg)
            }
            _ => false,
        }
    }
    let mut changed = false;
    for i in 0..n {
        // Destinations: GP registers 1..=28. Excluded: x0 — THE RETURN
        // REGISTER; `ret` hands x0 to the caller, so a late write to it is
        // observable even when nothing in this function reads it (aarch64
        // regression seeds: deleting `mov w0, w0` before ret changed the
        // returned value — the 32-bit form zero-extends x0). Also excluded:
        // x29 (fp), x30 (lr — `ret` reads it), sp, xzr.
        let dst = match kinds[i] {
            LineKind::Move { dst, .. } if (1..=28).contains(&dst) => dst,
            _ => continue,
        };
        let mut j = i + 1;
        let mut dead = false;
        while j < n {
            match kinds[j] {
                LineKind::Nop | LineKind::Directive => {}
                LineKind::Ret => {
                    dead = true;
                    break;
                }
                LineKind::Label
                | LineKind::Branch
                | LineKind::CondBranch
                | LineKind::CmpBranch
                | LineKind::Call => break,
                _ => {
                    if mentions_reg(&lines[j], dst) {
                        if pure_write(&lines[j], &kinds[j], dst) {
                            dead = true; // overwritten unread: move is dead
                        }
                        break;
                    }
                }
            }
            j += 1;
        }
        if dead {
            kinds[i] = LineKind::Nop;
            lines[i] = String::new();
            changed = true;
        }
    }
    changed
}

// ── Pass 4: Move chain optimization ──────────────────────────────────────────
//
// Pattern: mov A, B  ;  mov C, A → mov C, B
// This allows the first mov to potentially be dead-eliminated later.

fn eliminate_move_chains(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 1 < n {
        match kinds[i] {
            LineKind::Move {
                dst: dst1,
                src: src1,
                is_32bit: is_32bit1,
            } => {
                // Find next non-Nop instruction
                let mut j = i + 1;
                while j < n && kinds[j] == LineKind::Nop {
                    j += 1;
                }
                if j < n {
                    if let LineKind::Move {
                        dst: dst2,
                        src: src2,
                        is_32bit: is_32bit2,
                    } = kinds[j]
                    {
                        // mov dst1, src1 ; mov dst2, dst1 → mov dst2, src1
                        // Only safe when both moves use the same width.
                        if src2 == dst1 && dst2 != src1 && is_32bit1 == is_32bit2 {
                            let reg_fmt = if is_32bit2 { wreg_name } else { xreg_name };
                            lines[j] = format!("    mov {}, {}", reg_fmt(dst2), reg_fmt(src1));
                            kinds[j] = LineKind::Move {
                                dst: dst2,
                                src: src1,
                                is_32bit: is_32bit2,
                            };
                            changed = true;
                        }
                    }
                }
            }
            LineKind::MoveImm { dst: dst1 } | LineKind::MoveWide { dst: dst1 } => {
                // mov xN, #imm ; mov xM, xN → mov xM, #imm (copy the immediate)
                // Only when dst1 is a scratch register (x0-x15) not callee-saved
                if dst1 <= 15 {
                    let mut j = i + 1;
                    while j < n && kinds[j] == LineKind::Nop {
                        j += 1;
                    }
                    if j < n {
                        if let LineKind::Move {
                            dst: dst2,
                            src: src2,
                            is_32bit: _,
                        } = kinds[j]
                        {
                            if src2 == dst1 {
                                // Copy the immediate instruction, retargeted to dst2
                                let old_line = lines[i].trim();
                                // Replace the register in the first instruction
                                if let Some(new_line) = retarget_move_imm(old_line, dst2) {
                                    lines[j] = format!("    {}", new_line);
                                    kinds[j] = LineKind::MoveImm { dst: dst2 };
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    changed
}

// ── Overwritten-move elimination ─────────────────────────────────────────────
//
// The accumulator-staging emission leaves dead moves like `mov x0, x22` right
// before `add x0, x1, x22` (the add overwrites x0 without reading it). Delete
// any mov/MoveImm whose destination register is provably overwritten before it
// is read. Same-block, forward scan; any control flow or non-pure-write
// mention keeps the move.
fn eliminate_overwritten_moves(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        let dst = match kinds[i] {
            LineKind::Move { dst, .. } => dst,
            LineKind::MoveImm { dst } | LineKind::MoveWide { dst } => dst,
            _ => continue,
        };
        if dst >= 29 {
            continue; // never touch sp/fp-adjacent registers
        }
        let xn = xreg_name(dst);
        let wn = wreg_name(dst);
        let mut dead = false;
        let mut j = i + 1;
        while j < n {
            if kinds[j] == LineKind::Nop {
                j += 1;
                continue;
            }
            match kinds[j] {
                LineKind::Label
                | LineKind::Branch
                | LineKind::CondBranch
                | LineKind::CmpBranch
                | LineKind::Call
                | LineKind::Ret
                | LineKind::Directive => break,
                _ => {}
            }
            let t = &lines[j];
            let mentions = t
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|tok| tok == xn || tok == wn);
            if !mentions {
                j += 1;
                continue;
            }
            // First mention decides: a pure overwrite (dest written, not read)
            // makes the earlier move dead; anything else reads it.
            dead = match kinds[j] {
                LineKind::Move {
                    dst: d2, src: s2, ..
                } => d2 == dst && s2 != dst,
                LineKind::MoveImm { dst: d2 } | LineKind::MoveWide { dst: d2 } => d2 == dst,
                LineKind::LoadSp { reg, .. } | LineKind::LoadswSp { reg, .. } => reg == dst,
                LineKind::Alu => {
                    // Three-operand ALU: first operand is the destination.
                    // Dead only if dst is that dest and not among the sources.
                    let t2 = t.trim();
                    let ops = t2.split_once(' ').map(|x| x.1).unwrap_or("");
                    let mut parts = ops.splitn(2, ',');
                    let first = parts.next().unwrap_or("").trim();
                    let rest = parts.next().unwrap_or("");
                    let first_is_dst = first == xn || first == wn;
                    let rest_reads = rest
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .any(|tok| tok == xn || tok == wn);
                    first_is_dst && !rest_reads
                }
                _ => false,
            };
            break;
        }
        if dead {
            kinds[i] = LineKind::Nop;
            changed = true;
        }
    }
    changed
}

// ── Zero-extend move-chain folding ───────────────────────────────────────────
//
// `mov xA, xS ; mov wA, wA ; mov xD, xA` — the accumulator-path zero-extension
// idiom (the 32-bit self-move clears the top half; it is NOT a no-op, so
// eliminate_self_moves correctly keeps it). The chain computes xD = zext32(wS),
// exactly `mov wD, wS`. Sound only if A is not read later in the block.
fn fold_zext_move_chains(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n.saturating_sub(2) {
        let LineKind::Move {
            dst: a,
            src: s,
            is_32bit: false,
        } = kinds[i]
        else {
            continue;
        };
        let LineKind::Move {
            dst: a2,
            src: s2,
            is_32bit: true,
        } = kinds[i + 1]
        else {
            continue;
        };
        if a2 != a || s2 != a {
            continue; // middle must be mov wA, wA
        }
        let LineKind::Move {
            dst: d,
            src: s3,
            is_32bit: false,
        } = kinds[i + 2]
        else {
            continue;
        };
        if s3 != a || d == a || s == a {
            continue;
        }
        // A must be dead after the chain: no later READ before a boundary.
        // A write-only line (e.g. `mov x0, #15`) ends the old value's life.
        let (xa, wa) = (xreg_name(a), wreg_name(a));
        let mut a_live = false;
        for j in (i + 3)..n {
            match kinds[j] {
                // Control-flow boundaries are NOT "value is dead" proofs.
                // levkropp's break-as-dead default folded d2hi()'s return-
                // value chain (`mov x0,x22; mov w0,w0; mov x23,x0` right
                // before the epilogue) into a dead callee-saved register and
                // returned garbage: aarch64_fuzz seeds 0/2/5 at -O1, 40/100
                // mismatches. ABI-precise handling instead:
                // - ret implicitly reads the return-value registers x0/x1
                //   (and x8 for sret); other registers are dead there.
                LineKind::Ret => {
                    a_live = a == 0 || a == 1 || a == 8;
                    break;
                }
                // - calls read argument registers x0-x8; caller-saved temps
                //   x9-x18 are clobbered unread (dead); callee-saved x19+
                //   survive the call, so the scan must CONTINUE looking for
                //   readers after the call rather than assume death.
                LineKind::Call => {
                    if a <= 8 {
                        a_live = true;
                        break;
                    }
                    if a >= 19 {
                        continue;
                    }
                    break; // x9-x18: clobbered by the call without a read
                }
                // - labels and branches: the value may be read in the
                //   target/fall-through block; this single-block scan cannot
                //   see it, so it must be treated as live.
                LineKind::Label | LineKind::Branch | LineKind::CondBranch | LineKind::CmpBranch => {
                    a_live = true;
                    break;
                }
                LineKind::Nop => continue,
                _ => {}
            }
            let mentions = lines[j]
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|tok| *tok == xa || *tok == wa)
                .count();
            if mentions == 0 {
                continue;
            }
            if gp_reg_write_only(&lines[j], a) {
                break; // write-only: the chain's value in A is dead here
            }
            a_live = true;
            break;
        }
        if a_live {
            continue;
        }
        lines[i] = format!("    mov {}, {}", wreg_name(d), wreg_name(s));
        kinds[i] = LineKind::Move {
            dst: d,
            src: s,
            is_32bit: true,
        };
        kinds[i + 1] = LineKind::Nop;
        kinds[i + 2] = LineKind::Nop;
        changed = true;
    }
    changed
}

/// Retarget a move-immediate instruction to a different destination register.
/// E.g., `mov x0, #5` with new dest x14 → `mov x14, #5`
fn retarget_move_imm(line: &str, new_dst: u8) -> Option<String> {
    // Handle: mov xN, #imm  /  mov xN, :lo12:sym  /  movz xN, #imm  /  movn xN, #imm
    for prefix in &["mov ", "movz ", "movn "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            if let Some((_old_reg, imm_part)) = rest.split_once(", ") {
                let new_reg = if line.contains('w') && !imm_part.starts_with('w') {
                    // If original used w-register (e.g., mov w0, #5)
                    // check if the source had 'w' prefix
                    let old_first = rest.chars().next()?;
                    if old_first == 'w' {
                        wreg_name(new_dst)
                    } else {
                        xreg_name(new_dst)
                    }
                } else {
                    xreg_name(new_dst)
                };
                return Some(format!("{}{}, {}", prefix, new_reg, imm_part));
            }
        }
    }
    None
}

// ── Pass 5: Branch-over-branch fusion ────────────────────────────────────────
//
// Pattern:
//   b.cc .Lskip_N
//   b .target
//   .Lskip_N:
//
// Transform to:
//   b.!cc .target
//
// This is a very common pattern from the codegen: it emits a conditional branch
// to skip over an unconditional branch.

/// Estimate the distance (in instructions) from position `from` to the label
/// `target` in the assembly. Returns `None` if the label is not found.
fn estimate_branch_distance(
    lines: &[String],
    kinds: &[LineKind],
    from: usize,
    target: &str,
) -> Option<usize> {
    // Search both forward and backward for the target label
    let target_with_colon = format!("{}:", target);
    for idx in 0..lines.len() {
        if kinds[idx] == LineKind::Label && lines[idx].trim() == target_with_colon {
            // Count non-Nop lines between from and idx (each is one 4-byte instruction)
            let (lo, hi) = if from < idx { (from, idx) } else { (idx, from) };
            let count = (lo..hi)
                .filter(|&p| kinds[p] != LineKind::Nop && kinds[p] != LineKind::Label)
                .count();
            return Some(count);
        }
    }
    None
}

/// Maximum number of instructions a b.cond can reach (±1MB = ±262144 instructions).
/// Use a conservative threshold of 200,000 to leave margin.
const COND_BRANCH_SAFE_DISTANCE: usize = 200_000;

fn fuse_branch_over_branch(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i + 2 < n {
        if kinds[i] == LineKind::CondBranch {
            // Find the next two non-Nop instructions
            let mut j = i + 1;
            while j < n && kinds[j] == LineKind::Nop {
                j += 1;
            }
            if j >= n {
                i += 1;
                continue;
            }
            let mut k = j + 1;
            while k < n && kinds[k] == LineKind::Nop {
                k += 1;
            }
            if k >= n {
                i += 1;
                continue;
            }

            // Check pattern: b.cc .skip ; b .target ; .skip:
            if kinds[j] == LineKind::Branch && kinds[k] == LineKind::Label {
                if let (Some((cc, skip_target)), Some(real_target), Some(lbl)) = (
                    cond_branch_parts(&lines[i]),
                    branch_target(&lines[j]),
                    label_name(&lines[k]),
                ) {
                    if skip_target == lbl {
                        // Check if the real target is within safe b.cond range.
                        // If the label isn't found in this snippet (external target),
                        // assume it's in range since the unconditional branch already
                        // reached it, and b.cond has ±1MB range which is generous.
                        let in_range = estimate_branch_distance(lines, kinds, i, real_target)
                            .is_none_or(|d| d < COND_BRANCH_SAFE_DISTANCE);
                        if in_range {
                            if let Some(inv_cc) = invert_condition(cc) {
                                // Replace conditional branch with inverted condition to real target
                                lines[i] = format!("    b.{} {}", inv_cc, real_target);
                                // kinds[i] stays as CondBranch
                                // Remove the unconditional branch
                                kinds[j] = LineKind::Nop;
                                // Keep the label (might be targeted by other branches)
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

// ── Global register copy propagation ─────────────────────────────────────────
//
// After store forwarding converts loads into register moves, propagate those
// copies into subsequent instructions. For `mov xDST, xSRC`, replace
// references to xDST with xSRC in the immediately following instruction
// (within the same basic block).

fn propagate_register_copies(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;

    for i in 0..n {
        // Only process 64-bit register-to-register moves
        let (dst, src) = match kinds[i] {
            LineKind::Move {
                dst,
                src,
                is_32bit: false,
            } => {
                // Don't propagate sp/fp moves
                if dst >= 29 || src >= 29 {
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
        // with multiple destination registers (like ldp which has two dest regs
        // after the mnemonic, but our replace_source_reg_in_instruction only
        // treats the first operand as dest).
        match kinds[j] {
            LineKind::Label
            | LineKind::Branch
            | LineKind::Ret
            | LineKind::Directive
            | LineKind::LoadPairSp
            | LineKind::Call => continue,
            _ => {}
        }
        // Also skip ldp/ldaxr/ldxr/stxr/cas-family by checking instruction
        // text, since these have multiple destination registers or complex
        // operand semantics. `cas` covers the whole CAS/CASP family: for
        // CAS, operand 0 (Rs) receives the loaded old value (in-out), and
        // for CASP BOTH pair members of the compare pair (Xs, Xs+1) are
        // in-out — Xs+1 sits after the first comma, exactly where
        // replace_source_reg_in_instruction rewrites "source" operands.
        let trimmed_j = lines[j].trim();
        if trimmed_j.starts_with("ldp ")
            || trimmed_j.starts_with("ldaxr ")
            || trimmed_j.starts_with("ldxr ")
            || trimmed_j.starts_with("stxr ")
            || trimmed_j.starts_with("ldaxp ")
            || trimmed_j.starts_with("ldxp ")
            || trimmed_j.starts_with("cas")
        {
            continue;
        }
        // A pre/post-indexed writeback rewrites the BASE register after the
        // access.  Renaming the alias into such a line would redirect the
        // increment onto the copy's source (`mov x1, x5; str x9, [x1], #8`
        // → `str x9, [x5], #8` clobbers x5, which the alias promised to
        // preserve) or leave the copy's destination un-incremented for its
        // later readers.  The oracle supplies the base write the
        // destination-oriented operand parse cannot see.
        if implicitly_writes_a64(trimmed_j, src) || implicitly_writes_a64(trimmed_j, dst) {
            continue;
        }

        // Try to replace references to dst with src in line j
        let old_line = &lines[j];
        let dst_name = xreg_name(dst);
        if !old_line.contains(dst_name) {
            continue;
        }

        // Don't propagate into instructions that write to the same register
        // as the source (would create a self-reference)
        let src_name = xreg_name(src);

        // For move instructions, replace the source operand
        match kinds[j] {
            LineKind::Move {
                dst: dst2,
                src: src2,
                is_32bit: false,
            } if src2 == dst => {
                // mov X, dst -> mov X, src
                if dst2 != src {
                    lines[j] = format!("    mov {}, {}", xreg_name(dst2), src_name);
                    kinds[j] = LineKind::Move {
                        dst: dst2,
                        src,
                        is_32bit: false,
                    };
                    changed = true;
                }
            }
            _ => {
                // General case: try to replace the register in the instruction text.
                // Only replace in source operand positions (not destination).
                if let Some(new_line) =
                    replace_source_reg_in_instruction(old_line, dst_name, src_name)
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

// Shared peephole string utilities -- see backend/peephole_common.rs
use crate::backend::peephole_common::{replace_source_reg_in_instruction, replace_whole_word};

// ── Global dead store elimination ────────────────────────────────────────────
//
// Scans the entire function to find stack slot offsets that are never loaded.
// Stores to such slots are dead and can be eliminated.
// This runs after global store forwarding, which may have converted many loads
// to register moves, leaving the original stores dead.

// REJECTED ADOPTION: levkropp's GDSE escape-analysis rewrite (e03da2f1 /
// e20cdf6e) caused 9 regressions on our tree (volatile_loop, fp_cmp_inf,
// unroll_const4_*, gvn_param_alloca_load, hash_narrow_chain,
// reduction_two_sums, glibc_ld_builtins — all bisected to CCC_NO_GDSE).
// Our codegen emits sp-derived access patterns his tracker misclassifies.
// Keeping the conservative bail-on-address-of-sp version: sound by
// construction, and the loop-variant-base hazard his rewrite fixes cannot
// arise here because we never resolve loads through derived bases at all.
//
// ADOPTED from levkropp 1a432a1a (audited): function-scoped analysis. The
// peephole runs on the WHOLE MODULE text (backend/mod.rs), so the old
// whole-buffer scan had two inter-function bugs:
// (a) one address-of-sp anywhere disabled the pass for EVERY function in
//     the module (missed optimization, module-size dependent), and
// (b) the load scan collected slot ranges across all functions, so a load
//     at [sp, #16] in function A kept a dead store to [sp, #16] alive in
//     function B (different frames — pure missed optimization, but also a
//     latent soundness trap should ranges ever be used for forwarding).
// Split at .cfi_endproc boundaries; each frame is analyzed independently.
fn global_dead_store_elimination(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    let mut start = 0usize;
    for i in 0..n {
        if lines[i].trim().starts_with(".cfi_endproc") {
            changed |= gdse_one_function(lines, kinds, start, i + 1);
            start = i + 1;
        }
    }
    if start < n {
        changed |= gdse_one_function(lines, kinds, start, n);
    }
    changed
}

/// Entry-sp-relative displacement of sp at each line index (`disps[i]` is the
/// displacement in effect FOR line i's own accesses). `None` when the region
/// contains an sp write that cannot be modelled statically (register-indirect
/// adjustment) — slot-offset passes must then stay conservative.
///
/// Call argument staging (`sub sp, sp, #N` / `add sp, sp, #N` around the
/// outgoing-args area) and writeback push/pop pairs (`stp x9, x10,
/// [sp, #-16]!` ... `ldp x9, x10, [sp], #16`) move sp mid-function. Passes
/// that match slot offsets textually ("same [sp, #off]") would otherwise pair
/// accesses that resolve to DIFFERENT addresses across such an adjustment —
/// the aggregate_dse_soundness / alu_peepholes >8-argument call staging wrote
/// argument spills to frame slots and then read them through the shifted sp
/// while a deleted "duplicate" store had actually targeted a different byte.
/// Displacement resets at `.cfi_startproc` (every emitted function prologue).
fn sp_displacements(lines: &[String], kinds: &[LineKind], n: usize) -> Option<Vec<i64>> {
    let mut disps = vec![0i64; n];
    let mut disp: i64 = 0;
    for i in 0..n {
        let t = lines[i].trim();
        if kinds[i] == LineKind::Nop {
            // Eliminated lines do not execute in the final program: their sp
            // effect is gone (their text lingers for the printer's skip).
            disps[i] = disp;
            continue;
        }
        if kinds[i] == LineKind::Directive && t.contains(".cfi_startproc") {
            disp = 0;
        }
        disps[i] = disp;
        // Apply line i's sp effect for subsequent lines.
        if let Some(body) = t.strip_prefix("stp ").or_else(|| t.strip_prefix("ldp ")) {
            if let Some(bracket) = body.find('[') {
                let addr = &body[bracket..];
                if addr.starts_with("[sp") {
                    if let Some(rest) = addr.strip_prefix("[sp, #") {
                        if let Some(stripped) = rest.strip_suffix("]!") {
                            // Pre-index writeback: sp += offset before access.
                            let off: i64 = stripped.parse().ok()?;
                            disp += -off;
                            continue;
                        }
                    }
                    if let Some(rest) = addr.strip_prefix("[sp], #") {
                        // Post-index writeback: sp += offset after access.
                        let end = rest.find(']')?;
                        let off: i64 = rest[..end].parse().ok()?;
                        disp -= off;
                        continue;
                    }
                }
            }
        }
        if (t.starts_with("sub ") || t.starts_with("add ")) && t.contains("sp, sp, #") {
            let off: i64 = t.rsplit('#').next()?.parse().ok()?;
            if t.starts_with("sub ") {
                disp += off;
            } else {
                disp -= off;
            }
        }
    }
    Some(disps)
}

fn gdse_one_function(lines: &[String], kinds: &mut [LineKind], start: usize, end: usize) -> bool {
    let mut changed = false;
    // Safety check: if any instruction takes the address of sp (e.g., `add xN, sp, #off`),
    // stack slots could be accessed through pointers, so we must not eliminate any stores.
    // This is conservative but sound — it prevents miscompilation when arrays or structs
    // are allocated on the stack and passed by pointer to callees.
    for i in start..end {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        let trimmed = lines[i].trim();
        // Check for address-of-sp patterns:
        // - `add xN, sp, #offset` (common for array/struct address)
        // - `mov xN, sp` (copying stack pointer)
        // - `sub xN, sp, #N` (stack address computation)
        if (trimmed.starts_with("add ") || trimmed.starts_with("sub ")) && trimmed.contains(", sp,")
        {
            return false;
        }
        if trimmed.starts_with("mov ") && trimmed.contains(", sp") {
            // Check it's actually "mov xN, sp" not "mov sp, xN"
            if let Some(rest) = trimmed.strip_prefix("mov ") {
                if let Some((_, src)) = rest.split_once(", ") {
                    if src.trim() == "sp" {
                        return false;
                    }
                }
            }
        }
    }

    // Offsets are resolved through the shared sp-displacement model: a
    // `[sp, #N]` access inside an `sub sp` staging window reads frame byte
    // N + disp, not N (see sp_displacements for the miscompile history).
    let Some(disps) = sp_displacements(lines, kinds, end) else {
        return false;
    };

    // Phase 1: Collect all (offset, size) byte ranges that are loaded from,
    // in ENTRY-sp-relative frame coordinates (raw offset + current sp disp).
    // We must use byte-range overlap (not exact offset match) because a wide
    // store (e.g. `str x` at offset 16, 8 bytes) can be partially read by a
    // narrower load at a different offset (e.g. `ldr w` at offset 20, 4 bytes).
    let mut loaded_ranges: Vec<(i32, i32)> = Vec::new(); // (offset, size)
    for i in start..end {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        let sp_disp = disps[i];
        let mut push_load = |off: i64, size: i32, out: &mut Vec<(i32, i32)>| {
            if let Ok(off) = i32::try_from(off) {
                out.push((off, size));
            }
        };
        match kinds[i] {
            LineKind::LoadSp {
                offset, is_word, ..
            } => {
                let size = if is_word { 4 } else { 8 };
                push_load(offset as i64 + sp_disp, size, &mut loaded_ranges);
            }
            LineKind::LoadswSp { offset, .. } => {
                push_load(offset as i64 + sp_disp, 4, &mut loaded_ranges);
            }
            _ => {
                // Check for loads in Other instructions (e.g., ldp, ldrb, ldrh, etc.)
                let trimmed = lines[i].trim();
                // Writeback forms move sp themselves (already tracked above);
                // their raw offset is pre/post-adjustment — skip classification.
                if trimmed.contains("]!") || trimmed.contains("], #") {
                    continue;
                }
                let load_size = if trimmed.starts_with("ldp ") {
                    Some(16) // ldp loads two 8-byte registers
                } else if trimmed.starts_with("ldr x") || trimmed.starts_with("ldur x") {
                    Some(8)
                } else if trimmed.starts_with("ldr w")
                    || trimmed.starts_with("ldur w")
                    || trimmed.starts_with("ldrsw ")
                {
                    Some(4)
                } else if trimmed.starts_with("ldrh ") || trimmed.starts_with("ldrsh ") {
                    Some(2)
                } else if trimmed.starts_with("ldrb ") || trimmed.starts_with("ldrsb ") {
                    Some(1)
                } else if trimmed.starts_with("ldr ") || trimmed.starts_with("ldur ") {
                    Some(8) // default to 8 for unqualified ldr
                } else {
                    None
                };
                if let Some(sz) = load_size {
                    if trimmed.contains("[sp") {
                        if let Some(off) = extract_sp_offset(trimmed) {
                            push_load(off as i64 + sp_disp, sz, &mut loaded_ranges);
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Remove stores whose byte range does not overlap any load range.
    // Stores use the same displacement-adjusted coordinates as the loads.
    for i in start..end {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        let sp_disp = disps[i];
        if let LineKind::StoreSp {
            offset, is_word, ..
        } = kinds[i]
        {
            let store_size = if is_word { 4 } else { 8 };
            if let Ok(store_off) = i32::try_from(offset as i64 + sp_disp) {
                let overlaps_any_load = loaded_ranges.iter().any(|&(load_off, load_sz)| {
                    // Two ranges [a, a+as) and [b, b+bs) overlap iff a < b+bs && b < a+as
                    store_off < load_off + load_sz && load_off < store_off + store_size
                });
                if !overlaps_any_load {
                    kinds[i] = LineKind::Nop;
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Parse `[xN]`, `[xN, #k]` (no writeback) into (base register, offset).
fn parse_base_addr(inner_bracketed: &str) -> Option<(u8, i32)> {
    let inner = inner_bracketed.strip_prefix('[')?;
    let inner = inner.split_once(']').map(|(h, _)| h).unwrap_or(inner);
    let (base, off) = match inner.split_once(", ") {
        Some((b, o)) => (b.trim(), o.trim().strip_prefix('#')?.parse::<i32>().ok()?),
        None => (inner.trim(), 0),
    };
    let b = parse_reg(base);
    (b < 29).then_some((b, off))
}

/// Maintain the sp-derived base-register map for one line of text:
/// `add/sub xN, sp, #k` creates, `mov` copies or ends a lineage, `add/sub`
/// on a base adjusts or derives, calls end caller-saved lineages, and any
/// other write of a tracked register ends its lineage.
fn track_sp_bases(line: &str, kind: LineKind, base_off: &mut FxHashMap<u8, i32>) {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("add ").or_else(|| t.strip_prefix("sub ")) {
        let neg = t.starts_with("sub ");
        if let Some((dst_s, srcs)) = rest.split_once(", ") {
            if let Some((src_s, imm_s)) = srcs.split_once(", ") {
                let dst = parse_reg(dst_s.trim());
                let src = parse_reg(src_s.trim());
                let imm = imm_s
                    .trim()
                    .strip_prefix('#')
                    .and_then(|v| v.parse::<i32>().ok());
                if let Some(imm) = imm {
                    let signed = if neg { -imm } else { imm };
                    if src_s.trim() == "sp" && dst < 29 {
                        base_off.insert(dst, signed);
                        return;
                    }
                    if let Some(&off) = base_off.get(&src) {
                        if dst < 29 {
                            base_off.insert(dst, off + signed);
                        }
                    }
                }
            }
        }
    } else if let Some(rest) = t.strip_prefix("mov ") {
        if let Some((dst_s, src_s)) = rest.split_once(", ") {
            let (dst_s, src_s) = (dst_s.trim(), src_s.trim());
            let dst = parse_reg(dst_s);
            if src_s == "sp" {
                if dst < 29 {
                    base_off.insert(dst, 0);
                }
                return;
            }
            let src = parse_reg(src_s);
            if let Some(&off) = base_off.get(&src) {
                if dst < 29 {
                    base_off.insert(dst, off);
                }
            } else if dst < 29 {
                // mov xN, <non-base>: clobbered, lineage ends.
                base_off.remove(&dst);
            }
        }
    }
    // Calls clobber caller-saved registers: end those lineages.
    if kind == LineKind::Call {
        base_off.retain(|&r, _| r >= 19);
        return;
    }
    let mut dead = Vec::new();
    for &b in base_off.keys() {
        if gp_reg_written_broad(line, b) || written_gp_register(line, kind) == Some(b) {
            dead.push(b);
        }
    }
    for b in dead {
        base_off.remove(&b);
    }
}

/// Extract the numeric offset from an instruction containing `[sp, #N]` or `[sp]`.
fn extract_sp_offset(line: &str) -> Option<i32> {
    if let Some(start) = line.find("[sp") {
        let rest = &line[start..];
        if rest.starts_with("[sp]") {
            return Some(0);
        }
        if rest.starts_with("[sp, #") {
            let num_start = start + 6; // skip "[sp, #"
            let after = &line[num_start..];
            if let Some(end) = after.find(']') {
                return after[..end].parse::<i32>().ok();
            }
        }
    }
    None
}

/// Loop rotation: convert header-tested loops into bottom-tested loops.
///
/// ```text
/// .Lhdr:                      .Lhdr:                 (guard, runs once)
///     cmp a, b                    cmp a, b
///     b.cc .Lbody                 b.cc .Lbody
/// .Lskip:                       .Lskip:
///     b .Lexit                    b .Lexit
/// .Lbody:            →          .Lbody:
///     <body>                      <body>
///     b .Lhdr                     cmp a, b           (duplicated test)
///                                 b.cc .Lbody        (one branch per iteration)
///                                 b .Lskip           (taken once, on exit)
/// ```
///
/// Saves one unconditional branch per iteration. Sound because every register
/// the header test reads is live throughout the loop (it is read again next
/// iteration), so it is never clobbered by unrelated assignments, and loop
/// phis have already been updated to the next iteration's value at the latch
/// (backedge phi copies are emitted at the end of the latch, before the
/// backedge branch this pass replaces).
fn rotate_simple_loops(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        if kinds[i] != LineKind::Branch {
            continue;
        }
        let Some(header_target) = branch_target(lines[i].trim_start()) else {
            continue;
        };
        // Find the header label earlier in the text (a backward branch).
        let header_label = format!("{}:", header_target);
        let mut header_pos = None;
        for l in 0..i {
            if kinds[l] == LineKind::Label && lines[l].trim() == header_label {
                header_pos = Some(l);
                break;
            }
        }
        let Some(h) = header_pos else {
            continue;
        };

        // Single backedge only: this must be the one and only `b .Lhdr`.
        let backedge_count = (0..n)
            .filter(|&j| {
                kinds[j] == LineKind::Branch
                    && branch_target(lines[j].trim_start()) == Some(header_target)
            })
            .count();
        if backedge_count != 1 {
            continue;
        }

        // No calls, returns, or indirect branches inside the loop.
        let mut bad = false;
        for j in h..=i {
            match kinds[j] {
                LineKind::Ret | LineKind::Call | LineKind::CmpBranch => {
                    bad = true;
                    break;
                }
                _ => {
                    let t = lines[j].trim_start();
                    if t.starts_with("br ") || t.starts_with("blr ") {
                        bad = true;
                        break;
                    }
                }
            }
        }
        if bad {
            continue;
        }

        // Collect the header setup: pure, flag-preserving instructions after
        // the label, ending in a Compare, followed by a conditional branch.
        let mut setup: Vec<usize> = Vec::new();
        let mut pos = h + 1;
        let mut cj = None;
        while pos < i {
            if kinds[pos] == LineKind::Nop {
                pos += 1;
                continue;
            }
            match kinds[pos] {
                LineKind::CondBranch => {
                    cj = Some(pos);
                    break;
                }
                LineKind::Compare
                | LineKind::Move { .. }
                | LineKind::MoveImm { .. }
                | LineKind::MoveWide { .. }
                | LineKind::Sxtw { .. }
                | LineKind::LoadSp { .. }
                | LineKind::LoadswSp { .. }
                | LineKind::LoadPairSp { .. } => {
                    setup.push(pos);
                    if setup.len() > 3 {
                        break;
                    }
                    pos += 1;
                }
                _ => break,
            }
        }
        let Some(cj_pos) = cj else { continue };
        if setup.is_empty() || setup.len() > 3 {
            continue;
        }
        // The compare must be the last setup instruction: it sets the flags
        // the conditional branch consumes (moves/loads don't touch flags).
        if kinds[*setup.last().unwrap()] != LineKind::Compare {
            continue;
        }

        // The conditional branch gives the loop-back condition and body target.
        let Some((cc, body_target)) = cond_branch_parts(lines[cj_pos].trim_start()) else {
            continue;
        };
        // The body target must be a label inside the loop, after the cond branch.
        let body_label = format!("{}:", body_target);
        let body_found = ((cj_pos + 1)..=i)
            .any(|l| kinds[l] == LineKind::Label && lines[l].trim() == body_label);
        if !body_found {
            continue;
        }

        // The exit target is the first label after the cond branch: the
        // not-taken path of the header guard (typically the .Lskip trampoline
        // or the exit block itself).
        let mut exit_target = None;
        for l in (cj_pos + 1)..n {
            if kinds[l] == LineKind::Nop {
                continue;
            }
            if kinds[l] == LineKind::Label {
                exit_target = label_name(lines[l].trim());
            }
            break;
        }
        let Some(exit_lbl) = exit_target else {
            continue;
        };
        if exit_lbl == body_target || exit_lbl == header_target {
            continue;
        }

        // Build the rotated latch: setup copy + cond branch to body + exit.
        let mut repl = String::new();
        for &s in &setup {
            repl.push_str(&lines[s]);
            repl.push('\n');
        }
        repl.push_str(&format!("    b.{} {}", cc, body_target));
        repl.push('\n');
        repl.push_str(&format!("    b {}", exit_lbl));
        lines[i] = repl;
        // The slot now holds a multi-line sequence; keep branch passes from
        // re-examining it (its embedded branches are final).
        kinds[i] = LineKind::Other;
        changed = true;
    }
    changed
}

// ── Loop-carried store sinking ───────────────────────────────────────────────
//
// The slot-home discipline gives every SSA value a mandatory stack slot, so a
// loop-carried value is stored to its slot once per iteration even when
// nothing in the loop ever loads the slot back (the value stays live in a
// register; the slot is only read after the loop).  Sink such a store to the
// loop's fall-through exit:
//
//     .Lbody:                       .Lbody:
//       ...                           ...
//       str x0, [sp, #24]   -->       ...            (store removed)
//       ...                           ...
//       b.le .Lbody                   b.le .Lbody
//       b .Lexit                      str x0, [sp, #24]  (stored once)
//                                     b .Lexit
//
// Soundness conditions (all checked below): the slot is not otherwise
// referenced inside the loop, this is the only store to it, every path from
// the loop head to the fall-through exit executes the store (no labels or
// exiting branches between the head and the store, no labels between the
// store and the backedge), the only exit is the fall-through past the
// backedge, and the stored register is not rewritten between the store and
// the backedge.  Then the register holds the last-stored value at the exit,
// so storing it there once produces the same slot contents as the original.

/// Register number of a scalar register operand (`w0`/`x19`/`d24`/`s3` →
/// 0/19/24/3).  The class is ignored: a match on the number alone is a
/// conservative clobber indication.
fn reg_operand_num(op: &str) -> Option<u8> {
    let op = op.trim();
    let rest = op.strip_prefix(|c| matches!(c, 'w' | 'x' | 'd' | 's' | 'q' | 'b' | 'h'))?;
    rest.parse::<u8>().ok().filter(|&n| n <= 30)
}

/// Does this instruction write a register with the given number?
/// Conservative: the first operand (and the second for `ldp`) counts as a
/// destination for everything that is not a store, compare, or branch.
// ── Zero-store via wzr/xzr (levkropp 86b34508, audited port) ────────────────
//
// Storing a literal zero goes through a materialization: `mov x0, #0` then
// `strb w0, [x9]`. The zero register does it for free: `strb wzr, [x9]`.
// The mov is left for dead-move elimination (it may have other readers).
// Audit deltas vs his version: dst < 29 (his `<= 30` admitted the frame
// pointer and link register as tracked zero sources — sound but absurd,
// and x29-relative rewrites deserve no code path); everything else
// verified line-by-line (store-operand-only rewrite keeps a zr_reg BASE
// untouched, movn excluded as -1, multi-line slots skipped, scan stops at
// the first barrier/clobber/load).
fn fold_zero_stores(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    let mut changed = false;
    for i in 0..n {
        let zr_reg = match kinds[i] {
            LineKind::MoveImm { dst } | LineKind::MoveWide { dst } if dst < 29 => {
                let t = lines[i].trim();
                // movn #0 is -1, not zero; symbol operands fail the parse.
                if t.starts_with("movn") || lines[i].contains('\n') {
                    continue;
                }
                let zero = t
                    .rsplit_once(", ")
                    .and_then(|(_, imm)| imm.trim().strip_prefix('#'))
                    .is_some_and(|v| v.parse::<i64>() == Ok(0));
                if zero {
                    dst
                } else {
                    continue;
                }
            }
            _ => continue,
        };
        let (xname, wname) = (xreg_name(zr_reg), wreg_name(zr_reg));
        // Rewrite the value operand of the next store that reads R, stopping
        // at the first barrier, rewrite of R, or other memory op.
        for j in (i + 1)..n {
            match kinds[j] {
                LineKind::Nop => continue,
                LineKind::Move { .. }
                | LineKind::MoveImm { .. }
                | LineKind::MoveWide { .. }
                | LineKind::Alu
                | LineKind::Compare => {
                    if instr_clobbers_reg(lines[j].trim_start(), zr_reg) {
                        break;
                    }
                    continue;
                }
                LineKind::StoreSp { .. } | LineKind::MemOther | LineKind::StorePairSp => {
                    let t = lines[j].trim_start().to_string();
                    if !(t.starts_with("str") || t.starts_with("stp")) {
                        break; // a load under MemOther: stop
                    }
                    if let Some((mnem, rest)) = t.split_once(' ') {
                        if let Some((first, suffix)) = rest.split_once(',') {
                            let first = first.trim();
                            let zr = if first == xname {
                                "xzr"
                            } else if first == wname {
                                "wzr"
                            } else {
                                break;
                            };
                            lines[j] = format!("    {} {},{}", mnem, zr, suffix);
                            kinds[j] = classify_line(&lines[j]);
                            changed = true;
                        }
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    changed
}

fn instr_clobbers_reg(trimmed: &str, num: u8) -> bool {
    let mut it = trimmed.splitn(2, char::is_whitespace);
    let mnem = it.next().unwrap_or("");
    let ops = it.next().unwrap_or("");
    if ops.is_empty() {
        return false; // labels, directives, ret, bare branches
    }
    // Architectural implicit writes decide first: a pre/post-indexed base
    // register is rewritten even by the store/load mnemonics excluded
    // below (`strb w12, [x9], #1` clobbers x9 — the store exclusion is
    // only correct for the DATA operand), and bl/blr link through x30.
    if implicitly_writes_a64(trimmed, num) {
        return true;
    }
    // Stores and comparisons have no register destination (modulo the
    // writeback handled above).
    if mnem.starts_with("st")
        || mnem.starts_with("cmp")
        || mnem.starts_with("cmn")
        || mnem.starts_with("tst")
        || mnem.starts_with("fcmp")
    {
        return false;
    }
    // Branches never write a register — except the linking ones, whose
    // x30 write was reported by the oracle above.
    if mnem == "b"
        || mnem.starts_with("b.")
        || mnem == "br"
        || mnem == "cbz"
        || mnem == "cbnz"
        || mnem == "tbz"
        || mnem == "tbnz"
    {
        return false;
    }
    if mnem == "bl" || mnem == "blr" {
        // The target operand is a pure read (blr) or a symbol (bl).
        return false;
    }
    let mut operands = ops.split(", ");
    if let Some(first) = operands.next() {
        if reg_operand_num(first) == Some(num) {
            return true;
        }
    }
    if mnem == "ldp" {
        if let Some(second) = operands.next() {
            if reg_operand_num(second) == Some(num) {
                return true;
            }
        }
    }
    false
}

/// Parse `str <reg>, [sp, #off]` for scalar regs (w/x/s/d) → (reg_num, offset).
fn parse_sp_store_any(trimmed: &str) -> Option<(u8, i32)> {
    let rest = trimmed.strip_prefix("str ")?;
    let (reg_str, addr) = rest.split_once(", ")?;
    let num = reg_operand_num(reg_str)?;
    let off = parse_sp_offset(addr.trim())?;
    Some((num, off))
}

/// Byte size of a scalar `str` register operand (w/s=4, x/d=8).
fn store_reg_size(trimmed: &str) -> Option<u32> {
    let rest = trimmed.strip_prefix("str ")?;
    let (reg_str, _) = rest.split_once(", ")?;
    let r = reg_str.trim();
    if r.starts_with('w') || r.starts_with('s') {
        Some(4)
    } else if r.starts_with('x') || r.starts_with('d') {
        Some(8)
    } else {
        None
    }
}

// ── Overwritten store elimination ────────────────────────────────────────────
//
// Local dead-store peephole: `str R, [sp, #off]` followed by another store to
// the same slot with no intervening read (and no label, branch, call, or
// unknown memory op between). The first store is dead. This is the store
// analog of eliminate_overwritten_moves and catches cases the global DSE
// misses when it bails on a whole function due to one escaped frame pointer.
fn eliminate_overwritten_stores(lines: &[String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_STORE_DSE").is_ok() {
        return false;
    }
    // "Same slot" means the same FRAME BYTE, not the same textual offset: the
    // outgoing-args staging window (`sub sp, sp, #N` ... `add sp, sp, #N`)
    // reuses small offsets for call arguments, and a write there must never
    // retire an entry-frame store at the same raw offset (see
    // sp_displacements for the miscompile history). Unmodellable sp writes
    // disable the pass for the whole file.
    let Some(disps) = sp_displacements(lines, kinds, n) else {
        return false;
    };
    let mut changed = false;
    for i in 0..n {
        let LineKind::StoreSp {
            offset: off_i,
            is_word: word_i,
            ..
        } = kinds[i]
        else {
            continue;
        };
        let size_i = if word_i { 4u32 } else { 8 };
        let eff_i = off_i as i64 + disps[i];
        let mut j = i + 1;
        let mut steps = 0;
        while j < n && steps < 40 {
            steps += 1;
            if lines[j].contains('\n') {
                break; // multi-line slot may hide memory ops
            }
            match kinds[j] {
                LineKind::Nop => {
                    j += 1;
                    continue;
                }
                LineKind::StoreSp {
                    offset: off_j,
                    is_word: word_j,
                    ..
                } => {
                    let size_j = if word_j { 4u32 } else { 8 };
                    let eff_j = off_j as i64 + disps[j];
                    if eff_j == eff_i && size_j >= size_i {
                        // Fully overwritten with no intervening read.
                        kinds[i] = LineKind::Nop;
                        changed = true;
                    }
                    if eff_j < eff_i + size_i as i64 && eff_i < eff_j + size_j as i64 {
                        break; // overlapping store: stop either way
                    }
                    j += 1;
                    continue;
                }
                LineKind::LoadSp {
                    offset: off_j,
                    is_word: word_j,
                    ..
                } => {
                    let size_j = if word_j { 4u32 } else { 8 };
                    let eff_j = off_j as i64 + disps[j];
                    if eff_j < eff_i + size_i as i64 && eff_i < eff_j + size_j as i64 {
                        break; // reads our slot (or overlaps it)
                    }
                    j += 1;
                    continue;
                }
                LineKind::LoadswSp { offset: off_j, .. } => {
                    let eff_j = off_j as i64 + disps[j];
                    if eff_j < eff_i + size_i as i64 && eff_i < eff_j + 4 {
                        break;
                    }
                    j += 1;
                    continue;
                }
                // Straight-line, non-memory instructions are transparent.
                LineKind::Move { .. }
                | LineKind::MoveImm { .. }
                | LineKind::MoveWide { .. }
                | LineKind::Sxtw { .. }
                | LineKind::Compare
                | LineKind::Alu => {
                    j += 1;
                    continue;
                }
                // Labels, branches, calls, pair/unknown memory ops, anything
                // else: stop. `Other` covers unknown instructions that might
                // touch memory, so it stops the scan too.
                _ => break,
            }
        }
    }
    changed
}

/// Branch target label of a (sub-)line, for plain/conditional/compare branches.
fn any_branch_target(trimmed: &str) -> Option<&str> {
    if let Some(rest) = trimmed.strip_prefix("b ") {
        let t = rest.trim();
        if t.starts_with('.') {
            return Some(t);
        }
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("b.") {
        if let Some((_, t)) = rest.split_once(' ') {
            let t = t.trim();
            if t.starts_with('.') {
                return Some(t);
            }
        }
        return None;
    }
    for p in ["cbz ", "cbnz ", "tbz ", "tbnz "] {
        if trimmed.starts_with(p) {
            if let Some((_, t)) = trimmed.rsplit_once(", ") {
                let t = t.trim();
                if t.starts_with('.') {
                    return Some(t);
                }
            }
        }
    }
    None
}

fn sink_loop_carried_stores(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_STORE_SINK").is_ok() {
        return false;
    }
    // Flatten to virtual sub-lines: rotation leaves multi-line slots whose
    // embedded latch branches this pass must still see.
    let mut v: Vec<(usize, usize, String)> = Vec::new(); // (slot, sub_idx, text)
    for i in 0..n {
        if kinds[i] == LineKind::Nop {
            continue;
        }
        for (si, sub) in lines[i].split('\n').enumerate() {
            let t = sub.trim();
            if !t.is_empty() {
                v.push((i, si, t.to_string()));
            }
        }
    }
    let nv = v.len();
    // Label name -> virtual index.
    let mut labels: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (vi, (_, _, t)) in v.iter().enumerate() {
        if let Some(name) = t.strip_suffix(':') {
            labels.insert(name, vi);
        }
    }
    // Find loop regions: a conditional backward branch ends a bottom-tested
    // loop whose exit is the fall-through past the branch.
    let mut regions: Vec<(usize, usize)> = Vec::new(); // (head vi, backedge vi)
    for vj in 0..nv {
        let t = &v[vj].2;
        let conditional = t.starts_with("b.")
            || t.starts_with("cbz ")
            || t.starts_with("cbnz ")
            || t.starts_with("tbz ")
            || t.starts_with("tbnz ");
        if !conditional {
            continue;
        }
        let Some(target) = any_branch_target(t) else {
            continue;
        };
        let Some(&vi) = labels.get(target) else {
            continue;
        };
        if vi < vj {
            regions.push((vi, vj));
        }
    }
    // Largest regions first: sink each store as far out as possible.
    regions.sort_by_key(|&(h, j)| std::cmp::Reverse(j - h));

    let mut sunk_slot: Vec<bool> = vec![false; n];
    // (store slot, store text, backedge slot, backedge sub_idx)
    let mut edits: Vec<(usize, String, usize, usize)> = Vec::new();

    for &(h, j) in &regions {
        // Region-level hazards: calls, returns, or branches to unknown labels.
        let mut region_bad = false;
        for p in h..=j {
            let t = &v[p].2;
            if t.starts_with("bl ") || t.starts_with("blr ") || t == "ret" {
                region_bad = true;
                break;
            }
            if let Some(tgt) = any_branch_target(t) {
                if !labels.contains_key(tgt) {
                    region_bad = true;
                    break;
                }
            }
        }
        if region_bad {
            continue;
        }
        // The backedge slot must contain no other backward branch: splicing
        // would shift the second branch's sub-index.
        let bslot = v[j].0;
        let mut other_backedge = false;
        for p in h..j {
            if v[p].0 != bslot {
                continue;
            }
            if let Some(tgt) = any_branch_target(&v[p].2) {
                if let Some(&tp) = labels.get(tgt) {
                    if tp < p {
                        other_backedge = true;
                        break;
                    }
                }
            }
        }
        if other_backedge {
            continue;
        }
        // Exit branches: any branch in the region targeting outside it.
        // (The backedge at j targets the head, which is inside.)
        let mut exits: Vec<usize> = Vec::new();
        for p in h..j {
            if let Some(tgt) = any_branch_target(&v[p].2) {
                let tp = labels[tgt];
                if tp < h || tp > j {
                    exits.push(p);
                }
            }
        }
        // Candidate stores.
        for k in (h + 1)..j {
            let (slot, _, ref text) = v[k];
            if sunk_slot[slot] || lines[slot].contains('\n') {
                continue; // already sunk, or not a standalone line
            }
            if !matches!(kinds[slot], LineKind::StoreSp { .. } | LineKind::MemOther) {
                continue;
            }
            let Some((num, off)) = parse_sp_store_any(text) else {
                continue;
            };
            let debug = std::env::var("CCC_SINK_DEBUG").is_ok();
            let slot_ref = format!("[sp, #{}]", off);
            let mut bad = false;
            // (a) Nothing else in the region references this slot.
            for p in h..=j {
                if p == k {
                    continue;
                }
                let t = &v[p].2;
                if t.contains(&slot_ref) {
                    bad = true;
                    break;
                }
                // Pair ops cover a range: stp/ldp at base M spans M..M+16.
                if t.starts_with("stp ") || t.starts_with("ldp ") {
                    if let Some(base) = extract_sp_offset(t) {
                        if base <= off && off < base + 16 {
                            bad = true;
                            break;
                        }
                    }
                }
            }
            if bad && debug {
                eprintln!("sink bail: {} (slot re-referenced in region)", text);
            }
            // (b) No exiting branch anywhere (v1: fall-through exit only).
            if !bad && !exits.is_empty() {
                bad = true;
                if debug {
                    eprintln!("sink bail: {} ({} exit branches)", text, exits.len());
                }
            }
            // (c) No labels between the store and the backedge (no way to
            // reach the exit without passing the store), and none before the
            // store either except the head (every path to the exit passes k).
            if !bad {
                for p in (h + 1)..j {
                    if p == k {
                        continue;
                    }
                    if v[p].2.ends_with(':') {
                        bad = true;
                        if debug {
                            eprintln!("sink bail: {} (label {} in region)", text, v[p].2);
                        }
                        break;
                    }
                }
            }
            // (d) The stored register survives to the backedge.
            if !bad {
                for p in (k + 1)..=j {
                    if instr_clobbers_reg(&v[p].2, num) {
                        bad = true;
                        if debug {
                            eprintln!("sink bail: {} (clobbered by {})", text, v[p].2);
                        }
                        break;
                    }
                }
            }
            if bad {
                continue;
            }
            sunk_slot[slot] = true;
            edits.push((slot, format!("    {}", text), v[j].0, v[j].1));
            if debug {
                eprintln!("sink ok: {}", text);
            }
        }
    }

    if edits.is_empty() {
        return false;
    }
    for (slot, _, _, _) in &edits {
        kinds[*slot] = LineKind::Nop;
    }
    // Splice each sunk store into the backedge slot, right after the branch.
    // Insertions land after the backedge sub-line, so it keeps its original
    // sub-index no matter how many stores are spliced in.
    for (_, text, bj, bsub) in &edits {
        let mut subs: Vec<&str> = lines[*bj].split('\n').collect();
        subs.insert(bsub + 1, text.as_str());
        lines[*bj] = subs.join("\n");
        kinds[*bj] = LineKind::Other;
    }
    true
}

// ── Pass 6: Spill-slot threading through a scratch register ────────────────
//
// Pattern (RA artifact; the ring_fifo / struct_copy class): a value is
// staged through its stack slot even though every reader sits in the same
// straight-line region as the store, and the source register is overwritten
// in between:
//
//     str x0, [sp, #24]        ; spill
//     ... insns that overwrite x0 ...
//     ldr x0, [sp, #24]        ; reload of the spilled value
//
// Pass 1 only rewrites windows whose source register survives; once x0 is
// clobbered the reload must really read memory — unless a caller-saved
// register that nothing else touches threads the value across:
//
//     mov xR, x0               ; replaces (or precedes) the store
//     ...
//     mov x0, xR               ; replaces each reload
//
// Soundness rules, all checked before any rewrite:
//   * window = store+1 .. last paired reload; labels, branches, calls,
//     returns and pair accesses end it; an unclassified store (a possible
//     frame alias through a base register like x19) also ends it;
//   * no intervening store to the same slot offset;
//   * x-form loads pair only with x-form stores (`ldr xD` of a w-stored
//     slot reads the upper half's stale bytes — not reproducible by a move);
//   * the scratch register is mentioned NOWHERE in the window and NOWHERE
//     else in the function outside the rewritten lines, so its live range
//     is exactly the window;
//   * the store is deleted only when a function-wide textual census proves
//     the offset has no other reference at all (pair ops and stur/ldur
//     included); otherwise the store stays and only the reloads become
//     moves (readers across back edges still see the slot).
//
// Kill switch: CCC_NO_SPILL_THREAD.

/// Characteristic memory-operand text of an sp-relative slot reference.
fn sp_slot_token(offset: i32) -> String {
    format!("[sp, #{}]", offset)
}

/// All `[sp, #k]` slot offsets referenced by a line (any width or opcode).
fn sp_offsets_in_line(line: &str) -> Vec<i32> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while i + 6 <= b.len() {
        if &b[i..i + 5] == b"[sp, " {
            let mut j = i + 5;
            if j < b.len() && b[j] == b'#' {
                j += 1;
                let mut end = j;
                while end < b.len() && (b[end].is_ascii_digit() || b[end] == b'-') {
                    end += 1;
                }
                if let Ok(k) = line[j..end].parse::<i32>() {
                    out.push(k);
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// True when the line's token stream mentions the given register (x or w form).
fn line_mentions_reg(line: &str, reg: u8) -> bool {
    let xn = xreg_name(reg);
    let wn = wreg_name(reg);
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|tok| tok == xn || tok == wn)
}

fn thread_spill_slots(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_SPILL_THREAD").is_ok() {
        return false;
    }
    // Use SP-displacement-aware effective offsets (raw + disp) like
    // eliminate_adjacent_store_load / eliminate_overwritten_stores.
    // Same raw offset before and after a `sub sp, sp, #N` names different
    // bytes; matching raw offsets across it aliases frame slots with
    // outgoing-arg slots (920501-8.c: f=5 became p=15). Effective offsets
    // distinguish them, fixing the miscompile while still allowing
    // threading inside a single SP region.
    let Some(disps) = sp_displacements(lines, kinds, n) else {
        return false;
    };
    let mut changed = false;
    let mut i = 0;
    while i < n {
        let (s_reg, offset, s_word) = match kinds[i] {
            LineKind::StoreSp {
                reg,
                offset,
                is_word,
            } => (reg, offset, is_word),
            _ => {
                i += 1;
                continue;
            }
        };
        let eff_store = offset as i64 + disps[i];

        // Collect same-effective-offset, width-compatible reloads in the window.
        // (is_word == false is the 64-bit x-form.)
        let mut reloads: Vec<(usize, bool, bool)> = Vec::new(); // (idx, is_word, is_sw)
        let mut j = i + 1;
        let mut other_refs = 0usize; // slot refs in the window we do NOT rewrite
        while j < n {
            // SP adjustment changes effective meaning — window ends here.
            // (We also rely on effective matching, but breaking keeps the
            // window tight and matches the original pass's call/branch barriers.)
            let t_j = lines[j].trim_start();
            if (t_j.starts_with("sub ") || t_j.starts_with("add ")) && t_j.contains("sp, sp, #") {
                break;
            }
            match kinds[j] {
                LineKind::Nop => {}
                LineKind::LoadSp {
                    offset: o,
                    is_word: w,
                    reg: d,
                    ..
                } => {
                    let eff = o as i64 + disps[j];
                    if eff == eff_store {
                        if d >= 18 {
                            other_refs += 1;
                        } else if w || !s_word {
                            reloads.push((j, w, false));
                        } else {
                            other_refs += 1;
                        }
                    } else if (eff - eff_store).abs() < 8 {
                        other_refs += 1;
                    }
                }
                LineKind::LoadswSp {
                    offset: o, reg: d, ..
                } => {
                    let eff = o as i64 + disps[j];
                    if eff == eff_store {
                        if d >= 18 {
                            other_refs += 1;
                        } else {
                            reloads.push((j, true, true));
                        }
                    } else if (eff - eff_store).abs() < 8 {
                        other_refs += 1;
                    }
                }
                LineKind::StoreSp { offset: o, .. } => {
                    let eff = o as i64 + disps[j];
                    if eff == eff_store {
                        break;
                    }
                    if (eff - eff_store).abs() < 8 {
                        other_refs += 1;
                        break;
                    }
                }
                LineKind::Label
                | LineKind::Branch
                | LineKind::CondBranch
                | LineKind::CmpBranch
                | LineKind::Call
                | LineKind::Ret
                | LineKind::StorePairSp
                | LineKind::LoadPairSp
                | LineKind::LoadsbReg => break,
                LineKind::MemOther | LineKind::Other => {
                    let t = lines[j].trim_start();
                    let mut stored_overlap = false;
                    for k in sp_offsets_in_line(t) {
                        let eff_k = k as i64 + disps[j];
                        if (eff_k - eff_store).abs() < 8 && eff_k != eff_store {
                            other_refs += 1;
                        }
                        if eff_k == eff_store {
                            stored_overlap = stored_overlap
                                || t.starts_with("str ")
                                || t.starts_with("stur ")
                                || t.starts_with("stp ");
                        }
                    }
                    // Unclassified exact-offset reference in effective space
                    let mut has_exact = false;
                    for k in sp_offsets_in_line(t) {
                        if k as i64 + disps[j] == eff_store {
                            has_exact = true;
                            break;
                        }
                    }
                    if has_exact {
                        other_refs += 1;
                    }
                    if stored_overlap {
                        break;
                    }
                    if t.starts_with("str")
                        || t.starts_with("stur")
                        || t.starts_with("stp")
                        || t.starts_with("stl")
                    {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if reloads.is_empty() {
            i += 1;
            continue;
        }
        let last = reloads.last().map(|&(idx, _, _)| idx).unwrap_or(i);

        // The transform only fires when the slot has no reader this window
        // leaves behind (slices cannot grow; the kept-store variant would
        // need an inserted move), so census-proven total coverage is a
        // precondition, checked below.
        if other_refs != 0 {
            i += 1;
            continue;
        }

        // Function extent + textual census. The offset must have no reference
        // beyond this store and the reloads rewritten here.
        let func_start = {
            let mut k = i;
            while k > 0 && !is_function_boundary(&lines[k]) {
                k -= 1;
            }
            k
        };
        let func_end = {
            let mut k = i + 1;
            while k < n && !is_function_boundary(&lines[k]) {
                k += 1;
            }
            k
        };
        // Overlap-range textual census in effective space (exact tokens miss
        // byte-lane stores like `strb w0, [sp, #offset+3]`).
        let mut total_refs = 0usize;
        for k in func_start..func_end {
            for o in sp_offsets_in_line(&lines[k]) {
                let eff = o as i64 + disps[k];
                if (eff - eff_store).abs() < 8 {
                    total_refs += 1;
                }
            }
        }
        // 1 (the store) + all reloads must account for every reference.
        if total_refs != 1 + reloads.len() {
            i += 1;
            continue;
        }
        let all_covered = true;

        // Scratch selection: an x2-x17 register mentioned nowhere in the
        // window (excluding the lines being rewritten) and nowhere else in
        // the function.
        let mut scratch: Option<u8> = None;
        'cand: for cand in [2u8, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 16, 17] {
            if cand == s_reg {
                continue;
            }
            if reloads
                .iter()
                .any(|&(idx, _, _)| matches!(kinds[idx], LineKind::LoadSp { reg, .. } | LineKind::LoadswSp { reg, .. } if reg == cand))
            {
                continue;
            }
            for k in (i + 1)..=last {
                if reloads.iter().any(|&(idx, _, _)| idx == k) {
                    continue;
                }
                if line_mentions_reg(&lines[k], cand) {
                    continue 'cand;
                }
            }
            for k in func_start..func_end {
                if k == i || reloads.iter().any(|&(idx, _, _)| idx == k) {
                    continue;
                }
                if line_mentions_reg(&lines[k], cand) {
                    continue 'cand;
                }
            }
            scratch = Some(cand);
            break;
        }
        let Some(r) = scratch else {
            i += 1;
            continue;
        };
        let rx = xreg_name(r);
        let rw = wreg_name(r);
        let sx = xreg_name(s_reg);
        let sw = wreg_name(s_reg);

        // Replace the store with a move into the scratch when the slot has no
        // other reader; otherwise keep the store and put the move in front of
        // it (the slot stays intact for its other readers).
        if all_covered {
            lines[i] = if s_word {
                format!("    mov {}, {}", rw, sw)
            } else {
                format!("    mov {}, {}", rx, sx)
            };
            kinds[i] = LineKind::Move {
                dst: r,
                src: s_reg,
                is_32bit: s_word,
            };
        }
        for &(idx, w, is_sw) in &reloads {
            let dst_reg = match kinds[idx] {
                LineKind::LoadSp { reg, .. } => reg,
                LineKind::LoadswSp { reg, .. } => reg,
                _ => continue,
            };
            let dx = xreg_name(dst_reg);
            let dw = wreg_name(dst_reg);
            let new_line = match (is_sw, w) {
                (true, _) => format!("    sxtw {}, {}", dx, rw),
                (false, false) => format!("    mov {}, {}", dx, rx),
                (false, true) => format!("    mov {}, {}", dw, rw),
            };
            lines[idx] = new_line;
            kinds[idx] = if is_sw {
                LineKind::Sxtw {
                    dst: dst_reg,
                    src: r,
                }
            } else {
                LineKind::Move {
                    dst: dst_reg,
                    src: r,
                    is_32bit: w,
                }
            };
        }
        changed = true;
        i += 1;
    }
    changed
}

// ── Pass 7: adrp+add rematerialisation CSE ──────────────────────────────────
//
// The RA rematerialises `adrp xA, sym; add xA, xA, :lo12:sym` per use. When
// the identical pair repeats within one block and an earlier pair's register
// still holds the address, the second adrp — and, for the same register, the
// second add — is redundant:
//
//     adrp x0, ring            adrp x0, ring
//     add  x0, x0, :lo12:ring  add  x0, x0, :lo12:ring   <- both deleted
//
// Different registers keep the second `add` but retarget it at the first
// pair's register and drop the second `adrp`. A missed CSE is a missed opt;
// a wrong one a miscompile, so anything unrecognised invalidates every
// tracked entry. Kill switch: CCC_NO_ADRP_CSE.

/// `adrp xA, SYM` (non-empty, non-immediate symbol operand).
fn parse_adrp_only(line: &str) -> Option<(u8, String)> {
    let rest = line.trim().strip_prefix("adrp ")?;
    let mut it = rest.splitn(2, ',');
    let reg = parse_reg(it.next()?.trim());
    if reg == REG_NONE || reg >= 31 {
        return None;
    }
    let sym = it.next()?.trim();
    if sym.is_empty() || sym.starts_with('#') || sym.starts_with(':') {
        return None;
    }
    Some((reg, sym.to_string()))
}

/// `add xA, xB, :lo12:SYM` — an address completion (the paired form has A==B).
fn parse_adrp_pair_add(line: &str) -> Option<(u8, u8, String)> {
    let rest = line.trim().strip_prefix("add ")?;
    let mut it = rest.split(',');
    let dst = parse_reg(it.next()?.trim());
    let src = parse_reg(it.next()?.trim());
    let third = it.next()?.trim();
    if it.next().is_some() {
        return None;
    }
    let sym = third.strip_prefix(":lo12:")?.trim();
    if sym.is_empty() || dst == REG_NONE || dst >= 31 || src == REG_NONE {
        return None;
    }
    Some((dst, src, sym.to_string()))
}

/// Does this line define `reg`? `None` = unrecognised instruction: callers
/// must treat it as "kills every tracked entry".
fn line_defines_gp(line: &str, kind: LineKind, reg: u8) -> Option<bool> {
    if let Some(d) = written_gp_register(line, kind) {
        return Some(d == reg);
    }
    // Architectural implicit writes decide first: a pre/post-indexed base
    // register is redefined even when the mnemonic is a store (`str x0,
    // [x9], #8` writes x9), and `bl`/`blr` redefine x30.
    if implicitly_writes_a64(line, reg) {
        return Some(true);
    }
    match kind {
        // Coarse kinds whose def behaviour the textual allowlist below
        // decides; written_gp_register already handled the precise ones.
        LineKind::Move { .. }
        | LineKind::MoveImm { .. }
        | LineKind::MoveWide { .. }
        | LineKind::Sxtw { .. }
        | LineKind::LoadSp { .. }
        | LineKind::LoadswSp { .. } => return None,
        _ => {}
    }
    const USE_FIRST: &[&str] = &[
        "str", "stur", "stp", "stnp", "cmp", "cmn", "ccmp", "ccmn", "cbz", "cbnz", "tbz", "tbnz",
        "ret", "nop",
    ];
    const DEF_FIRST: &[&str] = &[
        "adrp", "adr", "add", "adds", "sub", "subs", "ldr", "ldur", "ldp", "ldnp", "ldrsw",
        "ldrsh", "ldrsb", "sxtw", "sxtb", "uxth", "uxtb", "lsl", "lsr", "asr", "ror", "madd",
        "msub", "mul", "mneg", "and", "ands", "orr", "orns", "eor", "eon", "bic", "bics", "csel",
        "csinc", "csinv", "csneg", "extr", "sbfm", "ubfm", "sbfx", "ubfx", "bfm", "mvn", "fmov",
        "scvtf", "ucvtf", "mrs",
    ];
    let t = line.trim();
    let (mnem, rest) = t.split_once(' ')?;
    if USE_FIRST.contains(&mnem) {
        return Some(false);
    }
    if DEF_FIRST.contains(&mnem) {
        let mut ops = rest.split(',');
        let first = ops.next()?.trim();
        if parse_reg(first) == reg {
            return Some(true);
        }
        // A pair load defines BOTH registers; checking only the first
        // operand lied `Some(false)` for the second — exactly the lie a
        // hoist/remat must never hear about the register it moves.
        if (mnem == "ldp" || mnem == "ldnp")
            && ops.next().is_some_and(|second| parse_reg(second.trim()) == reg)
        {
            return Some(true);
        }
        return Some(false);
    }
    None // unknown: kill all
}

fn cse_adrp_remat_pairs(lines: &mut [String], kinds: &mut [LineKind], n: usize) -> bool {
    if std::env::var("CCC_NO_ADRP_CSE").is_ok() {
        return false;
    }
    // (reg, sym, add_idx) — completed, still-valid canonical pairs.
    let mut done: Vec<(u8, String, usize)> = Vec::new();
    // (reg, sym, adrp_idx, canonical) — adrps awaiting the paired add;
    // `canonical` is set when this adrp is already proven redundant.
    let mut open: Vec<(u8, String, usize, Option<u8>)> = Vec::new();
    let mut changed = false;
    let mut i = 0;
    while i < n {
        match kinds[i] {
            LineKind::Nop => {
                i += 1;
                continue;
            }
            // Block boundaries and calls invalidate everything (adrp results
            // live in caller-saved registers).
            LineKind::Label
            | LineKind::Branch
            | LineKind::CondBranch
            | LineKind::CmpBranch
            | LineKind::Call
            | LineKind::Ret => {
                done.clear();
                open.clear();
                i += 1;
                continue;
            }
            _ => {}
        }

        // adrp lines are decided HERE, before any kill bookkeeping: the pair
        // being deleted would itself redefine the register, so it must not
        // invalidate its own canonical entry.
        if let Some((reg, sym)) = parse_adrp_only(&lines[i]) {
            let mut canonical: Option<u8> = None;
            for &(rr, ref ss, add_idx) in done.iter() {
                if *ss != sym {
                    continue;
                }
                let mut valid = true;
                for k in (add_idx + 1)..i {
                    if kinds[k] == LineKind::Nop {
                        continue;
                    }
                    if line_defines_gp(&lines[k], kinds[k], rr) != Some(false) {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    canonical = Some(rr);
                    break;
                }
            }
            if canonical.is_none() {
                // A fresh remat: its register definition retires same-reg
                // entries.
                done.retain(|&(rr, _, _)| rr != reg);
                open.retain(|&(rr, _, _, _)| rr != reg);
            }
            open.push((reg, sym, i, canonical));
            i += 1;
            continue;
        }

        // Completed pair? `add xA, xA, :lo12:sym` closing an open adrp.
        if let Some((dst, src, sym)) = parse_adrp_pair_add(&lines[i]) {
            if dst == src {
                if let Some(pos) = open
                    .iter()
                    .position(|&(r, ref s, _, _)| r == dst && *s == sym)
                {
                    let (_, _, adrp_idx, canonical) = open[pos];
                    open.remove(pos);
                    let mut creg = canonical;
                    if let Some(c) = creg {
                        // Re-verify between the (deleted) adrp and this add:
                        // only Nops may intervene.
                        for k in (adrp_idx + 1)..i {
                            if kinds[k] == LineKind::Nop {
                                continue;
                            }
                            if line_defines_gp(&lines[k], kinds[k], c) != Some(false) {
                                creg = None;
                                break;
                            }
                        }
                    }
                    if let Some(c) = creg {
                        if c == dst {
                            lines[adrp_idx] = String::new();
                            kinds[adrp_idx] = LineKind::Nop;
                            lines[i] = String::new();
                            kinds[i] = LineKind::Nop;
                        } else {
                            lines[i] = format!(
                                "    add {}, {}, :lo12:{}",
                                xreg_name(dst),
                                xreg_name(c),
                                sym
                            );
                            lines[adrp_idx] = String::new();
                            kinds[adrp_idx] = LineKind::Nop;
                        }
                        changed = true;
                        i += 1;
                        continue;
                    }
                    // No usable canonical: this pair becomes canonical.
                    done.push((dst, sym, i));
                    i += 1;
                    continue;
                }
            }
        }

        // Kill per-register entries on recognised defs...
        let mut recognized = false;
        for r in 0..=30u8 {
            if let Some(defines) = line_defines_gp(&lines[i], kinds[i], r) {
                recognized = true;
                if defines {
                    done.retain(|&(rr, _, _)| rr != r);
                    open.retain(|&(rr, _, _, _)| rr != r);
                }
            }
        }
        // ...or kill everything on unrecognised lines.
        if !recognized {
            done.clear();
            open.clear();
        }
        i += 1;
    }
    changed
}

// ── Pass 8: loop-invariant rematerialisation hoisting ───────────────────────
//
// The RA rematerialises pure values (big constants via movz/movk, global
// addresses via adrp+add, spilled frame addresses via sp-slot loads) AT EACH
// USE. Inside a loop every use is per-iteration, so hot loops re-execute the
// same pure sequence every trip (ring_fifo: `movz x0, #20000` at the header
// AND the latch; the ring base address reloaded from [sp,#16] per push).
// GCC/clang sink the constant into a register before the loop. DECISIONS.md
// [history/session41] blesses exactly this transform for the x86-64 PIE
// `leaq` shape ("hoisting is correct *and* profitable"); this is the AArch64
// twin.
//
// Operative rule for a def-sequence D at position p (loop [T..L], backedge
// at L, loop top label at T):
//   * D is pure: `movz xD,#imm` (+ contiguous movk chain on xD),
//     `adrp xD,sym` [+ `add xD,xD,:lo12:sym`], or `ldr xD,[sp,#off]`;
//   * the loop is call-free (a caller-saved dest would die at the call —
//     the original re-def at p is what saved it) and has a single backedge
//     (no other branch targets .Ltop);
//   * p is at the TOP of the header block: no mention of xD between the
//     label and p, and every mention of xD after p WITHIN the header block
//     is use-first (a reader of D's value);
//   * every OTHER def of xD in the loop is a textually identical sequence
//     (so xD carries the same value at every in-loop point both before and
//     after the transform); sp-slot loads additionally require no store in
//     the loop overlapping [off-7, off+7];
//   * transform: copy D above the loop-top label, delete the header copy.
//     Cross-block, same-value co-defs stay (they rewrite xD with the same
//     constant — harmless), and post-loop readers still see the constant.
//
// Kill switch: CCC_NO_INVAR_HOIST.

/// A maximal contiguous pure def-sequence starting at `idx`: returns the
/// sequence length and its dest register, or None. Sequences: movz [+movk*],
/// adrp [+ paired add], ldr-from-sp (single line).
fn pure_def_sequence(
    lines: &[String],
    kinds: &[LineKind],
    n: usize,
    idx: usize,
) -> Option<(usize, u8)> {
    let t = lines[idx].trim_start();
    // movz + contiguous movk chain
    if let Some(rest) = t.strip_prefix("movz ") {
        let dst = parse_reg(rest.split(',').next()?.trim());
        if dst == REG_NONE || dst >= 31 {
            return None;
        }
        let mut len = 1;
        let mut j = idx + 1;
        while j < n {
            let tj = lines[j].trim_start();
            if let Some(rj) = tj.strip_prefix("movk ") {
                let dj = parse_reg(rj.split(',').next()?.trim());
                if dj == dst {
                    len += 1;
                    j += 1;
                    continue;
                }
            }
            if kinds[j] == LineKind::Nop {
                j += 1;
                continue;
            }
            break;
        }
        return Some((len, dst));
    }
    // adrp [+ paired add]
    if let Some((dst, sym)) = parse_adrp_only(&lines[idx]) {
        let mut len = 1;
        let mut j = idx + 1;
        while j < n && kinds[j] == LineKind::Nop {
            j += 1;
        }
        if j < n {
            if let Some((d2, s2, sym2)) = parse_adrp_pair_add(&lines[j]) {
                if d2 == dst && s2 == dst && sym2 == sym {
                    len = 2;
                }
            }
        }
        return Some((len, dst));
    }
    // sp-slot load
    if let LineKind::LoadSp { reg, offset, .. } = kinds[idx] {
        if sp_offsets_in_line(t).len() == 1 && sp_offsets_in_line(t)[0] == offset {
            return Some((1, reg));
        }
    }
    None
}

/// Is the line a jump/call whose target label equals `label`?
fn branch_targets(line: &str, kind: LineKind, label: &str) -> bool {
    match kind {
        LineKind::Branch | LineKind::CondBranch | LineKind::CmpBranch => {
            line.split_whitespace().any(|tok| tok == label)
        }
        _ => false,
    }
}

fn hoist_loop_invariant_remats(
    lines: &mut Vec<String>,
    kinds: &mut Vec<LineKind>,
    mut n: usize,
) -> bool {
    if std::env::var("CCC_NO_INVAR_HOIST").is_ok() {
        return false;
    }
    let mut changed = false;
    // Collect backward branches first (indices stable this pass: no inserts
    // happen until a transform commits, and after a commit we restart).
    let mut i = 0;
    while i < n {
        let is_backedge = matches!(
            kinds[i],
            LineKind::Branch | LineKind::CondBranch | LineKind::CmpBranch
        );
        if !is_backedge {
            i += 1;
            continue;
        }
        // Backedge target: the referenced label defined EARLIER.
        let target: String = {
            let toks: Vec<&str> = lines[i].split_whitespace().collect();
            match toks.into_iter().rev().find(|t| t.starts_with('.')) {
                Some(t) => t.trim_end_matches(':').to_string(),
                None => {
                    i += 1;
                    continue;
                }
            }
        };
        let target = target.as_str();
        let Some(top) = (0..i).rev().find(|&k| {
            kinds[k] == LineKind::Label && lines[k].trim().trim_end_matches(':') == target
        }) else {
            i += 1;
            continue;
        };

        // ── loop validity ──
        // Entry edges into the loop top are the fallthrough and branches
        // from ABOVE the top (handled by inserting copies at each such
        // branch below). Branches from INSIDE the loop region to the top
        // are additional backedges — sound without copies, because nothing
        // in the loop defines dest after the transform (the value persists
        // across them).
        // Call-free loop.
        if (top..=i).any(|k| kinds[k] == LineKind::Call) {
            i += 1;
            continue;
        }
        // Candidate scan: every position in the loop, starting at the top.
        // The dest must be SELF-CONTAINED: defined exactly once (here), not
        // mentioned before, and only read afterwards — then hoisting the
        // pure sequence above the loop top preserves every value (the def
        // executes once instead of once per trip; every reader sees the same
        // constant/address either way).
        let mut transformed = false;
        let mut k = top;
        while k <= i {
            if kinds[k] == LineKind::Nop || lines[k].trim_start().starts_with(".cfi") {
                k += 1;
                continue;
            }
            let Some((seq_len, dest)) = pure_def_sequence(lines, kinds, n, k) else {
                k += 1;
                continue;
            };
            let seq_end = k + seq_len;
            // No other def of dest anywhere in the loop.
            let mut other_def = false;
            let mut m = top;
            while m <= i {
                if m >= k && m < seq_end {
                    m = seq_end;
                    continue;
                }
                if kinds[m] != LineKind::Nop
                    && !lines[m].trim_start().starts_with(".cfi")
                    && line_defines_gp(&lines[m], kinds[m], dest) == Some(true)
                {
                    other_def = true;
                    break;
                }
                m += 1;
            }
            if other_def {
                k += 1;
                continue;
            }
            // No mention of dest anywhere in the loop before the sequence.
            if (top..k).any(|m| line_mentions_reg(&lines[m], dest)) {
                k += 1;
                continue;
            }
            // All mentions after the sequence (to the backedge) are
            // use-first readers.
            let mut readers_ok = true;
            let mut m = seq_end;
            while m <= i {
                if line_mentions_reg(&lines[m], dest)
                    && line_defines_gp(&lines[m], kinds[m], dest) != Some(false)
                {
                    readers_ok = false;
                    break;
                }
                m += 1;
            }
            if !readers_ok {
                k += 1;
                continue;
            }
            // sp-slot loads: no overlapping store anywhere in the loop.
            if lines[k].trim_start().starts_with("ldr ") {
                let off = match kinds[k] {
                    LineKind::LoadSp { offset, .. } => offset,
                    _ => unreachable!(),
                };
                let mut bad = false;
                let mut m = top;
                while m <= i {
                    if let LineKind::StoreSp { offset: so, .. } = kinds[m] {
                        if (so - off).abs() < 8 {
                            bad = true;
                            break;
                        }
                    } else if matches!(kinds[m], LineKind::MemOther | LineKind::Other) {
                        let t = lines[m].trim_start();
                        if t.starts_with("str ") || t.starts_with("stur ") || t.starts_with("stp ")
                        {
                            for o in sp_offsets_in_line(t) {
                                if (o - off).abs() < 8 {
                                    bad = true;
                                    break;
                                }
                            }
                        }
                    }
                    if bad {
                        break;
                    }
                    m += 1;
                }
                if bad {
                    k += 1;
                    continue;
                }
            }

            // Dest must be dead after the loop: on paths whose seq block did
            // not run, the register held an older value — post-loop readers
            // must not observe the change.
            if ((i + 1)..n).any(|m| line_mentions_reg(&lines[m], dest)) {
                k += 1;
                continue;
            }

            // Entry edges: fallthrough (handled by inserting before the
            // label) plus every branch from ABOVE the loop that targets the
            // top. Entry branches must not read dest (the copy would change
            // what they see). Branches from inside the loop region to the
            // top are backedges — dest persists across them (nothing in the
            // loop defines it), no copy needed.
            let mut entry_branches: Vec<usize> = Vec::new();
            let mut ok = true;
            for m in 0..top {
                if branch_targets(&lines[m], kinds[m], target) {
                    if line_mentions_reg(&lines[m], dest) {
                        ok = false;
                        break;
                    }
                    entry_branches.push(m);
                }
            }
            if !ok {
                k += 1;
                continue;
            }

            // ── transform: delete the in-loop sequence first (indices are
            //    still valid), then insert copies before every entry branch
            //    (descending order keeps earlier indices valid), then insert
            //    the fallthrough copy right before the (relocated) label. ──
            let seq_lines: Vec<String> = (k..seq_end).map(|q| lines[q].clone()).collect();
            let seq_kinds: Vec<LineKind> = (k..seq_end).map(|q| kinds[q].clone()).collect();
            for _ in 0..seq_len {
                lines.remove(k);
                kinds.remove(k);
            }
            for &b in entry_branches.iter().rev() {
                for (off, l) in seq_lines.iter().enumerate() {
                    lines.insert(b + off, l.clone());
                    kinds.insert(b + off, seq_kinds[off].clone());
                }
            }
            let label_idx = (0..lines.len())
                .find(|&q| {
                    kinds[q] == LineKind::Label && lines[q].trim().trim_end_matches(':') == target
                })
                .unwrap_or(top);
            for (off, l) in seq_lines.iter().enumerate() {
                lines.insert(label_idx + off, l.clone());
                kinds.insert(label_idx + off, seq_kinds[off].clone());
            }
            transformed = true;
            break;
        }
        if transformed {
            changed = true;
            n = lines.len();
            i = 0; // loop shape changed: rescan
            continue;
        }
        i += 1;
        continue;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_store() {
        assert!(matches!(
            classify_line("    str x0, [sp, #16]"),
            LineKind::StoreSp {
                reg: 0,
                offset: 16,
                is_word: false
            }
        ));
        assert!(matches!(
            classify_line("    str w1, [sp, #24]"),
            LineKind::StoreSp {
                reg: 1,
                offset: 24,
                is_word: true
            }
        ));
    }

    #[test]
    fn test_classify_load() {
        assert!(matches!(
            classify_line("    ldr x0, [sp, #16]"),
            LineKind::LoadSp {
                reg: 0,
                offset: 16,
                is_word: false
            }
        ));
    }

    #[test]
    fn test_classify_loadsw() {
        assert!(matches!(
            classify_line("    ldrsw x0, [sp, #24]"),
            LineKind::LoadswSp { reg: 0, offset: 24 }
        ));
    }

    #[test]
    fn test_classify_move() {
        assert!(matches!(
            classify_line("    mov x14, x0"),
            LineKind::Move {
                dst: 14,
                src: 0,
                is_32bit: false
            }
        ));
    }

    #[test]
    fn test_classify_move_imm() {
        assert!(matches!(
            classify_line("    mov x0, #0"),
            LineKind::MoveImm { dst: 0 }
        ));
        assert!(matches!(
            classify_line("    mov x0, #-1"),
            LineKind::MoveImm { dst: 0 }
        ));
    }

    #[test]
    fn test_classify_branch() {
        assert_eq!(classify_line("    b .LBB1"), LineKind::Branch);
    }

    #[test]
    fn test_classify_cond_branch() {
        assert_eq!(classify_line("    b.ge .Lskip_0"), LineKind::CondBranch);
        assert_eq!(classify_line("    b.eq .LBB3"), LineKind::CondBranch);
    }

    #[test]
    fn test_classify_label() {
        assert_eq!(classify_line(".LBB1:"), LineKind::Label);
        assert_eq!(classify_line("sum_array:"), LineKind::Label);
    }

    #[test]
    fn test_classify_ret() {
        assert_eq!(classify_line("    ret"), LineKind::Ret);
    }

    #[test]
    fn test_classify_sxtw() {
        assert!(matches!(
            classify_line("    sxtw x0, w0"),
            LineKind::Sxtw { dst: 0, src: 0 }
        ));
    }

    #[test]
    fn test_adjacent_store_load_same_reg() {
        let input = "    str x0, [sp, #16]\n    ldr x0, [sp, #16]\n    ret\n";
        let result = peephole_optimize(input.to_string());
        // The load is forwarded to the store's register; with the sole reader
        // gone, GDSE additionally retires the now-dead store (nothing else in
        // the frame reads slot 16). Assert the observable forwarding result.
        assert!(!result.contains("ldr x0, [sp, #16]"));
        assert!(result.contains("ret"));
    }

    #[test]
    fn test_adjacent_store_load_diff_reg() {
        let input = "    str x0, [sp, #16]\n    ldr x1, [sp, #16]\n    add x2, x1, #1\n    ret\n";
        let result = peephole_optimize(input.to_string());
        // The load must never reach the final text: it is replaced by the
        // store's value (mov or propagated copy), and the then-dead store
        // is removed by DSE.
        assert!(
            !result.contains("ldr x1, [sp, #16]"),
            "load kept:\n{}",
            result
        );
        assert!(
            !result.contains("str x0, [sp, #16]"),
            "dead store kept:\n{}",
            result
        );
    }

    #[test]
    fn test_redundant_branch() {
        let input = "    b .LBB1\n.LBB1:\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("b .LBB1"));
        assert!(result.contains(".LBB1:"));
    }

    #[test]
    fn test_self_move() {
        let input = "    mov x0, x0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("mov x0, x0"));
    }

    #[test]
    fn test_branch_over_branch_fusion() {
        let input = "    b.ge .Lskip_0\n    b .LBB2\n.Lskip_0:\n    b .LBB4\n";
        let result = peephole_optimize(input.to_string());
        // Should become: b.lt .LBB2 (inverted ge → lt)
        assert!(result.contains("b.lt .LBB2"));
        // The unconditional branch to LBB2 should be eliminated
        assert!(!result.contains("    b .LBB2\n"));
    }

    #[test]
    fn test_move_chain() {
        // The copy-of-copy chain must not survive: the fold (or equivalent
        // copy propagation) rewrites the sole read of x13 to its source, so
        // `mov x13, x0` disappears in one form or another.
        let input = "    mov x0, x14\n    mov x13, x0\n    add x15, x13, #1\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mov x13, x0"),
            "chain not folded:\n{}",
            result
        );
        assert!(
            result.contains("add x15, x14, #1"),
            "value flow lost:\n{}",
            result
        );
    }

    #[test]
    fn test_move_imm_chain() {
        let input = "    mov x0, #0\n    mov x14, x0\n    add x15, x14, #1\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mov x14, x0"),
            "chain not folded:\n{}",
            result
        );
        assert!(
            result.contains("mov x14, #0"),
            "immediate not retargeted:\n{}",
            result
        );
    }

    #[test]
    fn test_store_loadsw_fusion() {
        let input = "    str w1, [sp, #24]\n    ldrsw x0, [sp, #24]\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ldrsw"));
        assert!(result.contains("sxtw x0, w1"));
    }

    // ── Pass 8: loop-invariant remat hoisting tests ────────────────────

    #[test]
    fn test_invar_hoist_slot_load() {
        // The seed load sits in a conditional block inside the loop; the
        // slot is written only before the loop. The load moves above the
        // loop top (and before the entry branch), the loop body loses it.
        let input = concat!(
            "main:\n",
            "    mov w0, #7\n",
            "    str w0, [sp, #40]\n",
            "    b.lt .LBB10\n",
            "    ldr w2, [sp, #40]\n",
            ".LBB10:\n",
            "    ldr w2, [sp, #40]\n",
            "    add w3, w2, #1\n",
            "    add w20, w20, #1\n",
            "    cmp w20, #100\n",
            "    b.lt .LBB10\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        let body = result.split(".LBB10:").nth(1).unwrap_or("");
        let body_end = body.find("    ret").unwrap_or(body.len());
        let loop_body = &body[..body_end];
        assert!(
            !loop_body.contains("[sp, #40]"),
            "load not hoisted from loop body:\n{}",
            result
        );
        // Empirically-verified shape: exactly one hoisted copy directly
        // before the loop-top label (outside the loop text), the entry
        // branch fed by an equivalent pure def (pass 6 may thread it to
        // `mov w2, w0` — value-identical), and w2 defined on every entry.
        let pre = &result[..result.find(".LBB10:").unwrap()];
        assert_eq!(
            pre.matches("ldr w2, [sp, #40]").count(),
            1,
            "label copy missing or duplicated:\n{}",
            result
        );
        assert!(
            pre.contains("mov w2, w0") || pre.contains("ldr w2, [sp, #40]\n    b.lt"),
            "entry path not fed:\n{}",
            result
        );
    }

    #[test]
    fn test_invar_hoist_blocked_by_in_loop_store() {
        // The loop stores into the slot: the load must stay.
        let input = concat!(
            "main:\n",
            "    mov w0, #7\n",
            "    str w0, [sp, #40]\n",
            "    b.lt .LBB10\n",
            ".LBB10:\n",
            "    ldr w2, [sp, #40]\n",
            "    add w3, w2, #1\n",
            "    str w3, [sp, #40]\n",
            "    add w20, w20, #1\n",
            "    cmp w20, #100\n",
            "    b.lt .LBB10\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        let after = result
            .split(".LBB10:")
            .nth(1)
            .unwrap_or("")
            .trim_start_matches('\n');
        assert!(
            after.starts_with("    ldr w2, [sp, #40]"),
            "load moved despite in-loop store:\n{}",
            result
        );
    }

    #[test]
    fn test_invar_hoist_blocked_by_post_loop_reader() {
        // x1 is read after the loop: paths that skip the original def block
        // would observe a changed value.
        let input = concat!(
            "main:\n",
            "    mov w0, #7\n",
            "    str w0, [sp, #40]\n",
            "    b.lt .LBB10\n",
            ".LBB10:\n",
            "    ldr w2, [sp, #40]\n",
            "    add w20, w20, #1\n",
            "    cmp w20, #100\n",
            "    b.lt .LBB10\n",
            "    add w5, w2, #9\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        let after = result
            .split(".LBB10:")
            .nth(1)
            .unwrap_or("")
            .trim_start_matches('\n');
        assert!(
            after.starts_with("    ldr w2, [sp, #40]"),
            "hoisted despite post-loop reader:\n{}",
            result
        );
    }

    #[test]
    fn test_invar_hoist_blocked_by_call() {
        let input = concat!(
            "main:\n",
            "    mov w0, #7\n",
            "    str w0, [sp, #40]\n",
            "    b.lt .LBB10\n",
            ".LBB10:\n",
            "    ldr w2, [sp, #40]\n",
            "    bl helper\n",
            "    add w20, w20, #1\n",
            "    cmp w20, #100\n",
            "    b.lt .LBB10\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        let after = result
            .split(".LBB10:")
            .nth(1)
            .unwrap_or("")
            .trim_start_matches('\n');
        assert!(
            after.starts_with("    ldr w2, [sp, #40]"),
            "hoisted despite call in loop:\n{}",
            result
        );
    }

    // ── Pass 6: spill-slot threading tests ─────────────────────────────

    #[test]
    fn test_spill_thread_basic() {
        // str + clobbered source + reload in one window: both disappear into
        // a scratch-threaded move pair (census: 1 store, 1 load).
        let input = concat!(
            "main:\n",
            "    mov x1, x3\n",
            "    str x0, [sp, #24]\n",
            "    add x0, x1, x3\n",
            "    ldr x0, [sp, #24]\n",
            "    str w0, [x9]\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("[sp, #24]"),
            "slot round-trip survived:\n{}",
            result
        );
        assert!(
            result.contains("mov x2, x0"),
            "scratch move missing:\n{}",
            result
        );
    }

    #[test]
    fn test_spill_thread_keeps_slot_for_other_reader() {
        // A second load in a later block: the store must survive.
        let input = concat!(
            "main:\n",
            "    str x0, [sp, #8]\n",
            "    add x0, x1, x3\n",
            "    ldr x0, [sp, #8]\n",
            "    cbz x0, .LBB1\n",
            ".LBB1:\n",
            "    ldr x5, [sp, #8]\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("str x0, [sp, #8]"),
            "store wrongly deleted:\n{}",
            result
        );
    }

    #[test]
    fn test_spill_thread_word_store_ldrsw() {
        // w-store + ldrsw reload: sxtw from the threaded scratch.
        let input = concat!(
            "main:\n",
            "    str w1, [sp, #12]\n",
            "    add w1, w2, w3\n",
            "    ldrsw x0, [sp, #12]\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ldrsw"), "ldrsw survived:\n{}", result);
        assert!(
            result.contains("sxtw x0, w"),
            "sxtw from scratch missing:\n{}",
            result
        );
    }

    #[test]
    fn test_spill_thread_call_blocks() {
        // A call in the window: no rewrite (the scratch could be clobbered
        // by the call and the value is caller-saved).
        let input = concat!(
            "main:\n",
            "    str x0, [sp, #16]\n",
            "    bl helper\n",
            "    ldr x0, [sp, #16]\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldr x0, [sp, #16]"),
            "window crossed a call:\n{}",
            result
        );
    }

    // ── Pass 7: adrp remat CSE tests ───────────────────────────────────

    #[test]
    fn test_adrp_cse_same_reg() {
        let input = concat!(
            "main:\n",
            "    adrp x0, ring\n",
            "    add x0, x0, :lo12:ring\n",
            "    mov x19, x0\n",
            "    adrp x0, ring\n",
            "    add x0, x0, :lo12:ring\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert_eq!(
            result.matches("adrp x0, ring").count(),
            1,
            "dup adrp:\n{}",
            result
        );
        assert_eq!(
            result.matches("add x0, x0, :lo12:ring").count(),
            1,
            "dup add:\n{}",
            result
        );
    }

    #[test]
    fn test_adrp_cse_killed_by_clobber() {
        let input = concat!(
            "main:\n",
            "    adrp x0, ring\n",
            "    add x0, x0, :lo12:ring\n",
            "    mov x0, x5\n",
            "    adrp x0, ring\n",
            "    add x0, x0, :lo12:ring\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert_eq!(
            result.matches("adrp x0, ring").count(),
            2,
            "clobbered pair reused:\n{}",
            result
        );
    }

    #[test]
    fn test_adrp_cse_different_symbol() {
        let input = concat!(
            "main:\n",
            "    adrp x0, ring\n",
            "    add x0, x0, :lo12:ring\n",
            "    adrp x0, tail\n",
            "    add x0, x0, :lo12:tail\n",
            "    ret\n",
        );
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("adrp x0, tail"),
            "wrong symbol CSEd:\n{}",
            result
        );
        assert!(
            result.contains("adrp x0, ring"),
            "first pair deleted:\n{}",
            result
        );
    }

    // ── Global store forwarding tests ─────────────────────────────────

    #[test]
    #[ignore] // GSF is disabled due to correctness bug in complex float-array code
    fn test_gsf_same_reg_elimination() {
        // Store x0 then load x0 from same slot (non-adjacent) — load is dead
        let input = "\
    str x0, [sp, #16]\n\
    add x1, x2, x3\n\
    ldr x0, [sp, #16]\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("str x0, [sp, #16]"));
        assert!(!result.contains("ldr x0, [sp, #16]"));
    }

    #[test]
    #[ignore] // GSF is disabled due to correctness bug in complex float-array code
    fn test_gsf_different_reg_forwarding() {
        // Store x5 then load x10 from same slot — replace load with mov
        let input = "\
    str x5, [sp, #32]\n\
    add x1, x2, x3\n\
    ldr x10, [sp, #32]\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("str x5, [sp, #32]"));
        assert!(!result.contains("ldr x10, [sp, #32]"));
        assert!(result.contains("mov x10, x5"));
    }

    #[test]
    fn test_gsf_invalidation_on_reg_overwrite() {
        // After x0 is overwritten, the mapping slot 16 → x0 is stale: a
        // forwarding of the STALE register value would be wrong. Pass 6
        // (spill-slot threading) now proves value equality explicitly: the
        // pre-clobber x0 is threaded through a dedicated scratch, so the
        // load disappears while x1 still receives the ORIGINAL x0.
        // (Strictly stronger than the old "the ldr survives" shape: it pins
        // the exact value flow — the wrong forwarding `mov x1, x0` after the
        // overwrite, and a post-overwrite memory reload, are both absent.)
        let input = "\
    str x0, [sp, #16]\n\
    mov x0, #42\n\
    ldr x1, [sp, #16]\n\
    add x3, x1, #1\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // Empirically-verified final shape (pass 6 threads, then copy-prop /
        // move-chain fold the dead x1 hop): capture BEFORE the clobber, use
        // of the captured original AFTER it, slot fully gone.
        assert!(
            result.contains("mov x2, x0") && result.contains("add x3, x2, #1"),
            "threaded original-value flow missing:\n{}",
            result
        );
        assert!(!result.contains("[sp, #16]"));
        assert!(
            !result.contains("mov x1, x0"),
            "stale forwarding:\n{}",
            result
        );
    }

    #[test]
    fn test_gsf_invalidation_at_jump_target() {
        // Mappings are invalidated at jump target labels
        let input = "\
    str x5, [sp, #16]\n\
    b .LBB1\n\
.LBB1:\n\
    ldr x5, [sp, #16]\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        // .LBB1 is a jump target, so mappings are invalidated
        assert!(result.contains("ldr x5, [sp, #16]"));
    }

    #[test]
    #[ignore] // GSF is disabled due to correctness bug in complex float-array code
    fn test_gsf_word_forwarding() {
        // Word store forwarded to word load
        let input = "\
    str w3, [sp, #24]\n\
    add x1, x2, x4\n\
    ldr w5, [sp, #24]\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ldr w5, [sp, #24]"));
        assert!(result.contains("mov w5, w3"));
    }

    #[test]
    #[ignore] // GSF is disabled due to correctness bug in complex float-array code
    fn test_gsf_ldrsw_forwarding() {
        // Word store forwarded to sign-extending load
        let input = "\
    str w1, [sp, #24]\n\
    add x2, x3, x4\n\
    ldrsw x5, [sp, #24]\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ldrsw x5, [sp, #24]"));
        assert!(result.contains("sxtw x5, w1"));
    }

    // ── Copy propagation tests ───────────────────────────────────────

    #[test]
    fn test_copy_prop_word_boundary() {
        // Ensure "x1" doesn't match inside "x11"
        assert_eq!(replace_whole_word("x11, x1", "x1", "x5"), "x11, x5");
    }

    #[test]
    fn test_copy_prop_no_false_match() {
        // x10 should not be affected when replacing x1
        assert_eq!(replace_whole_word("x10", "x1", "x5"), "x10");
    }

    #[test]
    fn test_dead_call_staging_move_removed_after_copy_prop() {
        let input = "    mov x9, x24\n    mov x0, x24\n    bl strlen\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("mov x9, x24"));
        assert!(result.contains("mov x0, x24"));
    }

    #[test]
    fn test_used_call_staging_move_preserved() {
        let input = "    mov x9, x24\n    add x24, x24, #8\n    ldr x0, [x9]\n    bl consume\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("mov x9, x24"));
    }

    #[test]
    fn test_vector_address_alias_propagation() {
        let input = "    mov x4, x5\n    ldr q2, [x4]\n    fmla v2.2d, v1.2d, v15.2d\n    str q2, [x4]\n    add x5, x5, #16\n    b .Lloop\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("mov x4, x5"));
        assert!(result.contains("ldr q2, [x5]"));
        assert!(result.contains("str q2, [x5]"));
    }

    #[test]
    fn test_destructive_pointer_update_fold() {
        let input = "    mov x0, x6\n    add x0, x6, #16\n    mov x8, x0\n    add w24, w24, #1\n    mov x6, x8\n    b .Lloop\n";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("add x6, x6, #16"));
        assert!(!result.contains("mov x8, x0"));
        assert!(!result.contains("mov x6, x8"));
    }

    // ── Dead store elimination tests ─────────────────────────────────

    #[test]
    fn test_dse_with_address_taken() {
        // Conservative GDSE: materializing ANY sp-derived address disables
        // store elimination (we do not track whether it is used), so the
        // store stays.
        let input = "\
    str w0, [sp, #16]\n\
    add x1, sp, #16\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("str w0, [sp, #16]"),
            "address-of-sp disables DSE conservatively:\n{}",
            result
        );
    }

    #[test]
    fn test_dse_with_escaping_address() {
        // The derived base is stored into memory here: the address escapes,
        // so no store elimination may happen at all.
        let input = "\
    str w0, [sp, #16]\n\
    add x1, sp, #16\n\
    str x1, [x9]\n\
    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("str w0, [sp, #16]"),
            "escaping address must preserve stores:\n{}",
            result
        );
    }

    #[test]
    fn test_dse_dead_fp_store_via_base() {
        // Conservative GDSE: any address-of-sp (`add x23, sp, #112`)
        // disables store elimination for the whole function — the store
        // through the derived base must SURVIVE. (levkropp's escape-
        // analysis rewrite that eliminated it caused 9 regressions on
        // this tree — volatile_loop, unroll_const4_*, fp_cmp_inf et al. —
        // and was rejected; see the REJECTED ADOPTION note at
        // global_dead_store_elimination.)
        let input = "\
f:
    add x23, sp, #112
    fmul d28, d27, d17
    str d28, [x23]
    fadd d0, d1, d1
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("str d28, [x23]"),
            "store via derived base kept (conservative GDSE):\n{}",
            result
        );
    }

    #[test]
    fn test_dse_live_fp_store_via_base_kept() {
        // Same conservative rule: with address-of-sp present, both the
        // store and the (differently-addressed) load survive untouched.
        let input = "\
f:
    add x23, sp, #112
    fmul d28, d27, d17
    str d28, [x23]
    ldr d0, [sp, #112]
    fadd d0, d0, d0
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("str d28"),
            "store kept under conservative GDSE:\n{}",
            result
        );
    }

    // ── x9 address-mov dead-value soundness ─────────────────────────────
    //
    // Regression: a 16-byte carrier load stages its address with
    // `mov x9, xN` and issues TWO loads (`ldr x0,[x9]`, `ldr x1,[x9,#8]`).
    // Copy propagation rewrites only the FIRST use to `[xN]`; the old
    // single-instruction lookahead in eliminate_unused_x9_address_moves
    // then believed x9 dead and deleted the mov, leaving the second load
    // reading a stale x9 (SIGSEGV on _Float128 globals).

    #[test]
    fn test_x9_address_mov_survives_second_use() {
        let input = "\
main:
    adrp x0, :got:g
    ldr x0, [x0, :got_lo12:g]
    mov x20, x0
    mov x9, x0
    ldr x0, [x9]
    ldr x1, [x9, #8]
    str x0, [sp, #32]
    str x1, [sp, #40]
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        // Invariant: a surviving read of x9 must have a live definition.
        // The broken pipeline produced `ldr x0, [x0]; ldr x1, [x9, #8]`
        // with the defining mov deleted — the second half read a stale x9.
        let stale_x9_read = result.contains("[x9") && !result.contains("mov x9,");
        // Both halves must load the global's 16 bytes: half one from the
        // GOT-resolved base (x0), half two from either a live x9 or the
        // propagated base.
        let second_half_ok = result.contains("ldr x1, [x9, #8]")
            || result.contains("ldr x1, [x0, #8]")
            || result.contains("mov x9,");
        assert!(
            !stale_x9_read,
            "x9 read without a live definition:\n{}",
            result
        );
        assert!(second_half_ok, "second carrier-load half lost:\n{}", result);
    }

    #[test]
    fn test_x9_address_mov_deleted_when_fully_dead() {
        // Positive case: ALL uses rewritten to the source register -> the
        // mov is provably dead and must still be removed.
        let input = "\
f:
    mov x9, x24
    ldr q0, [x24]
    ldr q1, [x24, #16]
    str q0, [sp, #16]
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mov x9, x24"),
            "dead address mov must be removed:\n{}",
            result
        );
    }

    #[test]
    fn test_x9_address_mov_kept_across_redef_and_use() {
        // x9 redefined (`mov x9, x5`) before its next read: the FIRST mov
        // is dead past the redefinition and may go; the redefining mov must
        // stay because `ldr x2, [x9]` reads it.
        let input = "\
f:
    mov x9, x24
    ldr x0, [x24]
    mov x9, x5
    ldr x2, [x9]
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mov x9, x24"),
            "mov dead before redefinition must be removed:\n{}",
            result
        );
        // Either the redefining mov survives for the read, or the read was
        // propagated straight to x5 — both keep x2 loading x5's value.
        assert!(
            result.contains("mov x9, x5") || result.contains("ldr x2, [x5]"),
            "x2 must still load x5's value:\n{}",
            result
        );
    }

    // ── Address-alias soundness ────────────────────────────────────────

    #[test]
    fn test_address_alias_mov_survives_cross_block_use() {
        // The mov's register is used as an address in a LATER block (after a
        // call): deleting the initializer must not happen even though the
        // in-window use is a pure address alias.
        let input = "\
f:
    adrp x0, tbl
    add x0, x0, :lo12:tbl
    mov x23, x0
    ldr x4, [x23, x28, lsl #3]
    mov x19, x4
    bl strcmp
.LBB9:
    ldr x19, [x23, x27, lsl #3]
    str x22, [x23, x27, lsl #3]
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mov x23, x0"),
            "defining mov must survive:\n{}",
            result
        );
        assert!(
            result.contains("[x23, x27, lsl #3]"),
            "cross-block use must not be rewritten:\n{}",
            result
        );
    }

    #[test]
    fn test_address_alias_folded_when_provably_dead() {
        // The intended optimization: a short-lived address alias whose
        // register is overwritten (without being read) after the uses.
        let input = "\
f:
    mov x9, x26
    ldr x4, [x9]
    str x4, [x9, #8]
    mov x9, x5
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mov x9, x26"),
            "dead alias mov removed:\n{}",
            result
        );
        assert!(
            result.contains("ldr x4, [x26]"),
            "use rewritten:\n{}",
            result
        );
        assert!(
            result.contains("str x4, [x26, #8]"),
            "use rewritten:\n{}",
            result
        );
    }

    // ── FP spill slot forwarding ─────────────────────────────────────────

    #[test]
    fn test_fp_slot_forward_basic() {
        let input = "\
f:
    fsub d0, d12, d21
    str d0, [sp, #224]
    fmul d3, d0, d0
    ldr d1, [sp, #224]
    fadd d4, d1, d3
    ret
g:
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("fmov d1, d0"),
            "reload forwarded:\n{}",
            result
        );
        assert!(!result.contains("ldr d1, [sp, #224]"));
    }

    #[test]
    fn test_fp_slot_forward_same_reg_deletes() {
        let input = "\
f:
    str d7, [sp, #64]
    ldr d7, [sp, #64]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("ldr d7, [sp, #64]"),
            "redundant reload deleted:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_slot_forward_blocked_by_source_clobber() {
        // d0 is overwritten between the store and the load: no forwarding.
        let input = "\
f:
    str d0, [sp, #224]
    fmul d0, d1, d2
    ldr d1, [sp, #224]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldr d1, [sp, #224]"),
            "clobbered source must not forward:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_slot_forward_blocked_by_unknown_store() {
        // A store through an untracked base may alias the slot: no forwarding.
        let input = "\
f:
    str d0, [sp, #224]
    str d5, [x9]
    ldr d1, [sp, #224]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldr d1, [sp, #224]"),
            "unknown-base store must block:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_slot_forward_via_materialized_base() {
        // Stores through an sp-derived base register are tracked.
        let input = "\
f:
    add x23, sp, #112
    fmul d28, d27, d17
    str d28, [x23]
    fadd d4, d1, d1
    ldr d6, [sp, #112]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("fmov d6, d28"),
            "base-tracked forward:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_slot_forward_gp_store_overlap_blocks() {
        // A GP word store into the upper half of the double slot must
        // invalidate the double's forward entry.
        let input = "\
f:
    str d0, [sp, #224]
    str w1, [sp, #228]
    ldr d1, [sp, #224]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldr d1, [sp, #224]"),
            "overlapping GP store must block:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_slot_forward_blocked_by_intervening_reload_of_source() {
        // The reload of d0 from a different slot clobbers the tracked source
        // of slot #232's entry: forwarding the second load as `fmov d1, d0`
        // would read the wrong value.
        let input = "\
f:
    str d0, [sp, #232]
    ldr d0, [sp, #224]
    ldr d1, [sp, #232]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("fmov d1, d0"),
            "reload of source must block forward:\n{}",
            result
        );
    }

    // ── FP field pair fusion (ldp/stp) ───────────────────────────────────

    #[test]
    fn test_fp_pair_loads_fused() {
        let input = "\
f:
    ldr d22, [x7]
    ldr d23, [x26]
    fsub d27, d22, d23
    ldr d28, [x7, #8]
    fsub d30, d28, d29
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldp d22, d28, [x7]"),
            "loads fused:\n{}",
            result
        );
        assert!(!result.contains("ldr d28, [x7, #8]"));
    }

    #[test]
    fn test_fp_pair_stores_fused() {
        let input = "\
f:
    str d10, [x8]
    str d22, [x8, #8]
    ldr d1, [x8]
    ldr d2, [x8, #8]
    fadd d0, d1, d2
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("stp d10, d22, [x8]"),
            "stores fused:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_pair_loads_blocked_by_store() {
        // A store between the loads may alias the second load's address.
        let input = "\
f:
    ldr d22, [x7]
    str d0, [x9]
    ldr d28, [x7, #8]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldr d28, [x7, #8]"),
            "store must block fusion:\n{}",
            result
        );
        assert!(!result.contains("ldp"));
    }

    #[test]
    fn test_fp_pair_stores_blocked_by_load() {
        // A load between the stores may read the second store's address.
        let input = "\
f:
    str d10, [x7]
    ldr d0, [x9]
    str d22, [x7, #8]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("str d22, [x7, #8]"),
            "load must block store fusion:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_pair_reversed_offsets() {
        let input = "\
f:
    ldr d28, [x7, #8]
    ldr d22, [x7]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ldp d22, d28, [x7]"),
            "reversed pair normalized:\n{}",
            result
        );
    }

    // ── Repeated slot load elimination ───────────────────────────────────

    #[test]
    fn test_repeated_slot_load_same_reg_deleted() {
        let input = "\
f:
    ldr x9, [sp, #392]
    ldr d28, [x9, #80]
    fmul d29, d28, d8
    ldr x9, [sp, #392]
    ldr d30, [x9, #56]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert_eq!(
            result.matches("ldr x9, [sp, #392]").count(),
            1,
            "second load deleted:\n{}",
            result
        );
    }

    #[test]
    fn test_repeated_slot_load_different_reg_mov() {
        // The repeat load becomes a mov, and copy-propagation then folds the
        // mov into the consumer — the second slot load disappears entirely.
        let input = "\
f:
    ldr x9, [sp, #392]
    ldr d28, [x9, #80]
    ldr x10, [sp, #392]
    ldr d30, [x10, #56]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("ldr x10, [sp, #392]"),
            "repeat load gone:\n{}",
            result
        );
        assert!(
            result.contains("ldr d30, [x9, #56]"),
            "consumer uses x9:\n{}",
            result
        );
    }

    #[test]
    fn test_repeated_slot_load_blocked_by_intervening_store() {
        // A store through an unknown base may write the frame slot.
        let input = "\
f:
    ldr x9, [sp, #392]
    str d0, [x26]
    ldr x9, [sp, #392]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert_eq!(
            result.matches("ldr x9, [sp, #392]").count(),
            2,
            "unknown-base store must block:\n{}",
            result
        );
    }

    #[test]
    fn test_repeated_slot_load_blocked_by_reg_clobber() {
        let input = "\
f:
    ldr x9, [sp, #392]
    add x9, x9, #8
    ldr x9, [sp, #392]
    ret
";
        // x9 was overwritten by the add: the second load must stay.
        let result = peephole_optimize(input.to_string());
        assert_eq!(
            result.matches("ldr x9, [sp, #392]").count(),
            2,
            "clobbered register must block:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_pair_latch_stores_no_overlap() {
        // Consecutive latch spill stores fuse pairwise; an already-paired line
        // must not be re-paired into an overlapping stp.
        let input = "\
f:
    add x19, x26, #56
    str d10, [sp, #344]
    str d22, [sp, #352]
    str d30, [sp, #360]
    str d19, [sp, #368]
    str d27, [sp, #376]
    mov x26, x19
    ldr d0, [sp, #344]
    ldr d1, [sp, #352]
    ldr d2, [sp, #360]
    ldr d3, [sp, #368]
    ldr d4, [sp, #376]
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("stp d10, d22, [sp, #344]"),
            "first pair:\n{}",
            result
        );
        assert!(
            result.contains("stp d30, d19, [sp, #360]"),
            "second pair:\n{}",
            result
        );
        assert!(
            !result.contains("stp d22"),
            "no overlapping re-fusion:\n{}",
            result
        );
    }

    // ── Zext move-chain folding ──────────────────────────────────────────

    #[test]
    fn test_zext_move_chain_folded() {
        // x0 is overwritten write-only after the chain, proving the chain
        // value dead in this block: fold is legal. (Without that overwrite
        // the trailing `ret` reads x0 as the return value and MUST block
        // the fold; see test_zext_move_chain_blocked_by_ret.)
        let input = "\
f:
    mov x0, x24
    mov w0, w0
    mov x26, x0
    mov x0, #7
    mul w4, w26, w25
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(result.contains("mov w26, w24"), "chain folded:\n{}", result);
        assert!(!result.contains("mov w0, w0"));
    }

    #[test]
    fn test_zext_move_chain_blocked_by_ret() {
        // The chain accumulator IS the return-value register: `ret` reads
        // x0, so folding would change the returned value (aarch64_fuzz
        // seeds 0/2/5 at -O1: d2hi return chain, 40/100 mismatches).
        let input = "\
f:
    mov x0, x22
    mov w0, w0
    mov x23, x0
    ret
";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mov w0, w0"),
            "zext chain into x0 before ret must not fold:\n{}",
            result
        );
    }

    #[test]
    fn test_zext_move_chain_blocked_when_accumulator_live() {
        let input = "\
f:
    mov x0, x24
    mov w0, w0
    mov x26, x0
    mul w4, w26, w25
    add w5, w0, #1
    ret
";
        // x0 is read after the chain: folding would lose the zext value.
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mov w0, w0"),
            "live accumulator blocks fold:\n{}",
            result
        );
    }

    #[test]
    fn test_sink_loop_carried_store() {
        // Bottom-tested loop: the slot is never read inside, the value stays
        // live in x20; the store can be sunk to the fall-through exit.
        let input = "\
f:
    mov x20, #0
    movz x21, #100
    b .Lguard
.Lbody:
    sxtw x22, w20
    add x0, x22, x20
    str x0, [sp, #24]
    mov x20, x0
    cmp w20, w21
    b.le .Lbody
    b .Lexit
.Lguard:
    cmp w20, w21
    b.le .Lbody
    b .Lexit
.Lexit:
    ldr x0, [sp, #24]
    ret
";
        let result = {
            let mut lines: Vec<String> = input.lines().map(String::from).collect();
            let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
            let n = lines.len();
            let changed = sink_loop_carried_stores(&mut lines, &mut kinds, n);
            let out = lines
                .iter()
                .zip(kinds.iter())
                .filter(|(_, k)| **k != LineKind::Nop)
                .map(|(l, _)| l.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(changed, "store should be sunk:\n{}", out);
            out
        };
        // Exactly one store remains, on the exit path after the backedge.
        assert_eq!(
            result.matches("str x0, [sp, #24]").count(),
            1,
            "\n{}",
            result
        );
        let be = result.find("b.le .Lbody").unwrap();
        let st = result.find("str x0, [sp, #24]").unwrap();
        assert!(st > be, "store after backedge:\n{}", result);
    }

    #[test]
    fn test_sink_blocked_when_slot_read_in_loop() {
        let input = "\
f:
    mov x20, #0
.Lbody:
    ldr x0, [sp, #24]
    add x0, x0, x20
    str x0, [sp, #24]
    mov x20, x0
    cmp w20, w21
    b.le .Lbody
    ldr x0, [sp, #24]
    ret
";
        let mut lines: Vec<String> = input.lines().map(String::from).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        assert!(!sink_loop_carried_stores(&mut lines, &mut kinds, n));
    }

    #[test]
    fn test_sink_blocked_by_mid_loop_exit() {
        let input = "\
f:
    mov x20, #0
.Lbody:
    cmp w20, w22
    b.gt .Lexit
    add x0, x20, #1
    str x0, [sp, #24]
    mov x20, x0
    cmp w20, w21
    b.le .Lbody
.Lexit:
    ldr x0, [sp, #24]
    ret
";
        let mut lines: Vec<String> = input.lines().map(String::from).collect();
        let mut kinds: Vec<LineKind> = lines.iter().map(|l| classify_line(l)).collect();
        let n = lines.len();
        assert!(!sink_loop_carried_stores(&mut lines, &mut kinds, n));
    }

    // ── eliminate_dead_pre_ret_moves ──────────────────────────────────────

    #[test]
    fn test_dead_pre_ret_move_removed() {
        // Staged-to-home move whose destination nothing reads before the
        // ret (bool_gate shape): dead, removed.
        let input = "f:\n    cset x0, ne\n    sxtw x0, w0\n    mov x5, x0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("mov x5, x0"),
            "dead staging mov kept:\n{}",
            result
        );
    }

    #[test]
    fn test_dead_pre_ret_x0_return_reg_kept() {
        // A write to x0 immediately before ret IS the return value: kept.
        let input = "f:\n    mov x0, x5\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mov x0, x5"),
            "return-register write deleted:\n{}",
            result
        );
    }

    #[test]
    fn test_dead_pre_ret_w0_zext_semantics_kept() {
        // `mov w0, w0` is a 32-bit write that zero-extends x0's low half —
        // observable through the caller's view of x0. Never delete.
        let input = "f:\n    mov x0, x22\n    mov w0, w0\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mov w0, w0"),
            "zext-into-x0 deleted:\n{}",
            result
        );
    }

    #[test]
    fn test_dead_pre_ret_read_blocks_removal() {
        // The move's destination is read by a store through an argument
        // pointer (which DSE cannot prove dead): the move is live and the
        // whole sequence must survive verbatim.
        let input = "f:\n    mov x5, x0\n    str x5, [x19]\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("mov x5, x0"),
            "live move deleted:\n{}",
            result
        );
        assert!(result.contains("str x5, [x19]"), "store lost:\n{}", result);
    }

    // ── fuse_and_cmp_zero ─────────────────────────────────────────────────

    #[test]
    fn test_and_cmp_fused_to_ands() {
        let input = "f:\n    and w5, w19, w20\n    cmp w5, #0\n    cset x0, ne\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ands w5, w19, w20"),
            "fusion missed:\n{}",
            result
        );
        assert!(
            !result.contains("cmp w5, #0"),
            "cmp kept after fusion:\n{}",
            result
        );
    }

    #[test]
    fn test_and_cmp_ccond_not_fused() {
        // b.cs consumes the C flag: cmp #0 sets C=1, ands sets the shifter
        // carry — the flags differ, so no fusion.
        let input =
            "f:\n    and w5, w19, w20\n    cmp w5, #0\n    b.cs .L1\n    ret\n.L1:\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(!result.contains("ands w5"), "C-consumer fused:\n{}", result);
        assert!(
            result.contains("cmp w5, #0"),
            "cmp dropped for C-consumer:\n{}",
            result
        );
    }

    // ── eliminate_redundant_zext_and ──────────────────────────────────────

    #[test]
    fn test_zext_and_after_ldrb_removed() {
        // ldrb already zero-extends: the #0xff re-mask is a no-op.
        let input =
            "f:\n    ldrb w5, [x19, x1]\n    and w4, w5, #0xff\n    add x9, x4, #1\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            !result.contains("and w4, w5, #0xff"),
            "redundant mask kept:\n{}",
            result
        );
        assert!(
            result.contains("mov w4, w5"),
            "mask not rewritten to mov:\n{}",
            result
        );
    }

    #[test]
    fn test_zext_and_unknown_source_kept() {
        // w6's high bits are unknown: the mask is semantically required.
        let input = "f:\n    mov w4, w6\n    and w5, w4, #0xff\n    add x9, x5, #1\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("and w5, w4, #0xff"),
            "needed mask removed:\n{}",
            result
        );
    }

    #[test]
    fn test_zext_and_never_rewrites_ands() {
        // ands defines flags for the cset: value-redundant or not, it must
        // stay an ands.
        let input =
            "f:\n    ldrb w5, [x19, x1]\n    ands w4, w5, #0xff\n    cset x9, ne\n    ret\n";
        let result = peephole_optimize(input.to_string());
        assert!(
            result.contains("ands w4, w5, #0xff"),
            "flag-setting ands rewritten:\n{}",
            result
        );
    }
    // ── Tests: the implicit-operand oracle ───────────────────────────────

    #[test]
    fn oracle_bl_blr_ret_have_their_link_register_contract() {
        // `bl sym` links through x30 without naming it.
        assert_eq!(classify_implicit_operands_a64("bl printf"), (0, 1u64 << 30));
        // `blr x3` READS the branch target and links through x30.
        assert_eq!(classify_implicit_operands_a64("blr x3"), (1 << 3, 1u64 << 30));
        // `br x3` reads the target and writes nothing.
        assert_eq!(classify_implicit_operands_a64("br x3"), (1 << 3, 0));
        // Bare `ret` returns through the link register: an implicit read.
        assert_eq!(classify_implicit_operands_a64("ret"), (1u64 << 30, 0));
        // `ret x4` names its target: nothing implicit left.
        assert_eq!(classify_implicit_operands_a64("ret x4"), (0, 0));
    }

    #[test]
    fn oracle_writeback_forms_write_their_base() {
        // The byte-copy loop registers: post-index writes the base the
        // destination-oriented scans never see.
        assert_eq!(classify_implicit_operands_a64("strb w12, [x9], #1").1, 1 << 9);
        assert_eq!(classify_implicit_operands_a64("ldrb w12, [x10], #1").1, 1 << 10);
        assert_eq!(classify_implicit_operands_a64("str x0, [x1, #8]!").1, 1 << 1);
        // Every epilogue: `ldp x29, x30, [sp], #frame` writes sp.
        assert_eq!(classify_implicit_operands_a64("ldp x29, x30, [sp], #16").1, 1 << 31);
        assert_eq!(classify_implicit_operands_a64("stp x29, x30, [sp, #-16]!").1, 1 << 31);
        // Plain offset forms do NOT write the base.
        assert_eq!(classify_implicit_operands_a64("ldr x0, [x1, #8]").1, 0);
        assert_eq!(classify_implicit_operands_a64("str x0, [sp, #8]").1, 0);
        // The base is the register inside the brackets, not the data reg.
        assert_eq!(classify_implicit_operands_a64("ldr x0, [x5], #8").1, 1 << 5);
        // Scaled register offsets do not confuse the bracket parse.
        assert_eq!(classify_implicit_operands_a64("ldr x0, [x1, x2, lsl #3]").1, 0);
    }

    #[test]
    fn oracle_non_instructions_name_nothing() {
        assert_eq!(classify_implicit_operands_a64(".LBB1:"), (0, 0));
        assert_eq!(classify_implicit_operands_a64(".cfi_startproc"), (0, 0));
        assert_eq!(classify_implicit_operands_a64(""), (0, 0));
        // Branches, compares and barriers touch no GP register implicitly.
        assert_eq!(classify_implicit_operands_a64("cmp x0, x1"), (0, 0));
        assert_eq!(classify_implicit_operands_a64("b.ne .LBB2"), (0, 0));
        assert_eq!(classify_implicit_operands_a64("dmb ish"), (0, 0));
        assert_eq!(classify_implicit_operands_a64("svc #0"), (0, 0));
    }

    #[test]
    fn instr_clobbers_reg_sees_the_post_index_base_write() {
        // The store exclusion is correct for the DATA operand but lied
        // about the base: sinking a store past this line with x9 as the
        // data register read the incremented pointer.
        assert!(instr_clobbers_reg("strb w12, [x9], #1", 9));
        assert!(instr_clobbers_reg("ldrb w12, [x10], #1", 10));
        assert!(!instr_clobbers_reg("strb w12, [x9], #1", 12));
        assert!(!instr_clobbers_reg("str x0, [x1, #8]", 1));
        // bl/blr redefine x30.
        assert!(instr_clobbers_reg("bl printf", 30));
        assert!(instr_clobbers_reg("blr x3", 30));
        // blr's operand is a read, not a clobber.
        assert!(!instr_clobbers_reg("blr x3", 3));
    }

    #[test]
    fn line_defines_gp_tells_the_truth_about_pair_loads_and_writeback() {
        // ldp defines BOTH registers — the second one used to be `Some(false)`.
        assert_eq!(line_defines_gp("ldp x9, x10, [sp]", LineKind::LoadPairSp, 9), Some(true));
        assert_eq!(line_defines_gp("ldp x9, x10, [sp]", LineKind::LoadPairSp, 10), Some(true));
        assert_eq!(line_defines_gp("ldp x9, x10, [sp]", LineKind::LoadPairSp, 11), Some(false));
        // A post-index store defines its base despite the store mnemonic.
        assert_eq!(
            line_defines_gp("str x0, [x9], #8", LineKind::MemOther, 9),
            Some(true)
        );
        assert_eq!(
            line_defines_gp("str x0, [x9], #8", LineKind::MemOther, 0),
            Some(false)
        );
    }

    #[test]
    fn copy_propagation_does_not_redirect_writeback_onto_the_alias_source() {
        // `mov x1, x5` + post-index store: renaming [x1] to [x5] would
        // increment x5 and corrupt every later x5 reader.
        let asm = concat!(
            "f:\n",
            "    mov x1, x5\n",
            "    str x9, [x1], #8\n",
            "    add x0, x5, x5\n",
            "    ret\n",
        );
        let result = peephole_optimize(asm.to_string());
        assert!(
            result.contains("str x9, [x1], #8"),
            "the writeback base must keep the copy's destination:\n{result}"
        );
    }

}

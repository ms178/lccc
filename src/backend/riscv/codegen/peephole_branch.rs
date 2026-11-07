//! RISC-V peephole: boolean folds, compare-and-branch fusion, and the
//! duplication of short exit tails into a conditional's fall-through arm.
//! A child of `peephole`, which owns the line model.
//!
//! The three passes close the gap between the select emitter's branchy form
//! and Clang 23.1.5's RV64 code for the same C:
//!
//! ```text
//! int f(int a, int b) { return a < b ? 7 : -3; }
//!
//!   lccc before                      Clang 23.1.5 / lccc after
//!   slt   a2, a0, a1                 blt a0, a1, .L
//!   li    t2, -3                     li  a0, -3
//!   beqz  a2, .Lsel_skip             ret
//!   li    t2, 7                   .L:
//! .Lsel_skip:                        li  a0, 7
//!   mv    a0, t2                     ret
//!   ret
//! ```
//!
//! `fuse_compare_branches` turns `slt t, a, b; beqz t, L` into `bge a, b,
//! L`; `duplicate_exit_tails` gives the fall-through arm its own copy of the
//! two-instruction exit and moves the pre-branch default into the taken
//! path; the copy folding that follows forwards each constant into a0.
//! `fold_boolean_negations` removes double negations of 0/1 values
//! (`seqz a3, t0; seqz t0, a3` is `snez t0, t0`) and folds `x >= 0` masks
//! (`slti; xori 1; neg` is `slti; addi -1`, Clang's `srli 63; addi -1`).

use super::{
    LineKind, OPAQUE_LINE, REG_NONE, RvLazyLiveness, branch_parts, classify_line, jump_target,
    label_name, named_gp_dest, parse_asm_i64, parse_reg, rv_bit, rv_function_ranges,
    rv_is_tail_line, rv_next_kept_line, rv_uses_defs,
};
use crate::common::fx_hash::FxHashMap;

/// The mnemonic and comma-separated operands of an instruction line, or
/// `None` for a line with a comment or without operands.
fn mnemonic_operands(line: &str) -> Option<(&str, Vec<&str>)> {
    let trimmed = line.trim();
    if trimmed.contains('#') {
        return None;
    }
    let (mnem, rest) = trimmed.split_once(char::is_whitespace)?;
    Some((mnem, rest.split(',').map(str::trim).collect()))
}

/// Lines a backward scan for a register's definition may cross: anything
/// but a control-flow boundary.
fn crosses_block_boundary(kind: LineKind) -> bool {
    matches!(
        kind,
        LineKind::Label | LineKind::Branch | LineKind::Jump | LineKind::Call | LineKind::Ret
    )
}

/// Whether `reg` holds 0 or 1 just before line `i`: its last definition in
/// the straight-line run ending at `i` is a set-on-compare, `seqz`/`snez`
/// or `andi reg, x, 1`.
fn boolean_before(lines: &[String], kinds: &[LineKind], i: usize, reg: u8) -> bool {
    let mut k = i;
    for _ in 0..16 {
        if k == 0 {
            return false;
        }
        k -= 1;
        match kinds[k] {
            LineKind::Nop => continue,
            kind if crosses_block_boundary(kind) || kind == LineKind::Directive => return false,
            _ => {}
        }
        if named_gp_dest(&lines[k], kinds[k]) != Some(reg)
            && rv_uses_defs(&lines[k], kinds[k]).1 & rv_bit(reg) == 0
        {
            continue;
        }
        let Some((mnem, ops)) = mnemonic_operands(&lines[k]) else {
            return false;
        };
        if ops.first().map(|d| parse_reg(d)) != Some(reg) {
            return false;
        }
        return match mnem {
            "seqz" | "snez" | "slt" | "sltu" | "slti" | "sltiu" => true,
            "andi" => ops.get(2).and_then(|v| parse_asm_i64(v)) == Some(1),
            _ => false,
        };
    }
    false
}

/// `seqz`/`snez` of a `seqz`/`snez` result, and `neg` of `xori s, 1` of a
/// 0/1 `s`, as one instruction when the intermediate dies.
///
/// * `seqz t, x; seqz d, t` is `snez d, x`; `snez t, x; seqz d, t` is
///   `seqz d, x`; `snez` of either keeps it (`snez` of 0/1 is the identity).
/// * `xori t, s, 1; neg[w] d, t` (or `sub[w] d, zero, t`) is `-(1 - s)` =
///   `s - 1`: `addi d, s, -1`, the same 0/-1 in both widths.
pub(super) fn fold_boolean_negations(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    let mut live = RvLazyLiveness::new(lines, kinds, n);
    let mut changed = false;
    for i in 0..n {
        if !matches!(kinds[i], LineKind::Alu | LineKind::Other) {
            continue;
        }
        let Some((m1, ops1)) = mnemonic_operands(&lines[i]) else {
            continue;
        };
        if !matches!(m1, "seqz" | "snez" | "xori") {
            continue;
        }
        let j = rv_next_kept_line(kinds, i + 1, n);
        if j >= n {
            continue;
        }
        let Some((m2, ops2)) = mnemonic_operands(&lines[j]) else {
            continue;
        };
        let folded = match (m1, ops1.as_slice(), m2, ops2.as_slice()) {
            ("seqz" | "snez", [t, x], "seqz" | "snez", [d, t2]) if t == t2 => {
                let mnem = match (m1, m2) {
                    ("seqz", "seqz") => "snez",
                    ("snez", "seqz") => "seqz",
                    _ => m1,
                };
                Some((*t, *d, format!("    {mnem} {d}, {x}")))
            }
            ("xori", [t, s, one], "neg" | "negw", [d, t2])
                if t == t2
                    && parse_asm_i64(one) == Some(1)
                    && boolean_before(lines, kinds, i, parse_reg(s)) =>
            {
                Some((*t, *d, format!("    addi {d}, {s}, -1")))
            }
            ("xori", [t, s, one], "sub" | "subw", [d, zero, t2])
                if t == t2
                    && matches!(*zero, "zero" | "x0")
                    && parse_asm_i64(one) == Some(1)
                    && boolean_before(lines, kinds, i, parse_reg(s)) =>
            {
                Some((*t, *d, format!("    addi {d}, {s}, -1")))
            }
            _ => None,
        };
        let Some((t, d, text)) = folded else {
            continue;
        };
        let (t, d) = (parse_reg(t), parse_reg(d));
        if t == REG_NONE || d == REG_NONE || (t != d && !live.dead_after(lines, kinds, j, t)) {
            continue;
        }
        lines[i] = text;
        kinds[i] = classify_line(&lines[i]);
        kinds[j] = LineKind::Nop;
        live.invalidate();
        changed = true;
    }
    changed
}

/// A 0/1 producer followed by `beqz`/`bnez` of its result, as one compare
/// and branch when the result dies on both edges (Clang 23.1.5's `blt a0,
/// a1, .L` for `a < b ? 7 : -3`):
///
/// | producer           | `beqz t, L`     | `bnez t, L`     |
/// |--------------------|-----------------|-----------------|
/// | `slt t, a, b`      | `bge a, b, L`   | `blt a, b, L`   |
/// | `sltu t, a, b`     | `bgeu a, b, L`  | `bltu a, b, L`  |
/// | `slti t, a, 0`     | `bgez a, L`     | `bltz a, L`     |
/// | `seqz`/`sltiu 1`   | `bnez a, L`     | `beqz a, L`     |
/// | `snez t, a`        | `beqz a, L`     | `bnez a, L`     |
///
/// The branch keeps its place and its B-type reach.
pub(super) fn fuse_compare_branches(
    lines: &mut [String],
    kinds: &mut [LineKind],
    n: usize,
) -> bool {
    let mut live = RvLazyLiveness::new(lines, kinds, n);
    let mut changed = false;
    for i in 0..n {
        if !matches!(kinds[i], LineKind::Alu | LineKind::Other) {
            continue;
        }
        let Some((m1, ops1)) = mnemonic_operands(&lines[i]) else {
            continue;
        };
        if !matches!(m1, "slt" | "sltu" | "slti" | "sltiu" | "seqz" | "snez") {
            continue;
        }
        let j = rv_next_kept_line(kinds, i + 1, n);
        if j >= n || kinds[j] != LineKind::Branch {
            continue;
        }
        let Some((m2, ops2)) = mnemonic_operands(&lines[j]) else {
            continue;
        };
        let ([tested, target], on_zero) = (ops2.as_slice(), m2 == "beqz") else {
            continue;
        };
        if !matches!(m2, "beqz" | "bnez") {
            continue;
        }
        let pick =
            |if_zero: &'static str, if_set: &'static str| if on_zero { if_zero } else { if_set };
        let reg_ok = |r: &str| parse_reg(r) != REG_NONE;
        let fused = match (m1, ops1.as_slice()) {
            ("slt", [t, a, b]) if t == tested && reg_ok(a) && reg_ok(b) => {
                format!("    {} {a}, {b}, {target}", pick("bge", "blt"))
            }
            ("sltu", [t, a, b]) if t == tested && reg_ok(a) && reg_ok(b) => {
                format!("    {} {a}, {b}, {target}", pick("bgeu", "bltu"))
            }
            ("slti", [t, a, k]) if t == tested && reg_ok(a) && parse_asm_i64(k) == Some(0) => {
                format!("    {} {a}, {target}", pick("bgez", "bltz"))
            }
            ("sltiu", [t, a, k]) if t == tested && reg_ok(a) && parse_asm_i64(k) == Some(1) => {
                format!("    {} {a}, {target}", pick("bnez", "beqz"))
            }
            ("seqz", [t, a]) if t == tested && reg_ok(a) => {
                format!("    {} {a}, {target}", pick("bnez", "beqz"))
            }
            ("snez", [t, a]) if t == tested && reg_ok(a) => {
                format!("    {} {a}, {target}", pick("beqz", "bnez"))
            }
            _ => continue,
        };
        let t = parse_reg(tested);
        if t == REG_NONE || !live.dead_after(lines, kinds, j, t) {
            continue;
        }
        lines[j] = fused;
        kinds[j] = classify_line(&lines[j]);
        kinds[i] = LineKind::Nop;
        live.invalidate();
        changed = true;
    }
    changed
}

/// Kept lines of an exit tail `duplicate_exit_tails` copies, the exit
/// (`ret` or a sibling call's `tail`) included.
const TAIL_MAX: usize = 4;
/// Kept lines of the arm between the branch and the label.
const ARM_MAX: usize = 4;
/// Kept lines of an exit tail an unconditional jump takes a copy of, the
/// exit included (the size LLVM's tail duplication grants a return block
/// at -O2: `li a0, 0; ret` for a switch's default).
const JUMP_TAIL_MAX: usize = 3;

/// A line an arm or a tail may hold: a register-only instruction with one
/// named destination, or a frame load (an epilogue restore).
fn plain_line(line: &str, kind: LineKind) -> bool {
    match kind {
        LineKind::LoadImm { .. }
        | LineKind::Move { .. }
        | LineKind::SextW { .. }
        | LineKind::LoadAddr { .. }
        | LineKind::LoadS0 { .. } => true,
        LineKind::Alu => named_gp_dest(line, kind).is_some(),
        _ => false,
    }
}

/// One `duplicate_exit_tails` rewrite: the default `def` before the branch
/// `branch`, whose only target is the label at `label`; the exit tail runs
/// from `label + 1` through `exit`.
struct TailSite {
    def: usize,
    label: usize,
    exit: usize,
}

/// The last line of the exit tail after the label at `l`: at most `max`
/// kept lines, plain ones and then the exit (`ret` or a sibling call's
/// `tail`).
fn exit_tail(
    lines: &[String],
    kinds: &[LineKind],
    l: usize,
    fend: usize,
    max: usize,
) -> Option<usize> {
    let mut kept = 0;
    for k in l + 1..fend {
        if kinds[k] == LineKind::Nop {
            continue;
        }
        kept += 1;
        if kept > max {
            return None;
        }
        if kinds[k] == LineKind::Ret
            || (kinds[k] == LineKind::Call && rv_is_tail_line(lines[k].trim()))
        {
            return Some(k);
        }
        if !plain_line(&lines[k], kinds[k]) {
            return None;
        }
    }
    None
}

/// The unconditional jumps of `[fstart, fend)` to a local label whose
/// block is a short exit tail: `(jump, label, exit)`.
fn jump_sites(
    lines: &[String],
    kinds: &[LineKind],
    (fstart, fend): (usize, usize),
) -> Vec<(usize, usize, usize)> {
    let labels: FxHashMap<&str, usize> = (fstart..fend)
        .filter(|&k| kinds[k] == LineKind::Label)
        .filter_map(|k| Some((label_name(&lines[k])?, k)))
        .filter(|(name, _)| name.starts_with(".L"))
        .collect();
    (fstart..fend)
        .filter(|&k| kinds[k] == LineKind::Jump)
        .filter_map(|k| {
            let l = *labels.get(jump_target(&lines[k])?)?;
            let exit = exit_tail(lines, kinds, l, fend, JUMP_TAIL_MAX)?;
            Some((k, l, exit))
        })
        .collect()
}

/// The rewrite of the label at `l` in the function `[fstart, fend)`, if it
/// matches (see `duplicate_exit_tails`).  `mentions` counts every line of
/// the whole file naming each local label, its definition included.
fn tail_site(
    lines: &[String],
    kinds: &[LineKind],
    (fstart, fend): (usize, usize),
    l: usize,
    mentions: &FxHashMap<String, usize>,
) -> Option<TailSite> {
    let name = label_name(&lines[l]).filter(|name| name.starts_with(".L"))?;
    // Exactly one other mention of the label anywhere in the file: a
    // conditional branch before it.  A jump-table word, a second branch, a
    // nonlocal goto from a nested function or an address taken would reach
    // the label without the moved default.
    if mentions.get(name) != Some(&2) {
        return None;
    }
    let b = (fstart..l).rev().find(|&k| {
        kinds[k] == LineKind::Branch
            && branch_parts(&lines[k]).map(|(_, _, target)| target) == Some(name)
    })?;
    // The arm: plain lines falling through into the label.
    let arm: Vec<usize> = (b + 1..l).filter(|&k| kinds[k] != LineKind::Nop).collect();
    if arm.is_empty()
        || arm.len() > ARM_MAX
        || !arm.iter().all(|&k| plain_line(&lines[k], kinds[k]))
    {
        return None;
    }
    let exit = exit_tail(lines, kinds, l, fend, TAIL_MAX)?;
    // The default: the plain line just before the branch, defining a
    // register the branch does not read and the arm overwrites before
    // (and without) reading it.
    let def = (fstart..b).rev().find(|&k| kinds[k] != LineKind::Nop)?;
    if !plain_line(&lines[def], kinds[def]) || matches!(kinds[def], LineKind::LoadS0 { .. }) {
        return None;
    }
    let r = named_gp_dest(&lines[def], kinds[def])?;
    let bit = rv_bit(r);
    if r == REG_NONE || rv_uses_defs(&lines[b], kinds[b]).0 & bit != 0 {
        return None;
    }
    let arm_reads = arm
        .iter()
        .any(|&k| rv_uses_defs(&lines[k], kinds[k]).0 & bit != 0);
    let arm_writes = arm
        .iter()
        .any(|&k| named_gp_dest(&lines[k], kinds[k]) == Some(r));
    if arm_reads || !arm_writes {
        return None;
    }
    Some(TailSite {
        def,
        label: l,
        exit,
    })
}

/// The local labels (`.L...` tokens) `line` names, each once.
fn local_label_tokens(line: &str) -> impl Iterator<Item = &str> {
    let mut seen: Vec<&str> = Vec::new();
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'))
        .filter(|tok| tok.starts_with(".L"))
        .filter(move |tok| {
            let fresh = !seen.contains(tok);
            if fresh {
                seen.push(tok);
            }
            fresh
        })
}

/// How many lines of the file name each local label -- the live lines and
/// the hidden originals of opaque functions (an `asm goto` may name one).
fn local_label_mentions(
    lines: &[String],
    kinds: &[LineKind],
    protected: &[(usize, String)],
) -> FxHashMap<String, usize> {
    let mut mentions: FxHashMap<String, usize> = FxHashMap::default();
    let live = lines
        .iter()
        .zip(kinds)
        .filter(|(_, kind)| **kind != LineKind::Nop)
        .map(|(line, _)| line.as_str());
    for line in live.chain(protected.iter().map(|(_, original)| original.as_str())) {
        for tok in local_label_tokens(line) {
            *mentions.entry(tok.to_string()).or_insert(0) += 1;
        }
    }
    mentions
}

/// Duplicate a short exit tail into the fall-through arm of the one
/// conditional branch that reaches its label, moving the default the arm
/// overrides into the taken path:
///
/// ```text
///   li   t2, 20          (default)          beqz a0, .L
///   beqz a0, .L                             li   t2, 10
///   li   t2, 10          (arm)              mv   a0, t2
/// .L:                                       ret
///   mv   a0, t2          (tail)      =>   .L:
///   ret                                     li   t2, 20
///                                           mv   a0, t2
///                                           ret
/// ```
///
/// Each path runs the instructions it ran before minus the default the arm
/// overwrote (two fewer on the fall-through path once the copies fold);
/// the taken path is unchanged.  The tail holds at most `TAIL_MAX` kept
/// lines ending in `ret` or a sibling call's `tail`, nothing but plain
/// register instructions and frame loads (no directive, so no CFI row is
/// duplicated); the arm at most `ARM_MAX` plain lines.
///
/// An unconditional jump to such a tail of at most `JUMP_TAIL_MAX` lines is
/// replaced by its own copy of it -- a switch's `j .Ldefault` before
/// `.Ldefault: li a0, 0; ret` -- which runs the same instructions minus the
/// jump; the label stays for its other references.
///
/// Functions with an opaque body are skipped.  Lines are inserted, so the
/// vectors are rebuilt; `protected` (indices of hidden opaque lines) is
/// remapped.
pub(super) fn duplicate_exit_tails(
    lines: &mut Vec<String>,
    kinds: &mut Vec<LineKind>,
    protected: &mut [(usize, String)],
) -> bool {
    let n = lines.len();
    let mut sites: Vec<TailSite> = Vec::new();
    let mut jumps: FxHashMap<usize, (usize, usize)> = FxHashMap::default();
    let mut mentions: Option<FxHashMap<String, usize>> = None;
    for (fstart, fend) in rv_function_ranges(lines, kinds, n) {
        if (fstart..fend).any(|k| lines[k].trim() == OPAQUE_LINE) {
            continue;
        }
        let mut busy_until = 0;
        for l in fstart..fend {
            if kinds[l] != LineKind::Label || l < busy_until {
                continue;
            }
            let mentions =
                mentions.get_or_insert_with(|| local_label_mentions(lines, kinds, protected));
            if let Some(site) = tail_site(lines, kinds, (fstart, fend), l, mentions) {
                busy_until = site.exit + 1;
                sites.push(site);
            }
        }
        // A jump to an arm site's label would reach it twice (no site).
        for (jump, label, exit) in jump_sites(lines, kinds, (fstart, fend)) {
            jumps.insert(jump, (label, exit));
        }
    }
    if sites.is_empty() && jumps.is_empty() {
        return false;
    }
    let extra: usize = sites.iter().map(|s| s.exit - s.label + 1).sum::<usize>()
        + jumps.values().map(|(l, e)| e - l).sum::<usize>();
    let mut new_lines: Vec<String> = Vec::with_capacity(n + extra);
    let mut new_kinds: Vec<LineKind> = Vec::with_capacity(n + extra);
    let mut remap: Vec<usize> = Vec::with_capacity(n);
    let mut next = sites.iter().peekable();
    let mut is_def = vec![false; n];
    for site in &sites {
        is_def[site.def] = true;
    }
    for i in 0..n {
        if let Some(&(label, exit)) = jumps.get(&i) {
            // The jump's own copy of the tail replaces it.
            for k in label + 1..=exit {
                new_lines.push(lines[k].clone());
                new_kinds.push(kinds[k]);
            }
            remap.push(new_lines.len());
            new_lines.push(lines[i].clone());
            new_kinds.push(LineKind::Nop);
            continue;
        }
        if let Some(site) = next.peek().filter(|s| s.label == i) {
            // The tail copy closes the arm; the default opens the taken
            // path right after the label.
            for k in i + 1..=site.exit {
                new_lines.push(lines[k].clone());
                new_kinds.push(kinds[k]);
            }
            remap.push(new_lines.len());
            new_lines.push(lines[i].clone());
            new_kinds.push(kinds[i]);
            new_lines.push(lines[site.def].clone());
            new_kinds.push(kinds[site.def]);
            next.next();
            continue;
        }
        remap.push(new_lines.len());
        new_lines.push(lines[i].clone());
        new_kinds.push(if is_def[i] { LineKind::Nop } else { kinds[i] });
    }
    for entry in protected.iter_mut() {
        entry.0 = remap[entry.0];
    }
    *lines = new_lines;
    *kinds = new_kinds;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::fx_hash::FxHashMap;

    fn prepare(asm: &str) -> (Vec<String>, Vec<LineKind>) {
        let lines: Vec<String> = asm.lines().map(str::to_owned).collect();
        let kinds = lines.iter().map(|l| classify_line(l)).collect();
        (lines, kinds)
    }

    /// The function's kept lines, trimmed.
    fn kept(lines: &[String], kinds: &[LineKind]) -> Vec<String> {
        lines
            .iter()
            .zip(kinds)
            .filter(|(_, k)| **k != LineKind::Nop)
            .map(|(l, _)| l.trim().to_owned())
            .collect()
    }

    /// Run the integer subset these passes produce from `f:` with a1/a2 set,
    /// a0 = `a0`; the a0 a `ret` hands back.
    fn run(lines: &[String], kinds: &[LineKind], a0: i64, a1: i64, a2: i64) -> i64 {
        let code: Vec<&str> = lines
            .iter()
            .zip(kinds)
            .filter(|(_, k)| **k != LineKind::Nop)
            .map(|(l, _)| l.trim())
            .collect();
        let labels: FxHashMap<&str, usize> = code
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.strip_suffix(':').map(|name| (name, i)))
            .collect();
        let mut regs: FxHashMap<u8, i64> = FxHashMap::default();
        regs.insert(parse_reg("a0"), a0);
        regs.insert(parse_reg("a1"), a1);
        regs.insert(parse_reg("a2"), a2);
        let get = |regs: &FxHashMap<u8, i64>, r: &str| -> i64 {
            if matches!(r, "zero" | "x0") {
                0
            } else {
                regs.get(&parse_reg(r)).copied().unwrap_or(0x5a5a_5a5a)
            }
        };
        let mut pc = 0;
        for _ in 0..1000 {
            let line = code[pc];
            pc += 1;
            if line.ends_with(':') {
                continue;
            }
            if line == "ret" {
                return get(&regs, "a0");
            }
            let (m, rest) = line.split_once(' ').expect("an instruction");
            let o: Vec<&str> = rest.split(',').map(str::trim).collect();
            let imm = |k: usize| parse_asm_i64(o[k]).expect("an immediate");
            let branch = |taken: bool, target: &str, pc: &mut usize| {
                if taken {
                    *pc = labels[target];
                }
            };
            match m {
                "beqz" => branch(get(&regs, o[0]) == 0, o[1], &mut pc),
                "bnez" => branch(get(&regs, o[0]) != 0, o[1], &mut pc),
                "bltz" => branch(get(&regs, o[0]) < 0, o[1], &mut pc),
                "bgez" => branch(get(&regs, o[0]) >= 0, o[1], &mut pc),
                "beq" => branch(get(&regs, o[0]) == get(&regs, o[1]), o[2], &mut pc),
                "bne" => branch(get(&regs, o[0]) != get(&regs, o[1]), o[2], &mut pc),
                "blt" => branch(get(&regs, o[0]) < get(&regs, o[1]), o[2], &mut pc),
                "bge" => branch(get(&regs, o[0]) >= get(&regs, o[1]), o[2], &mut pc),
                "bltu" => branch(
                    (get(&regs, o[0]) as u64) < (get(&regs, o[1]) as u64),
                    o[2],
                    &mut pc,
                ),
                "bgeu" => branch(
                    (get(&regs, o[0]) as u64) >= (get(&regs, o[1]) as u64),
                    o[2],
                    &mut pc,
                ),
                "j" => pc = labels[o[0]],
                _ => {
                    let s = |k: usize| get(&regs, o[k]);
                    let v = match m {
                        "li" => imm(1),
                        "mv" => s(1),
                        "seqz" => i64::from(s(1) == 0),
                        "snez" => i64::from(s(1) != 0),
                        "slt" => i64::from(s(1) < s(2)),
                        "sltu" => i64::from((s(1) as u64) < (s(2) as u64)),
                        "slti" => i64::from(s(1) < imm(2)),
                        "sltiu" => i64::from((s(1) as u64) < (imm(2) as u64)),
                        "xori" => s(1) ^ imm(2),
                        "andi" => s(1) & imm(2),
                        "addi" => s(1).wrapping_add(imm(2)),
                        "neg" => s(1).wrapping_neg(),
                        "negw" => i64::from((s(1) as i32).wrapping_neg()),
                        "sub" => s(1).wrapping_sub(s(2)),
                        "subw" => i64::from((s(1) as i32).wrapping_sub(s(2) as i32)),
                        "add" => s(1).wrapping_add(s(2)),
                        "xor" => s(1) ^ s(2),
                        "slli" => s(1) << imm(2),
                        "srli" => ((s(1) as u64) >> imm(2)) as i64,
                        _ => panic!("unmodelled `{line}`"),
                    };
                    regs.insert(parse_reg(o[0]), v);
                }
            }
        }
        panic!("no return");
    }

    const VALUES: [i64; 9] = [0, 1, -1, 2, -2, 1 << 32, i64::MIN, i64::MAX, 0x7fff_ffff];

    /// `seqz`/`snez` of a `seqz`/`snez` result folds for each of the four
    /// pairs, in place and into another register, and agrees with the
    /// original on every value; a still-read intermediate, a label between
    /// the two and a write between them keep the pair.
    #[test]
    fn double_negations_fold_exactly() {
        let mut folds = 0;
        for (m1, m2, want) in [
            ("seqz", "seqz", "snez"),
            ("snez", "seqz", "seqz"),
            ("seqz", "snez", "seqz"),
            ("snez", "snez", "snez"),
        ] {
            for d in ["t1", "a0"] {
                let tail = if d == "a0" { "" } else { "    mv a0, t1\n" };
                let asm = format!("f:\n    {m1} t1, a1\n    {m2} {d}, t1\n{tail}    ret\n");
                let (lines0, kinds0) = prepare(&asm);
                let (mut lines, mut kinds) = prepare(&asm);
                let n = lines.len();
                assert!(fold_boolean_negations(&mut lines, &mut kinds, n), "{asm}");
                assert_eq!(lines[1].trim(), format!("{want} {d}, a1"), "{asm}");
                assert_eq!(kinds[2], LineKind::Nop);
                for &v in &VALUES {
                    assert_eq!(run(&lines0, &kinds0, 0, v, 0), run(&lines, &kinds, 0, v, 0));
                }
                folds += 1;
            }
        }
        assert_eq!(folds, 8);
        for asm in [
            "f:\n    seqz t1, a1\n    seqz a0, t1\n    add a0, a0, t1\n    ret\n",
            "f:\n    seqz t1, a1\n.L1:\n    seqz a0, t1\n    ret\n",
            "f:\n    seqz t1, a1\n    li a2, 3\n    seqz a0, t1\n    ret\n",
            "f:\n    seqz t1, a1\n    seqz a0, t2\n    ret\n",
        ] {
            let (mut lines, mut kinds) = prepare(asm);
            let n = lines.len();
            assert!(!fold_boolean_negations(&mut lines, &mut kinds, n), "{asm}");
        }
    }

    /// `-(1 - b)` of a 0/1 `b` is `b - 1`: every boolean producer and every
    /// negation spelling, against the interpreter over pairs of boundary
    /// values; a non-boolean source, another immediate, a producer in an
    /// earlier block and a live intermediate keep the code.
    #[test]
    fn negated_boolean_complements_become_decrements() {
        let producers = [
            "slt t1, a1, a2",
            "sltu t1, a1, a2",
            "slti t1, a1, 0",
            "sltiu t1, a1, 5",
            "seqz t1, a1",
            "snez t1, a1",
            "andi t1, a1, 1",
        ];
        let negations = [
            "neg a0, t2",
            "negw a0, t2",
            "sub a0, zero, t2",
            "subw a0, x0, t2",
        ];
        let mut folds = 0;
        for p in producers {
            for neg in negations {
                let asm = format!("f:\n    {p}\n    xori t2, t1, 1\n    {neg}\n    ret\n");
                let (lines0, kinds0) = prepare(&asm);
                let (mut lines, mut kinds) = prepare(&asm);
                let n = lines.len();
                assert!(fold_boolean_negations(&mut lines, &mut kinds, n), "{asm}");
                assert_eq!(lines[2].trim(), "addi a0, t1, -1", "{asm}");
                for &x in &VALUES {
                    for &y in &VALUES {
                        assert_eq!(
                            run(&lines0, &kinds0, 0, x, y),
                            run(&lines, &kinds, 0, x, y),
                            "{asm} ({x}, {y})"
                        );
                    }
                }
                folds += 1;
            }
        }
        assert_eq!(folds, producers.len() * negations.len());
        for asm in [
            "f:\n    addi t1, a1, 0\n    xori t2, t1, 1\n    neg a0, t2\n    ret\n",
            "f:\n    slt t1, a1, a2\n    xori t2, t1, 3\n    neg a0, t2\n    ret\n",
            "f:\n    slt t1, a1, a2\n.L1:\n    xori t2, t1, 1\n    neg a0, t2\n    ret\n",
            "f:\n    slt t1, a1, a2\n    xori t2, t1, 1\n    neg a0, t2\n    add a0, a0, t2\n    ret\n",
            "f:\n    andi t1, a1, 3\n    xori t2, t1, 1\n    neg a0, t2\n    ret\n",
        ] {
            let (mut lines, mut kinds) = prepare(asm);
            let n = lines.len();
            assert!(!fold_boolean_negations(&mut lines, &mut kinds, n), "{asm}");
        }
    }

    /// Each 0/1 producer followed by `beqz`/`bnez` of its result becomes
    /// one compare-and-branch that agrees with the original on every pair
    /// of boundary values; a result read on either edge, another
    /// immediate and a line between keep both.
    #[test]
    fn compare_and_branch_fusion_is_exact() {
        let cases = [
            ("slt t1, a1, a2", "bge a1, a2", "blt a1, a2"),
            ("sltu t1, a1, a2", "bgeu a1, a2", "bltu a1, a2"),
            ("slti t1, a1, 0", "bgez a1", "bltz a1"),
            ("sltiu t1, a1, 1", "bnez a1", "beqz a1"),
            ("seqz t1, a1", "bnez a1", "beqz a1"),
            ("snez t1, a1", "beqz a1", "bnez a1"),
        ];
        for (producer, if_zero, if_set) in cases {
            for (branch, want) in [("beqz", if_zero), ("bnez", if_set)] {
                let asm = format!(
                    "f:\n    {producer}\n    {branch} t1, .L1\n    li a0, 7\n    ret\n.L1:\n    li a0, -3\n    ret\n"
                );
                let (lines0, kinds0) = prepare(&asm);
                let (mut lines, mut kinds) = prepare(&asm);
                let n = lines.len();
                assert!(fuse_compare_branches(&mut lines, &mut kinds, n), "{asm}");
                assert_eq!(kinds[1], LineKind::Nop);
                assert_eq!(lines[2].trim(), format!("{want}, .L1"), "{asm}");
                for &x in &VALUES {
                    for &y in &VALUES {
                        assert_eq!(run(&lines0, &kinds0, 0, x, y), run(&lines, &kinds, 0, x, y));
                    }
                }
            }
        }
        for asm in [
            // The result is read on the taken edge, or on the fall-through.
            "f:\n    slt t1, a1, a2\n    beqz t1, .L1\n    li a0, 7\n    ret\n.L1:\n    mv a0, t1\n    ret\n",
            "f:\n    slt t1, a1, a2\n    beqz t1, .L1\n    mv a0, t1\n    ret\n.L1:\n    li a0, -3\n    ret\n",
            // Not a sign test, not a zero test.
            "f:\n    slti t1, a1, 5\n    beqz t1, .L1\n    li a0, 7\n    ret\n.L1:\n    li a0, -3\n    ret\n",
            "f:\n    sltiu t1, a1, 2\n    beqz t1, .L1\n    li a0, 7\n    ret\n.L1:\n    li a0, -3\n    ret\n",
            // Not adjacent.
            "f:\n    slt t1, a1, a2\n    li a0, 1\n    beqz t1, .L1\n    li a0, 7\n    ret\n.L1:\n    ret\n",
        ] {
            let (mut lines, mut kinds) = prepare(asm);
            let n = lines.len();
            assert!(!fuse_compare_branches(&mut lines, &mut kinds, n), "{asm}");
        }
    }

    const SELECT: &str = "f:\n    li t2, 20\n    beqz a0, .Lsel_skip_1\n    li t2, 10\n.Lsel_skip_1:\n    mv a0, t2\n    ret\n";

    /// The select's exit tail moves into its fall-through arm and its
    /// default into the taken path: the layout Clang 23.1.5 emits, the
    /// same result for every condition, and an opaque line's index
    /// remapped past the inserted lines.
    #[test]
    fn exit_tails_are_duplicated_into_the_arm() {
        let asm = format!("{SELECT}g:\n    # riscv_volatile_opaque\n    ret\n");
        let (lines0, kinds0) = prepare(&asm);
        let (mut lines, mut kinds) = prepare(&asm);
        let opaque = lines
            .iter()
            .position(|l| l.trim() == OPAQUE_LINE)
            .expect("opaque line");
        let mut protected = vec![(opaque, "    nop".to_string())];
        assert!(duplicate_exit_tails(&mut lines, &mut kinds, &mut protected));
        let body: Vec<String> = kept(&lines, &kinds);
        assert_eq!(
            body[..9],
            [
                "f:",
                "beqz a0, .Lsel_skip_1",
                "li t2, 10",
                "mv a0, t2",
                "ret",
                ".Lsel_skip_1:",
                "li t2, 20",
                "mv a0, t2",
                "ret",
            ]
        );
        assert_eq!(lines[protected[0].0].trim(), OPAQUE_LINE);
        for c in [0, 1, -7, 1 << 40] {
            assert_eq!(run(&lines0, &kinds0, c, 0, 0), run(&lines, &kinds, c, 0, 0));
        }
    }

    /// Every way the label could be reached without the moved default, or
    /// the default could matter elsewhere, keeps the code as it was.
    #[test]
    fn exit_tail_duplication_refusals() {
        let refused = [
            // A second branch, and a jump-table word, reach the label.
            "f:\n    bnez a1, .Lsel_skip_1\n    li t2, 20\n    beqz a0, .Lsel_skip_1\n    li t2, 10\n.Lsel_skip_1:\n    mv a0, t2\n    ret\n",
            "f:\n    li t2, 20\n    beqz a0, .Lsel_skip_1\n    li t2, 10\n.Lsel_skip_1:\n    mv a0, t2\n    ret\n    .word .Lsel_skip_1 - .Ljt\n",
            // The branch reads the default's register.
            "f:\n    li a0, 20\n    beqz a0, .Lsel_skip_1\n    li a0, 10\n.Lsel_skip_1:\n    ret\n",
            // The arm reads it, or leaves it alone.
            "f:\n    li t2, 20\n    beqz a0, .Lsel_skip_1\n    addi t2, t2, 1\n.Lsel_skip_1:\n    mv a0, t2\n    ret\n",
            "f:\n    li t2, 20\n    beqz a0, .Lsel_skip_1\n    li t3, 10\n.Lsel_skip_1:\n    mv a0, t2\n    ret\n",
            // A tail too long, with a directive, or not ending in an exit.
            "f:\n    li t2, 20\n    beqz a0, .L2\n    li t2, 10\n.L2:\n    mv a0, t2\n    addi a0, a0, 1\n    addi a0, a0, 1\n    addi a0, a0, 1\n    ret\n",
            "f:\n    li t2, 20\n    beqz a0, .L2\n    li t2, 10\n.L2:\n    mv a0, t2\n    .cfi_def_cfa_offset 0\n    ret\n",
            "f:\n    li t2, 20\n    beqz a0, .L2\n    li t2, 10\n.L2:\n    mv a0, t2\n.L3:\n    ret\n",
            // A store as the default, and a function with an opaque body.
            "f:\n    sd t2, 0(a1)\n    beqz a0, .L2\n    li t2, 10\n.L2:\n    mv a0, t2\n    ret\n",
            "f:\n    li t2, 20\n    beqz a0, .L2\n    li t2, 10\n.L2:\n    # riscv_volatile_opaque\n    mv a0, t2\n    ret\n",
        ];
        for asm in refused {
            let (mut lines, mut kinds) = prepare(asm);
            let before = lines.clone();
            assert!(
                !duplicate_exit_tails(&mut lines, &mut kinds, &mut []),
                "{asm}"
            );
            assert_eq!(lines, before);
        }
    }

    /// Two 32-bit zero extensions of sign-extended words feeding an
    /// unsigned or equality compare: the compare reads the words and the
    /// four shifts go, with the same answer for every pair of boundary
    /// words.  A signed compare, an unknown source width, a single
    /// extension, a result read again and a difference used as a number
    /// keep the shifts.
    #[test]
    fn paired_zero_extensions_leave_unsigned_compares() {
        const EXT: &str = "f:\n    # lccc-ret-regs a0\n    # lccc-arg-ext a0 a1\n    slli a0, a0, 32\n    srli a0, a0, 32\n    slli a1, a1, 32\n    srli a1, a1, 32\n";
        let words: Vec<i64> = [
            0u32,
            1,
            2,
            12345,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_fffe,
            0xffff_ffff,
        ]
        .iter()
        .map(|&w| i64::from(w as i32))
        .collect();
        let consumers = [
            ("    sltu a0, a0, a1\n    ret\n", "sltu a0, a0, a1"),
            (
                "    bltu a0, a1, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n",
                "bltu a0, a1, .L1",
            ),
            (
                "    bgeu a1, a0, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n",
                "bgeu a1, a0, .L1",
            ),
            (
                "    beq a0, a1, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n",
                "beq a0, a1, .L1",
            ),
            (
                "    bne a0, a1, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n",
                "bne a0, a1, .L1",
            ),
            (
                "    sub t0, a0, a1\n    seqz a0, t0\n    ret\n",
                "sub t0, a0, a1",
            ),
            (
                "    xor t0, a0, a1\n    snez a0, t0\n    ret\n",
                "xor t0, a0, a1",
            ),
            (
                "    xor t0, a1, a0\n    bnez t0, .L1\n    li a0, 1\n    ret\n.L1:\n    li a0, 2\n    ret\n",
                "xor t0, a1, a0",
            ),
        ];
        for (consumer, want) in consumers {
            let asm = format!("{EXT}{consumer}");
            let (lines0, kinds0) = prepare(&asm);
            let (mut lines, mut kinds) = prepare(&asm);
            let n = lines.len();
            assert!(
                super::super::eliminate_redundant_sign_extensions(&mut lines, &mut kinds, n),
                "{asm}"
            );
            let body = kept(&lines, &kinds);
            assert_eq!(body[1], want, "{asm}");
            assert!(
                !body
                    .iter()
                    .any(|l| l.starts_with("slli") || l.starts_with("srli")),
                "{asm}"
            );
            for &x in &words {
                for &y in &words {
                    assert_eq!(
                        run(&lines0, &kinds0, x, y, 0),
                        run(&lines, &kinds, x, y, 0),
                        "{asm} ({x}, {y})"
                    );
                }
            }
        }
        for asm in [
            format!("{EXT}    slt a0, a0, a1\n    ret\n"),
            "f:\n    # lccc-ret-regs a0\n    slli a0, a0, 32\n    srli a0, a0, 32\n    slli a1, a1, 32\n    srli a1, a1, 32\n    sltu a0, a0, a1\n    ret\n".to_string(),
            "f:\n    # lccc-ret-regs a0\n    # lccc-arg-ext a0 a1\n    slli a0, a0, 32\n    srli a0, a0, 32\n    sltu a0, a0, a1\n    ret\n".to_string(),
            format!("{EXT}    sltu t0, a0, a1\n    add a0, a0, t0\n    ret\n"),
            format!("{EXT}    sub t0, a0, a1\n    add a0, t0, t0\n    ret\n"),
        ] {
            let (mut lines, mut kinds) = prepare(&asm);
            let n = lines.len();
            super::super::eliminate_redundant_sign_extensions(&mut lines, &mut kinds, n);
            let shifts = kept(&lines, &kinds).iter().filter(|l| l.starts_with("slli")).count();
            assert!(shifts >= 1, "{asm}");
        }
    }

    /// A jump to a short exit tail takes its own copy (the label stays for
    /// its other references), with the same result on every path; a tail
    /// longer than `JUMP_TAIL_MAX` keeps its jump.
    #[test]
    fn jumps_to_short_exit_tails_take_a_copy() {
        let asm = "f:\n    li t1, 7\n    beq a0, t1, .L1\n    j .L2\n.L1:\n    li a0, 3\n    ret\n.L2:\n    li a0, 0\n    ret\n";
        let (lines0, kinds0) = prepare(asm);
        let (mut lines, mut kinds) = prepare(asm);
        assert!(duplicate_exit_tails(&mut lines, &mut kinds, &mut []));
        let body = kept(&lines, &kinds);
        assert_eq!(
            body,
            [
                "f:",
                "li t1, 7",
                "beq a0, t1, .L1",
                "li a0, 0",
                "ret",
                ".L1:",
                "li a0, 3",
                "ret",
                ".L2:",
                "li a0, 0",
                "ret"
            ]
        );
        for c in [0, 7, -1] {
            assert_eq!(run(&lines0, &kinds0, c, 0, 0), run(&lines, &kinds, c, 0, 0));
        }
        let long = "f:\n    j .L2\n.L2:\n    addi a0, a0, 1\n    addi a0, a0, 1\n    addi a0, a0, 1\n    ret\n";
        let (mut lines, mut kinds) = prepare(long);
        assert!(!duplicate_exit_tails(&mut lines, &mut kinds, &mut []));
    }
}

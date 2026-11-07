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
    LineKind, OPAQUE_LINE, REG_NONE, RvLazyLiveness, branch_parts, classify_line, label_name,
    named_gp_dest, parse_asm_i64, parse_reg, rv_bit, rv_function_ranges, rv_is_tail_line,
    rv_next_kept_line, rv_uses_defs,
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
    // The tail: plain lines, then the exit.
    let mut kept = 0;
    let mut exit = None;
    for k in l + 1..fend {
        if kinds[k] == LineKind::Nop {
            continue;
        }
        kept += 1;
        if kept > TAIL_MAX {
            return None;
        }
        let exits = kinds[k] == LineKind::Ret
            || (kinds[k] == LineKind::Call && rv_is_tail_line(lines[k].trim()));
        if exits {
            exit = Some(k);
            break;
        }
        if !plain_line(&lines[k], kinds[k]) {
            return None;
        }
    }
    let exit = exit?;
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
/// duplicated); the arm at most `ARM_MAX` plain lines.  Functions with an
/// opaque body are skipped.  Lines are inserted, so the vectors are
/// rebuilt; `protected` (indices of hidden opaque lines) is remapped.
pub(super) fn duplicate_exit_tails(
    lines: &mut Vec<String>,
    kinds: &mut Vec<LineKind>,
    protected: &mut [(usize, String)],
) -> bool {
    let n = lines.len();
    let mut sites: Vec<TailSite> = Vec::new();
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
    }
    if sites.is_empty() {
        return false;
    }
    let extra: usize = sites.iter().map(|s| s.exit - s.label + 1).sum();
    let mut new_lines: Vec<String> = Vec::with_capacity(n + extra);
    let mut new_kinds: Vec<LineKind> = Vec::with_capacity(n + extra);
    let mut remap: Vec<usize> = Vec::with_capacity(n);
    let mut next = sites.iter().peekable();
    let mut is_def = vec![false; n];
    for site in &sites {
        is_def[site.def] = true;
    }
    for i in 0..n {
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

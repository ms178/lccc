//! Index-only LEA plus an add, fused into one SIB address.
//!
//! Instruction selection lowers a scaled term `x << k` to an index-only LEA
//! and the addition around it stays a separate ALU op:
//!
//! ```text
//!     leal 0(,%rdi,4), %eax   ; 4*x
//!     addl %esi, %eax         ; + b
//! ```
//!
//! x86 can carry both in one SIB — the addend becomes the base register:
//!
//! ```text
//!     leal (%rsi,%rdi,4), %eax
//! ```
//!
//! One instruction shorter and one flags write lighter than the pair.  The
//! immediate form fuses the same way, into the LEA's displacement slot.
//!
//! # When it is legal
//!
//! 1. **Exact adjacency**: the index-only `lea{l,q} 0(,%X, S), %T` is
//!    followed by `add{l,q} <src>, %T` at the same width, with only blank
//!    and debug-attribution lines between them, both pinned-free and
//!    comment-free.
//! 2. **Register addends avoid aliasing**: the addend must differ from the
//!    LEA destination (the fused base is read before the write) and from
//!    the index (a SIB cannot name one register twice).
//! 3. **The flags the add wrote are not observed**: the fusion deletes the
//!    add's flag write, so any flag reader before the next flag writer
//!    declines — labels do not release the scan, because fall-through still
//!    carries these flags.
//!
//! `CCC_PEEPHOLE_SKIP=lea_add_fuse` disables the pass.

use super::super::types::*;
use super::identical_blocks::reads_flags;
use super::pow2_cmov::writes_flags_text;
use super::relay_and_lea::{is_relayable_family, plain_gp_operand, split_two_operands};

/// Fuse every eligible index-only LEA + add pair.  Returns whether the text
/// changed.
pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut i = 0;
    while i < infos.len() {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }
        if let Some((add_i, new_lea)) = fuse_pair(store, infos, i) {
            if replace(store, infos, i, new_lea) && nop(store, infos, add_i) {
                changed = true;
            }
            i = add_i + 1;
            continue;
        }
        i += 1;
    }
    changed
}

/// Detect the pair anchored at `lea_i`; returns the add's line index and the
/// fused LEA text.
fn fuse_pair(
    store: &LineStore,
    infos: &[LineInfo],
    lea_i: usize,
) -> Option<(usize, String)> {
    if infos[lea_i].pinned || infos[lea_i].volatile_access {
        return None;
    }
    let raw = store.get(lea_i);
    if raw.contains('#') {
        return None;
    }
    let lea = infos[lea_i].trimmed(raw);
    let (width, index, scale, t_spelling) = index_only_lea(lea)?;

    // Next real instruction: blank/debug lines may interleave, nothing else.
    let mut add_i = lea_i + 1;
    while add_i < infos.len() && infos[add_i].is_steppable() {
        add_i += 1;
    }
    if add_i >= infos.len() {
        return None;
    }
    if infos[add_i].pinned || infos[add_i].volatile_access {
        return None;
    }
    let raw_add = store.get(add_i);
    if raw_add.contains('#') {
        return None;
    }
    let add = infos[add_i].trimmed(raw_add);
    let (add_mnem, add_rest) = add.split_once(char::is_whitespace)?;
    if add_mnem != format!("add{width}") {
        return None;
    }
    let (src, dst) = split_two_operands(add_rest)?;
    if dst.trim() != t_spelling {
        return None; // not the LEA's destination
    }

    let t_fam = plain_gp_operand(t_spelling)?;
    let x_fam = plain_gp_operand(index)?;
    if !is_relayable_family(t_fam) || !is_relayable_family(x_fam) {
        return None;
    }

    // Shape the fused address: reg addend → base, immediate → displacement.
    // SIB register fields are 64-bit spelled with their `%` — an address
    // operand may not name a 32-bit register, and the family oracle reads
    // operands by their full spelling.
    let index64 = super::super::types::REG_NAMES[0][x_fam as usize];
    let address = if let Some(imm) = src.strip_prefix('$') {
        let v: i64 = imm.parse().ok()?;
        i32::try_from(v).ok()?;
        format!("{v}(,{index64},{scale})")
    } else {
        let y_fam = plain_gp_operand(src)?;
        if !is_relayable_family(y_fam) {
            return None;
        }
        if y_fam == t_fam {
            return None; // the fused base is read before the LEA writes %T
        }
        if y_fam == x_fam {
            return None; // one SIB cannot name the same register twice
        }
        let base64 = super::super::types::REG_NAMES[0][y_fam as usize];
        format!("({base64}, {index64}, {scale})")
    };

    // The add's flags must not be observed before the next flag writer;
    // labels do not end the scan, fall-through carries them.
    let mut j = add_i + 1;
    while j < infos.len() {
        if infos[j].is_nop() {
            j += 1;
            continue;
        }
        let text = infos[j].trimmed(store.get(j));
        if reads_flags(store, infos, j) {
            return None;
        }
        if writes_flags_text(text) {
            break;
        }
        j += 1;
    }

    if !all_editable(infos, &[lea_i, add_i]) {
        return None;
    }
    Some((add_i, format!("    lea{width} {address}, {t_spelling}")))
}

/// `lea{l,q} 0(,%X, S), %T` → `(width, index, scale, dst)`.
fn index_only_lea(lea: &str) -> Option<(char, &str, u8, &str)> {
    let (mnem, rest) = lea.split_once(char::is_whitespace)?;
    let width = mnem.chars().last().filter(|c| *c == 'l' || *c == 'q')?;
    if !mnem.starts_with("lea") {
        return None;
    }
    let (addr, dst) = split_two_operands(rest)?;
    let addr = addr.trim();
    let inner = addr.strip_prefix("0(")?.strip_suffix(')')?;
    let mut it = inner.split(',');
    let empty = it.next()?;
    let index = it.next()?;
    let scale = it.next()?;
    if !empty.trim().is_empty() || it.next().is_some() {
        return None; // has a base register or extra fields: not index-only
    }
    let scale_v: u8 = scale.trim().parse().ok()?;
    if !matches!(scale_v, 1 | 2 | 4 | 8) {
        return None;
    }
    Some((width, index.trim(), scale_v, dst.trim()))
}

fn replace(store: &mut LineStore, infos: &mut [LineInfo], idx: usize, text: String) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    replace_line(store, &mut infos[idx], idx, text);
    true
}

fn nop(store: &mut LineStore, infos: &mut [LineInfo], idx: usize) -> bool {
    let _ = store;
    mark_nop(&mut infos[idx]);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_fuse(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frame(body: &str) -> String {
        format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n")
    }

    #[test]
    fn a_register_addend_becomes_the_base() {
        let out = run_fuse(&frame(concat!(
            "    leal 0(,%rdi,4), %eax\n",
            "    addl %esi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("leal (%rsi, %rdi, 4), %eax"), "{out}");
        assert!(!out.contains("addl"), "{out}");
    }

    #[test]
    fn an_immediate_addend_becomes_the_displacement() {
        let out = run_fuse(&frame(concat!(
            "    leaq 0(,%rdi,8), %r9\n",
            "    addq $24, %r9\n",
            "    ret\n",
        )));
        assert!(out.contains("leaq 24(,%rdi,8), %r9"), "{out}");
        assert!(!out.contains("addq"), "{out}");
    }

    #[test]
    fn a_register_equal_to_the_destination_is_refused() {
        // The fused base is read before the LEA writes %T; if the addend WERE
        // %T the read would see the pre-LEA value, not the scaled index.
        let out = run_fuse(&frame(concat!(
            "    leal 0(,%rdi,4), %eax\n",
            "    addl %eax, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("leal 0(,%rdi,4), %eax"), "{out}");
        assert!(out.contains("addl %eax, %eax"), "{out}");
    }

    #[test]
    fn a_register_equal_to_the_index_is_refused() {
        let out = run_fuse(&frame(concat!(
            "    leal 0(,%rdi,4), %eax\n",
            "    addl %edi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("addl"), "{out}");
    }

    #[test]
    fn a_forward_flag_reader_is_refused() {
        let out = run_fuse(&frame(concat!(
            "    leal 0(,%rdi,4), %eax\n",
            "    addl %esi, %eax\n",
            "    setl %cl\n",
            "    ret\n",
        )));
        assert!(out.contains("addl %esi, %eax"), "{out}");
        assert!(!out.contains("leal (%rsi"), "{out}");
    }

    #[test]
    fn anything_but_immediate_adjacency_declines() {
        let out = run_fuse(&frame(concat!(
            "    leal 0(,%rdi,4), %eax\n",
            "    movl %ecx, %edx\n",
            "    addl %esi, %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("leal 0(,%rdi,4), %eax"), "{out}");
        assert!(out.contains("addl %esi, %eax"), "{out}");
    }
}

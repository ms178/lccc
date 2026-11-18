//! Narrow reload of a just-stored vector register, forwarded as a register
//! lane extraction.
//!
//! # The shape (RH-09i)
//!
//! A vector value is homed into a frame slot and a single lane is read
//! straight back — the `float g(__m256 a) { return a[3]; }` family, and
//! every constant-index lane read of a vector parameter:
//!
//! ```text
//!     vmovdqu %ymm0, 8(%rsp)      # home the parameter
//!     vmovss 20(%rsp), %xmm2      # read lane 3 back
//! ```
//!
//! The register still holds every bit of the slot, so the reload is a lane
//! extraction that never needs to touch memory — and the memory round trip
//! it replaces carries a store-to-load-forwarding penalty on the exact
//! shape (narrow load from a wide store) that both vendors' forwarding
//! predictors handle worst.  GCC 16.2 emits a single shuffle for the same
//! read.
//!
//! The rewrite picks the UNIQUE instruction with bit-exact load semantics:
//!
//! | reload                 | lane | replacement                                  |
//! |------------------------|------|----------------------------------------------|
//! | `(v)movss` (4-byte)    | 0–3  | `(v)insertps $((l<<6)\|0xE)` (zmask clears 1–3) |
//! | `(v)movsd`/`movq`→XMM  | 0    | `(v)movq` src → dst (zeroes 127:64)          |
//! | `(v)movsd`/`movq`→XMM  | 1    | `vpsrldq $8` src → dst (shifts in zeros)     |
//! | `movl` → GP            | 0–3  | `(v)extractps $l` src → r32                  |
//! | `movq` → GP            | 0–1  | `(v)pextrq $l` src → r64                     |
//!
//! Every rewrite is ONE instruction — the line stores the peephole walks
//! are strictly line-parallel, so a replacement may never change the line
//! count.  Reads of a YMM store's upper half and the legacy forms with no
//! 3-operand encoding simply decline.
//!
//! Every XMM-destination replacement reproduces the scalar load's zeroing
//! exactly: the memory forms of `movss`/`movsd` zero the rest of the
//! destination's 128 bits, VEX encodings zero 255:128 as well, and
//! `vinsertps`' zmask, `vmovq` and `vextractf128` do the same.  Legacy-SSE
//! reloads are answered with legacy `insertps`/`movq`/`extractps`/`pextrq`
//! (same 128-bit semantics, bits 255:128 untouched on both sides); a legacy
//! reload of a YMM store's upper half cannot occur (legacy stores spill 16
//! bytes).  SSE4.1 is required for `insertps`/`extractps`/`pextrq` and is
//! checked against the active ISA; without it the reload stays.
//!
//! # Soundness
//!
//! The pending store survives only across inert attribution lines
//! ([`LineInfo::is_steppable`]), exactly like the over-aligned forwarding
//! pass next door: EVERY other line — another store, a call, a label, any
//! write to the stored register, anything unrecognised — clears it.  The
//! reload must sit fully inside the stored bytes at a lane-aligned offset.
//! Pinned and volatile lines (inline asm, volatile objects) never match.
//!
//! `CCC_PEEPHOLE_SKIP=vector_lane_reload` disables the pass.

use super::super::types::*;

/// One tracked full-width vector store: `mnemonic %reg, D(%rsp)`.
struct PendingStore<'a> {
    reg: &'a str,  // "%xmm0" / "%ymm0"
    base: &'a str, // "(%rsp)" or "(%rbp)" spelling, with the leading '('
    disp: i64,     // slot displacement
    width: i64,    // stored bytes: 16 or 32
    vex: bool,     // VEX-encoded store (v-prefixed)
}

/// `sse41`/`avx` are parameters rather than reads of the process-global ISA
/// mirror, same law as `fuse_load_bswap` next door: the caller owns the
/// TU's options, and unit tests exercise both arms without racing through
/// `set_current`.
pub(super) fn run(store: &mut LineStore, infos: &mut [LineInfo], sse41: bool, avx: bool) -> bool {
    if !sse41 {
        return false; // every replacement needs insertps/extractps/pextrq
    }
    let mut changed = false;
    let mut i = 0;
    while i < infos.len() {
        if infos[i].is_nop() || infos[i].pinned || infos[i].volatile_access {
            i += 1;
            continue;
        }
        let raw = store.get(i);
        if raw.contains('#') {
            i += 1;
            continue;
        }
        // Byte screen before the per-line allocation: only `(v)mov…` lines
        // can open a window, and the owned copy exists solely because the
        // borrow of `store` must end before `replace_line` mutates it.
        let trimmed = infos[i].trimmed(raw);
        if !looks_like_mov(trimmed) {
            i += 1;
            continue;
        }
        let line = trimmed.to_string();
        let Some(pending) = parse_vector_store(&line) else {
            i += 1;
            continue;
        };

        // Walk the window the store survives: steppable lines only.
        let mut j = i + 1;
        while j < infos.len() {
            if infos[j].is_nop() || infos[j].is_steppable() {
                j += 1;
                continue;
            }
            if infos[j].pinned || infos[j].volatile_access {
                break;
            }
            let raw_j = store.get(j);
            if raw_j.contains('#') {
                break;
            }
            let reload_ref = infos[j].trimmed(raw_j);
            if !looks_like_mov(reload_ref) {
                break;
            }
            let reload = reload_ref.to_string();
            let Some((text, clobbers_source)) = rewrite_reload(&pending, &reload, avx) else {
                break; // not a matching reload: the blunt clear
            };
            replace_line(store, &mut infos[j], j, text);
            changed = true;
            if clobbers_source {
                break; // the rewrite overwrote the stored register itself
            }
            // The register and the slot still agree; keep scanning for
            // further reloads of the same store.
            j += 1;
        }
        i += 1;
    }
    changed
}

/// Parse `(v)mov{aps,ups,dqa,dqu} %xmmN|%ymmN, D(%rsp|%rbp)`.
fn parse_vector_store(line: &str) -> Option<PendingStore<'_>> {
    let (mnem, rest) = line.split_once(char::is_whitespace)?;
    let vex = mnem.starts_with('v');
    let body = if vex { &mnem[1..] } else { mnem };
    if !matches!(body, "movaps" | "movups" | "movdqa" | "movdqu") {
        return None;
    }
    let (src, dst) = split_operands(rest)?;
    let (reg, width) = if src.starts_with("%ymm") {
        (src, 32)
    } else if src.starts_with("%xmm") {
        (src, 16)
    } else {
        return None;
    };
    if width == 32 && !vex {
        return None; // malformed: a legacy mnemonic cannot store %ymm
    }
    let (disp, base) = parse_frame_slot(dst)?;
    Some(PendingStore {
        reg,
        base,
        disp,
        width,
        vex,
    })
}

/// Parse the reload against the pending store; return the replacement line
/// and whether it overwrites the stored register (ending the forwarding
/// window).
fn rewrite_reload(p: &PendingStore<'_>, line: &str, avx: bool) -> Option<(String, bool)> {
    let (mnem, rest) = line.split_once(char::is_whitespace)?;
    let (src, dst) = split_operands(rest)?;
    let (disp, base) = parse_frame_slot(src)?;
    if base != p.base {
        return None;
    }
    let delta = disp.checked_sub(p.disp)?;

    // Encoding-class discipline, same law as the full-width forwarder next
    // door: a VEX reload answers with VEX (zeroing 255:128), a legacy
    // reload with legacy (preserving them).  VEX replacements additionally
    // require AVX to be on — a VEX reload line proves it, but check anyway.
    let vex = mnem.starts_with('v');
    if vex && !avx {
        return None;
    }
    let v = if vex { "v" } else { "" };
    let body = if vex { &mnem[1..] } else { mnem };
    // The stored register spelled as an XMM, for 128-bit replacements.
    let xmm_src = format!("%xmm{}", &p.reg[4..]);
    let clobbers_source = dst == xmm_src || dst == p.reg;
    // A reload whose destination is the SLOT'S BASE REGISTER redefines what
    // every later `D(base)` means: forwarding this line is still correct
    // (same value, same write), but the scan must not continue past it —
    // and the shape is pathological enough (a lane extract into %rsp/%rbp)
    // that refusing outright is both simpler and safer.  Every width
    // spelling of the two frame bases aliases the same family.
    if matches!(
        dst,
        "%rsp" | "%esp" | "%sp" | "%spl" | "%rbp" | "%ebp" | "%bp" | "%bpl"
    ) {
        return None;
    }

    let text = match body {
        "movss" if dst.starts_with("%xmm") => {
            if delta < 0 || delta % 4 != 0 || delta > p.width.min(16) - 4 {
                return None; // outside the low 128 bits: no 1-insn form
            }
            let lane = delta / 4;
            let imm = (lane << 6) | 0b1110;
            if vex {
                Some(format!("    vinsertps ${imm}, {xmm_src}, {xmm_src}, {dst}"))
            } else {
                // Legacy `insertps $imm, %src, %dst` writes every lane of
                // the destination (lane 0 from the source, 1–3 zeroed by
                // the zmask), so the 2-operand form is a full overwrite.
                Some(format!("    insertps ${imm}, {xmm_src}, {dst}"))
            }
        }
        "movsd" | "movq" if dst.starts_with("%xmm") => {
            if delta < 0 || delta % 8 != 0 || delta > p.width.min(16) - 8 {
                return None;
            }
            match delta / 8 {
                0 => Some(format!("    {v}movq {xmm_src}, {dst}")),
                // `(v)psrldq $8` shifts the high qword to lane 0 and fills
                // with zeros: bit-exact scalar-load semantics in one
                // instruction.  The legacy form is 2-operand
                // (destination-in-place), so it needs dst == src.
                1 if vex => Some(format!("    vpsrldq $8, {xmm_src}, {dst}")),
                1 if dst == xmm_src => Some(format!("    psrldq $8, {dst}")),
                _ => None,
            }
        }
        // A GP-destination `mov` carries no encoding class of its own, so
        // the STORE's class decides: a VEX store means the upper YMM state
        // may be dirty, and the extract must be VEX to avoid the legacy-SSE
        // transition stall; a legacy store answers with legacy SSE4.1.
        "movl" if is_gp_operand(dst) => {
            if delta < 0 || delta % 4 != 0 || delta > p.width.min(16) - 4 {
                return None; // GP forms have no scratch for the upper half
            }
            let gv = if p.vex { "v" } else { "" };
            let lane = delta / 4;
            Some(format!("    {gv}extractps ${lane}, {xmm_src}, {dst}"))
        }
        "movq" if is_gp_operand(dst) => {
            if delta < 0 || delta % 8 != 0 || delta > p.width.min(16) - 8 {
                return None;
            }
            let gv = if p.vex { "v" } else { "" };
            let lane = delta / 8;
            Some(format!("    {gv}pextrq ${lane}, {xmm_src}, {dst}"))
        }
        _ => None,
    }?;
    Some((text, clobbers_source))
}

/// `mov…` or `vmov…` at the start of the trimmed line — the only shapes
/// either parser can accept, screened before any allocation.
fn looks_like_mov(t: &str) -> bool {
    let b = t.as_bytes();
    b.starts_with(b"mov") || b.starts_with(b"vmov")
}

/// A plain general-purpose register operand: `%r…`/`%e…` spelling with no
/// memory reference and no vector/mask register family.
fn is_gp_operand(op: &str) -> bool {
    op.starts_with('%')
        && !op.contains('(')
        && !op.starts_with("%xmm")
        && !op.starts_with("%ymm")
        && !op.starts_with("%zmm")
        && !op.starts_with("%mm")
        && !op.starts_with("%k")
}

/// `D(%rsp)` / `D(%rbp)` → `(D, base spelling)`.  Bare base only.
fn parse_frame_slot(op: &str) -> Option<(i64, &str)> {
    let open = op.find('(')?;
    let base = &op[open..];
    if base != "(%rsp)" && base != "(%rbp)" {
        return None;
    }
    let disp_txt = &op[..open];
    if disp_txt.is_empty() {
        return Some((0, base));
    }
    disp_txt.parse::<i64>().ok().map(|d| (d, base))
}

/// Split `a, b` at the top-level comma (no nested parens in these shapes).
fn split_operands(rest: &str) -> Option<(&str, &str)> {
    let (a, b) = rest.split_once(',')?;
    Some((a.trim_ascii(), b.trim_ascii()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_pass(asm: &str) -> String {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        run(&mut store, &mut infos, true, true);
        (0..store.len())
            .filter(|&i| !infos[i].is_nop())
            .map(|i| store.get(i).trim_ascii().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn frame(body: &str) -> String {
        format!("f:\n.cfi_startproc\n{body}.cfi_endproc\n")
    }

    #[test]
    fn the_motivating_lane3_float_read_forwards() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %ymm0, 8(%rsp)\n",
            "    vmovss 20(%rsp), %xmm2\n",
            "    ret\n",
        )));
        assert!(out.contains("vinsertps $206, %xmm0, %xmm0, %xmm2"), "{out}");
        assert!(!out.contains("vmovss"), "{out}");
    }

    #[test]
    fn lane0_float_uses_insertps_with_zmask() {
        let out = run_pass(&frame(concat!(
            "    vmovaps %xmm1, (%rsp)\n",
            "    vmovss (%rsp), %xmm0\n",
            "    ret\n",
        )));
        assert!(out.contains("vinsertps $14, %xmm1, %xmm1, %xmm0"), "{out}");
    }

    #[test]
    fn an_upper_half_float_lane_declines() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %ymm0, (%rsp)\n",
            "    vmovss 24(%rsp), %xmm1\n",
            "    ret\n",
        )));
        assert!(out.contains("vmovss 24(%rsp), %xmm1"), "{out}");
    }

    #[test]
    fn a_double_lane0_is_a_zeroing_movq() {
        let out = run_pass(&frame(concat!(
            "    vmovups %xmm3, 16(%rsp)\n",
            "    vmovsd 16(%rsp), %xmm0\n",
            "    ret\n",
        )));
        assert!(out.contains("vmovq %xmm3, %xmm0"), "{out}");
    }

    #[test]
    fn a_double_lane1_is_a_zero_filling_byte_shift() {
        let out = run_pass(&frame(concat!(
            "    vmovups %xmm3, 16(%rsp)\n",
            "    vmovsd 24(%rsp), %xmm0\n",
            "    ret\n",
        )));
        assert!(out.contains("vpsrldq $8, %xmm3, %xmm0"), "{out}");
    }

    #[test]
    fn a_reload_into_the_slot_base_register_declines() {
        // Writing the base register redefines every later `D(%rsp)`; the
        // pass must refuse the line entirely (which also ends the window
        // before any later reload can be mis-forwarded).
        let out = run_pass(&frame(concat!(
            "    vmovdqu %xmm2, (%rsp)\n",
            "    movq 8(%rsp), %rbp\n",
            "    ret\n",
        )));
        assert!(out.contains("movq 8(%rsp), %rbp"), "{out}");
    }

    #[test]
    fn a_reload_into_the_stored_register_ends_the_window() {
        let out = run_pass(&frame(concat!(
            "    vmovaps %xmm1, (%rsp)\n",
            "    vmovss 4(%rsp), %xmm1\n",
            "    vmovss 8(%rsp), %xmm2\n",
            "    ret\n",
        )));
        // First reload forwards, then %xmm1 no longer mirrors the slot.
        assert!(out.contains("vinsertps $78, %xmm1, %xmm1, %xmm1"), "{out}");
        assert!(out.contains("vmovss 8(%rsp), %xmm2"), "{out}");
    }

    #[test]
    fn a_dword_gp_read_becomes_extractps() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %xmm2, (%rsp)\n",
            "    movl 12(%rsp), %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("vextractps $3, %xmm2, %eax"), "{out}");
    }

    #[test]
    fn a_qword_gp_read_becomes_pextrq() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %xmm2, (%rsp)\n",
            "    movq 8(%rsp), %rdx\n",
            "    ret\n",
        )));
        assert!(out.contains("vpextrq $1, %xmm2, %rdx"), "{out}");
    }

    #[test]
    fn an_intervening_instruction_clears_the_store() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %ymm0, 8(%rsp)\n",
            "    movq %rax, %rcx\n",
            "    vmovss 20(%rsp), %xmm2\n",
            "    ret\n",
        )));
        assert!(out.contains("vmovss 20(%rsp), %xmm2"), "{out}");
    }

    #[test]
    fn a_misaligned_lane_offset_declines() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %ymm0, 8(%rsp)\n",
            "    vmovss 18(%rsp), %xmm2\n",
            "    ret\n",
        )));
        assert!(out.contains("vmovss 18(%rsp), %xmm2"), "{out}");
    }

    #[test]
    fn a_read_past_the_store_declines() {
        let out = run_pass(&frame(concat!(
            "    vmovaps %xmm1, (%rsp)\n",
            "    vmovss 16(%rsp), %xmm0\n",
            "    ret\n",
        )));
        assert!(out.contains("vmovss 16(%rsp), %xmm0"), "{out}");
    }

    #[test]
    fn a_different_slot_base_declines() {
        let out = run_pass(&frame(concat!(
            "    vmovaps %xmm1, (%rsp)\n",
            "    vmovss (%rbp), %xmm0\n",
            "    ret\n",
        )));
        assert!(out.contains("vmovss (%rbp), %xmm0"), "{out}");
    }

    #[test]
    fn a_gp_read_of_the_upper_ymm_half_declines() {
        let out = run_pass(&frame(concat!(
            "    vmovdqu %ymm0, (%rsp)\n",
            "    movl 20(%rsp), %eax\n",
            "    ret\n",
        )));
        assert!(out.contains("movl 20(%rsp), %eax"), "{out}");
    }

    #[test]
    fn two_reloads_of_one_store_both_forward() {
        let out = run_pass(&frame(concat!(
            "    vmovaps %xmm1, (%rsp)\n",
            "    vmovss 4(%rsp), %xmm0\n",
            "    vmovss 8(%rsp), %xmm2\n",
            "    ret\n",
        )));
        assert!(out.contains("vinsertps $78, %xmm1, %xmm1, %xmm0"), "{out}");
        assert!(out.contains("vinsertps $142, %xmm1, %xmm1, %xmm2"), "{out}");
    }
}

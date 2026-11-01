//! Load + mask + flag consumer → `test $imm, mem`.
//!
//! ```text
//!     movslq G(%rip), %r11            testb $32, G(%rip)
//!     andl $32, %r11d          ->     je .LBB2
//!     je .LBB2
//! ```
//!
//! The backend materialises `if (G & 32)` as a load into a scratch register, a
//! destructive `and`, and the branch.  When the masked value is dead after the
//! `and`, only its flags matter, and `test` can read the memory operand
//! directly.  GCC 16.2 emits exactly this (`testb $32, G(%rip)`,
//! `testb $3, G+2(%rip)`, `testw $257, 1(%rdi)`); it saves the load, the
//! scratch register and one instruction, and frees the register for the
//! allocator's later users.
//!
//! # Semantics
//!
//! Let `v` be the 64-bit register value the load produces from the `W`
//! memory bytes (zero- or sign-extended to 32 or 64 bits), and `imm_A` the
//! mask at the `and`/`test` width `A`.  The flags come from `r = v & imm_A`
//! taken at width `A`:
//!
//! * **ZF** is `r == 0`.  Every bit of `v` is either a memory bit, a copy of
//!   the memory sign bit (sign extension) or zero, so `r == 0` iff
//!   `mem & m == 0` for the *effective memory mask* `m` = the mask bits that
//!   land on memory bits, plus the memory sign bit when any mask bit lands on
//!   a sign-extension bit ([`effective_memory_mask`]).  `m` may then be
//!   narrowed to the smallest 1/2/4-byte window that covers its set bytes —
//!   little-endian, so byte `k` lives at displacement `+k` — and the window
//!   never reaches past the `W` bytes the original load read, so the narrow
//!   access touches only bytes the program already touched.
//! * **SF/PF** depend on the bit positions, which narrowing moves.  The
//!   narrowed form therefore needs every consumer to read ZF only
//!   ([`walk_flag_consumers`] over every path, branch targets included).
//! * The **flag-exact** form — `test` at width `A` over the same low bytes,
//!   offset 0 — exists whenever `A <= 8·W` (every one of the `A` result bits
//!   is a memory bit, so `test{A} $imm_A, mem` computes the identical `r`):
//!   ZF/SF/PF are identical and CF/OF are cleared by both.  It is used for
//!   any consumer (`js`, `jp`, `cmovs`, …).
//! * **AF** is architecturally undefined after both `and` and `test`, so the
//!   value a whole-flags reader (`pushf`, `lahf`, inline asm) would see may
//!   differ; any such reader vetoes the fold
//!   (the same walk's `saw_whole_reader`).
//!
//! # Soundness conditions
//!
//! * The load and the mask are adjacent (only debug/bookkeeping lines may sit
//!   between), so the memory read moves by zero instructions that could write
//!   memory or redefine an address register.
//! * Neither line is pinned: C `volatile` accesses are pinned by
//!   `pin_volatile_access_regions`, so an MMIO access never changes width.
//! * The loaded register is dead after the mask — across the branch, by the
//!   whole-function [`FileLiveness`] oracle.  If the address used that
//!   register (`movl (%rax), %eax`), the register now keeps the address
//!   instead of the loaded value; nobody reads it, by the same proof.
//! * Segment-prefixed and relocation-decorated operands (`%fs:`, `@GOTPCREL`,
//!   `@tpoff`) are refused: their displacement is not a plain byte address.

use super::super::types::*;
use super::flag_peepholes::walk_flag_consumers;
use super::liveness::FileLiveness;
use super::relay_and_lea::{plain_gp_operand, split_two_operands};

/// How a load fills its destination register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadShape {
    /// Bytes read from memory.
    pub(super) bytes: u32,
    /// Sign- (true) or zero-extension of the memory value.
    pub(super) signed: bool,
    /// Bits the extension fills (32 or 64); above it the register is zero
    /// (a 32-bit destination write zero-extends to 64).
    pub(super) ext_bits: u32,
}

/// The load mnemonics this fold understands, with their register width
/// (`true` = 64-bit destination name).  Byte/word `mov` into a partial
/// register merges with the old contents and is deliberately absent.
fn load_shape(mnemonic: &str) -> Option<(LoadShape, bool)> {
    let s = |bytes, signed, ext_bits| LoadShape {
        bytes,
        signed,
        ext_bits,
    };
    Some(match mnemonic {
        "movl" => (s(4, false, 32), false),
        "movq" => (s(8, false, 64), true),
        "movzbl" => (s(1, false, 32), false),
        "movzbq" => (s(1, false, 64), true),
        "movzwl" => (s(2, false, 32), false),
        "movzwq" => (s(2, false, 64), true),
        "movsbl" => (s(1, true, 32), false),
        "movsbq" => (s(1, true, 64), true),
        "movswl" => (s(2, true, 32), false),
        "movswq" => (s(2, true, 64), true),
        "movslq" => (s(4, true, 64), true),
        _ => return None,
    })
}

/// All-ones in the low `bits` bits (`bits` in `0..=64`).
fn low_mask(bits: u32) -> u64 {
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    }
}

/// The mask an `and`/`test` immediate applies at width `bits` (8/16/32/64),
/// or `None` when the immediate is not encodable at that width.  Narrow
/// widths accept both the signed and the unsigned spelling (`$-1`, `$255`);
/// 64-bit forms carry a sign-extended imm32.
pub(super) fn mask_at_width(imm: i64, bits: u32) -> Option<u64> {
    match bits {
        8 | 16 | 32 => {
            let lo = -(1i64 << (bits - 1));
            let hi = (1i64 << bits) - 1;
            (lo..=hi)
                .contains(&imm)
                .then_some(imm as u64 & low_mask(bits))
        }
        64 => i32::try_from(imm).is_ok().then_some(imm as u64),
        _ => None,
    }
}

/// The mask over the memory bits whose zero-test equals the zero-test of
/// `extend(mem) & imm_at_width` at width `bits` (see the module docs).
pub(super) fn effective_memory_mask(shape: LoadShape, imm_mask: u64, bits: u32) -> u64 {
    let mem_bits = shape.bytes * 8;
    let in_width = imm_mask & low_mask(bits);
    let mut m = in_width & low_mask(mem_bits);
    if shape.signed {
        let ext = in_width & !low_mask(mem_bits) & low_mask(shape.ext_bits);
        if ext != 0 {
            m |= 1u64 << (mem_bits - 1);
        }
    }
    m
}

/// The smallest 1/2/4-byte window `(byte_offset, size)` covering every set
/// byte of `m`, kept inside the `bytes` the load read.  `None` for an empty
/// mask or one that needs all 8 bytes.
pub(super) fn narrow_window(m: u64, bytes: u32) -> Option<(u32, u32)> {
    if m == 0 {
        return None;
    }
    let lo = m.trailing_zeros() / 8;
    let hi = (63 - m.leading_zeros()) / 8;
    let span = hi - lo + 1;
    let size = [1u32, 2, 4].into_iter().find(|&s| s >= span)?;
    if size > bytes {
        return None;
    }
    Some((lo.min(bytes - size), size))
}

/// `mem` displaced by `off` bytes: `G(%rip)` → `G+2(%rip)`, `-8(%rbp)` →
/// `-6(%rbp)`, `(%rdi)` → `1(%rdi)`.  `None` for anything but a plain
/// `[sym][±n](...)` operand.
pub(super) fn displace(mem: &str, off: u32) -> Option<String> {
    if off == 0 {
        return Some(mem.to_string());
    }
    let open = mem.find('(')?;
    let (disp, base) = mem.split_at(open);
    let off = i64::from(off);
    if disp.is_empty() {
        return Some(format!("{off}{base}"));
    }
    if let Ok(n) = disp.parse::<i64>() {
        return Some(format!("{}{base}", n.checked_add(off)?));
    }
    // `sym`, `sym+n` or `sym-n`.
    let (sym, n) = match disp.rfind(['+', '-']) {
        Some(p) if p > 0 => match disp[p..].parse::<i64>() {
            Ok(n) => (&disp[..p], n),
            Err(_) => (disp, 0),
        },
        _ => (disp, 0),
    };
    let sym_ok = !sym.is_empty()
        && !sym.as_bytes()[0].is_ascii_digit()
        && sym
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$');
    if !sym_ok {
        return None;
    }
    let n = n.checked_add(off)?;
    Some(match n.cmp(&0) {
        std::cmp::Ordering::Greater => format!("{sym}+{n}{base}"),
        std::cmp::Ordering::Less => format!("{sym}{n}{base}"),
        std::cmp::Ordering::Equal => format!("{sym}{base}"),
    })
}

/// The `test` line for mask `m` over `size` bytes at `mem`.
fn test_line(size: u32, m: u64, mem: &str) -> String {
    match size {
        1 => format!("    testb ${}, {mem}", m & 0xff),
        2 => format!("    testw ${}, {mem}", m & 0xffff),
        4 => format!("    testl ${}, {mem}", m as u32 as i32),
        _ => format!("    testq ${}, {mem}", m as i64),
    }
}

/// Width (bits) and register-name row of a mask instruction, or `None`.
fn mask_op(text: &str) -> Option<(u32, usize, &str)> {
    let (bits, row, rest) = if let Some(r) = text.strip_prefix("andl ") {
        (32, 1, r)
    } else if let Some(r) = text.strip_prefix("andq ") {
        (64, 0, r)
    } else if let Some(r) = text.strip_prefix("testl ") {
        (32, 1, r)
    } else if let Some(r) = text.strip_prefix("testq ") {
        (64, 0, r)
    } else if let Some(r) = text.strip_prefix("testw ") {
        (16, 2, r)
    } else if let Some(r) = text.strip_prefix("testb ") {
        (8, 3, r)
    } else {
        return None;
    };
    Some((bits, row, rest))
}

/// See the module documentation.
pub(super) fn fold_load_mask_into_memory_test(
    store: &mut LineStore,
    infos: &mut [LineInfo],
) -> bool {
    let len = store.len();
    let mut liveness: Option<FileLiveness> = None;
    let mut changed = false;
    let mut i = 0;
    while i < len {
        if infos[i].is_nop() || infos[i].pinned || infos[i].volatile_access {
            i += 1;
            continue;
        }
        let Some(cand) = candidate(store, infos, i, len) else {
            i += 1;
            continue;
        };
        let Candidate {
            j,
            fam,
            shape,
            bits,
            imm,
            mem,
            encodable,
        } = cand;
        // Every consumer of the mask's flags, on every path (branch targets
        // included; a sibling call ends a path like `call`).  Someone must
        // consume them -- a dead mask is dead-code territory, not ours -- and
        // no whole-flags reader may observe the undefined AF.
        let facts = walk_flag_consumers(store, infos, j + 1, 0);
        if !facts.proved || !facts.saw_consumer || facts.saw_whole_reader {
            i += 1;
            continue;
        }
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        if lv.live_after(j, fam) != Some(false) {
            i += 1;
            continue;
        }
        let m = effective_memory_mask(shape, imm, bits);
        let exact = (encodable && bits <= shape.bytes * 8).then_some((0u32, bits / 8, imm));
        let narrow = narrow_window(m, shape.bytes)
            .filter(|_| !facts.saw_non_zf)
            .map(|(off, size)| (off, size, (m >> (8 * off)) & low_mask(size * 8)));
        let Some((off, size, mask)) = narrow.or(exact) else {
            i += 1;
            continue;
        };
        if mask == 0 {
            // `x & 0` (or a mask entirely above a zero extension): the result
            // is constant and belongs to constant folding, not to a memory
            // access we would invent.
            i += 1;
            continue;
        }
        let Some(addr) = displace(&mem, off) else {
            i += 1;
            continue;
        };
        mark_nop(&mut infos[i]);
        replace_line(store, &mut infos[j], j, test_line(size, mask, &addr));
        lv.refresh_at(store, infos, j);
        changed = true;
        i = j + 1;
    }
    changed
}

struct Candidate {
    j: usize,
    fam: RegId,
    shape: LoadShape,
    bits: u32,
    imm: u64,
    mem: String,
    /// `imm` is encodable as a `test` immediate at `bits` (always for the
    /// load form, whose mask came from an `and`/`test` immediate).
    encodable: bool,
}

/// Parse the load at `i` and its adjacent mask instruction.
fn candidate(store: &LineStore, infos: &[LineInfo], i: usize, len: usize) -> Option<Candidate> {
    let load = infos[i].trimmed(store.get(i));
    let (mnemonic, rest) = load.split_once(' ')?;
    if mnemonic == "movabsq" || rest.trim_start().starts_with('$') {
        return immediate_candidate(store, infos, i, len, mnemonic, rest.trim());
    }
    let (shape, wide_dst) = load_shape(mnemonic)?;
    let (mem, dst) = split_two_operands(rest.trim())?;
    if !mem.contains('(') || mem.contains(':') || mem.contains('@') {
        return None;
    }
    let fam = plain_gp_operand(dst)?;
    if dst != REG_NAMES[usize::from(!wide_dst)][fam as usize] {
        return None;
    }
    let j = next_non_debug(store, infos, i + 1, len);
    if j >= len || infos[j].is_nop() || infos[j].pinned || infos[j].volatile_access {
        return None;
    }
    let (bits, row, args) = mask_op(infos[j].trimmed(store.get(j)))?;
    let (imm_text, reg) = split_two_operands(args)?;
    if reg != REG_NAMES[row][fam as usize] {
        return None;
    }
    let imm = mask_at_width(imm_text.strip_prefix('$')?.parse::<i64>().ok()?, bits)?;
    Some(Candidate {
        j,
        fam,
        shape,
        bits,
        imm,
        mem: mem.to_string(),
        encodable: true,
    })
}

/// The mask-in-a-register form: `movabsq $2^40, %rax; andq L(%rip), %rax`
/// (also `movq`/`movl $imm` and `test` with either operand order).  The
/// memory operand is read at the instruction's own width with no extension,
/// so the "load" is `bits/8` bytes wide and every mask bit is a memory bit.
fn immediate_candidate(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    len: usize,
    mnemonic: &str,
    rest: &str,
) -> Option<Candidate> {
    let (imm_text, dst) = split_two_operands(rest)?;
    let value = imm_text.strip_prefix('$')?.parse::<i64>().ok()?;
    let fam = plain_gp_operand(dst)?;
    // The register's value as a mask at its written width.
    let reg_mask = match mnemonic {
        "movabsq" | "movq" if dst == REG_NAMES[0][fam as usize] => value as u64,
        "movl" if dst == REG_NAMES[1][fam as usize] => mask_at_width(value, 32)?,
        _ => return None,
    };
    let j = next_non_debug(store, infos, i + 1, len);
    if j >= len || infos[j].is_nop() || infos[j].pinned || infos[j].volatile_access {
        return None;
    }
    let op = infos[j].trimmed(store.get(j));
    let (bits, row, args) = if let Some(r) = op.strip_prefix("andq ") {
        (64, 0, r)
    } else if let Some(r) = op.strip_prefix("testq ") {
        (64, 0, r)
    } else if let Some(r) = op.strip_prefix("andl ") {
        (32, 1, r)
    } else if let Some(r) = op.strip_prefix("testl ") {
        (32, 1, r)
    } else {
        return None;
    };
    // A 32-bit op reads the low half of a 64-bit immediate; a 64-bit op on a
    // `movl`-written register sees the zero-extended value.  Both are what
    // `reg_mask` restricted to `bits` describes.
    let reg = REG_NAMES[row][fam as usize];
    let (a, b) = split_two_operands(args)?;
    let mem = match (a == reg, b == reg) {
        (false, true) if op.starts_with("and") || op.starts_with("test") => a,
        (true, false) if op.starts_with("test") => b,
        _ => return None,
    };
    if !mem.contains('(') || mem.contains(':') || mem.contains('@') {
        return None;
    }
    // The address must not use the mask register: it holds the immediate,
    // not whatever the address meant, and the fold deletes that write.
    if (0..4).any(|r| mem.contains(REG_NAMES[r][fam as usize])) {
        return None;
    }
    let imm = reg_mask & low_mask(bits);
    let encodable = bits == 32 || i32::try_from(imm as i64).is_ok();
    Some(Candidate {
        j,
        fam,
        shape: LoadShape {
            bytes: bits / 8,
            signed: false,
            ext_bits: bits,
        },
        bits,
        imm,
        mem: mem.to_string(),
        encodable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(asm: &str) -> (bool, String) {
        let mut store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let c = fold_load_mask_into_memory_test(&mut store, &mut infos);
        (c, store.build_result(|i| infos[i].is_nop()))
    }

    fn func(body: &str) -> String {
        format!(
            "foo:\n.cfi_startproc\n{body}.LBB1:\n    jmp fast@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 0\n.LBB2:\n    jmp slow@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 0\n.cfi_endproc\n"
        )
    }

    /// Reference model: evaluate the original pair and the rewritten `test`
    /// on every memory value of interest and compare ZF (and SF/PF when the
    /// form claims to be exact).
    fn zf_of_pair(shape: LoadShape, imm: u64, bits: u32, mem: u64) -> bool {
        let mb = shape.bytes * 8;
        let raw = mem & low_mask(mb);
        let mut v = raw;
        if shape.signed && raw >> (mb - 1) & 1 == 1 {
            v |= low_mask(shape.ext_bits) & !low_mask(mb);
        }
        (v & imm & low_mask(bits)) == 0
    }

    #[test]
    fn effective_mask_matches_the_reference_on_every_shape() {
        let shapes = [
            "movl", "movq", "movzbl", "movzbq", "movzwl", "movzwq", "movsbl", "movsbq", "movswl",
            "movswq", "movslq",
        ];
        let imms: [i64; 14] = [
            1,
            0x20,
            0x80,
            0xff,
            0x100,
            0x8000,
            0x1_0100,
            0x3_0000,
            0x8000_0000,
            -1,
            -16,
            -0x100,
            0x7fff_ffff,
            -0x8000_0000,
        ];
        // Memory values: each single bit, all ones, sign-bit patterns.
        let mut mems: Vec<u64> = (0..64).map(|b| 1u64 << b).collect();
        mems.extend([0, u64::MAX, 0x80, 0x8000, 0x8000_0000, 0x7f, 0xff00]);
        for name in shapes {
            let (shape, _) = load_shape(name).unwrap();
            for bits in [8u32, 16, 32, 64] {
                for imm in imms {
                    let Some(mask) = mask_at_width(imm, bits) else {
                        continue;
                    };
                    let m = effective_memory_mask(shape, mask, bits);
                    for &mem in &mems {
                        let mem = mem & low_mask(shape.bytes * 8);
                        assert_eq!(
                            zf_of_pair(shape, mask, bits, mem),
                            mem & m == 0,
                            "{name} imm={imm} bits={bits} mem={mem:#x} m={m:#x}"
                        );
                        if let Some((off, size)) = narrow_window(m, shape.bytes) {
                            assert!(off + size <= shape.bytes, "{name} window past the load");
                            let w = (m >> (8 * off)) & low_mask(size * 8);
                            let bytes = (mem >> (8 * off)) & low_mask(size * 8);
                            assert_eq!(mem & m == 0, bytes & w == 0, "{name} imm={imm} window");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mask_encodability_follows_the_instruction_width() {
        assert_eq!(mask_at_width(255, 8), Some(0xff));
        assert_eq!(mask_at_width(-1, 8), Some(0xff));
        assert_eq!(mask_at_width(256, 8), None);
        assert_eq!(mask_at_width(-129, 8), None);
        assert_eq!(mask_at_width(-16, 32), Some(0xffff_fff0));
        assert_eq!(mask_at_width(4_294_967_295, 32), Some(0xffff_ffff));
        assert_eq!(mask_at_width(-16, 64), Some(0xffff_ffff_ffff_fff0));
        assert_eq!(mask_at_width(0x8000_0000, 64), None);
    }

    #[test]
    fn windows_are_minimal_and_stay_inside_the_load() {
        assert_eq!(narrow_window(0x20, 4), Some((0, 1)));
        assert_eq!(narrow_window(0x3_0000, 4), Some((2, 1)));
        assert_eq!(narrow_window(0x101, 2), Some((0, 2)));
        assert_eq!(narrow_window(0x1_0100, 4), Some((1, 2)));
        // Bytes 2..=3 fit a word at byte 2.
        assert_eq!(narrow_window(0xffff_0000, 8), Some((2, 2)));
        // Bytes 2..=4 span three bytes: a dword at byte 2 still fits.
        assert_eq!(narrow_window(0xff_ffff_0000, 8), Some((2, 4)));
        // Bytes 5..=7 need a dword too, slid back to byte 4 so it stays
        // inside the eight loaded bytes rather than reading bytes 5..=8.
        assert_eq!(narrow_window(0xffff_ff00_0000_0000, 8), Some((4, 4)));
        assert_eq!(narrow_window(0x8000_0000_0000_0001, 8), None);
        assert_eq!(narrow_window(0, 4), None);
        // Bytes 1..=2 fit an (unaligned) word at byte 1.
        assert_eq!(narrow_window(0xff_ff00, 4), Some((1, 2)));
        // A 3-byte span (bytes 1..=3) inside a 4-byte load needs the dword,
        // which slides back to offset 0.
        assert_eq!(narrow_window(0xffff_ff00, 4), Some((0, 4)));
    }

    #[test]
    fn displacement_rewrites_every_plain_form() {
        assert_eq!(displace("G(%rip)", 2).as_deref(), Some("G+2(%rip)"));
        assert_eq!(displace("G+4(%rip)", 1).as_deref(), Some("G+5(%rip)"));
        assert_eq!(displace("G-4(%rip)", 4).as_deref(), Some("G(%rip)"));
        assert_eq!(displace("G-4(%rip)", 1).as_deref(), Some("G-3(%rip)"));
        assert_eq!(displace("(%rdi)", 1).as_deref(), Some("1(%rdi)"));
        assert_eq!(displace("-8(%rbp)", 3).as_deref(), Some("-5(%rbp)"));
        assert_eq!(
            displace("(%rax,%rcx,4)", 2).as_deref(),
            Some("2(%rax,%rcx,4)")
        );
        assert_eq!(displace(".LC0(%rip)", 1).as_deref(), Some(".LC0+1(%rip)"));
        assert_eq!(displace("G(%rip)", 0).as_deref(), Some("G(%rip)"));
        assert_eq!(displace("1+G(%rip)", 1), None);
    }

    #[test]
    fn dispatch_shape_becomes_a_byte_memory_test() {
        let (c, out) = run(&func(
            "    movslq G(%rip), %r11\n    andl $32, %r11d\n    je .LBB2\n",
        ));
        assert!(c, "{out}");
        assert!(out.contains("testb $32, G(%rip)"), "{out}");
        assert!(!out.contains("movslq") && !out.contains("andl"), "{out}");
    }

    #[test]
    fn high_bytes_narrow_to_a_displaced_byte_and_word() {
        let (_, out) = run(&func(
            "    movl G(%rip), %eax\n    andl $196608, %eax\n    jne .LBB2\n",
        ));
        assert!(out.contains("testb $3, G+2(%rip)"), "{out}");
        let (_, out) = run(&func(
            "    movl 8(%rdi), %ecx\n    testl $65792, %ecx\n    je .LBB2\n",
        ));
        assert!(out.contains("testw $257, 9(%rdi)"), "{out}");
    }

    #[test]
    fn sign_extension_bits_map_to_the_memory_sign_bit() {
        // (long)*(signed char *)p & 0x100 tests the sign bit of the byte.
        let (_, out) = run(&func(
            "    movsbq (%rdi), %rax\n    testq $256, %rax\n    jne .LBB2\n",
        ));
        assert!(
            out.contains("testb $-128, (%rdi)") || out.contains("testb $128, (%rdi)"),
            "{out}"
        );
        // Zero extension: bits above the byte are zero, the mask has no
        // memory bit left and the fold stays away.
        let (c, out) = run(&func(
            "    movzbl (%rdi), %eax\n    andl $256, %eax\n    jne .LBB2\n",
        ));
        assert!(!c, "{out}");
    }

    #[test]
    fn sign_consumers_keep_the_flag_exact_width() {
        // `js` reads SF: a byte test would move the sign to bit 7.  The
        // and-width test over the same bytes is exact.
        let (_, out) = run(&func(
            "    movl G(%rip), %eax\n    andl $-2147483648, %eax\n    js .LBB2\n",
        ));
        assert!(out.contains("testl $-2147483648, G(%rip)"), "{out}");
        // A byte load masked at dword width has no exact form (bits 8..31
        // are extension bits), so a sign consumer refuses the fold.
        let (c, out) = run(&func(
            "    movzbl (%rdi), %eax\n    andl $1, %eax\n    js .LBB2\n",
        ));
        assert!(!c, "{out}");
    }

    #[test]
    fn live_registers_volatile_and_decorated_operands_are_refused() {
        // The masked value is returned on the fall-through path.
        let (c, out) = run(concat!(
            "foo:\n.cfi_startproc\n",
            "    movl G(%rip), %eax\n    andl $32, %eax\n    je .LBB2\n    ret\n",
            ".LBB2:\n    jmp slow@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 0\n.cfi_endproc\n"
        ));
        assert!(!c, "{out}");
        for load in [
            "    movl %fs:G@tpoff, %eax\n",
            "    movq G@GOTPCREL(%rip), %rax\n",
            "    movl %fs:8(%rax), %eax\n",
        ] {
            let mask = if load.contains("movq") {
                "    andq $32, %rax\n"
            } else {
                "    andl $32, %eax\n"
            };
            let (c, out) = run(&func(&format!("{load}{mask}    je .LBB2\n")));
            assert!(!c, "{out}");
        }
        // A whole-flags reader observes AF, which and/test leave undefined.
        let (c, out) = run(&func(
            "    movl G(%rip), %eax\n    andl $32, %eax\n    pushfq\n    popq %rcx\n    je .LBB2\n",
        ));
        assert!(!c, "{out}");
    }

    #[test]
    fn register_masks_narrow_to_the_memory_byte() {
        // (L & 0x100000000) != 0: GCC 16.2 `movzbl L+4(%rip)`; one byte test.
        let (_, out) = run(&func(
            "    movabsq $4294967296, %rax\n    andq L(%rip), %rax\n    jne .LBB2\n",
        ));
        assert!(out.contains("testb $1, L+4(%rip)"), "{out}");
        assert!(!out.contains("movabsq"), "{out}");
        // A 64-bit mask with no imm32 encoding has no exact form: `js`
        // refuses.
        let (c, out) = run(&func(
            "    movabsq $-9223372036854775808, %rax\n    andq L(%rip), %rax\n    js .LBB2\n",
        ));
        assert!(!c, "{out}");
        // The mask register names the address: refused.
        let (c, out) = run(&func(
            "    movq $8, %rax\n    andq (%rax), %rax\n    jne .LBB2\n",
        ));
        assert!(!c, "{out}");
        // `test` with the memory operand first, at dword width.
        let (_, out) = run(&func(
            "    movl $65536, %ecx\n    testl (%rdi), %ecx\n    je .LBB2\n",
        ));
        assert!(out.contains("testb $1, 2(%rdi)"), "{out}");
    }

    #[test]
    fn non_adjacent_consumers_and_sibling_paths_are_walked() {
        // cmov consumer after flag-neutral moves (the `s` kernel).
        let (_, out) = run(concat!(
            "foo:\n.cfi_startproc\n",
            "    movslq G(%rip), %rdi\n    andl $128, %edi\n    movl $3, %ecx\n",
            "    movl $5, %eax\n    cmovnel %ecx, %eax\n    ret\n.cfi_endproc\n"
        ));
        assert!(
            out.contains("testb $-128, G(%rip)") || out.contains("testb $128, G(%rip)"),
            "{out}"
        );
        // Both arms are marked sibling calls: the flags die at each.
        let (c, out) = run(concat!(
            "foo:\n.cfi_startproc\n",
            "    movslq G(%rip), %r11\n    andl $32, %r11d\n    je .LBB2\n",
            "    jmp fast@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 0\n",
            ".LBB2:\n    jmp slow@PLT\n    # LCCC_TAIL_CALL\n    # LCCC_CALL_ARGS 0\n",
            ".cfi_endproc\n"
        ));
        assert!(c && out.contains("testb $32, G(%rip)"), "{out}");
    }

    #[test]
    fn address_register_reuse_and_non_adjacent_pairs() {
        // The load overwrote its own base: the test keeps the base intact.
        let (_, out) = run(&func(
            "    movl (%rax), %eax\n    andl $1, %eax\n    je .LBB2\n",
        ));
        assert!(out.contains("testb $1, (%rax)"), "{out}");
        // A store between the load and the mask would change what the
        // memory operand reads: adjacency is required.
        let (c, out) = run(&func(
            "    movl G(%rip), %eax\n    movl $0, G(%rip)\n    andl $1, %eax\n    je .LBB2\n",
        ));
        assert!(!c, "{out}");
    }
}

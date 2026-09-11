//! Compare-and-branch fusion pass.
//!
//! Fuses cmp + setCC + test + jCC sequences into a single conditional jump,
//! eliminating the boolean materialization overhead from the codegen model.

use super::super::types::*;
use super::liveness::FileLiveness;

/// Maximum number of store/load offsets tracked during compare-and-branch fusion.
const MAX_TRACKED_STORE_LOAD_OFFSETS: usize = 4;

/// Size of the instruction lookahead window for compare-and-branch fusion.
/// 12 (not 8): transparent-skipped live code between the boolean
/// materialization and its test consumes window slots without being part of
/// the fused shape.
const CMP_FUSION_LOOKAHEAD: usize = 12;

fn parse_stack_operand(text: &str, source: bool) -> Option<(u8, i32)> {
    let (_, operands) = text.split_once(char::is_whitespace)?;
    let (src, dst) = operands.split_once(',')?;
    let op = if source { src.trim() } else { dst.trim() };
    for (suffix, base) in [("(%rsp)", 4u8), ("(%rbp)", 5u8)] {
        if let Some(n) = op.strip_suffix(suffix) {
            let off = if n.trim().is_empty() {
                0
            } else {
                n.trim().parse::<i32>().ok()?
            };
            return Some((base, off));
        }
    }
    None
}

/// True when the instruction READS the EFLAGS register without rewriting it
/// (or rewrites only part of it — see `flags_partial_writers`). A reader on
/// the fall-through path after a fused jcc would observe the PRODUCER cmp's
/// flags instead of the dropped `testq`'s flags: the fusion must refuse.
fn flags_reader(t: &str) -> bool {
    const READERS: &[&str] = &[
        "cmov", "set", "adc", "sbb", "rcl", "rcr", "pushfq", "popfq", "sahf", "lahf",
    ];
    READERS.iter().any(|p| t.starts_with(p)) || (t.starts_with('j') && !t.starts_with("jmp"))
}

/// True when the instruction ends the flags-observation window: it rewrites
/// all six arithmetic flags, so any later reader observes ITS flags on both
/// the original and the fused path. `inc`/`dec` deliberately do NOT qualify
/// (they preserve CF, so a later `adc` would still see the dropped
/// producer's vs. the fused cmp's carry).
fn flags_full_writer(t: &str) -> bool {
    const WRITERS: &[&str] = &[
        "cmp", "test", "add", "sub", "and", "or", "xor", "neg", "mul", "imul", "div", "idiv",
        "shl", "shr", "sar", "sal", "rol", "ror", "bt", "bts", "btr", "btc", "popfq", "clc", "stc",
        "cmc", "call", "ret", "jmp",
    ];
    WRITERS.iter().any(|p| t.starts_with(p))
}

/// Parse `testq %rX, %rX` / `testl %eXd, %eXd` / `testw %Xw, %Xw` /
/// `testb %rXb, %rXb` as a single-register test of a 64/32/16/8-bit register
/// name. Returns the register text.
fn parse_self_test(t: &str) -> Option<&str> {
    // NOTE: the prefixes carry the separating space but NOT the `%`: the
    // returned register text must include it (`%r9b`, not `r9b`) so callers
    // can compare against canonical spellings and feed
    // `register_family_fast` (which requires the `%` prefix).
    for mnem in ["testq ", "testl ", "testw ", "testb "] {
        if let Some(rest) = t.strip_prefix(mnem) {
            let mut parts = rest.split(", ");
            let a = parts.next()?.trim();
            let b = parts.next()?.trim();
            if a == b && !a.contains(',') && !a.contains('(') {
                // Return a 'static slice of the register text.
                let start = mnem.len();
                let end = start + a.len();
                let full = t;
                let leaked = &full[start..end];
                return Some(leaked);
            }
        }
    }
    None
}

/// Bytes transferred by a stack-slot access of this width, for boolean-slot
/// validation. A carrier reload recovers the boolean only if it reads no
/// more bytes than the carrier-source store wrote meaningfully (`movl` + a
/// 64-bit reload reads 4 stale upper bytes and must refuse).
fn move_size_bytes(size: MoveSize) -> u32 {
    match size {
        MoveSize::Q | MoveSize::SD => 8,
        MoveSize::L | MoveSize::SLQ | MoveSize::SS => 4,
        MoveSize::W => 2,
        MoveSize::B => 1,
    }
}

/// Parse `movzbl/movzbq <setcc-text>, <dst>` where the source is EXACTLY the
/// setCC destination text. Returns `Some(Ok(fam))` for a well-formed GP
/// destination of the matching width, `Some(Err(()))` when the source
/// matches but the destination is exotic (fusion must refuse — the old code
/// fused these with a vacuous liveness gate), and `None` when the line is
/// not such an extension. Exact-text matching (not family matching) is what
/// excludes `%ah`-style high-byte aliasing of families 0-3.
fn parse_bool_extension(line: &str, setcc_text: &str) -> Option<Result<RegId, ()>> {
    // (prefix, REG_NAMES row): movzbl widens to 32 bits (row 1), movzbq to
    // 64 bits (row 0). Matching the destination against the canonical
    // spelling table enforces the width and yields the family directly.
    for (prefix, row) in [("movzbl ", 1usize), ("movzbq ", 0usize)] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let (src, dst) = rest.split_once(',')?;
            if src.trim() != setcc_text {
                return None;
            }
            let dst = dst.trim();
            match REG_NAMES[row].iter().position(|&n| n == dst) {
                Some(f) => return Some(Ok(f as RegId)),
                None => return Some(Err(())),
            }
        }
    }
    None
}

/// Parse `movslq <src>, <dst>` operand families. `None` when unparseable or
/// when either operand is not a plain register (stack forms are
/// LoadRbp-classified and handled by the reload arm; anything else is
/// refused by the caller).
fn parse_movslq_regs(line: &str) -> Option<(RegId, RegId)> {
    let rest = line.strip_prefix("movslq ")?;
    let (src, dst) = rest.split_once(',')?;
    let src = src.trim();
    let dst = dst.trim();
    if src.contains('(') || dst.contains('(') || src.contains(' ') || dst.contains(' ') {
        return None;
    }
    Some((register_family_fast(src), register_family_fast(dst)))
}

/// Bitmask of boolean-carrier families for `reg_refs` touch checks. Families
/// above `REG_GP_MAX` (and `REG_NONE`) contribute no bits: `reg_refs` is
/// GP-only, and a non-GP carrier means the setCC destination failed to
/// parse, in which case the generalized arms never fire anyway.
fn carrier_mask(setcc_fam: RegId, relay_fam: Option<RegId>) -> u16 {
    let mut mask = 0u16;
    if setcc_fam <= REG_GP_MAX {
        mask |= 1u16 << setcc_fam;
    }
    if let Some(f) = relay_fam {
        if f <= REG_GP_MAX {
            mask |= 1u16 << f;
        }
    }
    mask
}

/// True when `line` may be SKIPPED (but PRESERVED, never NOPed) between a
/// compare and its fused branch: the scheduler hoisted real, live code into
/// the boolean materialization (e.g. nbody's XMM constant load between the
/// step-counter `movzbl` and its `testb`). Preservation is what makes this
/// sound — the instruction executes byte-identically; the only requirements
/// are (a) flags-pure, so the fused jcc still observes the producer cmp's
/// flags, and (b) carrier-untouched (checked by the caller against the final
/// carrier mask, since a line skipped before the relay appeared could
/// otherwise clobber the relay family). Stack-slot traffic that reaches here
/// (non-carrier spills, XMM traffic) is likewise preserved verbatim; a
/// transparent line that references a tracked boolean slot is refused by the
/// post-scan check (it would observe the NOPed store's absence).
fn is_fusion_transparent(line: &str, kind: LineKind) -> bool {
    match kind {
        LineKind::Other { .. }
        | LineKind::Push { .. }
        | LineKind::Pop { .. }
        | LineKind::StoreXmmRbp { .. }
        | LineKind::LoadXmmRbp { .. }
        | LineKind::StoreRbp { .. }
        | LineKind::LoadRbp { .. } => {}
        // Labels, directives, branches, calls, returns, setCC/tests and
        // inline asm are never transparent (control flow or flags traffic).
        _ => return false,
    }
    let tok = line.split([' ', '\t']).next().unwrap_or("");
    // Every `mov*`/`vmov*` form is flags-transparent (moves, zero/sign
    // extensions, scalar/packed FP moves — no x86 mnemonic sharing these
    // prefixes writes flags). `leave` shares the `lea` prefix but is only
    // matched below by exact token, so it stays excluded.
    if tok.starts_with("mov") || tok.starts_with("vmov") {
        return true;
    }
    if tok.starts_with("nop") || tok.starts_with("prefetch") {
        return true;
    }
    matches!(
        tok,
        "lea"
            | "leaq"
            | "leal"
            | "leaw"
            | "push"
            | "pushq"
            | "pushl"
            | "pushw"
            | "pop"
            | "popq"
            | "popl"
            | "popw"
    )
}

pub(super) fn fuse_compare_and_branch(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let len = store.len();

    let mut i = 0;
    while i < len {
        if infos[i].kind != LineKind::Cmp {
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

        // Second must be setCC. Capture the destination family AND validate
        // the exact destination spelling: the family id alone is NOT
        // sufficient (`%ah` aliases family 0), so every byte operand below
        // is matched against the canonical LOW-byte spelling textually. This
        // also closes a latent hole where the legacy `%al` arms fired for a
        // setCC that wrote a different byte register.
        let setcc_fam: RegId = match infos[seq_indices[1]].kind {
            LineKind::SetCC { reg } => reg,
            _ => {
                i += 1;
                continue;
            }
        };
        let set_line = infos[seq_indices[1]].trimmed(store.get(seq_indices[1]));
        let cc = match parse_setcc(set_line) {
            Some(c) => c,
            None => {
                i += 1;
                continue;
            }
        };
        let setcc_text: Option<&str> = set_line.split_whitespace().nth(1);
        let setcc_valid =
            setcc_fam <= REG_GP_MAX && setcc_text == Some(REG_NAMES[3][setcc_fam as usize]);
        let setcc_is_al = setcc_text == Some("%al");

        // Scan the boolean materialization: the carrier starts as the setCC
        // destination byte and optionally widens through ONE zero-extending
        // relay (`movzbl/movzbq`). Everything skipped here is either boolean
        // plumbing (NOPed after fusion) or transparent live code (PRESERVED —
        // tracked in `transparent_skip` and excluded from the NOP range and
        // the store/load census). Track StoreRbp offsets so we can bail out
        // if any store's slot is potentially read by another basic block (no
        // matching load nearby).
        let mut test_idx = None;
        // Final carrier of the boolean: legacy `%al`→`%rax` (corpus-validated
        // no-gate contract, tracked by `legacy_ext_seen`), or a generalized
        // relay destination family (`relay_fam` — every non-legacy shape is
        // gated on exact liveness below).
        let mut legacy_ext_seen = false;
        let mut relay_fam: Option<RegId> = None;
        let mut legacy_pure = setcc_is_al;
        let mut transparent_skip = [false; CMP_FUSION_LOOKAHEAD];
        // Boolean slots: stack slots stored FROM a carrier, with the count of
        // meaningful boolean bytes. A carrier-destined reload is plumbing
        // only if it reads such a slot within its meaningful width.
        let mut bool_slots: [(u8, i32, u32); MAX_TRACKED_STORE_LOAD_OFFSETS] =
            [(0, 0, 0); MAX_TRACKED_STORE_LOAD_OFFSETS];
        let mut bool_count = 0usize;
        let mut store_offsets: [(u8, i32); MAX_TRACKED_STORE_LOAD_OFFSETS] =
            [(0, 0); MAX_TRACKED_STORE_LOAD_OFFSETS];
        let mut store_count = 0usize;
        let mut scan = 2;
        while scan < seq_count {
            let si = seq_indices[scan];
            let line = infos[si].trimmed(store.get(si));

            // SOUNDNESS: user inline asm is opaque. A template line can
            // textually mimic every accepted relay/extension shape here
            // (`movzbq %al, %rax` is even a documented ALTERNATIVE()
            // length-placeholder idiom) — fusing across it would then NOP
            // user bytes and leave the template reading a %al that the
            // deleted setCC no longer defines. Abort the fusion instead.
            if infos[si].kind == LineKind::InlineAsm {
                break;
            }

            // Relay hop of the setcc result. A later test MUST be of the
            // relay's destination — a test of a different register is a
            // DIFFERENT value (zlib-ng zng_deflateSetParams: size<4's setb
            // was fused with the test of a slotted `new_strategy`, so a
            // 4-byte buffer became Z_BUF_ERROR).
            //
            // Legacy exact shapes `movzbq %al, %rax` / `movzbl %al, %eax`
            // (corpus-validated no-gate contract). The `%al` requirement ties
            // them to the actual setCC destination: without it a setCC to a
            // different byte followed by a stale-`%al` extension would fuse.
            if setcc_is_al && (line == "movzbq %al, %rax" || line == "movzbl %al, %eax") {
                legacy_ext_seen = true;
                scan += 1;
                continue;
            }
            // Carrier families known SO FAR (the relay may not have appeared
            // yet; lines above it must still avoid the setCC byte).
            let carriers = carrier_mask(setcc_fam, relay_fam);
            let is_carrier = |fam: RegId| fam <= REG_GP_MAX && carriers & (1u16 << fam) != 0;
            // Boolean extension from `%al` (legacy relay to a non-rax
            // register, e.g. `movzbq %al, %r12`). Unparseable destinations
            // refuse: the old code fused them with a vacuous gate.
            if setcc_is_al && relay_fam.is_none() {
                match parse_bool_extension(line, "%al") {
                    Some(Ok(dfam)) => {
                        relay_fam = Some(dfam);
                        legacy_pure = false;
                        scan += 1;
                        continue;
                    }
                    Some(Err(())) => break,
                    None => {}
                }
            }
            // Generalized boolean extension (NEW capability): `movzbl/movzbq`
            // from EXACTLY the setCC destination text to any GP register
            // (RA-homed booleans such as nbody's `setl %r9b` / `movzbl
            // %r9b, %r9d`). With a `%al` setCC the arms above consume every
            // `%al`-source extension first, so this arm only sees non-`%al`
            // sources then.
            if setcc_valid && relay_fam.is_none() {
                if let Some(setcc_src) = setcc_text {
                    match parse_bool_extension(line, setcc_src) {
                        Some(Ok(dfam)) => {
                            relay_fam = Some(dfam);
                            legacy_pure = false;
                            scan += 1;
                            continue;
                        }
                        Some(Err(())) => break,
                        None => {}
                    }
                }
            }
            // Boolean-slot store (plumbing): a carrier-source stack store is
            // NOPed after fusion (justified by the pre-existing store/load
            // matching below) and recorded as a boolean slot so a later
            // carrier reload can be validated. A NON-carrier store falls
            // through to the transparent arm (preserved, excluded from the
            // matching census). An untracked carrier store poisons the
            // fusion (never NOP an unverified store).
            if let LineKind::StoreRbp { reg, size, .. } = infos[si].kind {
                if is_carrier(reg) {
                    match parse_stack_operand(line, false) {
                        Some((base, off))
                            if store_count < MAX_TRACKED_STORE_LOAD_OFFSETS
                                && bool_count < MAX_TRACKED_STORE_LOAD_OFFSETS =>
                        {
                            store_offsets[store_count] = (base, off);
                            store_count += 1;
                            // A relay source holds a canonical 0/1 in all
                            // bits (32-bit writes zero-extend), so the full
                            // store width is meaningful; the raw setCC byte
                            // carries exactly one meaningful byte. (The
                            // legacy `%al`→`%rax` extension leaves
                            // `relay_fam` unset by design, so compare against
                            // the canonical carrier instead.)
                            let canon = relay_fam.or(if legacy_ext_seen { Some(0) } else { None });
                            let bytes = if Some(reg) == canon {
                                move_size_bytes(size)
                            } else {
                                1
                            };
                            bool_slots[bool_count] = (base, off, bytes);
                            bool_count += 1;
                            scan += 1;
                            continue;
                        }
                        _ => {
                            store_count = usize::MAX;
                            break;
                        }
                    }
                }
            }
            // Stack reload: a carrier-destined load is plumbing ONLY if it
            // reads a boolean slot recorded earlier in this scan within its
            // meaningful width (spill+reload of the boolean); anything else
            // redefines the carrier with a foreign value and the fusion must
            // refuse. (The old code skipped — and NOPed — ANY stack load,
            // deleting unrelated reloads.) Non-carrier loads fall through to
            // the transparent arm.
            let mut load_info: Option<(RegId, u32, Option<(u8, i32)>)> = None;
            if let LineKind::LoadRbp { reg, size, .. } = infos[si].kind {
                load_info = Some((reg, move_size_bytes(size), parse_stack_operand(line, true)));
            } else if line.starts_with("movsbq ")
                || line.starts_with("movswq ")
                || line.starts_with("movzbq ")
                || line.starts_with("movzwq ")
                || line.starts_with("movsbl ")
                || line.starts_with("movzbl ")
            {
                // Textual fallback for sign/zero-extending loads the
                // classifier missed; the destination is the post-comma
                // register. (Register-to-register foreign `movzbq` lines also
                // land here with no stack slot: a carrier destination then
                // correctly refuses, anything else falls through.)
                if let Some(comma) = line.rfind(',') {
                    let dst = line[comma + 1..].trim();
                    if !dst.contains('(') && !dst.contains(' ') {
                        // `movsbl/movzbl` and the `*bq` forms read 1 byte,
                        // the `*wq` forms 2 (5th mnemonic character).
                        let bytes = if line.as_bytes()[4] == b'w' { 2 } else { 1 };
                        load_info = Some((
                            register_family_fast(dst),
                            bytes,
                            parse_stack_operand(line, true),
                        ));
                    }
                }
            }
            if let Some((dfam, load_bytes, slot)) = load_info {
                if is_carrier(dfam) {
                    let hit = slot.is_some_and(|(base, off)| {
                        bool_slots[..bool_count]
                            .iter()
                            .any(|&(b, o, bytes)| b == base && o == off && load_bytes <= bytes)
                    });
                    if hit {
                        scan += 1;
                        continue;
                    }
                    break;
                }
            }
            // Sign-extension widening: plumbing ONLY when it widens the
            // carrier within its own family (`movslq %r9d, %r9`, `cltq` with
            // an rax carrier — the identity on a 0/1 value). A widening that
            // touches a carrier any other way redefines or leaks the boolean
            // and the fusion must refuse; an untouched one falls through to
            // the transparent arm. (The old code skipped — and NOPed — ANY
            // cltq/movslq, deleting unrelated sign-extensions. Stack movslq
            // is LoadRbp-classified and handled above, never here.)
            if (line == "cltq" || line.starts_with("movslq "))
                && !matches!(infos[si].kind, LineKind::LoadRbp { .. })
            {
                let (src_fam, dst_fam) = if line == "cltq" {
                    (0, 0)
                } else {
                    match parse_movslq_regs(line) {
                        Some(pair) => pair,
                        None => break,
                    }
                };
                if is_carrier(src_fam) || is_carrier(dst_fam) {
                    if src_fam == dst_fam && is_carrier(src_fam) {
                        scan += 1;
                        continue;
                    }
                    break;
                }
            }
            // Check for test: legacy rax carrier (ONLY with the legacy
            // extension seen and no relay — an extension-less `testl %eax`
            // reads stale upper bits, not the boolean, and a rax test after
            // `movzbl %al, %r12d` tests a DIFFERENT value), or the
            // generalized test below.
            if setcc_is_al
                && legacy_ext_seen
                && relay_fam.is_none()
                && (line == "testq %rax, %rax" || line == "testl %eax, %eax")
            {
                test_idx = Some(scan);
                break;
            }
            // Generalized test (NEW capability): with a relay, any-width
            // self-test of the relay family (the value is 0/1, so all widths
            // agree); without one, ONLY a byte self-test of EXACTLY the
            // setCC destination text (wider tests would read stale upper
            // bits). Byte tests match by exact spelling — never by family —
            // so `%ah` cannot alias in.
            if let Some(regtext) = parse_self_test(line) {
                let is_byte = line.starts_with("testb ");
                let ok = match relay_fam {
                    Some(f) => {
                        if is_byte {
                            regtext == REG_NAMES[3][f as usize]
                        } else {
                            register_family_fast(regtext) == f
                        }
                    }
                    None => is_byte && setcc_valid && Some(regtext) == setcc_text,
                };
                if ok {
                    test_idx = Some(scan);
                    break;
                }
            }
            // Transparent live code (NEW capability): preserved verbatim,
            // only skipped for TEST discovery. Soundness: flags-pure (fused
            // jcc still sees the producer cmp's flags); carrier-untouched
            // and boolean-slot-untouched (both re-verified post-scan against
            // the final carrier mask and tracked stores, since a line
            // skipped before the relay appeared could otherwise clobber the
            // relay family or observe a NOPed store's absence).
            if is_fusion_transparent(line, infos[si].kind) {
                legacy_pure = false;
                transparent_skip[scan] = true;
                scan += 1;
                continue;
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
            let mut load_offsets: [(u8, i32); MAX_TRACKED_STORE_LOAD_OFFSETS] =
                [(0, 0); MAX_TRACKED_STORE_LOAD_OFFSETS];
            let mut load_count = 0usize;
            for ri in range_start..=range_end {
                // Transparent (preserved) lines are invisible to the
                // matching census: a preserved load must not satisfy a
                // plumbing store (the store is NOPed while the preserved
                // load keeps reading the slot — the post-scan check refuses
                // such shapes instead).
                if (2..=test_scan).any(|s| transparent_skip[s] && seq_indices[s] == ri) {
                    continue;
                }
                let text = infos[ri].trimmed(store.get(ri));
                let off = if matches!(infos[ri].kind, LineKind::LoadRbp { .. }) {
                    parse_stack_operand(text, true)
                } else if infos[ri].kind == LineKind::Nop {
                    let raw = store.get(ri).trim();
                    parse_stack_operand(raw, true)
                } else if text.starts_with("movsbq ")
                    || text.starts_with("movswq ")
                    || text.starts_with("movzbq ")
                    || text.starts_with("movzwq ")
                    || text.starts_with("movsbl ")
                    || text.starts_with("movzbl ")
                {
                    parse_stack_operand(text, true)
                } else {
                    None
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

        let jmp_line =
            infos[seq_indices[test_scan + 1]].trimmed(store.get(seq_indices[test_scan + 1]));
        let (is_jne, branch_target) = if let Some(target) = jmp_line.strip_prefix("jne ") {
            (true, target.trim())
        } else if let Some(target) = jmp_line.strip_prefix("je ") {
            (false, target.trim())
        } else {
            i += 1;
            continue;
        };

        // Post-scan verification for every NON-legacy shape (legacy-pure =
        // setCC %al + legacy extension + rax test keeps its corpus-validated
        // no-gate contract; ALL other shapes gate here).
        let needs_gate = relay_fam.is_some() || !legacy_pure;
        if needs_gate {
            // The setCC family must be nameable for the gate (a non-GP
            // family means the destination failed to parse, in which case no
            // generalized arm could have fired — defensive).
            if setcc_fam > REG_GP_MAX {
                i += 1;
                continue;
            }
            let jcc_pos = seq_indices[test_scan + 1];
            // (a) Transparent lines are re-verified against the FINAL carrier
            // mask: a line skipped before the relay appeared could otherwise
            // clobber the relay family. (b) No transparent line may reference
            // a tracked boolean slot: preserved slot traffic would observe
            // the NOPed store's absence.
            let final_mask = carrier_mask(setcc_fam, relay_fam);
            let mut refuse = false;
            for s in 2..=test_scan {
                if !transparent_skip[s] {
                    continue;
                }
                let ti = seq_indices[s];
                if (infos[ti].reg_refs & final_mask) != 0 {
                    refuse = true;
                    break;
                }
                let tline = infos[ti].trimmed(store.get(ti));
                let slot_hit = parse_stack_operand(tline, false)
                    .or_else(|| parse_stack_operand(tline, true))
                    .is_some_and(|(base, off)| {
                        store_offsets[..store_count]
                            .iter()
                            .any(|&(b, o)| b == base && o == off)
                    });
                if slot_hit {
                    refuse = true;
                    break;
                }
            }
            if refuse {
                if std::env::var_os("CCC_DEBUG_CMP_FUSE").is_some() {
                    eprintln!("[CMPFUSE] refusing: transparent line touches carrier/slot");
                }
                i += 1;
                continue;
            }
            // (c) LIVENESS GATE (real this time — the historical relay gate
            // mapped the %-less relay text through `register_family_fast`,
            // which requires a `%` prefix, so it evaluated to REG_NONE and
            // never fired): every family whose definition is NOPed must be
            // dead after the fused jump. Unknown liveness refuses.
            let lv = FileLiveness::new(store, infos);
            let setcc_dead = lv.live_after(jcc_pos, setcc_fam) == Some(false);
            let relay_dead = relay_fam
                .map(|f| lv.live_after(jcc_pos, f) == Some(false))
                .unwrap_or(true);
            if !setcc_dead || !relay_dead {
                if std::env::var_os("CCC_DEBUG_CMP_FUSE").is_some() {
                    eprintln!(
                        "[CMPFUSE] refusing non-legacy fusion (setcc_fam={} setcc_live_after={:?} relay_live_after={:?})",
                        setcc_fam,
                        lv.live_after(jcc_pos, setcc_fam),
                        relay_fam.map(|f| lv.live_after(jcc_pos, f))
                    );
                }
                i += 1;
                continue;
            }
        }

        // Flags-reader guard (pure correctness, applies to every fusion):
        // the fall-through path after the fused jcc now carries the
        // PRODUCER cmp's flags where the dropped `testq`'s flags used to
        // be. A reader (cmov/setCC/adc/sbb/conditional jump/pushfq) before
        // the next full flag writer would observe the wrong flags.
        let guard_end = (seq_indices[test_scan + 1] + 64).min(len);
        let mut flags_hazard = false;
        for g in (seq_indices[test_scan + 1] + 1)..guard_end {
            if infos[g].is_nop() || matches!(infos[g].kind, LineKind::Directive | LineKind::Empty) {
                continue;
            }
            // Inline asm may read or write flags through raw encodings the
            // textual flags_reader/flags_full_writer predicates cannot see
            // (`.byte $0x83, $0xd0, $0x00` is an adc). Treat the region as a
            // flags hazard.
            if infos[g].kind == LineKind::InlineAsm {
                flags_hazard = true;
                break;
            }
            let gt = infos[g].trimmed(store.get(g)).to_string();
            if flags_reader(&gt) {
                flags_hazard = true;
                break;
            }
            if flags_full_writer(&gt) {
                break;
            }
        }
        if flags_hazard {
            if std::env::var_os("CCC_DEBUG_CMP_FUSE").is_some() {
                eprintln!("[CMPFUSE] flags-reader hazard after fused jcc — refusing");
            }
            i += 1;
            continue;
        }

        let fused_cc = if is_jne { cc } else { invert_cc(cc) };
        let fused_jcc = format!("    j{} {}", fused_cc, branch_target);

        // NOP out everything from setCC through the test, EXCEPT transparent
        // live code (preserved verbatim in place).
        for s in 1..=test_scan {
            if transparent_skip[s] {
                continue;
            }
            mark_nop(&mut infos[seq_indices[s]]);
        }
        // Replace the jne/je with the fused conditional jump
        let idx = seq_indices[test_scan + 1];
        replace_line(store, &mut infos[idx], idx, fused_jcc);

        changed = true;
        i = idx + 1;
    }

    changed
}

/// Late fusion for boolean materializations that survive stack-slot pinning and
/// frame compaction. The stack slot must occur exactly twice in the function
/// text (one store and its matching load), proving it is an unobservable
/// compiler temporary.
pub(super) fn fuse_late_compare_bool_spills(asm: &mut String) -> bool {
    let mut lines: Vec<String> = asm.lines().map(str::to_string).collect();
    let mut changed = false;

    // Inline-asm regions are user-authored bytes; this pass runs on raw text
    // (phase 9) where `LineKind::InlineAsm` pinning no longer exists, so it
    // must exclude every line inside a `#APP`..`#NO_APP` span from matching,
    // and the "slot occurs exactly 3 times" census below must ignore the
    // possibility of template text mentioning the slot (an asm mention makes
    // the count differ, which correctly suppresses the fusion).
    let mut in_asm = vec![false; lines.len()];
    let mut inside = false;
    for (idx, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t == "#APP" {
            inside = true;
        } else if t == "#NO_APP" {
            inside = false;
        }
        in_asm[idx] = inside;
    }
    let asm_free = |i: usize| !in_asm[i];

    // First collapse a constant-false predecessor of a spilled-boolean join.
    // This turns the remaining compare path into a single-predecessor fallthrough.
    let active0: Vec<usize> = (0..lines.len())
        .filter(|i| !lines[*i].trim().is_empty())
        .collect();
    for p in 0..active0.len().saturating_sub(2) {
        let a = active0[p];
        let b = active0[p + 1];
        let c = active0[p + 2];
        if !asm_free(a) || !asm_free(b) || !asm_free(c) {
            continue;
        }
        if lines[a].trim() != "xorl %eax, %eax" {
            continue;
        }
        let Some(slot) = lines[b]
            .trim()
            .strip_prefix("movq %rax, ")
            .map(str::to_string)
        else {
            continue;
        };
        let Some(join) = lines[c].trim().strip_prefix("jmp ").map(str::to_string) else {
            continue;
        };
        if lines.iter().filter(|l| l.contains(&slot)).count() != 3 {
            continue;
        }
        let Some(li) = lines.iter().position(|l| l.trim() == format!("{}:", join)) else {
            continue;
        };
        let tail: Vec<usize> = ((li + 1)..lines.len())
            .filter(|i| !lines[*i].trim().is_empty())
            .take(3)
            .collect();
        if tail.len() != 3 || tail.iter().any(|&i| !asm_free(i)) {
            continue;
        }
        let load = lines[tail[0]].trim();
        let load_ok = [
            "movsbq ", "movswq ", "movslq ", "movzbq ", "movzwq ", "movq ",
        ]
        .iter()
        .any(|q| {
            load.strip_prefix(q)
                .map(|r| r == format!("{}, %rax", slot))
                .unwrap_or(false)
        });
        if !load_ok
            || (lines[tail[1]].trim() != "testq %rax, %rax"
                && lines[tail[1]].trim() != "testl %eax, %eax")
        {
            continue;
        }
        let Some(target) = lines[tail[2]]
            .trim()
            .strip_prefix("je ")
            .map(str::to_string)
        else {
            continue;
        };
        lines[a].clear();
        lines[b].clear();
        lines[c] = format!("    jmp {}", target);
        changed = true;
    }

    // Count explicit branch references; labels with none are transparent joins.
    let mut refs = crate::common::fx_hash::FxHashMap::<String, usize>::default();
    for l in &lines {
        let t = l.trim();
        if t.starts_with('j') {
            if let Some(x) = t.split_whitespace().last() {
                *refs.entry(x.to_string()).or_default() += 1;
            }
        }
    }
    let active: Vec<usize> = (0..lines.len())
        .filter(|i| {
            let t = lines[*i].trim();
            if t.is_empty() {
                return false;
            }
            if let Some(label) = t.strip_suffix(':') {
                return refs.get(label).copied().unwrap_or(0) != 0;
            }
            true
        })
        .collect();
    let mut p = 0;
    while p + 6 < active.len() {
        let ix = &active[p..p + 7];
        // User asm bytes are immutable: no fused rewrite may touch, NOP, or
        // consume a line inside a `#APP`..`#NO_APP` region.
        if ix.iter().any(|&i| !asm_free(i)) {
            p += 1;
            continue;
        }
        let t: Vec<&str> = ix.iter().map(|i| lines[*i].trim()).collect();
        if !(t[0].starts_with("cmp") || t[0].starts_with("test")) {
            p += 1;
            continue;
        }
        let Some(cc) = parse_setcc(t[1]) else {
            p += 1;
            continue;
        };
        if !(t[2].starts_with("movzbl %al, %eax") || t[2].starts_with("movzbq %al, %rax")) {
            p += 1;
            continue;
        }
        let Some(slot) = t[3].strip_prefix("movq %rax, ") else {
            p += 1;
            continue;
        };
        let load_ok = [
            "movsbq ", "movswq ", "movslq ", "movzbq ", "movzwq ", "movq ",
        ]
        .iter()
        .any(|q| {
            t[4].strip_prefix(q)
                .map(|r| r == format!("{}, %rax", slot))
                .unwrap_or(false)
        });
        if !load_ok || (t[5] != "testq %rax, %rax" && t[5] != "testl %eax, %eax") {
            p += 1;
            continue;
        }
        let (nonzero, target) = if let Some(x) = t[6].strip_prefix("jne ") {
            (true, x)
        } else if let Some(x) = t[6].strip_prefix("je ") {
            (false, x)
        } else {
            p += 1;
            continue;
        };
        if lines
            .iter()
            .any(|l| l.trim_start().starts_with("leaq ") && l.contains(slot))
        {
            p += 1;
            continue;
        }
        let fused = (if nonzero { cc } else { invert_cc(cc) }).to_string();
        let target = target.to_string();
        for q in 1..=5 {
            lines[ix[q]].clear();
        }
        lines[ix[6]] = format!("    j{} {}", fused, target);
        changed = true;
        p += 7;
    }
    if changed {
        *asm = lines.join("\n") + "\n";
    }
    changed
}

#[cfg(test)]
mod compare_branch_fusion_tests {
    //! Tests for [`super::fuse_compare_and_branch`] — compare/branch fusion.
    //!
    //! Structure: positives prove each accepted shape fuses; negatives prove
    //! each legality rule actually blocks. Every negative here corresponds to
    //! a real defect class (latent miscompiles the generalized scan closes),
    //! and they are named for the rule they pin.
    //!
    //! The tests drive the WHOLE peephole pipeline rather than this pass
    //! alone (the `narrow_copy_fold_tests` convention): a rewrite that is
    //! individually sound but that a neighbouring pass then misreads is still
    //! a bug.

    use super::super::peephole_optimize;

    fn run(input: &str) -> String {
        peephole_optimize(input.to_string())
    }

    /// Wrap a body in the CFI markers `FileLiveness` needs to analyse a
    /// function. Without them liveness answers `None` and every gated
    /// fusion is skipped, so a test that forgets this passes vacuously.
    fn f(body: &str) -> String {
        format!(".text\nf:\n.cfi_startproc\n{}\n.cfi_endproc\n", body)
    }

    // ── A. positives: accepted shapes fuse ────────────────────────────────

    #[test]
    fn a_legacy_rax_shape_fuses_without_a_liveness_gate() {
        // The corpus-validated contract: setCC %al + extension + rax test
        // fuses even with %rax live after the jump (pinned, not endorsed —
        // the new shapes all gate).
        let out = run(&f(
            "    cmpl %eax, %ebx\n    setl %al\n    movzbq %al, %rax\n    testq %rax, %rax\n    jne .Lx\n    movq %rax, %rdi\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jl .Lx"), "{}", out);
        assert!(!out.contains("setl"), "{}", out);
    }

    #[test]
    fn a_nbody_step_counter_with_hoisted_xmm_load_fuses() {
        // P2 (nbody `advance`): `setl %r9b; movzbl %r9b, %r9d` with a
        // flags-pure XMM constant load hoisted between the extension and its
        // `testb`. The movsd is PRESERVED, the boolean is gone, `je`+`setl`
        // becomes `jge`.
        let out = run(&f(
            "    cmpl $5000000, 24(%rsp)\n    setl %r9b\n    movzbl %r9b, %r9d\n    movsd .LCFP_0(%rip), %xmm4\n    testb %r9b, %r9b\n    je .Lexit\n    movl $1, %eax\n    addsd %xmm4, %xmm0\n    ret\n.Lexit:\n    xorl %eax, %eax\n    addsd %xmm4, %xmm0\n    ret",
        ));
        assert!(out.contains("jge .Lexit"), "{}", out);
        assert!(out.contains("movsd .LCFP_0(%rip), %xmm4"), "{}", out);
        assert!(!out.contains("setl"), "{}", out);
        assert!(!out.contains("testb"), "{}", out);
    }

    #[test]
    fn a_direct_byte_test_without_relay_fuses() {
        // No extension at all: `testb` reads exactly the setCC byte.
        let out = run(&f(
            "    cmpq %rax, %rbx\n    setb %cl\n    testb %cl, %cl\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jb .Lx"), "{}", out);
        assert!(!out.contains("setb"), "{}", out);
    }

    #[test]
    fn a_relay_to_a_different_family_fuses() {
        // RA-homed boolean: setCC %cl widened into %r10, tested there.
        // (%r10 is caller-saved: a callee-saved relay would be live at
        // `ret` via RET_LIVE and correctly refused.)
        let out = run(&f(
            "    cmpq %rax, %rbx\n    setb %cl\n    movzbq %cl, %r10\n    testq %r10, %r10\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jb .Lx"), "{}", out);
        assert!(!out.contains("setb"), "{}", out);
        assert!(!out.contains("testq %r10"), "{}", out);
    }

    #[test]
    fn a_width_variant_relay_test_fuses() {
        // The relayed value is 0/1, so a 32-bit test of a 64-bit relay
        // agrees with the boolean.
        let out = run(&f(
            "    cmpq %rax, %rbx\n    setb %cl\n    movzbq %cl, %r10\n    testl %r10d, %r10d\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jb .Lx"), "{}", out);
        assert!(!out.contains("setb"), "{}", out);
    }

    #[test]
    fn a_boolean_spill_and_reload_still_fuses() {
        // The classic boolean-slot round-trip: carrier-stored, carrier-
        // reloaded within its width, then tested. Legacy-pure, ungated.
        let out = run(&f(
            "    cmpl %eax, %ebx\n    setl %al\n    movzbq %al, %rax\n    movq %rax, 8(%rsp)\n    movq 8(%rsp), %rax\n    testq %rax, %rax\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jl .Lx"), "{}", out);
        assert!(!out.contains("setl"), "{}", out);
    }

    #[test]
    fn a_non_carrier_load_is_preserved_not_deleted() {
        // An unrelated stack reload between the materialization and its test
        // must SURVIVE (the old code NOPed it). The slot stays readable in
        // the output (preserved verbatim or forwarded — either keeps the
        // value live).
        let out = run(&f(
            "    cmpl %eax, %ebx\n    setl %al\n    movzbq %al, %rax\n    movq 8(%rsp), %rbx\n    testq %rax, %rax\n    jne .Lx\n    movq %rbx, %rax\n    ret\n.Lx:\n    movq %rbx, %rax\n    ret",
        ));
        assert!(out.contains("jl .Lx"), "{}", out);
        assert!(!out.contains("setl"), "{}", out);
        assert!(out.contains("8(%rsp)"), "{}", out);
    }

    #[test]
    fn a_same_carrier_widening_is_plumbing() {
        // `movslq %r9d, %r9` on the 0/1 carrier is the identity: skipped and
        // NOPed, and the fusion proceeds.
        let out = run(&f(
            "    cmpl $5000000, 24(%rsp)\n    setl %r9b\n    movzbl %r9b, %r9d\n    movslq %r9d, %r9\n    testq %r9, %r9\n    je .Lexit\n    movl $1, %eax\n    ret\n.Lexit:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jge .Lexit"), "{}", out);
        assert!(!out.contains("movslq"), "{}", out);
        assert!(!out.contains("setl"), "{}", out);
    }

    // ── B. negatives: legality rules block ────────────────────────────────

    #[test]
    fn b_high_byte_aliasing_never_fuses() {
        // `%ah` aliases family 0: `movzbl %ah, %eax` after `setl %al` reads
        // a stale byte, not the boolean. Exact-spelling matching refuses.
        let out = run(&f(
            "    cmpq %rax, %rbx\n    setl %al\n    movzbl %ah, %eax\n    testl %eax, %eax\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jne .Lx"), "{}", out);
        assert!(out.contains("setl %al"), "{}", out);
    }

    #[test]
    fn b_carrier_clobber_between_extension_and_test_refuses() {
        // `movq %rbx, %r9` redefines the carrier with a foreign value; the
        // test then reads that value, not the boolean. Post-scan touch
        // check refuses (even though the clobber is transparent-eligible).
        let out = run(&f(
            "    cmpl $5000000, 24(%rsp)\n    setl %r9b\n    movzbl %r9b, %r9d\n    movq %rbx, %r9\n    testb %r9b, %r9b\n    je .Lexit\n    movl $1, %eax\n    ret\n.Lexit:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("je .Lexit"), "{}", out);
        assert!(out.contains("setl %r9b"), "{}", out);
    }

    #[test]
    fn b_live_carrier_after_the_jump_refuses() {
        // The NOPed definitions would be observed: %r9 is read on the
        // fall-through path, so the liveness gate refuses.
        let out = run(&f(
            "    cmpl $5000000, 24(%rsp)\n    setl %r9b\n    movzbl %r9b, %r9d\n    movsd .LCFP_0(%rip), %xmm4\n    testb %r9b, %r9b\n    je .Lexit\n    movq %r9, %rax\n    ret\n.Lexit:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("je .Lexit"), "{}", out);
        assert!(out.contains("setl %r9b"), "{}", out);
    }

    #[test]
    fn b_extensionless_wide_test_refuses() {
        // Without an extension, `testl %r9d` reads stale upper bits above
        // the setCC byte — not the boolean. Only `testb` may match directly.
        let out = run(&f(
            "    cmpq %rax, %rbx\n    setl %r9b\n    testl %r9d, %r9d\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jne .Lx"), "{}", out);
        assert!(out.contains("setl %r9b"), "{}", out);
    }

    #[test]
    fn b_foreign_stack_reload_of_the_carrier_refuses() {
        // `movq 8(%rsp), %rax` redefines the carrier from a slot the scan
        // never saw stored: a foreign value, not the boolean. (The old code
        // skipped — and NOPed — this load.)
        let out = run(&f(
            "    cmpl %eax, %ebx\n    setl %al\n    movzbq %al, %rax\n    movq 8(%rsp), %rax\n    testq %rax, %rax\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jne .Lx"), "{}", out);
        assert!(out.contains("setl %al"), "{}", out);
    }

    #[test]
    fn b_foreign_sign_extension_of_the_carrier_refuses() {
        // `movslq (%rdi), %rax` redefines the carrier from memory: not the
        // boolean. (The old code skipped — and NOPed — ANY movslq, deleting
        // this load.)
        let out = run(&f(
            "    cmpl %eax, %ebx\n    setl %al\n    movzbq %al, %rax\n    movslq (%rdi), %rax\n    testq %rax, %rax\n    jne .Lx\n    movl $1, %eax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jne .Lx"), "{}", out);
        assert!(out.contains("setl %al"), "{}", out);
    }

    #[test]
    fn b_live_relay_registers_refuse() {
        // The historical relay gate mapped the %-less relay text through
        // `register_family_fast` (which requires `%`), evaluated to
        // REG_NONE, and NEVER fired: `setb %al; movzbq %al, %r10` fused with
        // %r10 live. The real gate refuses. (%r10 is caller-saved, so the
        // refusal comes from the explicit read below, not RET_LIVE.)
        let out = run(&f(
            "    cmpq %rax, %rbx\n    setb %al\n    movzbq %al, %r10\n    testq %r10, %r10\n    jne .Lx\n    movq %r10, %rax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jne .Lx"), "{}", out);
        assert!(out.contains("setb %al"), "{}", out);
    }

    #[test]
    fn b_transparent_line_touching_the_boolean_slot_refuses() {
        // A preserved non-carrier load of the boolean slot would observe the
        // NOPed store's absence. The gate passes here (rax is dead); only
        // the slot-touch rule refuses.
        let out = run(&f(
            "    cmpl %eax, %ebx\n    setl %al\n    movzbq %al, %rax\n    movq %rax, 8(%rsp)\n    movq 8(%rsp), %rbx\n    movq 8(%rsp), %rax\n    testq %rax, %rax\n    jne .Lx\n    movq %rbx, %rdi\n    movq %rdi, %rax\n    ret\n.Lx:\n    xorl %eax, %eax\n    ret",
        ));
        assert!(out.contains("jne .Lx"), "{}", out);
        assert!(out.contains("setl %al"), "{}", out);
    }
}

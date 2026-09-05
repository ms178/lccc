//! Compare-and-branch fusion pass.
//!
//! Fuses cmp + setCC + test + jCC sequences into a single conditional jump,
//! eliminating the boolean materialization overhead from the codegen model.

use super::super::types::*;
use super::liveness::FileLiveness;

/// Maximum number of store/load offsets tracked during compare-and-branch fusion.
const MAX_TRACKED_STORE_LOAD_OFFSETS: usize = 4;

/// Size of the instruction lookahead window for compare-and-branch fusion.
const CMP_FUSION_LOOKAHEAD: usize = 8;

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

/// Parse `testq %rX, %rX` / `testl %eXd, %eXd` / `testb %rXb, %rXb` as a
/// single-register test of a 64/32/8-bit register name. Returns the register
/// text.
fn parse_self_test(t: &str) -> Option<&str> {
    for (mnem, w) in [("testq %", 1), ("testl %", 1), ("testb %", 1)] {
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

        // Second must be setCC
        if !matches!(infos[seq_indices[1]].kind, LineKind::SetCC { .. }) {
            i += 1;
            continue;
        }
        let set_line = infos[seq_indices[1]].trimmed(store.get(seq_indices[1]));
        let cc = match parse_setcc(set_line) {
            Some(c) => c,
            None => {
                i += 1;
                continue;
            }
        };

        // Scan for testq %rax, %rax pattern.
        // Track StoreRbp offsets so we can bail out if any store's slot is
        // potentially read by another basic block (no matching load nearby).
        let mut test_idx = None;
        // Final carrier of the boolean: %rax/%eax (legacy, corpus-validated
        // contract) or a register-relay destination (%rX — NEW capability,
        // gated on exact liveness below).
        let mut relay_reg: Option<String> = None;
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

            // Relay hop of the setcc result: `movzbq %al, %rax` /
            // `movzbl %al, %eax` (legacy rax carrier, corpus-validated) or a
            // register relay `movzbq %al, %rX` / `movzbl %al, %rXd` (RA-homed
            // booleans). A later test MUST be of the relay's destination —
            // a test of a different register is a DIFFERENT value (zlib-ng
            // zng_deflateSetParams: size<4's setb was fused with the test of
            // a slotted `new_strategy`, so a 4-byte buffer became
            // Z_BUF_ERROR).
            if line == "movzbq %al, %rax" || line == "movzbl %al, %eax" {
                scan += 1;
                continue;
            }
            if let Some(dst) = line
                .strip_prefix("movzbq %al, %")
                .or_else(|| line.strip_prefix("movzbl %al, %"))
            {
                if !dst.contains(',') && !dst.contains('(') {
                    relay_reg = Some(dst.trim().to_string());
                    scan += 1;
                    continue;
                }
            }
            // Skip store/load to rbp (pre-parsed fast check).
            if let LineKind::StoreRbp { offset, .. } = infos[si].kind {
                if store_count < MAX_TRACKED_STORE_LOAD_OFFSETS {
                    if let Some(slot) = parse_stack_operand(line, false) {
                        store_offsets[store_count] = slot;
                        store_count += 1;
                    } else {
                        store_count = usize::MAX;
                        break;
                    }
                } else {
                    store_count = usize::MAX;
                    break;
                }
                scan += 1;
                continue;
            }
            if matches!(infos[si].kind, LineKind::LoadRbp { .. })
                || ((line.starts_with("movsbq ")
                    || line.starts_with("movswq ")
                    || line.starts_with("movzbq ")
                    || line.starts_with("movzwq ")
                    || line.starts_with("movsbl ")
                    || line.starts_with("movzbl "))
                    && parse_stack_operand(line, true).is_some())
            {
                scan += 1;
                continue;
            }
            if line == "cltq" || line.starts_with("movslq ") {
                scan += 1;
                continue;
            }
            // Check for test: legacy rax carrier (ONLY when no non-rax
            // relay intervenes — a rax test after `movzbl %al, %r12d` tests
            // a DIFFERENT value), or a test of the relay's destination
            // register.
            if relay_reg.is_none() && (line == "testq %rax, %rax" || line == "testl %eax, %eax") {
                test_idx = Some(scan);
                break;
            }
            if let Some(relay) = relay_reg.clone() {
                if line == format!("testq %{}, %{}", relay, relay)
                    || line == format!("testl %{}, %{}", relay, relay)
                {
                    test_idx = Some(scan);
                    break;
                }
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

        // NEW capability gate: a non-rax carrier relay is only sound when
        // BOTH the carrier register and %rax are dead after the fused jump
        // (their definitions are being NOPed; a live later reader would see
        // stale values). The legacy rax path keeps its corpus-validated
        // contract (no deadness check; the relay lands in %rax whose byte
        // the setCC wrote and the test consumed).
        let relay_fam = relay_reg
            .as_deref()
            .map(register_family_fast)
            .filter(|&f| f != REG_NONE);
        if let Some(fam) = relay_fam {
            let mut lv = FileLiveness::new(store, infos);
            let jcc_pos = seq_indices[test_scan + 1];
            let relay_dead = lv.live_after(jcc_pos, fam) == Some(false);
            let al_dead = lv.live_after(jcc_pos, 0) == Some(false);
            if !relay_dead || !al_dead {
                if std::env::var_os("CCC_DEBUG_CMP_FUSE").is_some() {
                    eprintln!(
                        "[CMPFUSE] refusing relay carrier {:?} (relay_dead={} al_dead={})",
                        relay_reg, relay_dead, al_dead
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

        // NOP out everything from setCC through testq
        for s in 1..=test_scan {
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

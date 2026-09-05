//! Global store forwarding pass.
//!
//! Tracks register→slot mappings across the function, forwarding stored values
//! to subsequent loads. At a label reached only by fallthrough (not a jump
//! target), register state from the previous instruction is fully known,
//! so we can safely forward across such labels.
//!
//! For labels that ARE jump targets, all mappings are invalidated because the
//! jump source may have different register values.

use super::super::types::*;
use super::helpers::*;

// ── Data structures ──────────────────────────────────────────────────────────

/// A tracked store mapping: we know that stack slot at `offset` contains the
/// value that was in register `reg_id` with the given `size`.
#[derive(Clone, Copy)]
struct SlotMapping {
    reg_id: RegId,
    size: MoveSize,
}

/// A slot entry for flat-array store forwarding.
#[derive(Clone, Copy)]
struct SlotEntry {
    offset: i32,
    mapping: SlotMapping,
    active: bool,
}

/// Small inline vector for register->offset tracking (avoids heap allocation
/// for the common case of <=4 offsets per register).
#[derive(Clone, Default)]
struct SmallVec {
    inline: [i32; 4],
    len: u8,
    overflow: Option<Vec<i32>>,
}

impl SmallVec {
    #[inline]
    fn push(&mut self, val: i32) {
        if let Some(ref mut ov) = self.overflow {
            ov.push(val);
        } else if (self.len as usize) < 4 {
            self.inline[self.len as usize] = val;
            self.len += 1;
        } else {
            let mut v = Vec::with_capacity(8);
            v.extend_from_slice(&self.inline[..4]);
            v.push(val);
            self.overflow = Some(v);
        }
    }

    #[inline]
    fn clear(&mut self) {
        self.len = 0;
        self.overflow = None;
    }

    #[inline]
    fn remove_val(&mut self, val: i32) {
        if let Some(ref mut ov) = self.overflow {
            ov.retain(|&v| v != val);
        } else {
            let n = self.len as usize;
            for j in 0..n {
                if self.inline[j] == val {
                    self.inline[j] = self.inline[n - 1];
                    self.len -= 1;
                    return;
                }
            }
        }
    }

    #[inline]
    fn iter(&self) -> SmallVecIter<'_> {
        SmallVecIter { sv: self, idx: 0 }
    }
}

struct SmallVecIter<'a> {
    sv: &'a SmallVec,
    idx: usize,
}

impl<'a> Iterator for SmallVecIter<'a> {
    type Item = i32;
    #[inline]
    fn next(&mut self) -> Option<i32> {
        if let Some(ref ov) = self.sv.overflow {
            if self.idx < ov.len() {
                let v = ov[self.idx];
                self.idx += 1;
                Some(v)
            } else {
                None
            }
        } else if self.idx < self.sv.len as usize {
            let v = self.sv.inline[self.idx];
            self.idx += 1;
            Some(v)
        } else {
            None
        }
    }
}

/// Jump target analysis result for global store forwarding.
struct JumpTargets {
    is_jump_target: Vec<bool>,
    has_non_numeric_jump_targets: bool,
}

// ── State management helpers ─────────────────────────────────────────────────

/// Clear all slot→register mappings.
#[inline]
fn invalidate_all_mappings(slot_entries: &mut Vec<SlotEntry>, reg_offsets: &mut [SmallVec; 16]) {
    slot_entries.clear();
    for rs in reg_offsets.iter_mut() {
        rs.clear();
    }
}

/// Deactivate a single slot entry and remove its offset from the per-register tracking.
#[inline]
fn deactivate_entry(entry: &mut SlotEntry, reg_offsets: &mut [SmallVec; 16]) {
    let old_reg = entry.mapping.reg_id;
    entry.active = false;
    reg_offsets[old_reg as usize].remove_val(entry.offset);
}

/// Invalidate slot mappings at a given offset.
fn invalidate_slots_at(
    slot_entries: &mut [SlotEntry],
    reg_offsets: &mut [SmallVec; 16],
    offset: i32,
    access_size: i32,
) {
    for entry in slot_entries.iter_mut().filter(|e| e.active) {
        let hit = if access_size == 0 {
            entry.offset == offset
        } else {
            ranges_overlap(
                offset,
                access_size,
                entry.offset,
                entry.mapping.size.byte_size(),
            )
        };
        if hit {
            deactivate_entry(entry, reg_offsets);
        }
    }
}

/// Remove all slot mappings backed by a given register (flat array version).
fn invalidate_reg_flat(
    slot_entries: &mut [SlotEntry],
    reg_offsets: &mut [SmallVec; 16],
    reg_id: RegId,
) {
    let offsets = &reg_offsets[reg_id as usize];
    for offset in offsets.iter() {
        for entry in slot_entries.iter_mut().rev() {
            if entry.active && entry.offset == offset && entry.mapping.reg_id == reg_id {
                entry.active = false;
                break;
            }
        }
    }
    reg_offsets[reg_id as usize].clear();
}

// ── Jump target collection ───────────────────────────────────────────────────

fn collect_jump_targets(store: &LineStore, infos: &[LineInfo], len: usize) -> JumpTargets {
    let mut max_label_num: u32 = 0;
    for i in 0..len {
        if infos[i].kind == LineKind::Label {
            let trimmed = infos[i].trimmed(store.get(i));
            if let Some(n) = parse_label_number(trimmed) {
                if n > max_label_num {
                    max_label_num = n;
                }
            }
        }
    }
    let mut is_jump_target = vec![false; (max_label_num + 1) as usize];
    let mut has_non_numeric_jump_targets = false;
    let mut has_indirect_jump = false;
    for i in 0..len {
        match infos[i].kind {
            LineKind::Jmp | LineKind::CondJmp => {
                let trimmed = infos[i].trimmed(store.get(i));
                if let Some(target) = extract_jump_target(trimmed) {
                    if let Some(n) = parse_dotl_number(target) {
                        if (n as usize) < is_jump_target.len() {
                            is_jump_target[n as usize] = true;
                        }
                    } else {
                        has_non_numeric_jump_targets = true;
                    }
                }
            }
            LineKind::JmpIndirect => {
                has_indirect_jump = true;
            }
            _ => {}
        }
    }
    if has_indirect_jump {
        for v in is_jump_target.iter_mut() {
            *v = true;
        }
        has_non_numeric_jump_targets = true;
    }
    JumpTargets {
        is_jump_target,
        has_non_numeric_jump_targets,
    }
}

// ── Per-instruction handlers ─────────────────────────────────────────────────

fn gsf_handle_label(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    targets: &JumpTargets,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut [SmallVec; 16],
    prev_was_unconditional_jump: bool,
) {
    let label_name = infos[i].trimmed(store.get(i));
    let is_target = if let Some(n) = parse_label_number(label_name) {
        (n as usize) < targets.is_jump_target.len() && targets.is_jump_target[n as usize]
    } else {
        targets.has_non_numeric_jump_targets
    };
    if prev_was_unconditional_jump || is_target {
        // A label with a non-fallthrough predecessor is a CFG merge.  ABI
        // callee-saved status says nothing about values assigned to that
        // register on different *intra-function* paths: a loop back-edge can
        // overwrite %rbx/r12-r15 while an entry edge still carries an older
        // slot->register equality.  Retaining that equality would rewrite a
        // stack reload on every later iteration to the back-edge's unrelated
        // register value.  Until this text pass has path-sensitive join-state
        // intersection, only a proven fallthrough label may retain mappings.
        invalidate_all_mappings(slot_entries, reg_offsets);
    }
}

fn gsf_handle_store(
    reg: RegId,
    offset: i32,
    size: MoveSize,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut [SmallVec; 16],
) {
    invalidate_slots_at(slot_entries, reg_offsets, offset, size.byte_size());
    if is_valid_gp_reg(reg) {
        slot_entries.push(SlotEntry {
            offset,
            mapping: SlotMapping { reg_id: reg, size },
            active: true,
        });
        reg_offsets[reg as usize].push(offset);
    }
    if slot_entries.len() > 64 {
        slot_entries.retain(|e| e.active);
    }
}

fn gsf_handle_load(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    load_reg: RegId,
    load_offset: i32,
    load_size: MoveSize,
    slot_entries: &mut [SlotEntry],
    reg_offsets: &mut [SmallVec; 16],
) -> bool {
    let mut changed = false;
    let mapping = slot_entries
        .iter()
        .rev()
        .find(|e| e.active && e.offset == load_offset)
        .map(|e| e.mapping);
    if let Some(mapping) = mapping {
        // A qword store followed by a dword load is also forwardable: x86
        // `movl` observes exactly the low 32 bits and zero-extends its
        // destination.  Keep the forwarded move at the LOAD width; turning a
        // same-family Q->L load into a no-op would be wrong because the movl
        // zero-extension is architecturally observable.
        let exact_width = mapping.size == load_size;
        let qword_to_dword = mapping.size == MoveSize::Q && load_size == MoveSize::L;
        if (exact_width || qword_to_dword) && mapping.reg_id != REG_NONE {
            let is_epilogue_restore = matches!(load_reg, 3 | 12 | 13 | 14 | 15)
                && load_offset < 0
                && is_near_epilogue(infos, i);
            if exact_width && load_reg == mapping.reg_id && !is_epilogue_restore {
                mark_nop(&mut infos[i]);
                changed = true;
            } else if load_reg != REG_NONE {
                let store_reg_str = reg_id_to_name(mapping.reg_id, load_size);
                let load_reg_str = reg_id_to_name(load_reg, load_size);
                let new_text = format!(
                    "    {} {}, {}",
                    load_size.mnemonic(),
                    store_reg_str,
                    load_reg_str
                );
                replace_line(store, &mut infos[i], i, new_text);
                changed = true;
            }
        }
    }
    if is_valid_gp_reg(load_reg) {
        invalidate_reg_flat(slot_entries, reg_offsets, load_reg);
    }
    changed
}

fn gsf_handle_other(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    dest_reg: RegId,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut [SmallVec; 16],
    rbp_is_frame: bool,
) {
    // SOUNDNESS: if rbp is NOT the frame pointer, any %rbp reference in an
    // Other instruction is a pointer dereference / address computation that may
    // read or write arbitrary memory. Invalidate ALL mappings so a stack slot is
    // never forwarded across a potentially-aliasing pointer operation.
    if !rbp_is_frame && infos[i].reg_refs & (1u16 << 5) != 0 {
        invalidate_all_mappings(slot_entries, reg_offsets);
    }

    if is_valid_gp_reg(dest_reg) {
        invalidate_reg_flat(slot_entries, reg_offsets, dest_reg);
        if dest_reg == 0 {
            let trimmed = infos[i].trimmed(store.get(i));
            if trimmed.starts_with("div")
                || trimmed.starts_with("idiv")
                || trimmed.starts_with("mul")
                || trimmed == "cqto"
                || trimmed == "cqo"
                || trimmed == "cdq"
            {
                invalidate_reg_flat(slot_entries, reg_offsets, 2);
            }
        }
    }

    if dest_reg == REG_NONE && infos[i].rbp_offset != RBP_OFFSET_NONE {
        invalidate_slots_at(slot_entries, reg_offsets, infos[i].rbp_offset, 0);
    }

    if infos[i].has_indirect_mem {
        invalidate_all_mappings(slot_entries, reg_offsets);
    } else if infos[i].rbp_offset != RBP_OFFSET_NONE {
        // A folded memory operand is a RANGE access, not a point: x87
        // `fstpt -24(%rbp)` writes 10 bytes, `movdqu` 16. Treating it as
        // 1 byte kept forwarded mappings for the neighboring slots alive
        // across a wide store (the i686 twin miscompiled spectral_norm
        // exactly this way). 16 bytes covers every x86 access width.
        invalidate_slots_at(slot_entries, reg_offsets, infos[i].rbp_offset, 16);
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Returns true if the given line's base register is `%rbp`.
/// Used to decide whether a StoreRbp/LoadRbp line is a genuine stack-slot
/// access (base %rsp, or base %rbp with rbp as frame pointer) vs a pointer
/// dereference (base %rbp with rbp as a data register under -fomit-frame-pointer).
fn line_base_is_rbp(trimmed: &str) -> bool {
    // Look for a parenthesized memory operand containing "%rbp" (e.g. "(%rbp)",
    // "8(%rbp)", "(%rbp,%rax,4)"). A bare register move "movq %rax, %rbp" has no
    // parenthesized operand and returns false.
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let start = i;
            let mut depth = 1;
            let mut j = i + 1;
            while j < bytes.len() && depth > 0 {
                if bytes[j] == b'(' {
                    depth += 1;
                } else if bytes[j] == b')' {
                    depth -= 1;
                }
                j += 1;
            }
            if trimmed[start..j].contains("%rbp") {
                return true;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    false
}

pub(super) fn global_store_forwarding(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = store.len();
    if len == 0 {
        return false;
    }

    // SOUNDNESS: if rbp is NOT the frame pointer (e.g. -fomit-frame-pointer
    // with rbp used as a general register), then `offset(%rbp)` accesses are
    // pointer dereferences. We must not store-forward them as stack slots, and
    // they must be treated as opaque indirect memory (alias anything).
    // Track per-function: reset at each .cfi_startproc; set true at
    // `movq %rsp, %rbp` (frame-pointer establishment).
    let mut rbp_is_frame = false;

    let jump_targets = collect_jump_targets(store, infos, len);

    let mut slot_entries: Vec<SlotEntry> = Vec::new();
    let mut reg_offsets: [SmallVec; 16] = Default::default();
    let mut changed = false;
    let mut prev_was_unconditional_jump = false;

    for i in 0..len {
        if infos[i].is_nop() || infos[i].kind == LineKind::Empty {
            continue;
        }

        // Per-function frame-pointer tracking.
        if infos[i].kind == LineKind::Directive {
            let dt = infos[i].trimmed(store.get(i));
            if dt == ".cfi_startproc" {
                rbp_is_frame = false;
            }
        } else if matches!(infos[i].kind, LineKind::Other { .. }) {
            let ot = infos[i].trimmed(store.get(i));
            if ot == "movq %rsp, %rbp" || ot == "movl %esp, %ebp" {
                rbp_is_frame = true;
            }
        }

        let was_uncond_jump = prev_was_unconditional_jump;
        prev_was_unconditional_jump = false;

        match infos[i].kind {
            LineKind::Label => {
                gsf_handle_label(
                    store,
                    infos,
                    i,
                    &jump_targets,
                    &mut slot_entries,
                    &mut reg_offsets,
                    was_uncond_jump,
                );
            }

            LineKind::StoreRbp { reg, offset, size } => {
                // If this is a pointer dereference (rbp not the frame pointer),
                // it may write ANY memory, so invalidate all mappings and do not
                // record a stack-slot mapping.
                if !rbp_is_frame && line_base_is_rbp(infos[i].trimmed(store.get(i))) {
                    invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
                } else {
                    gsf_handle_store(reg, offset, size, &mut slot_entries, &mut reg_offsets);
                }
            }

            LineKind::LoadRbp {
                reg: load_reg,
                offset: load_offset,
                size: load_size,
            } => {
                // A pointer-deref load (rbp not the frame pointer) must not be
                // forwarded from a slot mapping.
                if !rbp_is_frame && line_base_is_rbp(infos[i].trimmed(store.get(i))) {
                    // Treat as opaque read; still invalidate the dest register
                    // mapping for the loaded reg.
                    if is_valid_gp_reg(load_reg) {
                        invalidate_reg_flat(&mut slot_entries, &mut reg_offsets, load_reg);
                    }
                } else {
                    changed |= gsf_handle_load(
                        store,
                        infos,
                        i,
                        load_reg,
                        load_offset,
                        load_size,
                        &mut slot_entries,
                        &mut reg_offsets,
                    );
                }
            }

            LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret => {
                invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
                prev_was_unconditional_jump = true;
            }

            LineKind::Call => {
                // SOUND FIX: a function call may write through a pointer that
                // aliases ANY stack slot whose address escaped this function
                // (a local passed by reference, or an alloca). Forwarding a
                // stack-slot store across a call would then produce a stale
                // value. Invalidate ALL slot mappings on a call, not just the
                // caller-saved registers. (Registers too, conservatively.)
                invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
            }

            // SOUNDNESS: user inline assembly is opaque — its template may
            // write ANY register (the emitter substitutes operands for `%0`,
            // `%1`, ... so a template output can overwrite a register whose
            // slot→register equality this pass is holding) and may store
            // through ANY pointer (an "m" operand, or a register the template
            // dereferences). Both halves of every tracked mapping die here.
            // Falling through to `_ => {}` instead forwarded a stale register
            // across an asm block that redefined it: `movq %rcx, 32(%rsp)`
            // followed by an asm block reusing %rcx as its output register,
            // then `movq 32(%rsp), %r11` — rewritten to `movq %rcx, %r11`,
            // feeding the asm's fresh output where the pre-asm value was
            // required (observed as a framecall_1 miscompile from the
            // peephole_families harness).
            LineKind::InlineAsm => {
                invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
            }

            // SOUNDNESS: push/pop shift RSP, so every %rsp-relative slot
            // offset in the shifted window refers to a different physical
            // slot. Any mapping recorded before the push is stale after it;
            // invalidate everything (conservative).
            LineKind::Push { .. } | LineKind::Pop { .. } => {
                invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
            }

            LineKind::SetCC { reg } => {
                if is_valid_gp_reg(reg) {
                    invalidate_reg_flat(&mut slot_entries, &mut reg_offsets, reg);
                }
            }

            LineKind::Other { dest_reg } => {
                gsf_handle_other(
                    store,
                    infos,
                    i,
                    dest_reg,
                    &mut slot_entries,
                    &mut reg_offsets,
                    rbp_is_frame,
                );
            }

            LineKind::CondJmp => {
                // Keep state on the *linear fall-through* edge. The following
                // instruction is reached only when this branch falls through;
                // a taken edge always enters through a label, where
                // gsf_handle_label invalidates mappings at the merge. Clearing
                // here loses a provably local store-to-load forwarding win.
            }

            LineKind::Cmp | LineKind::Directive => {}

            // A scalar-FP / SSE store rewrites frame bytes: every GP mapping
            // whose slot range intersects it is stale. When %rbp is a data
            // register the `(%rbp)` form is an opaque pointer write. (These
            // lines were `Other` before the XMM slot kinds existed and were
            // range-invalidated through `gsf_handle_other`; falling into
            // `_ => {}` silently kept stale GP mappings alive.)
            LineKind::StoreXmmRbp { offset, size } => {
                if !rbp_is_frame && infos[i].reg_refs & (1u16 << 5) != 0 {
                    invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
                } else {
                    invalidate_slots_at(&mut slot_entries, &mut reg_offsets, offset, size.byte_size());
                }
            }
            // An XMM load writes no GP register and no memory: mappings stay
            // valid (a later GP reload of the same slot may still forward).
            LineKind::LoadXmmRbp { .. } => {}

            _ => {}
        }
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::peephole_common::LineStore;

    /// Build line infos with inline-asm regions pinned exactly like the
    /// peephole driver's `pin_inline_asm_regions` does.
    fn build_pinned(asm: &str) -> (LineStore, Vec<LineInfo>) {
        let store = LineStore::new(asm.to_string());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let mut in_asm = false;
        for i in 0..infos.len() {
            let t = infos[i].trimmed(store.get(i)).to_string();
            if t == "#APP" {
                in_asm = true;
                continue;
            }
            if t == "#NO_APP" {
                in_asm = false;
                continue;
            }
            if in_asm {
                infos[i] = LineInfo {
                    kind: LineKind::InlineAsm,
                    ext_kind: ExtKind::None,
                    trim_start: infos[i].trim_start,
                    has_indirect_mem: true,
                    rbp_offset: RBP_OFFSET_NONE,
                    reg_refs: u16::MAX,
                    pinned: true,
                };
            }
        }
        (store, infos)
    }

    #[test]
    fn inline_asm_output_register_kills_slot_forwarding() {
        // The exact framecall_1 shape: a spill of %rcx, an asm block whose
        // output operand lands in %rcx, then a reload of the slot. The load
        // must survive verbatim — forwarding it to %rcx would read the asm's
        // fresh output where the pre-asm value is required.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rcx, 32(%rsp)\n",
            "#APP\n",
            "    movq %rdx, %rcx\n",
            "    addq $3, %rcx\n",
            "#NO_APP\n",
            "    movq 32(%rsp), %r11\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        let reload_idx = (0..store.len())
            .find(|&i| store.get(i).contains("movq 32(%rsp), %r11"))
            .expect("slot reload must survive");
        assert!(
            !infos[reload_idx].is_nop(),
            "the slot reload must not be rewritten to the asm output register"
        );
    }

    #[test]
    fn forwarding_within_plain_straight_line_still_works() {
        // Same shape without the asm region: the load IS forwardable.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rcx, 32(%rsp)\n",
            "    movl %edx, %edx\n",
            "    movq 32(%rsp), %r11\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movq %rcx, %r11")),
            "plain straight-line forwarding must still fire"
        );
    }
}

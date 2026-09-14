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
use super::fp_liveness::FpLiveness;
use super::helpers::*;
use super::liveness::FileLiveness;

// ── Data structures ──────────────────────────────────────────────────────────

/// Per-register slot-home tracking (fast invalidation index).
///
/// Indices `0..16` are the GP families (`%rax`..`%r15`). Indices `24..40`
/// are the XMM families (`%xmm0`..`%xmm15`, the `24..=39` id space of
/// [`is_xmm_family`]) — the scalar-FP store→load forwarding added to this
/// pass needs exactly the same "which slots hold this register's value"
/// bookkeeping for XMM homes as the GP side always had. Indices `16..24`
/// are unused (no register family maps there).
type RegHomes = [SmallVec; 40];

/// A tracked store mapping: we know that stack slot at `offset` contains the
/// value that was in register `reg_id` with the given `size`, or — when
/// `reg_id == REG_NONE` — the immediate `imm` that was stored (an
/// `movX $imm, slot` line). Immediate-backed mappings are immune to
/// register clobbers: memory does not change when a register does.
///
/// `reg_id` may also be an XMM family id (`24..=39`, see [`is_xmm_family`])
/// for scalar-FP stores (`movsd|movss|movq|movd %xmmN, slot`): the slot then
/// holds that XMM register's low `size.byte_size()` bits. XMM-backed
/// mappings die when the backing XMM is rewritten (any line whose AT&T
/// destination is that XMM), at calls (all sixteen XMM registers are
/// caller-saved under SysV), and at every control-flow merge — exactly the
/// GP invalidation model, through the shared [`RegHomes`] index.
#[derive(Clone, Copy)]
struct SlotMapping {
    reg_id: RegId,
    size: MoveSize,
    /// Valid iff `reg_id == REG_NONE` (immediate store).
    imm: i64,
    /// The stored value's bits 32..63 were provably ZERO at store time
    /// (the backing register's last write was a 32-bit operation, which
    /// zero-extends architecturally). Valid iff `reg_id != REG_NONE`.
    /// This is what makes a `movq %rX, slot` → `movl slot, %rXd` round
    /// trip a no-op: the movl observes exactly the low 32 bits and
    /// zero-extends its destination to the value the register already
    /// holds.
    zext: bool,
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
fn invalidate_all_mappings(slot_entries: &mut Vec<SlotEntry>, reg_offsets: &mut RegHomes) {
    slot_entries.clear();
    for rs in reg_offsets.iter_mut() {
        rs.clear();
    }
}

/// Deactivate a single slot entry and remove its offset from the per-register
/// tracking. Immediate-backed entries (reg_id == REG_NONE) have no register
/// bookkeeping to unwind.
#[inline]
fn deactivate_entry(entry: &mut SlotEntry, reg_offsets: &mut RegHomes) {
    if entry.mapping.reg_id != REG_NONE {
        let old_reg = entry.mapping.reg_id;
        reg_offsets[old_reg as usize].remove_val(entry.offset);
    }
    entry.active = false;
}

/// Invalidate slot mappings at a given offset.
fn invalidate_slots_at(
    slot_entries: &mut [SlotEntry],
    reg_offsets: &mut RegHomes,
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
fn invalidate_reg_flat(slot_entries: &mut [SlotEntry], reg_offsets: &mut RegHomes, reg_id: RegId) {
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

/// Architectural zero-extension tracking for the store-forwarding scan.
///
/// `zext[fam]` answers: "are this family's bits 32..63 provably zero on the
/// fallthrough path into the next line?" The ONLY producer is a 32-bit
/// register write — every x86-64 instruction that names a 32-bit destination
/// zero-extends it (Intel SDM vol. 1 §3.4.1.1) — so a store of that register
/// followed by a `movl slot, %fam-d` reload writes exactly the bits the
/// register already holds. Sub-32-bit writes preserve the upper bits (the
/// flag survives); 64-bit and unknown writes clear it; CFG merges, calls,
/// rets, jumps and inline asm clear everything (the state is a per-path
/// fact, exactly like the slot mappings this feeds).
fn gsf_update_zext(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    reg_zext: &mut [bool; 16],
    targets: &JumpTargets,
    prev_was_unconditional_jump: bool,
) {
    match infos[i].kind {
        LineKind::Label => {
            // Mirror gsf_handle_label's merge detection: only a proven
            // fallthrough-only label keeps single-path knowledge.
            let label_name = infos[i].trimmed(store.get(i));
            let is_target = if let Some(n) = parse_label_number(label_name) {
                (n as usize) < targets.is_jump_target.len() && targets.is_jump_target[n as usize]
            } else {
                targets.has_non_numeric_jump_targets
            };
            if prev_was_unconditional_jump || is_target {
                *reg_zext = [false; 16];
            }
        }
        LineKind::Jmp | LineKind::JmpIndirect | LineKind::Ret | LineKind::Call => {
            *reg_zext = [false; 16];
        }
        LineKind::Push { .. } => {}
        LineKind::Pop { reg } => {
            if is_valid_gp_reg(reg) {
                reg_zext[reg as usize] = false;
            }
        }
        LineKind::InlineAsm => *reg_zext = [false; 16],
        _ => {
            let t = infos[i].trimmed(store.get(i));
            // SWAP/CAS instructions write their FIRST operand too (the
            // last-comma destination parse sees only the memory operand of
            // the emitter's `xchg %reg, (mem)` atomics, and cmpxchg
            // conditionally rewrites %rax with the memory value). A stale
            // TRUE on the swapped family would let a later Q→L self-reload
            // be nop'd although the upper half is the swapped-in memory
            // bits. These are rare atomic forms — clear everything.
            if t.starts_with("xchg")
                || t.starts_with("lock xchg")
                || t.starts_with("cmpxchg")
                || t.starts_with("lock cmpxchg")
                || t.starts_with("vzeroall")
            {
                *reg_zext = [false; 16];
                return;
            }
            let dest = get_dest_reg(&infos[i]);
            if is_valid_gp_reg(dest) {
                match dest_write_width(t) {
                    Some(32) => reg_zext[dest as usize] = true,
                    Some(_) => reg_zext[dest as usize] = false,
                    // 8/16-bit (or unparseable) destination: partial or
                    // unknown write; keep the previous flag. Sub-word
                    // writes preserve the upper half, so a stale TRUE flag
                    // stays correct; a stale FALSE flag only loses the
                    // optimization.
                    None => {}
                }
            }
            // Implicit-output families (`cqto` -> rdx, `idiv` -> rax:rdx,
            // `cpuid`, ...): written at unknown width; clear conservatively.
            for fam in 0u8..=REG_GP_MAX {
                if Some(fam) != dest_range_ok(dest)
                    && writes_family(&infos[i], infos[i].trimmed(store.get(i)), fam)
                {
                    reg_zext[fam as usize] = false;
                }
            }
        }
    }
}

/// Option<RegId> adapter: `Some(fam)` for a valid GP family id, `None`
/// otherwise (so the implicit-output loop can compare against the classified
/// destination without a type dance).
fn dest_range_ok(dest: RegId) -> Option<u8> {
    if is_valid_gp_reg(dest) {
        Some(dest)
    } else {
        None
    }
}

/// Width (in bits) of the register a line's trailing operand names, when the
/// line has a plain register destination. `None` for memory/immediate
/// destinations, sub-32-bit names, and unrecognized forms.
fn dest_write_width(trimmed: &str) -> Option<u32> {
    let comma = trimmed.rfind(',')?;
    let dest = trimmed[comma + 1..].trim();
    if dest.len() < 2 || !dest.starts_with('%') {
        return None;
    }
    for (tier, names) in REG_NAMES.iter().enumerate() {
        if names.iter().any(|n| *n == dest) {
            return match tier {
                0 => Some(64),
                1 => Some(32),
                // Sub-word destinations are partial writes: report None so
                // the caller keeps the previous zero-extension flag.
                _ => None,
            };
        }
    }
    None
}

fn gsf_handle_label(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    targets: &JumpTargets,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut RegHomes,
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
    zext: bool,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut RegHomes,
) {
    invalidate_slots_at(slot_entries, reg_offsets, offset, size.byte_size());
    if is_valid_gp_reg(reg) {
        slot_entries.push(SlotEntry {
            offset,
            mapping: SlotMapping {
                reg_id: reg,
                size,
                imm: 0,
                zext,
            },
            active: true,
        });
        reg_offsets[reg as usize].push(offset);
    }
    if slot_entries.len() > 64 {
        slot_entries.retain(|e| e.active);
    }
}

/// Record an immediate store (`movX $imm, off(%rbp)`). The mapping is
/// register-independent: no reg_offsets engagement, and only a store to an
/// overlapping offset can kill it.
fn gsf_handle_store_imm(
    imm: i64,
    offset: i32,
    size: MoveSize,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut RegHomes,
) {
    invalidate_slots_at(slot_entries, reg_offsets, offset, size.byte_size());
    slot_entries.push(SlotEntry {
        offset,
        mapping: SlotMapping {
            reg_id: REG_NONE,
            size,
            imm,
            zext: false,
        },
        active: true,
    });
    if slot_entries.len() > 64 {
        slot_entries.retain(|e| e.active);
    }
}

/// Does `imm` fit the immediate field of a `mov<size> $imm, mem` store?
/// Mirrors the emitter's `direct_store_imm` contract exactly:
/// * Q — `movq $imm32` SIGN-extends, so only [i32::MIN, i32::MAX] is legal;
/// * L — the raw imm32 field covers [i32::MIN, u32::MAX];
/// * W/B — the width's own signed-or-unsigned field range.
fn imm_fits_store(imm: i64, size: MoveSize) -> bool {
    match size {
        MoveSize::Q => (i32::MIN as i64..=i32::MAX as i64).contains(&imm),
        MoveSize::L => (i32::MIN as i64..=u32::MAX as i64).contains(&imm),
        MoveSize::W => (i16::MIN as i64..=u16::MAX as i64).contains(&imm),
        MoveSize::B => (i8::MIN as i64..=u8::MAX as i64).contains(&imm),
        // FP/SSE moves carry no immediate: an immediate-backed mapping can
        // never be created for them (parse_imm_slot_store only matches the
        // integer mov mnemonics).
        MoveSize::SLQ | MoveSize::SD | MoveSize::SS => false,
    }
}

/// Store mnemonic for a MoveSize with an AT&T immediate source.
fn mov_imm_store_mnemonic(size: MoveSize) -> &'static str {
    match size {
        MoveSize::Q => "movq",
        MoveSize::L => "movl",
        MoveSize::W => "movw",
        MoveSize::B => "movb",
        MoveSize::SLQ | MoveSize::SD | MoveSize::SS => "movq",
    }
}

/// Parse the first `%xmmN` operand in `text` into its XMM family id
/// (`24..=39`, the space [`is_xmm_family`] defines). Lines classified as
/// `StoreXmmRbp`/`LoadXmmRbp` carry exactly one XMM operand by construction
/// (the other end of the move is the frame slot), so "first" is also "the".
fn parse_xmm_family(text: &str) -> Option<RegId> {
    let mut rest = text;
    while let Some(pos) = rest.find("%xmm") {
        let after = &rest[pos + 4..];
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        if end > 0 {
            if let Ok(n) = after[..end].parse::<u8>() {
                if n < 16 {
                    return Some(24 + n);
                }
            }
        }
        rest = &rest[pos + 4..];
    }
    None
}

/// XMM-operand mnemonic for a scalar move size: the `MoveSize::L` slot form
/// is `movd` (not `movl`) when an XMM register is the operand, and the
/// SD/SS forms are `movsd`/`movss`.
fn xmm_mnemonic(size: MoveSize) -> Option<&'static str> {
    match size {
        MoveSize::SD => Some("movsd"),
        MoveSize::SS => Some("movss"),
        MoveSize::Q => Some("movq"),
        MoveSize::L => Some("movd"),
        MoveSize::W | MoveSize::B | MoveSize::SLQ => None,
    }
}

/// Record a scalar-FP store (`movsd|movss|movq|movd %xmmN, off(%rbp)`):
/// the slot now holds the XMM register's low `size` bits. Range-invalidate
/// every intersecting mapping first (shared slot semantics), then push the
/// XMM-backed mapping with its home index in [`RegHomes`].
fn gsf_handle_store_xmm(
    xmm: RegId,
    offset: i32,
    size: MoveSize,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut RegHomes,
) {
    invalidate_slots_at(slot_entries, reg_offsets, offset, size.byte_size());
    slot_entries.push(SlotEntry {
        offset,
        mapping: SlotMapping {
            reg_id: xmm,
            size,
            imm: 0,
            zext: false,
        },
        active: true,
    });
    reg_offsets[xmm as usize].push(offset);
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
    reg_offsets: &mut RegHomes,
    lv: &mut FileLiveness,
    imm_materialize_env: bool,
    zext_nop_env: bool,
) -> bool {
    let mut changed = false;
    // Whether the load instruction still writes its destination register
    // after this handler. A DELETED load (nop'd by the same-register
    // elision or a pair fusion) writes nothing: running the dest-register
    // mapping invalidation anyway would spuriously kill mappings for a
    // register whose value did not change (the cascaded relay class: the
    // first fused copy load invalidated the source register's mappings for
    // every later load that could have nop'd).
    let mut load_still_writes_reg = true;
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
        // zero-extension is architecturally observable — UNLESS the stored
        // value's upper half was already zero (mapping.zext, tracked from
        // the backing register's last 32-bit write, which zero-extends by
        // architecture): then the movl writes exactly the bits the register
        // already holds and the load is a pure no-op.
        let exact_width = mapping.size == load_size;
        let qword_to_dword = mapping.size == MoveSize::Q && load_size == MoveSize::L;
        if (exact_width || qword_to_dword)
            && mapping.reg_id != REG_NONE
            && !is_xmm_family(mapping.reg_id)
        {
            let is_epilogue_restore = matches!(load_reg, 3 | 12 | 13 | 14 | 15)
                && load_offset < 0
                && is_near_epilogue(infos, i);
            if exact_width && load_reg == mapping.reg_id && !is_epilogue_restore {
                mark_nop(&mut infos[i]);
                changed = true;
                load_still_writes_reg = false;
            } else if qword_to_dword
                && load_reg == mapping.reg_id
                && mapping.zext
                && zext_nop_env
                && !is_epilogue_restore
            {
                // Zext-backed Q->L self-reload: the register already holds
                // the zero-extended low half; the movl is a no-op.
                mark_nop(&mut infos[i]);
                changed = true;
                load_still_writes_reg = false;
            } else if load_reg != REG_NONE {
                // PAIR FUSION (register-backed): when the very next
                // instruction is a plain store of the loaded register to
                // another slot (`movX %reg, slot2`), the load+store relay
                // collapses to a single store of the MAPPING's source
                // register straight to slot2. This is the load→reload relay
                // the single-line forward cannot touch when the mapping's
                // register differs from the load's destination.
                if exact_width
                    && try_pair_fuse_reg_store(
                        store,
                        infos,
                        i,
                        mapping.reg_id,
                        load_reg,
                        load_size,
                        lv,
                    )
                {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    load_still_writes_reg = false;
                } else {
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
        } else if (exact_width || qword_to_dword)
            && mapping.reg_id == REG_NONE
            && load_reg != REG_NONE
        {
            // IMMEDIATE-BACKED mapping: the slot holds a known constant.
            // Pair-fuse with a following `movX %reg, slot2` into a single
            // `movX $imm, slot2` (the memcpy-shaped relay: a just-stored
            // constant reloaded and re-stored). When no relay follows (or
            // the imm is outside the relay store's immediate field but
            // still inside the load's), MATERIALIZED the load directly:
            // `movX slot, %reg` → `movX $imm, %reg` trades a 4-byte memory
            // read for an equally-sized immediate operand and removes the
            // load — and lets the dead-store pass kill a store whose only
            // reader this was. `movabsq`-class imms (outside imm32) keep
            // the load: the 10-byte materialization is not worth a 4-byte
            // read. A Q-store imm is sign-extended imm32 by construction,
            // so the Q→L narrow sees exactly the low 32 bits the movl
            // would have read.
            if imm_fits_store(mapping.imm, load_size) {
                if try_pair_fuse_imm_store(store, infos, i, mapping.imm, load_reg, load_size, lv) {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    load_still_writes_reg = false;
                } else if imm_materialize_env {
                    let load_reg_str = reg_id_to_name(load_reg, load_size);
                    let new_text = format!(
                        "    {} ${}, {}",
                        mov_imm_store_mnemonic(load_size),
                        mapping.imm,
                        load_reg_str
                    );
                    replace_line(store, &mut infos[i], i, new_text);
                    changed = true;
                }
            }
        } else if is_xmm_family(mapping.reg_id) && load_reg != REG_NONE {
            // XMM-BACKED mapping read by a GP load: bit-exact reinterpret
            // forwards. The stored SD(8)/Q(8)/SS(4)/L(4) bytes equal the
            // backing XMM's low bits, and the GP move forms observe exactly
            // those bits:
            //   * 8-byte load  (Q): `movq %xmmN, %reg` copies the low 64;
            //   * 4-byte load  (L): `movd %xmmN, %reg` copies the low 32.
            // SLQ loads never forward (they sign-extend: not a reinterpret).
            let bytes_match = mapping.size.byte_size() == load_size.byte_size();
            if bytes_match && matches!(load_size, MoveSize::Q | MoveSize::L) {
                let is_epilogue_restore = matches!(load_reg, 3 | 12 | 13 | 14 | 15)
                    && load_offset < 0
                    && is_near_epilogue(infos, i);
                if !is_epilogue_restore
                    && try_pair_fuse_xmm_store(
                        store,
                        infos,
                        i,
                        mapping.reg_id,
                        load_reg,
                        load_size,
                        lv,
                    )
                {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    load_still_writes_reg = false;
                } else if !is_epilogue_restore {
                    let xmm_name = format!("%xmm{}", mapping.reg_id - 24);
                    let mnemonic = if load_size == MoveSize::Q {
                        "movq"
                    } else {
                        "movd"
                    };
                    let new_text = format!(
                        "    {} {}, {}",
                        mnemonic,
                        xmm_name,
                        reg_id_to_name(load_reg, load_size)
                    );
                    replace_line(store, &mut infos[i], i, new_text);
                    changed = true;
                }
            }
        }
    }
    if load_still_writes_reg && is_valid_gp_reg(load_reg) {
        invalidate_reg_flat(slot_entries, reg_offsets, load_reg);
    }
    changed
}

/// PAIR FUSION register side: the next non-NOP line after `i` must be a
/// plain same-width store `movX %load_reg, off2(%rbp)` (StoreRbp kind, exact
/// text match on the source register), and `src_reg` must be a valid GP
/// register distinct from the load's destination. Rewrite the store to use
/// the mapping's source register directly.
fn try_pair_fuse_reg_store(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    src_reg: RegId,
    load_reg: RegId,
    size: MoveSize,
    lv: &mut FileLiveness,
) -> bool {
    if !is_valid_gp_reg(src_reg) || src_reg == load_reg {
        return false;
    }
    let j = next_non_nop(infos, i + 1, infos.len());
    if j >= infos.len() {
        return false;
    }
    let LineKind::StoreRbp {
        reg: st_reg,
        offset: st_off,
        size: st_size,
    } = infos[j].kind
    else {
        return false;
    };
    if st_reg != load_reg || st_size != size {
        return false;
    }
    // LIVENESS GATE: deleting the load removes the ONLY write of load_reg in
    // this pair; the register's value must be dead on every path after the
    // fused store, or a later read would observe the pre-load value (the
    // -O0 alloca ping-pong miscompile class: the load's destination was
    // re-read by a THIRD instruction after the pair). Only the exact
    // dataflow answer is accepted — unanalysable functions keep the load.
    if lv.live_after(j, load_reg) != Some(false) {
        return false;
    }
    // The line must be a PLAIN register store (no flags side-channel, no
    // extra operands): `    movX %src, off(%base)`. Preserve the matched
    // addressing base verbatim — an rbp line must not silently become an
    // rsp line (different physical slot).
    let src_name = reg_id_to_name(load_reg, size);
    for base in ["%rbp", "%rsp"] {
        // `trimmed` strips the indentation: compare against the bare text.
        let want = format!("{} {}, {}({})", size.mnemonic(), src_name, st_off, base);
        if infos[j].trimmed(store.get(j)) == want {
            let new_text = format!(
                "    {} {}, {}({})",
                size.mnemonic(),
                reg_id_to_name(src_reg, size),
                st_off,
                base
            );
            replace_line(store, &mut infos[j], j, new_text);
            lv.refresh_at(store, infos, j);
            return true;
        }
    }
    false
}

/// PAIR FUSION immediate side: the next non-NOP line after `i` must be a
/// plain same-width store `movX %load_reg, off2(%rbp)`. Rewrite it to
/// `movX $imm, off2(%rbp)` and report success (the caller NOPs the load).
fn try_pair_fuse_imm_store(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    imm: i64,
    load_reg: RegId,
    size: MoveSize,
    lv: &mut FileLiveness,
) -> bool {
    let j = next_non_nop(infos, i + 1, infos.len());
    if j >= infos.len() {
        return false;
    }
    let LineKind::StoreRbp {
        reg: st_reg,
        offset: st_off,
        size: st_size,
    } = infos[j].kind
    else {
        return false;
    };
    if st_reg != load_reg || st_size != size {
        return false;
    }
    // LIVENESS GATE: same contract as the register-side fusion — the deleted
    // load was the pair's write of load_reg, so the register must be dead on
    // every path after the fused store.
    if lv.live_after(j, load_reg) != Some(false) {
        return false;
    }
    let src_name = reg_id_to_name(load_reg, size);
    for base in ["%rbp", "%rsp"] {
        // `trimmed` strips the indentation: compare against the bare text.
        let want = format!("{} {}, {}({})", size.mnemonic(), src_name, st_off, base);
        if infos[j].trimmed(store.get(j)) == want {
            let new_text = format!(
                "    {} ${}, {}({})",
                mov_imm_store_mnemonic(size),
                imm,
                st_off,
                base
            );
            replace_line(store, &mut infos[j], j, new_text);
            lv.refresh_at(store, infos, j);
            return true;
        }
    }
    false
}

/// PAIR FUSION XMM side (GP load): the next non-NOP line after `i` must be a
/// plain same-width GP store `movX %load_reg, off2(%base)` whose value came
/// from a slot backed by `%xmmN`. Rewrite the store to `movq/movd %xmmN,
/// off2(%base)` — the XMM register's low bits are exactly the bytes the
/// relay would have stored — and report success (the caller NOPs the load).
/// Gate: the load's GP destination must be dead after the fused store (the
/// deleted load was its only write in the pair).
fn try_pair_fuse_xmm_store(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    src_xmm: RegId,
    load_reg: RegId,
    size: MoveSize,
    lv: &mut FileLiveness,
) -> bool {
    if !is_valid_gp_reg(load_reg) {
        return false;
    }
    let j = next_non_nop(infos, i + 1, infos.len());
    if j >= infos.len() {
        return false;
    }
    let LineKind::StoreRbp {
        reg: st_reg,
        offset: st_off,
        size: st_size,
    } = infos[j].kind
    else {
        return false;
    };
    if st_reg != load_reg || st_size != size {
        return false;
    }
    if lv.live_after(j, load_reg) != Some(false) {
        return false;
    }
    let src_name = reg_id_to_name(load_reg, size);
    // The GP store mnemonic is the ordinary one (movq/movl); the XMM source
    // operand uses the XMM form for L (`movd`, never `movl %xmm`).
    let Some(xmm_src_mnemonic) = xmm_mnemonic(size) else {
        return false;
    };
    for base in ["%rbp", "%rsp"] {
        let want = format!("{} {}, {}({})", size.mnemonic(), src_name, st_off, base);
        if infos[j].trimmed(store.get(j)) == want {
            let new_text = format!(
                "    {} %xmm{}, {}({})",
                xmm_src_mnemonic,
                src_xmm - 24,
                st_off,
                base
            );
            replace_line(store, &mut infos[j], j, new_text);
            lv.refresh_at(store, infos, j);
            return true;
        }
    }
    false
}

/// PAIR FUSION XMM relay (XMM load): `movX slot, %xmmD` followed by a plain
/// same-width XMM store `movX %xmmD, off2(%base)`. Rewrite the store to use
/// the mapping's backing register directly (`movsd %xmmS, off2(%base)` for
/// an XMM-backed mapping) and NOP the load. Gates:
/// * the store's source operand must textually be exactly `%xmmD`;
/// * the load's XMM destination must be dead after the fused store
///   (`FpLiveness`, fail-closed) — the deleted load was its only write.
fn try_pair_fuse_xmm_relay(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    src_xmm: RegId,
    load_xmm: RegId,
    size: MoveSize,
    fp: &mut FpLiveness,
) -> bool {
    let Some(mnemonic) = xmm_mnemonic(size) else {
        return false;
    };
    let j = next_non_nop(infos, i + 1, infos.len());
    if j >= infos.len() {
        return false;
    }
    let LineKind::StoreXmmRbp {
        offset: st_off,
        size: st_size,
    } = infos[j].kind
    else {
        return false;
    };
    if st_size != size {
        return false;
    }
    if !fp.xmm_dead_after(store, infos, j, (load_xmm - 24) as u32, &[i, j]) {
        return false;
    }
    let load_name = format!("%xmm{}", load_xmm - 24);
    for base in ["%rbp", "%rsp"] {
        let want = format!("{} {}, {}({})", mnemonic, load_name, st_off, base);
        if infos[j].trimmed(store.get(j)) == want {
            let new_text = format!(
                "    {} %xmm{}, {}({})",
                mnemonic,
                src_xmm - 24,
                st_off,
                base
            );
            replace_line(store, &mut infos[j], j, new_text);
            fp.refresh_at(store, infos, j);
            return true;
        }
    }
    false
}

/// PAIR FUSION GP→XMM relay: `movsd slot, %xmmD` (slot backed by GP register
/// `%rS`) followed by `movsd %xmmD, off2(%base)`. The relay stores the
/// slot's low 8/4 bytes, which equal `%rS`'s low 64/32 bits, so the pair
/// collapses to a single GP store `movq/movd %rS, off2(%base)`. Gates:
/// * the store's source operand must textually be exactly `%xmmD`;
/// * the load's XMM destination must be dead after the fused store
///   (`FpLiveness`, fail-closed);
/// * the GP source's mappings stay valid by construction (the mapping
///   validity itself proves `%rS` still holds the stored value).
fn try_pair_fuse_gp_to_xmm_relay(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    src_gp: RegId,
    load_xmm: RegId,
    size: MoveSize,
    lv: &mut FileLiveness,
    fp: &mut FpLiveness,
) -> bool {
    if !is_valid_gp_reg(src_gp) {
        return false;
    }
    let Some(xmm_mn) = xmm_mnemonic(size) else {
        return false;
    };
    let j = next_non_nop(infos, i + 1, infos.len());
    if j >= infos.len() {
        return false;
    }
    let LineKind::StoreXmmRbp {
        offset: st_off,
        size: st_size,
    } = infos[j].kind
    else {
        return false;
    };
    if st_size != size {
        return false;
    }
    if !fp.xmm_dead_after(store, infos, j, (load_xmm - 24) as u32, &[i, j]) {
        return false;
    }
    let load_name = format!("%xmm{}", load_xmm - 24);
    // The fused store is the GP form at the same width: 8 bytes → movq,
    // 4 bytes → movl (never `movd %eax`: a GP→memory store uses the GP
    // mnemonics).
    let gp_store_mnemonic = match size {
        MoveSize::Q | MoveSize::SD => "movq",
        MoveSize::L | MoveSize::SS => "movl",
        _ => return false,
    };
    let gp_src_name = match size {
        MoveSize::Q | MoveSize::SD => reg_id_to_name(src_gp, MoveSize::Q),
        MoveSize::L | MoveSize::SS => reg_id_to_name(src_gp, MoveSize::L),
        _ => return false,
    };
    for base in ["%rbp", "%rsp"] {
        let want = format!("{} {}, {}({})", xmm_mn, load_name, st_off, base);
        if infos[j].trimmed(store.get(j)) == want {
            let new_text = format!(
                "    {} {}, {}({})",
                gp_store_mnemonic, gp_src_name, st_off, base
            );
            replace_line(store, &mut infos[j], j, new_text);
            lv.refresh_at(store, infos, j);
            fp.refresh_at(store, infos, j);
            return true;
        }
    }
    false
}

/// XMM-load-side handler: forward a scalar-FP store to a subsequent
/// scalar-FP load of the same slot.
///
/// Semantics matrix (all forms copy exactly `size.byte_size()` bits out of
/// the backing register, which is what the store put in the slot):
/// * same register, MERGE load (movsd/movss: preserve dest's upper bits) →
///   the load writes the value its destination already holds → pure no-op;
/// * same register, FULL load (movq/movd: zero the upper bits) → NOT a
///   no-op (the zeroing is observable) — keep the load;
/// * different register → plain forward to a reg-reg move with identical
///   merge/zero semantics (`movsd/movss` merge like the load; `movq/movd`
///   zero-extend like the load), or pair-fuse a following store relay;
/// * GP-backed mapping → pair-fuse the store relay to a GP store only (a
///   plain `movq %rS, %xmmD` is NOT equivalent to a merge load).
fn gsf_handle_load_xmm(
    store: &mut LineStore,
    infos: &mut [LineInfo],
    i: usize,
    load_offset: i32,
    load_size: MoveSize,
    slot_entries: &mut [SlotEntry],
    reg_offsets: &mut RegHomes,
    lv: &mut FileLiveness,
    fp: &mut FpLiveness,
) -> bool {
    let mut changed = false;
    let mut load_still_writes_xmm = true;
    let Some(dst) = parse_xmm_family(infos[i].trimmed(store.get(i))) else {
        return false;
    };
    let mapping = slot_entries
        .iter()
        .rev()
        .find(|e| e.active && e.offset == load_offset)
        .map(|e| e.mapping);
    if let Some(mapping) = mapping {
        let bytes_match = mapping.size.byte_size() == load_size.byte_size();
        if bytes_match && is_xmm_family(mapping.reg_id) {
            let src = mapping.reg_id;
            if src == dst && matches!(load_size, MoveSize::SD | MoveSize::SS) {
                // MERGE self-reload: identity. See the semantics matrix.
                mark_nop(&mut infos[i]);
                changed = true;
                load_still_writes_xmm = false;
            } else if src != dst {
                if try_pair_fuse_xmm_relay(store, infos, i, src, dst, load_size, fp) {
                    mark_nop(&mut infos[i]);
                    changed = true;
                    load_still_writes_xmm = false;
                } else {
                    let Some(mnemonic) = xmm_mnemonic(load_size) else {
                        return changed;
                    };
                    // Preserve the line's own encoding family: a VEX-encoded
                    // load (vmovsd/vmovss) is rewritten with the 3-operand
                    // VEX form (merge semantics identical to the load);
                    // legacy loads get the legacy 2-operand form. The Q/L
                    // XMM forms have no merge distinction (both zero the
                    // upper bits), so the plain form serves both families.
                    let v_encoded = infos[i].trimmed(store.get(i)).starts_with('v');
                    let (new_text, ok) = match (load_size, v_encoded) {
                        (MoveSize::SD, true) => (
                            format!(
                                "    vmovsd %xmm{}, %xmm{}, %xmm{}",
                                dst - 24,
                                src - 24,
                                dst - 24
                            ),
                            true,
                        ),
                        (MoveSize::SS, true) => (
                            format!(
                                "    vmovss %xmm{}, %xmm{}, %xmm{}",
                                dst - 24,
                                src - 24,
                                dst - 24
                            ),
                            true,
                        ),
                        (MoveSize::L, true) => {
                            // No VEX reg-reg vmovd exists; keep the load.
                            (String::new(), false)
                        }
                        (MoveSize::Q, true) => (
                            // VMOVQ xmm, xmm exists as a 2-operand VEX form;
                            // emitting legacy SSE in a VEX context risks the
                            // AVX-SSE transition penalty.
                            format!("    vmovq %xmm{}, %xmm{}", src - 24, dst - 24),
                            true,
                        ),
                        _ => (
                            format!("    {} %xmm{}, %xmm{}", mnemonic, src - 24, dst - 24),
                            true,
                        ),
                    };
                    if ok {
                        replace_line(store, &mut infos[i], i, new_text);
                        changed = true;
                    }
                }
            }
        } else if bytes_match && is_valid_gp_reg(mapping.reg_id) {
            // GP-backed mapping read by an XMM load: relay fusion only.
            if try_pair_fuse_gp_to_xmm_relay(
                store,
                infos,
                i,
                mapping.reg_id,
                dst,
                load_size,
                lv,
                fp,
            ) {
                mark_nop(&mut infos[i]);
                changed = true;
                load_still_writes_xmm = false;
            }
        }
    }
    if load_still_writes_xmm {
        invalidate_reg_flat(slot_entries, reg_offsets, dst);
    }
    changed
}

fn gsf_handle_other(
    store: &LineStore,
    infos: &[LineInfo],
    i: usize,
    dest_reg: RegId,
    slot_entries: &mut Vec<SlotEntry>,
    reg_offsets: &mut RegHomes,
    rbp_is_frame: bool,
) {
    // SOUNDNESS: any write to %rsp reshuffles every %rsp-relative slot
    // offset, so every offset-keyed mapping is stale afterwards. Mappings
    // here are keyed by raw offset with no (base, offset) distinction and
    // no rsp-bias tracker, so only a full invalidation is sound — exactly
    // like the Push/Pop arm below (`subq $8, %rsp` before a push cascade
    // is the same hazard as the pushes themselves).
    //
    // Without this, `movq %rax, 24(%rsp)` ... `subq $8, %rsp` ...
    // `movl 24(%rsp), %eax` forwarded the pre-shift slot-24 value (still
    // sitting in %rax) into the post-shift load that actually reads
    // pre-shift slot 32 (machinst_window_alloc_wide_copy under
    // CCC_NO_SMALL_SLOTS=1: the 15th printf argument). Push/Pop were
    // already covered; only explicit %rsp arithmetic escaped.
    //
    // Cost: ~zero. Mappings are always empty at the prologue `subq`
    // (the `movq %rsp, %rbp` above already cleared them via the rbp arm),
    // and past a mid-function adjust the next push/call/label clears them
    // anyway, so no live forwarding window is lost.
    if dest_reg == 4 {
        invalidate_all_mappings(slot_entries, reg_offsets);
    }

    // SOUNDNESS: `leave`/`enter` rewrite %rsp with no AT&T destination, so
    // `dest_reg` is REG_NONE and the arm above cannot see them — and in a
    // framed function the rbp arm below stays silent too. The emitter never
    // produces them today (only inline asm does, and that is a barrier), but
    // the framework classifies them as reachable text (`leave` in
    // epilogue_merge/fp_liveness/liveness) and a future compact-epilogue
    // optimization could emit `leave`; a stale mapping across a frame
    // teardown would then miscompile silently. Insurance, zero cost
    // (mappings are always empty where a teardown can stand).
    {
        let t = infos[i].trimmed(store.get(i));
        if t == "leave" || t == "leaveq" || t.starts_with("enter") {
            invalidate_all_mappings(slot_entries, reg_offsets);
        }
    }

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
    } else if is_xmm_family(dest_reg) {
        // The line's AT&T destination is an XMM register (vmulsd, cvtsi2sd,
        // movq %rax, %xmmN, …): that register no longer holds the value any
        // XMM-backed mapping recorded, so every mapping it backs dies. The
        // write may be partial (movhpd writes only bits 127:64) — killing
        // is then merely conservative. Partial LOW writes (pinsrw and
        // friends) also name the destination and are killed correctly.
        invalidate_reg_flat(slot_entries, reg_offsets, dest_reg);
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

/// Parse an immediate store line `    movX $imm, off(%rbp)` /
/// `    movX $imm, off(%rsp)`. Returns (imm, offset, size) when the line is
/// exactly that shape (two operands, plain mnemonic, stack-slot destination)
/// and `preparsed_offset` matches the destination's offset. `None` otherwise.
fn parse_imm_slot_store(raw: &str, preparsed_offset: i32) -> Option<(i64, i32, MoveSize)> {
    if preparsed_offset == RBP_OFFSET_NONE {
        return None;
    }
    let s = raw.trim_start();
    // Two operands exactly.
    let mut parts = s.split(", ");
    let head = parts.next()?;
    let tail = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // `movX $imm` — the size rides the mnemonic suffix.
    let Some((mnem, imm_str)) = head.split_once(' ') else {
        return None;
    };
    let size = match mnem {
        "movq" => MoveSize::Q,
        "movl" => MoveSize::L,
        "movw" => MoveSize::W,
        "movb" => MoveSize::B,
        _ => return None,
    };
    let imm_str = imm_str.strip_prefix('$')?;
    let imm = if let Some(hex) = imm_str.strip_prefix("0x") {
        i64::from_str_radix(hex, 16).ok()?
    } else {
        imm_str.parse::<i64>().ok()?
    };
    // The destination must be exactly `off(%rbp)` or `off(%rsp)`.
    for base in ["(%rbp)", "(%rsp)"] {
        let want = format!("{}{}", preparsed_offset, base);
        if tail == want {
            return Some((imm, preparsed_offset, size));
        }
    }
    None
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

    // Kill switches for the S23/S24 store-forwarding extensions (project
    // convention: every optimization is reversible from the environment for
    // A/B isolation and emergency rollback):
    // * CCC_NO_XMM_SF disables the XMM store→load forwarding (XMM-backed
    //   mappings are not recorded; slot invalidation still happens).
    // * CCC_NO_IMM_MATERIALIZED disables the immediate materialization of
    //   immediate-backed loads (the pair fusion stays).
    // * CCC_NO_ZEXT_NOP disables the zext-backed Q→L self-reload no-op.
    let xmm_forward_env = std::env::var("CCC_NO_XMM_SF").is_err();
    let imm_materialize_env = std::env::var("CCC_NO_IMM_MATERIALIZED").is_err();
    let zext_nop_env = std::env::var("CCC_NO_ZEXT_NOP").is_err();

    let mut slot_entries: Vec<SlotEntry> = Vec::new();
    // [SmallVec; 40] has no Default (array Default stops at N = 32).
    let mut reg_offsets: RegHomes = std::array::from_fn(|_| SmallVec::default());
    let mut reg_zext: [bool; 16] = [false; 16];
    let mut lv = FileLiveness::new(store, infos);
    let mut fp = FpLiveness::new(store, infos);
    let mut changed = false;
    let mut prev_was_unconditional_jump = false;

    // SOUNDNESS: the XMM-register restore instructions (xrstor/xrstors/
    // fxrstor) write ALL sixteen XMM registers with no AT&T destination
    // operand — `parse_dest_reg_fast` reports REG_NONE for them, so the
    // Other-arm XMM invalidation below cannot see them, and a stale
    // XMM-backed mapping would forward a pre-restore value. They are
    // kernel-context-switch code and never appear in ordinary emission;
    // one scan of the file disables XMM-home tracking entirely when any is
    // present. (xsave/fxsave only WRITE memory; vzeroupper preserves the
    // low 128 bits every scalar mapping tracks — neither is a hazard.)
    let xmm_forward_ok = xmm_forward_env
        && (0..len).all(|i| {
            let t = infos[i].trimmed(store.get(i));
            // xrstor/xrstors/fxrstor rewrite all XMM registers with no AT&T
            // destination; vzeroall zeroes ALL 128 bits of every XMM (unlike
            // vzeroupper, which preserves the low 128 every scalar mapping
            // tracks).
            !(t.starts_with("xrstor") || t.starts_with("fxrstor") || t.starts_with("vzeroall"))
        });

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

        // Zero-extension state must be updated BEFORE the line's own dispatch:
        // the store handler reads the flag of the value the line STORES, and a
        // store line never changes any register's extension state.
        gsf_update_zext(
            store,
            infos,
            i,
            &mut reg_zext,
            &jump_targets,
            was_uncond_jump,
        );

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
                    gsf_handle_store(
                        reg,
                        offset,
                        size,
                        reg_zext[reg as usize],
                        &mut slot_entries,
                        &mut reg_offsets,
                    );
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
                        &mut lv,
                        imm_materialize_env,
                        zext_nop_env,
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
                // Immediate store detection: `movX $imm, off(%rbp/%rsp)` — a
                // plain two-operand constant store to a stack slot. These
                // lines classify as Other (StoreRbp requires a register
                // source); recording them gives the load side mappings that
                // survive register clobbers (memory does not change when a
                // register does) — the rax-relay copy pattern of a struct
                // initialized with constants.
                if let Some((imm, off, size)) =
                    parse_imm_slot_store(store.get(i), infos[i].rbp_offset)
                {
                    let is_rbp_base = infos[i].trimmed(store.get(i)).contains("(%rbp)");
                    if !is_rbp_base || rbp_is_frame {
                        gsf_handle_store_imm(imm, off, size, &mut slot_entries, &mut reg_offsets);
                        // An immediate store writes NO register and the slot
                        // is recorded: nothing else to do for this line.
                        continue;
                    }
                }
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
            //
            // XMM-HOME TRACKING: in frame mode the store also creates an
            // XMM-backed mapping (the slot now provably holds the register's
            // low bits) so a subsequent scalar load of the same slot can
            // forward — the FP counterpart of the GP store→load forwarding
            // this pass has always done.
            LineKind::StoreXmmRbp { offset, size } => {
                if !rbp_is_frame && infos[i].reg_refs & (1u16 << 5) != 0 {
                    invalidate_all_mappings(&mut slot_entries, &mut reg_offsets);
                } else if xmm_forward_ok
                    && (rbp_is_frame || !line_base_is_rbp(infos[i].trimmed(store.get(i))))
                {
                    if let Some(xmm) = parse_xmm_family(infos[i].trimmed(store.get(i))) {
                        gsf_handle_store_xmm(
                            xmm,
                            offset,
                            size,
                            &mut slot_entries,
                            &mut reg_offsets,
                        );
                    } else {
                        invalidate_slots_at(
                            &mut slot_entries,
                            &mut reg_offsets,
                            offset,
                            size.byte_size(),
                        );
                    }
                } else {
                    invalidate_slots_at(
                        &mut slot_entries,
                        &mut reg_offsets,
                        offset,
                        size.byte_size(),
                    );
                }
            }
            // An XMM load writes no GP register and no memory: GP mappings
            // stay valid (a later GP reload of the same slot may still
            // forward). The load's XMM destination IS written, so XMM-backed
            // mappings for that register die (the register no longer holds
            // its stored value) — and the load may itself forward from an
            // XMM/GP-backed mapping of the same slot.
            LineKind::LoadXmmRbp { offset, size } => {
                if !rbp_is_frame && line_base_is_rbp(infos[i].trimmed(store.get(i))) {
                    // Opaque pointer-dereference read (rbp is a data
                    // register): never forward, but still kill the dest
                    // register's mappings (it is rewritten here).
                    if let Some(dst) = parse_xmm_family(infos[i].trimmed(store.get(i))) {
                        invalidate_reg_flat(&mut slot_entries, &mut reg_offsets, dst);
                    }
                } else if xmm_forward_ok {
                    changed |= gsf_handle_load_xmm(
                        store,
                        infos,
                        i,
                        offset,
                        size,
                        &mut slot_entries,
                        &mut reg_offsets,
                        &mut lv,
                        &mut fp,
                    );
                } else if let Some(dst) = parse_xmm_family(infos[i].trimmed(store.get(i))) {
                    invalidate_reg_flat(&mut slot_entries, &mut reg_offsets, dst);
                }
            }

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

    #[test]
    fn rsp_arithmetic_kills_slot_forwarding() {
        // `subq $8, %rsp` between a store and a same-offset load reshuffles
        // every %rsp-relative slot: the load reads a DIFFERENT physical
        // slot than the store wrote. Forwarding the stored register (here
        // %rax, still intact) into the load miscompiles — observed as the
        // 15th printf argument going wrong in
        // machinst_window_alloc_wide_copy under CCC_NO_SMALL_SLOTS=1.
        // Push/Pop already invalidate; explicit %rsp arithmetic must too.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rax, 24(%rsp)\n",
            "    subq $8, %rsp\n",
            "    movl 24(%rsp), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(
            !global_store_forwarding(&mut store, &mut infos),
            "no forwarding may fire across an %rsp adjustment"
        );
        let reload_idx = (0..store.len())
            .find(|&i| store.get(i).contains("movl 24(%rsp), %eax"))
            .expect("slot reload must survive verbatim");
        assert!(
            !infos[reload_idx].is_nop(),
            "the post-shift reload must not be forwarded or deleted"
        );
    }

    #[test]
    fn rsp_add_kills_slot_forwarding() {
        // Epilogue direction: `addq $N, %rsp` shifts slots the other way.
        // Same hazard, same invalidation.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rbx, 16(%rsp)\n",
            "    addq $16, %rsp\n",
            "    movq 16(%rsp), %r12\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(
            !global_store_forwarding(&mut store, &mut infos),
            "no forwarding may fire across an %rsp add"
        );
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movq 16(%rsp), %r12")),
            "the post-shift reload must survive verbatim"
        );
    }

    #[test]
    fn leave_kills_slot_forwarding() {
        // `leave` rewrites %rsp with no AT&T destination (`dest_reg` is
        // REG_NONE), so the explicit-`%rsp`-write arm cannot see it — and
        // with a frame pointer live the rbp arm stays silent too. Only
        // inline asm produces `leave` today (a barrier), but the framework
        // classifies it as reachable text and a future compact epilogue
        // could emit one; a stale mapping across the teardown would
        // miscompile. Pin the invalidation with a framed fragment.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    pushq %rbp\n",
            "    movq %rsp, %rbp\n",
            "    movq %rax, 24(%rsp)\n",
            "    leave\n",
            "    movl 24(%rsp), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(
            !global_store_forwarding(&mut store, &mut infos),
            "no forwarding may fire across a frame teardown"
        );
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movl 24(%rsp), %eax")),
            "the post-teardown reload must survive verbatim"
        );
    }

    // ── Fix E: immediate materialization ────────────────────────────────────

    #[test]
    fn immediate_backed_gp_load_materializes() {
        // `movl $5, slot` followed by a plain (non-relay) load: the load
        // becomes `movl $5, %eax` — same count, no memory read.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $5, 24(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movl 24(%rsp), %eax\n",
            "    addq %r10, %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movl $5, %eax")),
            "the load must be materialized as an immediate"
        );
        assert!(
            !(0..store.len()).any(|i| store.get(i).contains("movl 24(%rsp), %eax")),
            "the memory load must be gone"
        );
    }

    #[test]
    fn immediate_backed_q_to_l_load_materializes() {
        // A Q-store immediate is sign-extended imm32; the L load observes
        // exactly its low 32 bits, which `movl $imm, %eax` writes.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq $-7, 32(%rsp)\n",
            "    xorl %ecx, %ecx\n",
            "    movl 32(%rsp), %ecx\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movl $-7, %ecx")),
            "the narrow load must be materialized at the low 32 bits"
        );
    }

    #[test]
    fn wide_immediate_load_keeps_memory_form() {
        // An imm outside imm32 cannot be expressed as `movq $imm, %reg`
        // (that form sign-extends imm32); the 10-byte movabsq is not worth
        // the trade — the load must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movabsq $0x123456789, %rax\n",
            "    movq %rax, 40(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movq 40(%rsp), %r11\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        // The mapping here is REGISTER-backed (movq %rax, slot), so the
        // plain GP forward rewrites it to `movq %rax, %r11` — that is the
        // pre-existing behavior and fine. The interesting contract is that
        // no `movabsq $0x123456789, %r11` ever appears; pin the forward.
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            !(0..store.len()).any(|i| store.get(i).contains("movabsq $0x123456789, %r11")),
            "no movabsq materialization may be invented"
        );
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movq %rax, %r11")),
            "the register-backed forward applies instead"
        );
    }

    // ── Fix E: XMM store→load forwarding ───────────────────────────────────

    #[test]
    fn fp_store_fp_load_same_register_merges_to_nop() {
        // movsd merge semantics: the reload writes the value its
        // destination already holds → identity → nop.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movsd 72(%rsp), %xmm6\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        let reload = (0..store.len())
            .find(|&i| store.get(i).contains("movsd 72(%rsp), %xmm6"))
            .expect("reload line index");
        assert!(infos[reload].is_nop(), "the self-reload must be nop'd");
    }

    #[test]
    fn fp_store_fp_load_other_register_forwards() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd %xmm6, %xmm0")),
            "the load must forward to a reg-reg move"
        );
        assert!(
            !(0..store.len()).any(|i| store.get(i).contains("movsd 72(%rsp), %xmm0")),
            "the memory load must be gone"
        );
    }

    #[test]
    fn fp_load_store_relay_fuses_to_direct_store() {
        // The struct_copy money shape: load into a scratch XMM then store it
        // to another slot collapses to a single store from the mapping's
        // register (the scratch is dead after the pair). The scratch is
        // %xmm12 — `ret` reads %xmm0/%xmm1 in the FP liveness model, so a
        // return-register scratch would (correctly) block the fusion.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    movsd 72(%rsp), %xmm12\n",
            "    movsd %xmm12, 88(%rsp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd %xmm6, 88(%rsp)")),
            "the relay must fuse to a direct store from %xmm6"
        );
        let load_idx = (0..store.len())
            .find(|&i| store.get(i).contains("movsd 72(%rsp), %xmm12"))
            .expect("relay load line");
        assert!(infos[load_idx].is_nop(), "the relay load must be nop'd");
    }

    #[test]
    fn fp_store_gp_load_forwards_bits() {
        // Cross-family: the integer reload of a just-stored double's bits
        // becomes `movq %xmm6, %rax` — same count, no load.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movq 72(%rsp), %rax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movq %xmm6, %rax")),
            "the GP load must forward from the XMM register"
        );
    }

    #[test]
    fn fp_store_gp_load_gp_store_relay_fuses() {
        // The struct_copy integer relay: movq slot,%r11; movq %r11,slot2
        // with an XMM-backed slot fuses to `movq %xmm6, slot2`. The scratch
        // is %r11 (dead at `ret`); %rax would (correctly) block the fusion.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    movq 72(%rsp), %r11\n",
            "    movq %r11, 48(%rsp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movq %xmm6, 48(%rsp)")),
            "the relay must fuse to a direct XMM store"
        );
        let load_idx = (0..store.len())
            .find(|&i| store.get(i).contains("movq 72(%rsp), %r11"))
            .expect("relay load line");
        assert!(infos[load_idx].is_nop(), "the relay load must be nop'd");
    }

    #[test]
    fn call_between_fp_store_and_load_blocks_forwarding() {
        // Every XMM register is caller-saved: a call rewrites the backing
        // register even though the slot survives — the forward must not
        // fire (mapping invalidation at Call).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    callq foo\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd 72(%rsp), %xmm0")),
            "the post-call reload must survive verbatim"
        );
    }

    #[test]
    fn xmm_redefinition_between_store_and_load_blocks_forwarding() {
        // An intervening write of the backing XMM (here: vmulsd with %xmm6
        // as destination) kills the mapping — the reload must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    vmulsd %xmm2, %xmm6, %xmm6\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd 72(%rsp), %xmm0")),
            "the reload after an xmm rewrite must survive verbatim"
        );
    }

    #[test]
    fn jump_target_label_between_fp_store_and_load_blocks_forwarding() {
        // A label that is a jump target is a merge point: single-path
        // knowledge dies with it.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    testq %rax, %rax\n",
            "    je .LBB1\n",
            "    movsd %xmm6, 72(%rsp)\n",
            ".LBB1:\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd 72(%rsp), %xmm0")),
            "the merge-point reload must survive verbatim"
        );
    }

    #[test]
    fn xmm_restore_instruction_disables_xmm_forwarding() {
        // xrstor rewrites ALL XMM registers with no AT&T destination; its
        // presence must disable XMM-home tracking file-wide.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    xrstor 8(%rdi)\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd 72(%rsp), %xmm0")),
            "the reload across an XMM restore must survive verbatim"
        );
    }

    #[test]
    fn movd_self_reload_is_not_a_nop() {
        // `movd slot, %xmm5` ZEROES bits 127:32 of its destination; even
        // when the low 32 bits already match, the zeroing is observable —
        // unlike movsd/movss (merge forms). The load must survive.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movd %xmm5, 32(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movd 32(%rsp), %xmm5\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movd 32(%rsp), %xmm5")),
            "the zero-upper self-reload must survive (not a merge form)"
        );
    }

    #[test]
    fn gp_store_fp_load_relay_fuses_to_gp_store() {
        // GP-backed slot read by an XMM load that immediately stores it:
        // `movq %rax, slot; movsd slot, %xmm5; movsd %xmm5, slot2` —
        // the pair collapses to `movq %rax, slot2`.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movq %rax, 48(%rsp)\n",
            "    movsd 48(%rsp), %xmm5\n",
            "    movsd %xmm5, 64(%rsp)\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movq %rax, 64(%rsp)")),
            "the GP→XMM relay must fuse to a direct GP store"
        );
        let load_idx = (0..store.len())
            .find(|&i| store.get(i).contains("movsd 48(%rsp), %xmm5"))
            .expect("relay load line");
        assert!(infos[load_idx].is_nop(), "the relay load must be nop'd");
    }

    // ── Red-team audit hardening contracts ──────────────────────────────────

    #[test]
    fn xchg_between_zext_def_and_reload_blocks_q_to_l_nop() {
        // RED-TEAM: `xchgq %rax, (%rcx)` swaps FULL 64-bit %rax with memory
        // (the last-comma destination parse sees only the memory operand).
        // A stale TRUE zext flag on %rax would nop the Q→L self-reload
        // although the upper half now holds swapped-in memory bits. The
        // swap/cas guard must clear the zext state.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movl $7, %eax\n",
            "    xchgq %rax, (%rcx)\n",
            "    movq %rax, 32(%rsp)\n",
            "    movq %rcx, %r10\n",
            "    movl 32(%rsp), %eax\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        // Track the reload by its pre-transform index: the pre-existing
        // plain forward may legitimately rewrite it (to `movl %eax, %eax`
        // — equivalent: both forms write the zero-extended low 32 bits),
        // but it must never be DELETED.
        let reload = (0..store.len())
            .find(|&i| store.get(i).contains("movl 32(%rsp), %eax"))
            .expect("reload line index");
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            !infos[reload].is_nop(),
            "the Q→L self-reload must be deleted by neither the zext nop \
             (upper half is the swapped-in memory bits) nor anything else"
        );
    }

    #[test]
    fn vzeroall_disables_xmm_forwarding() {
        // RED-TEAM: vzeroall zeroes ALL 128 bits of every XMM (unlike
        // vzeroupper, which preserves the low 128 every scalar mapping
        // tracks). Its presence must disable XMM-home tracking file-wide.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    vzeroall\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        let _ = global_store_forwarding(&mut store, &mut infos);
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd 72(%rsp), %xmm0")),
            "the reload across vzeroall must survive verbatim"
        );
    }

    #[test]
    fn vzeroupper_preserves_scalar_xmm_forwarding() {
        // vzeroupper only zeroes bits 255:128; the low 128 bits every SD/SS
        // mapping tracks survive — the forward MUST still fire.
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    movsd %xmm6, 72(%rsp)\n",
            "    vzeroupper\n",
            "    movsd 72(%rsp), %xmm0\n",
            "    ret\n",
            ".cfi_endproc\n",
        );
        let (mut store, mut infos) = build_pinned(asm);
        assert!(global_store_forwarding(&mut store, &mut infos));
        assert!(
            (0..store.len()).any(|i| store.get(i).contains("movsd %xmm6, %xmm0")),
            "the forward must fire across vzeroupper (low 128 preserved)"
        );
    }
}

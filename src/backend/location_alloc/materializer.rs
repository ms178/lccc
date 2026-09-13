//! GLA plan materialization (IR rewrites) — split from the former monolithic `location_alloc.rs`.
//! See the module docs in [`super`] for the full design and policy record.

use super::*;

/// Instantiate a planner-certified remat template at a fresh destination.
/// Templates come exclusively from [`rematerializable_template`], i.e.
/// `GlobalAddr` or a `Copy` of a constant — there is deliberately no
/// `Load`/`GetElementPtr` arm: re-reading memory at a later program point
/// is not a sound reproduction of an SSA value (the memory may have been
/// written since), and GEP inputs are not static. The catch-all fails loud
/// rather than cloning with the OLD dest, which would mint a duplicate
/// definition that fails verification in a far less obvious place.
pub(super) fn clone_remat(template: &Instruction, name: Value) -> Instruction {
    match template {
        Instruction::GlobalAddr { name: gname, .. } => Instruction::GlobalAddr {
            dest: name,
            name: gname.clone(),
        },
        Instruction::Copy {
            src: src @ Operand::Const(_),
            ..
        } => Instruction::Copy {
            dest: name,
            src: src.clone(),
        },
        other => {
            unreachable!(
                "rematerializable_template certifies only GlobalAddr/const-Copy, got {other:?}"
            )
        }
    }
}

/// Event scheduled at an ORIGINAL local point `p`; Load/Remat/Anchor all
/// execute immediately before original instruction `p` (`p == n`: before
/// the terminator). Loads/remats sort before anchors at the same point (the
/// anchor follows a def that the loads do not depend on).
#[derive(Clone, Copy, Debug)]
pub(super) enum Event {
    Load { vid: u32, name: u32 },
    Remat { vid: u32, name: u32 },
    Anchor { vid: u32 },
}

/// One edge whose reload lives in a new trampoline block.
pub(super) struct EdgeTramp {
    pub(super) pred: usize,
    pub(super) succ: usize,
    pub(super) label: BlockId,
    /// (vid, fresh name built inside the trampoline).
    pub(super) defs: Vec<(u32, Value)>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn materialize(
    func: &mut IrFunction,
    live: &LivenessResult,
    succs: &analysis::FlatAdj,
    plan: Plan,
    all_types: &FxHashMap<u32, IrType>,
    templates: &FxHashMap<u32, Instruction>,
) -> usize {
    let entry_memory = plan.entry_memory.clone();
    let nblocks = func.blocks.len();
    // Fail-closed id space: `next_value_id` is authoritative in principle,
    // but floor it against every id already present in the IR so an
    // upstream pass that minted an id without syncing its counter can never
    // make us emit a duplicate definition.
    let mut next_val = func.next_value_id;
    for b in &func.blocks {
        for i in &b.instructions {
            if let Some(d) = i.dest() {
                next_val = next_val.max(d.0.saturating_add(1));
            }
        }
    }
    // Saturating: if any existing block label is u32::MAX, the id space for
    // new trampoline blocks is exhausted; the first allocation below then
    // routes through `id_overflow` and the whole plan aborts with zero
    // edits instead of overflowing here.
    let mut next_block_id = func
        .blocks
        .iter()
        .map(|b| b.label.0)
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    let remat_ids: FxHashSet<u32> = plan.remats.iter().copied().collect();
    let gap_ids: FxHashSet<u32> = plan.gaps.iter().map(|g| g.vid).collect();
    let split_vids: Vec<u32> = gap_ids.union(&remat_ids).copied().collect();

    // Fail-closed rollback state: every mutation below touches only the
    // blocks and the two id counters. If the unconditional structural
    // verification fails after materialization, the function is restored
    // verbatim and the plan applies ZERO edits (equivalent to the gate
    // being off) rather than emitting malformed IR downstream.
    let saved_blocks = func.blocks.clone();
    let saved_next_value_id = func.next_value_id;
    let saved_next_label = func.next_label;

    // Checked fresh-id allocator. The whole plan is built into local
    // structures BEFORE the first IR mutation (the block sweeps below), so
    // an exhausted u32 id space simply aborts the plan with zero edits —
    // the fail-closed contract. Every site that needs an id must go
    // through this closure instead of `expect`-ing one.
    let id_overflow = std::cell::Cell::new(false);
    let mut fresh_value = |next_val: &mut u32| -> Option<Value> {
        match next_value(next_val) {
            some @ Some(_) => some,
            None => {
                id_overflow.set(true);
                None
            }
        }
    };

    // ── Slot values (alloca instructions inserted AFTER all sweeps) ──
    let mut slot_of: FxHashMap<u32, Value> = FxHashMap::default();
    for &vid in &gap_ids {
        if !all_types.contains_key(&vid) {
            continue;
        }
        let Some(slot) = fresh_value(&mut next_val) else {
            break;
        };
        slot_of.insert(vid, slot);
    }

    let mut events: Vec<Vec<(usize, Event)>> = vec![Vec::new(); nblocks];
    // name of the memory cluster serving point `n` in block bi, if any
    let mut name_at_exit: Vec<FxHashMap<u32, u32>> = vec![FxHashMap::default(); nblocks];
    let mut applied: FxHashSet<u32> = FxHashSet::default();

    // ── Per-block use clusters ──
    for bi in 0..nblocks {
        let n = func.blocks[bi].instructions.len();
        let mem_at = |vid: u32, p: usize| -> bool {
            if remat_ids.contains(&vid) {
                return true;
            }
            // The half-open memory span is [f, t); the read AT point `t` is
            // also served from memory: its reload executes immediately
            // before that instruction, which is precisely the boundary
            // reload that starts the next register piece.
            if plan.gaps_of(vid, bi).iter().any(|&(f, t)| f <= p && p <= t) {
                return true;
            }
            // Entry-memory blocks serve the first read cluster from the
            // slot; after that cluster the reloaded name is resident.
            if entry_memory.contains(&(vid, bi)) {
                let first = block_use_points(live, func, bi, vid)
                    .into_iter()
                    .min()
                    .unwrap_or(n);
                return p <= first;
            }
            false
        };

        for &vid in &split_vids {
            let uses = block_use_points(live, func, bi, vid);
            // Walk read points; group consecutive memory-served points into
            // one cluster with a fresh name; register points need no event.
            let mut open_name: Option<u32> = None;
            let mut prev_mem = false;
            for p in uses.iter().copied() {
                let mem = mem_at(vid, p);
                if mem {
                    if !prev_mem || open_name.is_none() {
                        let Some(name) = fresh_value(&mut next_val).map(|v| v.0) else {
                            // id space exhausted; the pre-sweep guard aborts
                            // the whole (partially built) plan.
                            continue;
                        };
                        let ev = if remat_ids.contains(&vid) {
                            Event::Remat { vid, name }
                        } else {
                            Event::Load { vid, name }
                        };
                        events[bi].push((p, ev));
                        open_name = Some(name);
                        applied.insert(vid);
                    }
                    if p == n {
                        if let Some(name) = open_name {
                            name_at_exit[bi].insert(vid, name);
                        }
                    }
                } else {
                    open_name = None;
                }
                prev_mem = mem;
            }
        }

        // Capture anchors: store immediately after the unique def.
        for &vid in &gap_ids {
            let Some((db, di, is_phi)) = find_def(func, vid) else {
                continue;
            };
            if db != bi {
                continue;
            }
            let point = if is_phi {
                first_non_phi(&func.blocks[bi])
            } else {
                di + 1
            };
            let point = point.min(n);
            events[bi].push((point, Event::Anchor { vid }));
            applied.insert(vid);
        }
    }

    // ── Edge planning: successor φ incoming operands ──
    let mut edge_tramps: Vec<EdgeTramp> = Vec::new();
    // in-place edge renames (pred block, succ block) applied to φ
    // incomings without rerouting the edge.
    let mut inline_edge_rename: Vec<(usize, usize, Vec<(u32, u32)>)> = Vec::new();
    for bi in 0..nblocks {
        for &s_raw in succs.row(bi) {
            let s = s_raw as usize;
            let mut renames: Vec<(u32, u32)> = Vec::new();
            let mut deferred: Vec<u32> = Vec::new();
            for &vid in &split_vids {
                if !phi_incoming_uses(func, s, bi, vid) {
                    continue;
                }
                if let Some(&name) = name_at_exit[bi].get(&vid) {
                    renames.push((vid, name));
                } else {
                    // Register-resident at exit? Then no rename.
                    let n = func.blocks[bi].instructions.len();
                    let own = plan.gaps_of(vid, bi);
                    let mem_exit = remat_ids.contains(&vid)
                        || own.iter().any(|&(_, t)| t >= n)
                        || entry_memory.contains(&(vid, bi));
                    if !mem_exit {
                        continue; // original name is correct on the edge
                    }
                    deferred.push(vid);
                }
            }
            if succs.len(bi) == 1 {
                // Single successor: serve deferred values with an end-of-block
                // load/remat that is effectively on the unique edge.
                let mut extra = Vec::new();
                for vid in deferred.drain(..) {
                    let Some(name) = fresh_value(&mut next_val).map(|v| v.0) else {
                        continue;
                    };
                    let n = func.blocks[bi].instructions.len();
                    let ev = if remat_ids.contains(&vid) {
                        Event::Remat { vid, name }
                    } else {
                        Event::Load { vid, name }
                    };
                    events[bi].push((n, ev));
                    applied.insert(vid);
                    extra.push((vid, name));
                }
                renames.extend(extra);
                if !renames.is_empty() {
                    inline_edge_rename.push((bi, s, renames));
                }
            } else if !deferred.is_empty() {
                // Fan-out edge: isolate it with a trampoline so the reload
                // never executes on the other edges.
                //
                // Fail closed for an IndirectBranch predecessor: its jump
                // target is a RUNTIME blockaddress operand, not a static
                // terminator edge, so retargeting `possible_targets` would
                // not redirect execution (the jump still lands in the
                // successor with the φ expecting a name only defined in the
                // trampoline). Leave that φ incoming on the original
                // source-less value; the un-rewritten def stays resident
                // along the edge exactly as without GLA. This shape has
                // never been observed, but the gate must be incapable of
                // miscompiling it.
                if matches!(
                    func.blocks[bi].terminator,
                    Terminator::IndirectBranch { .. }
                ) {
                    if split_debug_enabled() {
                        eprintln!(
                            "[GLA] {}: skip edge remat b{}->b{}: indirect-branch pred",
                            func.name, bi, s
                        );
                    }
                    continue;
                }
                if next_block_id == u32::MAX {
                    id_overflow.set(true);
                    continue;
                }
                let label = BlockId(next_block_id);
                next_block_id += 1;
                let mut defs = Vec::new();
                for vid in deferred {
                    let Some(nv) = fresh_value(&mut next_val) else {
                        continue;
                    };
                    defs.push((vid, nv));
                    applied.insert(vid);
                    renames.push((vid, nv.0));
                }
                edge_tramps.push(EdgeTramp {
                    pred: bi,
                    succ: s,
                    label,
                    defs,
                });
                inline_edge_rename.push((bi, s, renames));
            } else if !renames.is_empty() {
                inline_edge_rename.push((bi, s, renames));
            }
        }
    }

    // Deterministic event order: point, then loads/remats before anchors.
    for ev in events.iter_mut() {
        ev.sort_by_key(|(p, e)| {
            (
                *p,
                matches!(e, Event::Anchor { .. }) as u8,
                match e {
                    Event::Load { vid, .. } | Event::Remat { vid, .. } | Event::Anchor { vid } => {
                        *vid
                    }
                },
            )
        });
        ev.dedup_by(|a, b| match (&a.1, &b.1) {
            (Event::Load { vid: va, name: na }, Event::Load { vid: vb, name: nb }) => {
                va == vb && na == nb
            }
            (Event::Remat { vid: va, name: na }, Event::Remat { vid: vb, name: nb }) => {
                va == vb && na == nb
            }
            (Event::Anchor { vid: a }, Event::Anchor { vid: b }) => a == b,
            _ => false,
        });
    }

    // Fail closed before the first mutation: an exhausted id space made the
    // plan unnameable, so apply NONE of it rather than emitting a plan with
    // missing reloads or a duplicate/truncated block label.
    if id_overflow.get() {
        eprintln!(
            "[GLA] {}: fresh id space exhausted; aborting {} planned split(s) \
             with zero edits",
            func.name,
            plan.remats.len() + plan.gaps.len()
        );
        return 0;
    }

    // ── Sweep every block in original coordinates ──
    for bi in 0..nblocks {
        let old = std::mem::take(&mut func.blocks[bi].instructions);
        let n = old.len();
        let mut out: Vec<Instruction> = Vec::with_capacity(old.len() + events[bi].len() + 2);
        let mut active: FxHashMap<u32, u32> = FxHashMap::default();
        let mut ev_idx = 0usize;

        for (i, mut inst) in old.into_iter().enumerate() {
            while ev_idx < events[bi].len() && events[bi][ev_idx].0 == i {
                emit_event(
                    &events[bi][ev_idx].1,
                    &mut out,
                    &slot_of,
                    all_types,
                    templates,
                    &mut active,
                );
                ev_idx += 1;
            }
            if !matches!(inst, Instruction::Phi { .. }) && !active.is_empty() {
                replace_values_in_inst(&mut inst, &active, false);
            }
            out.push(inst);
        }
        // Trailing events at point n (before the terminator).
        while ev_idx < events[bi].len() && events[bi][ev_idx].0 == n {
            emit_event(
                &events[bi][ev_idx].1,
                &mut out,
                &slot_of,
                all_types,
                templates,
                &mut active,
            );
            ev_idx += 1;
        }
        debug_assert_eq!(
            ev_idx,
            events[bi].len(),
            "all scheduled events must be consumed"
        );
        func.blocks[bi].instructions = out;
        if !active.is_empty() {
            replace_values_in_terminator(&mut func.blocks[bi].terminator, &active);
        }
    }

    // ── φ incoming rewrites ──
    // First rename operands (pred label still the original), then reroute
    // the whole edge to its trampoline exactly once (that relabels every
    // incoming on the edge, including register values that pass through).
    for (bi, s, renames) in &inline_edge_rename {
        let old_pred = func.blocks[*bi].label;
        for (vid, name) in renames {
            rewrite_phi_edge(func, *s, old_pred, *vid, *name, None);
        }
    }
    for t in &edge_tramps {
        let old_pred = func.blocks[t.pred].label;
        for inst in func.blocks[t.succ].instructions.iter_mut() {
            let Instruction::Phi { incoming, .. } = inst else {
                continue;
            };
            for (_op, p) in incoming.iter_mut() {
                if *p == old_pred {
                    *p = t.label;
                }
            }
        }
    }

    // ── Build trampoline blocks and retarget edges ──
    for t in &edge_tramps {
        let mut insts = Vec::new();
        for (vid, nv) in &t.defs {
            if remat_ids.contains(vid) {
                let tmpl = templates.get(vid).cloned().expect("remat template");
                insts.push(clone_remat(&tmpl, *nv));
            } else {
                insts.push(Instruction::Load {
                    volatile: false,
                    dest: *nv,
                    ptr: slot_of[vid],
                    ty: all_types[vid],
                    seg_override: AddressSpace::Default,
                });
            }
        }
        // Copy labels out before any mutable borrow.
        let old_target = func.blocks[t.succ].label;
        let new_label = t.label;
        // Non-renamed φ incoming values pass through the trampoline with no
        // copy (the value remains live across the unconditional branch).
        func.blocks.push(BasicBlock {
            label: new_label,
            instructions: insts,
            terminator: Terminator::Branch(old_target),
            source_spans: Vec::new(),
        });
        retarget_edge(&mut func.blocks[t.pred].terminator, old_target, new_label);
    }

    // ── Insert volatile capture allocas (coordinates are now settled) ──
    for (&vid, &slot) in &slot_of {
        let ty = all_types[&vid];
        insert_entry_alloca(func, slot, ty, true);
    }

    func.next_value_id = next_val;
    // Keep the module-level "next unused label" honest for any trampoline
    // block we appended (next_block_id starts at max(label)+1).
    func.next_label = func.next_label.max(next_block_id);
    let n_applied = applied.len();
    // Unconditional fail-closed structural verification: the shipped
    // feature must never emit malformed IR regardless of environment
    // knobs. On any violation the function is restored byte-for-byte and
    // zero edits are reported (gate-off behavior). With CCC_VERIFY_REGALLOC
    // set, a violation panics instead, for backtraces in development.
    let slots: FxHashSet<u32> = slot_of.values().map(|v| v.0).collect();
    if let Err(msg) = verify_rewrite(func, &slots) {
        if std::env::var_os("CCC_VERIFY_REGALLOC").is_some() {
            panic!("[GLA] {}: rewrite failed verification: {}", func.name, msg);
        }
        eprintln!(
            "[GLA] {name}: rewrite failed structural verification ({msg}); \
             aborting plan with zero edits",
            name = func.name
        );
        // Every mutation above touches only blocks and the two id
        // counters; restore exactly those members.
        func.blocks = saved_blocks;
        func.next_value_id = saved_next_value_id;
        func.next_label = saved_next_label;
        return 0;
    }
    if split_debug_enabled() {
        eprintln!(
            "[GLA] {} applied: {} values ({} remat, {} capture slots), {} trampolines",
            func.name,
            n_applied,
            remat_ids.len(),
            slot_of.len(),
            edge_tramps.len()
        );
    }
    n_applied
}

pub(super) fn emit_event(
    event: &Event,
    out: &mut Vec<Instruction>,
    slot_of: &FxHashMap<u32, Value>,
    all_types: &FxHashMap<u32, IrType>,
    templates: &FxHashMap<u32, Instruction>,
    active: &mut FxHashMap<u32, u32>,
) {
    match *event {
        Event::Load { vid, name } => {
            out.push(Instruction::Load {
                volatile: false,
                dest: Value(name),
                ptr: slot_of[&vid],
                ty: all_types[&vid],
                seg_override: AddressSpace::Default,
            });
            active.insert(vid, name);
        }
        Event::Remat { vid, name } => {
            let tmpl = templates.get(&vid).cloned().expect("remat template");
            out.push(clone_remat(&tmpl, Value(name)));
            active.insert(vid, name);
        }
        Event::Anchor { vid } => {
            out.push(Instruction::Store {
                volatile: false,
                val: Operand::Value(Value(vid)),
                ptr: slot_of[&vid],
                ty: all_types[&vid],
                seg_override: AddressSpace::Default,
            });
        }
    }
}

/// Rewrite (and optionally reroute) one φ incoming edge `pred -> phi_block`.
/// When `new_pred` is given (a trampoline), EVERY incoming operand on that
/// predecessor edge is re-labelled to the trampoline, including values that
/// merely pass through.
pub(super) fn rewrite_phi_edge(
    func: &mut IrFunction,
    phi_block: usize,
    pred: BlockId,
    vid: u32,
    new_name: u32,
    new_pred: Option<BlockId>,
) {
    for inst in func.blocks[phi_block].instructions.iter_mut() {
        let Instruction::Phi { incoming, .. } = inst else {
            continue;
        };
        for (op, p) in incoming.iter_mut() {
            if *p != pred {
                continue;
            }
            if let Operand::Value(v) = op {
                if v.0 == vid {
                    *op = Operand::Value(Value(new_name));
                }
            }
            if let Some(np) = new_pred {
                *p = np;
            }
        }
    }
}

/// Reroute every static occurrence of terminator edge target `old` to
/// `new` (a fan-out trampoline). The edge
/// planner builds exactly one trampoline per unique (pred, succ) pair
/// (build_cfg deduplicates successor labels), so a CondBranch whose two
/// arms — or two Switch cases — name the same block are the SAME edge with
/// one shared φ incoming; leaving a duplicate arm on the old target would
/// enter the successor expecting names that are only defined in the
/// trampoline. All occurrences must move together.
pub(super) fn retarget_edge(term: &mut Terminator, old: BlockId, new: BlockId) {
    match term {
        Terminator::Branch(t) if *t == old => *t = new,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if *true_label == old {
                *true_label = new;
            }
            if *false_label == old {
                *false_label = new;
            }
        }
        Terminator::Switch { cases, default, .. } => {
            if *default == old {
                *default = new;
            }
            for (_, t) in cases.iter_mut() {
                if *t == old {
                    *t = new;
                }
            }
        }
        Terminator::IndirectBranch {
            possible_targets, ..
        } => {
            for t in possible_targets.iter_mut() {
                if *t == old {
                    *t = new;
                }
            }
        }
        _ => {}
    }
}

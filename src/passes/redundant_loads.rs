//! Late memory-value forwarding within basic blocks.
//!
//! Representation-compatible stores also establish available values. This makes repeated
//! aggregate field updates register dataflow without speculating any access
//! or pretending that two unrelated pointer roots are disjoint.
//!
//! After strength reduction, array/struct field accesses appear as
//! `Load(GEP(base, const_off))` with plain constant offsets. When the
//! frontend emits the same field access in several statements (e.g.
//! `bodies[j].mass` in each of three velocity updates), the duplicate loads
//! survive GVN because GEP CSE is disabled (distinct GEP value ids key the
//! value numbers). This pass merges loads of the same linear-form address
//! within a block when every intervening store is provably non-aliasing
//! (`alias::forms_disjoint`).
//!
//! Adapted from levkropp/lccc (Aug 19, 2026) with two hard gates this tree
//! requires:
//!  * **volatile loads never merge** — each volatile read is an observable
//!    side effect (C11 5.1.2.3); collapsing two reads into one changes the
//!    program's observable behavior.  (The upstream original predates the
//!    IR volatile flags entirely.)
//!  * volatile STORES are memory writes like any other store for the
//!    aliasing math — the form-based retain handles them; no special case
//!    is needed for soundness of merging *non-volatile* loads across them.
//!
//! A duplicate's SSA uses are rewritten function-wide after analysis; the
//! canonical load is earlier in the same block, so every dominated use stays
//! dominated (SSA use sites of the duplicate are, by SSA construction,
//! dominated by the duplicate's block, hence by the canonical def's block).
//!
//! MULTI-DEF GUARD (same class as the gvn/copy_prop guard): a value with
//! more than one definition (post-phi coalescing webs) has position-
//! dependent content.  Rewriting a duplicate load's uses onto a MULTI-DEF
//! canonical (its id redefined between the canonical load and the use
//! feeds the consumer the redefined content), or rewriting the uses of a
//! MULTI-DEF duplicate function-wide (uses after its other defs expect
//! those contents), are both unsound position-independent rewrites — the
//! free_area_init_node GVN bug class.  Refuse and keep the recomputation.

use super::alias;
use crate::common::fx_hash::FxHashMap;
use crate::ir::reexports::{Instruction, IrFunction, Operand, Terminator, Value};

/// Full-width integer carriers have identical bits for signed/unsigned
/// loads. Narrow carriers may require different extension/normalization, and
/// equal-sized floats/pointers are not interchangeable untyped SSA values.
fn same_memory_value_type(
    a: crate::common::types::IrType,
    b: crate::common::types::IrType,
) -> bool {
    use crate::common::types::IrType::{I64, U64};
    a == b || matches!((a, b), (I64, U64) | (U64, I64))
}

type AvailableLoad = (alias::LinForm, crate::common::types::IrType, Value, Value);

fn retain_available_across_call(
    func: &IrFunction,
    block: usize,
    instruction: usize,
    available: &mut Vec<AvailableLoad>,
    analysis: &super::ipa_pta::IpaPta,
) {
    let before = available.len();
    available
        .retain(|(_, _, _, pointer)| !analysis.may_call_modify(func, block, instruction, *pointer));
    if std::env::var_os("CCC_IPA_PTA_STATS").is_some() {
        eprintln!(
            "[IPA-PTA] consumer function={} block={} instruction={} available={}->{}",
            func.name,
            block,
            instruction,
            before,
            available.len()
        );
    }
}

pub(crate) fn run(func: &mut IrFunction) -> usize {
    run_impl(func, None)
}

pub(crate) fn run_with_ipa_pta(func: &mut IrFunction, analysis: &super::ipa_pta::IpaPta) -> usize {
    run_impl(func, Some(analysis))
}

fn run_impl(func: &mut IrFunction, ipa: Option<&super::ipa_pta::IpaPta>) -> usize {
    if std::env::var("CCC_NO_REDUNDANT_LOADS").is_ok() {
        return 0;
    }
    if func.blocks.is_empty() {
        return 0;
    }
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    let frames = alias::LoopFrames::build_with_cfg(func, &cfg);
    let multi_def = super::gvn::find_multi_def_values(func);

    // Analysis phase: immutable borrow of func plus a defs map over it.
    let mut all_rewrites: FxHashMap<u32, u32> = FxHashMap::default();
    let mut per_block_removals: Vec<Vec<usize>> = Vec::with_capacity(func.blocks.len());
    {
        let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
        for block in func.blocks.iter() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest()
                    && !multi_def.contains(&dest.0)
                {
                    defs.insert(dest.0, inst);
                }
            }
        }

        for bi in 0..func.blocks.len() {
            let frame = frames.block_frame[bi];
            // (form, type, canonical dest, address value).  Retaining the
            // original address is what lets IPA-PTA answer a call query per
            // available load rather than conservatively clearing the list.
            let mut available: Vec<AvailableLoad> = Vec::new();
            let mut removed: Vec<usize> = Vec::new();

            for (ii, inst) in func.blocks[bi].instructions.iter().enumerate() {
                // SYM-REDEFINITION GUARD: a form's symbol leaves include
                // copy-web values (multi-def ids). Any instruction that
                // (re)defines such an id makes every available form
                // referencing it position-stale — the web's content at the
                // load site differs from its content later in the block.
                // Drop exactly those entries; single-def SSA ids are never
                // redefined, so the guard costs nothing there.
                if let Some(dest) = inst.dest() {
                    if !available.is_empty() {
                        available.retain(|(f, _, _, _)| !f.syms.iter().any(|s| s.0 == dest.0));
                    }
                }
                match inst {
                    Instruction::Load {
                        dest,
                        ptr,
                        ty,
                        seg_override,
                        volatile,
                    } => {
                        if *volatile {
                            // Observable side effect: must not be merged away.
                            continue;
                        }
                        if *seg_override != crate::common::types::AddressSpace::Default
                            || ty.is_long_double()
                            || ty.is_128bit()
                        {
                            continue;
                        }
                        let Some(form) = alias::resolve_in_frame(func, &defs, &frames, frame, *ptr)
                        else {
                            continue;
                        };
                        if let Some((_, _, canon, _)) = available
                            .iter()
                            .find(|(f, t, _, _)| *f == form && same_memory_value_type(*t, *ty))
                        {
                            // MULTI-DEF GUARD: see the module doc. A multi-def
                            // canonical's content is not position-stable, and a
                            // multi-def duplicate's uses are not all reading
                            // this load. Keep both loads in that case.
                            if !multi_def.contains(&canon.0) && !multi_def.contains(&dest.0) {
                                all_rewrites.insert(dest.0, canon.0);
                                removed.push(ii);
                                continue;
                            }
                        }
                        available.push((form, *ty, *dest, *ptr));
                    }
                    Instruction::Store {
                        ptr,
                        ty,
                        val,
                        volatile,
                        seg_override,
                    } => {
                        if *seg_override != crate::common::types::AddressSpace::Default {
                            available.clear();
                            continue;
                        }
                        match alias::resolve_in_frame(func, &defs, &frames, frame, *ptr) {
                            Some(sform) => {
                                let ssz = crate::passes::loop_memory_promote::byte_size(*ty);
                                let keep = |entry: &AvailableLoad| {
                                    match (
                                        crate::passes::loop_memory_promote::byte_size(entry.1),
                                        ssz,
                                    ) {
                                        // Keep ONLY loads provably disjoint from the store:
                                        // a may-aliasing store clobbers the loaded value,
                                        // so anything not proven disjoint must be
                                        // invalidated. (The inverted predicate kept stale
                                        // loads: gvn_global_partial_store forwarded a word
                                        // load across a byte store to the same address.)
                                        (Some(lsz), Some(ssz)) => {
                                            alias::forms_disjoint(&entry.0, lsz, &sform, ssz, true)
                                        }
                                        // Unknown size: not provably disjoint.
                                        _ => false,
                                    }
                                };
                                available.retain(keep);
                                // Copy is not a truncating or bit-reinterpreting load.
                                // Only a representation-compatible, single-definition value may be
                                // forwarded; narrow stores of wider operands stay real.
                                if !*volatile
                                    && ssz.is_some()
                                    && let Operand::Value(value) = val
                                    && !multi_def.contains(&value.0)
                                    && defs
                                        .get(&value.0)
                                        .and_then(|i| i.result_type())
                                        .is_some_and(|source_ty| {
                                            same_memory_value_type(source_ty, *ty)
                                        })
                                {
                                    available.push((sform, *ty, *value, *ptr));
                                }
                            }
                            None => available.clear(),
                        }
                    }
                    Instruction::Intrinsic {
                        op, dest_ptr, args, ..
                    } => {
                        // A modeled `VecStore*` is a memory write like any
                        // scalar Store: resolve its (base, index, disp)
                        // address and retain only provably-disjoint loads.
                        // This is what lets the SLP'd nbody pair loop keep
                        // its `bodies[j].mass` load available across the
                        // packed vx/vy stores (stride-period field
                        // disjointness) instead of re-loading it for the
                        // scalar vz tail. Unmodeled write intrinsics
                        // (dest_ptr set, family not in the table) and
                        // unresolvable addresses fail closed: clear.
                        if let Some((true, ssz, base, index, disp)) =
                            alias::vec_intrinsic_access(op, args)
                        {
                            match alias::resolve_vec_addr_in_frame(
                                func, &defs, &frames, frame, base, index, disp,
                            ) {
                                Some(sform) => {
                                    let keep = |entry: &AvailableLoad| match (
                                        crate::passes::loop_memory_promote::byte_size(entry.1),
                                        Some(ssz),
                                    ) {
                                        (Some(lsz), Some(_)) => {
                                            alias::forms_disjoint(&entry.0, lsz, &sform, ssz, true)
                                        }
                                        _ => false,
                                    };
                                    available.retain(keep);
                                }
                                None => available.clear(),
                            }
                        } else if dest_ptr.is_some() || inst.may_write_memory() {
                            available.clear();
                        }
                        // Read-only / pure intrinsics: no memory effect.
                    }
                    // Calls, atomics, inline asm, variadics: unknown memory
                    // effects, kill everything.  Intrinsics with a write
                    // pointer kill everything (read-only intrinsics are fine).
                    Instruction::Call { .. } | Instruction::CallIndirect { .. } => {
                        if let Some(analysis) = ipa {
                            retain_available_across_call(func, bi, ii, &mut available, analysis);
                        } else {
                            available.clear();
                        }
                    }
                    Instruction::Memcpy { .. }
                    | Instruction::AtomicRmw { .. }
                    | Instruction::AtomicCmpxchg { .. }
                    | Instruction::AtomicStore { .. }
                    | Instruction::Fence { .. }
                    | Instruction::InlineAsm { .. }
                    | Instruction::VaStart { .. }
                    | Instruction::VaEnd { .. }
                    | Instruction::VaCopy { .. } => {
                        available.clear();
                    }
                    Instruction::AtomicLoad { .. } => {
                        // Atomics are fenced memory effects: conservative kill.
                        available.clear();
                    }
                    Instruction::StackRestore { .. } | Instruction::DynAlloca { .. } => {
                        available.clear();
                    }
                    other if other.may_write_memory() => available.clear(),
                    _ => {}
                }
            }
            per_block_removals.push(removed);
        }
    } // defs dropped: mutation phase

    let mut total = 0usize;
    for (bi, removed) in per_block_removals.into_iter().enumerate() {
        if removed.is_empty() {
            continue;
        }
        let mut removed_iter = removed.iter().copied();
        let mut next_remove = removed_iter.next();
        let mut idx = 0usize;
        func.blocks[bi].instructions.retain(|_| {
            let cur = idx;
            idx += 1;
            if Some(cur) == next_remove {
                next_remove = removed_iter.next();
                total += 1;
                false
            } else {
                true
            }
        });
        // Parallel spans array must stay in lockstep with instructions.
        if !func.blocks[bi].source_spans.is_empty() {
            let mut removed_iter2 = removed.iter().copied();
            let mut next2 = removed_iter2.next();
            let mut idx2 = 0usize;
            func.blocks[bi].source_spans.retain(|_| {
                let cur = idx2;
                idx2 += 1;
                if Some(cur) == next2 {
                    next2 = removed_iter2.next();
                    false
                } else {
                    true
                }
            });
        }
    }

    if !all_rewrites.is_empty() {
        // Store operands can themselves be forwarded loads, including defs
        // in dominators appearing later in layout. Flatten once before the
        // one-step canonical operand visitor, so no use names a removed load.
        let rewrites = all_rewrites.clone();
        for replacement in all_rewrites.values_mut() {
            for _ in 0..rewrites.len() {
                let Some(next) = rewrites.get(replacement) else {
                    break;
                };
                *replacement = *next;
            }
            debug_assert!(!rewrites.contains_key(replacement), "cyclic SSA forwarding");
        }
        for block in func.blocks.iter_mut() {
            for inst in block.instructions.iter_mut() {
                crate::ir::instruction::replace_values_in_inst_map(inst, &all_rewrites);
            }
            // Canonical terminator walk.  The hand-rolled match this replaces
            // covered conditional branches, switches and returns but not
            // `IndirectBranch`: a duplicate load whose result is a computed-goto
            // target survived as a reference to a value this pass had just
            // removed.  (Predates the consolidation, which is why it is fixed
            // here rather than inherited.)
            crate::ir::instruction::replace_values_in_terminator_map(
                &mut block.terminator,
                &all_rewrites,
            );
        }
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volatile_loads_are_never_merged() {
        // Two volatile loads of the same global must both survive: each read
        // is an observable side effect (C11 5.1.2.3).
        let mut func = crate::ir::reexports::IrFunction {
            pass_policy: Default::default(),
            name: "vol".into(),
            return_type: crate::common::types::IrType::I32,
            params: vec![],
            blocks: vec![crate::ir::reexports::BasicBlock {
                label: crate::ir::reexports::BlockId(1),
                instructions: vec![
                    crate::ir::reexports::Instruction::GlobalAddr {
                        dest: Value(0),
                        name: "g".into(),
                    },
                    crate::ir::reexports::Instruction::Load {
                        dest: Value(1),
                        ptr: Value(0),
                        ty: crate::common::types::IrType::I32,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: true,
                    },
                    crate::ir::reexports::Instruction::Load {
                        dest: Value(2),
                        ptr: Value(0),
                        ty: crate::common::types::IrType::I32,
                        seg_override: crate::common::types::AddressSpace::Default,
                        volatile: true,
                    },
                    crate::ir::reexports::Instruction::BinOp {
                        dest: Value(3),
                        op: crate::ir::reexports::IrBinOp::Add,
                        lhs: Operand::Value(Value(1)),
                        rhs: Operand::Value(Value(2)),
                        ty: crate::common::types::IrType::I32,
                    },
                ],
                terminator: Terminator::Return(Some(Operand::Value(Value(3)))),
                source_spans: Vec::new(),
            }],
            is_variadic: false,
            is_declaration: false,
            is_static: false,
            is_inline: false,
            is_always_inline: false,
            x86_isa_needs: Default::default(),
            is_noinline: false,
            target_features: Vec::new(),
            next_value_id: 4,
            fp_expr_tags: Default::default(),
            next_label: 2,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: false,
            has_inlined_calls: false,
            param_alloca_values: vec![],
            uses_sret: false,
            is_fastcall: false,
            regparm: None,
            is_naked: false,
            no_instrument: false,
            global_init_label_blocks: vec![],
            ret_eightbyte_classes: vec![],
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
            ssp: Default::default(),
            body_span: crate::common::source::Span::dummy(),
            terminator_spans: Default::default(),
        };
        let removed = run(&mut func);
        assert_eq!(removed, 0, "volatile loads must not merge");
        let loads = func.blocks[0]
            .instructions
            .iter()
            .filter(|i| matches!(i, crate::ir::reexports::Instruction::Load { .. }))
            .count();
        assert_eq!(loads, 2);
    }

    /// A duplicate load whose only consumer is an `IndirectBranch` target: the
    /// terminator match this pass used to carry covered conditional branches,
    /// switches and returns but not `IndirectBranch`, so the branch kept naming
    /// the value the pass had just deleted.  The canonical terminator walk
    /// covers every variant, which is why it is used instead of a match.
    ///
    /// Shape: [g = &t; a = load g; b = load g; indirect-branch b] — `b` is a
    /// duplicate of `a` and must be removed *and* redirected.
    #[test]
    fn indirect_branch_target_is_redirected_when_its_load_is_merged() {
        use crate::common::types::{AddressSpace, IrType};
        use crate::ir::reexports::{BasicBlock, BlockId};

        let mut func = IrFunction::new("tgt".into(), IrType::Void, vec![], false);
        func.blocks.push(BasicBlock {
            label: BlockId(1),
            instructions: vec![
                Instruction::GlobalAddr {
                    dest: Value(0),
                    name: "g".into(),
                },
                Instruction::Load {
                    dest: Value(1),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(2),
                    ptr: Value(0),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
            ],
            terminator: Terminator::IndirectBranch {
                target: Operand::Value(Value(2)),
                possible_targets: vec![BlockId(2), BlockId(3)],
            },
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(2),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });
        func.blocks.push(BasicBlock {
            label: BlockId(3),
            instructions: vec![],
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        });

        let merged = run(&mut func);
        assert!(merged > 0, "the duplicate load must be merged");

        let live: std::collections::HashSet<u32> = func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .filter_map(|i| i.dest())
            .map(|d| d.0)
            .collect();
        let Terminator::IndirectBranch { target, .. } = &func.blocks[0].terminator else {
            panic!("terminator changed shape");
        };
        let Operand::Value(t) = target else {
            panic!("indirect branch target must stay a value");
        };
        assert!(
            live.contains(&t.0),
            "terminator target v{} names a value the pass removed (live defs: {live:?})",
            t.0
        );
    }
}

#[cfg(test)]
mod stored_value_tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst};

    fn store(ptr: u32, value: u32, ty: IrType) -> Instruction {
        Instruction::Store {
            ptr: Value(ptr),
            val: Operand::Value(Value(value)),
            ty,
            volatile: false,
            seg_override: AddressSpace::Default,
        }
    }
    fn load(dest: u32, ptr: u32, ty: IrType) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty,
            volatile: false,
            seg_override: AddressSpace::Default,
        }
    }
    fn base() -> IrFunction {
        let mut f = IrFunction::new("forward".into(), IrType::U64, vec![], false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            source_spans: vec![],
            instructions: vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(1),
                    param_idx: 1,
                    ty: IrType::Ptr,
                },
                Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 2,
                    ty: IrType::U64,
                },
                Instruction::GetElementPtr {
                    dest: Value(3),
                    base: Value(0),
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::Ptr,
                },
            ],
            terminator: Terminator::Return(None),
        });
        f
    }

    #[test]
    fn store_load_store_load_chain_rewrites_to_a_live_definition() {
        let mut f = base();
        f.blocks[0].instructions.extend([
            store(0, 2, IrType::U64),
            load(4, 0, IrType::U64),
            store(3, 4, IrType::U64),
            load(5, 3, IrType::U64),
        ]);
        f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(5))));
        f.blocks[0].source_spans =
            vec![crate::common::source::Span::dummy(); f.blocks[0].instructions.len()];
        assert_eq!(run(&mut f), 2);
        assert!(matches!(
            f.blocks[0].terminator,
            Terminator::Return(Some(Operand::Value(Value(2))))
        ));
        for inst in &f.blocks[0].instructions {
            inst.for_each_used_value(|v| assert!(v != 4 && v != 5, "dangling forwarded value"));
        }
        assert_eq!(
            f.blocks[0].instructions.len(),
            f.blocks[0].source_spans.len()
        );
    }

    #[test]
    fn only_compatible_producer_store_load_types_forward() {
        for ty in [IrType::U8, IrType::I64, IrType::F64, IrType::U64] {
            let mut f = base();
            f.blocks[0]
                .instructions
                .extend([store(0, 2, ty), load(4, 0, ty)]);
            assert_eq!(
                run(&mut f),
                usize::from(matches!(ty, IrType::U64 | IrType::I64)),
                "{ty:?}"
            );
        }
    }

    #[test]
    fn signed_unsigned_forwarding_requires_full_width_carriers() {
        for (signed, unsigned) in [
            (IrType::I8, IrType::U8),
            (IrType::I16, IrType::U16),
            (IrType::I32, IrType::U32),
            (IrType::I64, IrType::U64),
        ] {
            for (source, target) in [(signed, unsigned), (unsigned, signed)] {
                let mut f = base();
                if let Instruction::ParamRef { ty, .. } = &mut f.blocks[0].instructions[2] {
                    *ty = source;
                }
                f.blocks[0]
                    .instructions
                    .extend([store(0, 2, source), load(4, 0, target)]);
                f.blocks[0].terminator = Terminator::Return(Some(Operand::Value(Value(4))));
                let full = source == IrType::I64 || source == IrType::U64;
                assert_eq!(run(&mut f), usize::from(full), "{source:?} -> {target:?}");
                assert!(
                    matches!(f.blocks[0].terminator, Terminator::Return(Some(Operand::Value(v)))
                    if v == Value(if full { 2 } else { 4 }))
                );
            }
        }
    }

    #[test]
    fn unknown_alias_kills_but_same_root_disjoint_store_preserves() {
        for ptr in [1, 3] {
            let mut f = base();
            f.blocks[0].instructions.extend([
                store(0, 2, IrType::U64),
                store(ptr, 2, IrType::U64),
                load(4, 0, IrType::U64),
            ]);
            assert_eq!(run(&mut f), usize::from(ptr == 3));
        }
    }

    #[test]
    fn volatile_segment_and_multidef_sources_do_not_forward() {
        for case in 0..4 {
            let mut f = base();
            let mut s = store(0, 2, IrType::U64);
            if let Instruction::Store {
                volatile,
                seg_override,
                ..
            } = &mut s
            {
                *volatile = case == 0;
                if case == 1 {
                    *seg_override = AddressSpace::SegFs;
                }
            }
            f.blocks[0].instructions.push(s);
            if case == 2 {
                f.blocks[0].instructions.push(Instruction::ParamRef {
                    dest: Value(2),
                    param_idx: 3,
                    ty: IrType::U64,
                });
            }
            let mut l = load(4, 0, IrType::U64);
            if case == 3
                && let Instruction::Load { volatile, .. } = &mut l
            {
                *volatile = true;
            }
            f.blocks[0].instructions.push(l);
            assert_eq!(run(&mut f), 0, "case {case}");
        }
    }

    #[test]
    fn redefined_base_copy_does_not_resolve_through_its_last_definition() {
        let mut f = base();
        f.blocks[0].instructions.extend([
            Instruction::Copy {
                dest: Value(6),
                src: Operand::Value(Value(0)),
            },
            store(6, 2, IrType::U64),
            Instruction::Copy {
                dest: Value(6),
                src: Operand::Value(Value(1)),
            },
            load(7, 6, IrType::U64),
        ]);
        assert_eq!(run(&mut f), 0);
    }
}

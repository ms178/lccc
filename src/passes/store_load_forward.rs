//! Store-to-load forwarding for alloca fields.
//!
//! After inlining and unrolling, aggregate code looks like
//! `store v1 -> [agg + 0]; store v2 -> [agg + 8]; ...; load [agg + 0]`
//! spread across a chain of blocks. When the load's address is a
//! compile-time-constant offset of the same alloca root and the same value is
//! stored on every incoming path, the load can be replaced by a Copy of the
//! stored SSA value — turning struct memory traffic into register dataflow
//! (the SROA effect for fully-scalarized aggregates).
//!
//! A forward dataflow computes, per block, the map (root, offset) -> stored
//! value. At control-flow joins only entries on which ALL predecessors agree
//! survive. Semantics of the transfer function are DEFAULT-CLOSED: only
//! instructions whose memory effects this pass models precisely (Store, Load,
//! Memcpy with tracked destination) may keep the map alive — EVERY other
//! instruction that can write memory (calls, intrinsics via dest_ptr, va_arg,
//! atomics incl. AtomicInc, inline asm, stack restore, DynAlloca) clears it.
//! This is the lesson of the aggregate_copy_forward bug family: the fail-open
//! allowlist variant of this pass (levkropp 0980060d) missed Intrinsic writes
//! exactly the way the SSE __m128i-init deletion bug did.
//!
//! Additional guards over the original:
//!   * volatile / semantic_volatile allocas are never tracked;
//!   * stores through pointers with a non-default segment override are not
//!     forwarded (they may not even target the tracked object);
//!   * escaped roots (address stored to memory, passed as operand of any
//!     unmodeled instruction) are dropped from tracking entirely, so a
//!     write through a reloaded pointer cannot be missed;
//!   * one shared transfer function for analysis and rewrite (the original
//!     kept two copies that could drift apart).
//!
//! Disable with CCC_NO_SL_FORWARD=1.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis;
use crate::ir::reexports::{CallInfo, Instruction, IrFunction, Operand, Value};

/// A constant byte path from an alloca root: (root value id, byte offset).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FieldPath {
    root: u32,
    offset: i64,
}

fn type_size(ty: IrType) -> i64 {
    use crate::common::types::IrType::*;
    match ty {
        I8 | U8 => 1,
        I16 | U16 => 2,
        I32 | U32 | F32 => 4,
        I64 | U64 | F64 | Ptr => 8,
        _ => 16,
    }
}

/// Resolve pointer values to (alloca root, constant byte offset), and compute
/// the set of roots whose address escapes the modeled instruction set.
/// Escaped roots are excluded from `paths` entirely (fail closed).
fn build_field_paths(func: &IrFunction) -> FxHashMap<u32, FieldPath> {
    let mut paths: FxHashMap<u32, FieldPath> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca {
                dest,
                volatile,
                semantic_volatile,
                ..
            } = inst
            {
                if !volatile && !semantic_volatile {
                    paths.insert(
                        dest.0,
                        FieldPath {
                            root: dest.0,
                            offset: 0,
                        },
                    );
                }
            }
        }
    }
    loop {
        let mut changed = false;
        for block in &func.blocks {
            for inst in &block.instructions {
                let derived = match inst {
                    Instruction::GetElementPtr {
                        dest,
                        base,
                        offset: Operand::Const(c),
                        ..
                    } => c.to_i64().and_then(|off| {
                        paths.get(&base.0).map(|p| {
                            (
                                dest.0,
                                FieldPath {
                                    root: p.root,
                                    offset: p.offset + off,
                                },
                            )
                        })
                    }),
                    Instruction::Copy {
                        dest,
                        src: Operand::Value(src),
                    } => paths.get(&src.0).copied().map(|p| (dest.0, p)),
                    _ => None,
                };
                if let Some((dest, path)) = derived {
                    if !paths.contains_key(&dest) {
                        paths.insert(dest, path);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Escape analysis, default-closed: any use of a tracked pointer outside
    // the modeled shapes (GEP base / Copy src derivation above, Store ptr,
    // Load ptr, Memcpy dest) escapes its ROOT. In particular a pointer used
    // as a STORED VALUE, a call argument, an intrinsic arg/dest_ptr, a phi
    // input, a select arm, or a terminator operand escapes — after that a
    // write through a reloaded alias could invalidate entries we would never
    // see. Escaped roots are removed from tracking wholesale.
    let mut escaped: FxHashSet<u32> = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::GetElementPtr { base, offset, .. } => {
                    // Variable-offset GEP derives an untracked pointer from
                    // the root: writes through it are invisible to the map.
                    if matches!(offset, Operand::Value(_)) {
                        if let Some(p) = paths.get(&base.0) {
                            escaped.insert(p.root);
                        }
                    }
                }
                Instruction::Copy { .. } => {}
                Instruction::Store { val, ptr: _, .. } => {
                    if let Operand::Value(v) = val {
                        if let Some(p) = paths.get(&v.0) {
                            escaped.insert(p.root);
                        }
                    }
                }
                Instruction::Load { .. } => {}
                Instruction::Memcpy { src, dest: _, .. } => {
                    // src is read-only: no escape. dest is modeled in the
                    // transfer function (kills overlapping entries).
                    let _ = src;
                }
                _ => {
                    // EVERYTHING else: any referenced tracked pointer escapes
                    // its root (visitors cover Intrinsic args + dest_ptr,
                    // call args, va_arg, atomics, selects, phis, asm).
                    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                        if let Operand::Value(v) = op {
                            if let Some(p) = paths.get(&v.0) {
                                escaped.insert(p.root);
                            }
                        }
                    });
                    crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                        if let Some(p) = paths.get(&v.0) {
                            escaped.insert(p.root);
                        }
                    });
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                if let Some(p) = paths.get(&v.0) {
                    escaped.insert(p.root);
                }
            }
        });
    }
    paths.retain(|_, p| !escaped.contains(&p.root));
    paths
}

/// Shared transfer function: apply one instruction's memory effect to the
/// running (root, offset) -> (value, size) map. When `rewrite` is set,
/// forwardable loads are replaced by Copies (counted in `changed`).
fn apply_inst(
    inst: &mut Instruction,
    paths: &FxHashMap<u32, FieldPath>,
    map: &mut FxHashMap<FieldPath, (Operand, i64)>,
    rewrite: bool,
    changed: &mut usize,
) {
    match inst {
        Instruction::Store {
            val,
            ptr,
            ty,
            seg_override,
            volatile,
            ..
        } => {
            if *seg_override != AddressSpace::Default {
                // Segment-relative store: target unknown, kill everything.
                map.clear();
                return;
            }
            if let Some(fp) = paths.get(&ptr.0).copied() {
                let size = type_size(*ty);
                // Kill overlapping field entries (a wide store covers narrower
                // fields starting within its range).
                map.retain(|ofp, &mut (_v, fs)| {
                    ofp.root != fp.root
                        || ofp.offset + fs <= fp.offset
                        || fp.offset + size <= ofp.offset
                });
                if !*volatile {
                    map.insert(fp, (*val, size));
                }
            } else {
                // Store through an untracked pointer may alias anything.
                map.clear();
            }
        }
        Instruction::Load {
            dest,
            ptr,
            ty,
            seg_override,
            volatile,
            ..
        } => {
            if *seg_override != AddressSpace::Default {
                return;
            }
            // Volatile loads are observable side effects (C11 5.1.2.3):
            // the memory read must actually happen.
            if *volatile {
                return;
            }
            if let Some(fp) = paths.get(&ptr.0) {
                if let Some(&(stored_op, store_size)) = map.get(fp) {
                    let is_self_copy = match stored_op {
                        Operand::Value(v) => v.0 == dest.0,
                        Operand::Const(_) => false,
                    };
                    if rewrite && store_size == type_size(*ty) && !is_self_copy {
                        *inst = Instruction::Copy {
                            dest: *dest,
                            src: stored_op,
                        };
                        *changed += 1;
                    }
                }
            }
        }
        Instruction::Memcpy { dest, size, .. } => match paths.get(&dest.0).copied() {
            Some(d) => {
                let sz = *size as i64;
                map.retain(|fp, &mut (_v, fs)| {
                    fp.root != d.root || fp.offset + fs <= d.offset || d.offset + sz <= fp.offset
                });
            }
            None => map.clear(),
        },
        // Pure value computations: no memory effect.
        Instruction::Alloca { .. }
        | Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::Cast { .. }
        | Instruction::Copy { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::Phi { .. }
        | Instruction::Select { .. }
        | Instruction::ParamRef { .. }
        | Instruction::StackSave { .. }
        | Instruction::Call {
            info:
                CallInfo {
                    is_pure: true, ..
                }
                | CallInfo {
                    is_const: true, ..
                },
            ..
        }
        | Instruction::CallIndirect {
            info:
                CallInfo {
                    is_pure: true, ..
                }
                | CallInfo {
                    is_const: true, ..
                },
            ..
        } => {}
        // DEFAULT-CLOSED: every other instruction may write memory (calls,
        // intrinsics via dest_ptr, va_arg family, ALL atomics incl.
        // AtomicInc, inline asm, stack restore, dyn alloca, PGO counters,
        // fences with unknown pairing...). Kill the whole map.
        _ => {
            map.clear();
        }
    }
}

/// Run store-to-load forwarding on a function. Returns loads replaced.
pub(crate) fn run(func: &mut IrFunction) -> usize {
    if std::env::var("CCC_NO_SL_FORWARD").is_ok() {
        return 0;
    }
    let paths = build_field_paths(func);
    if paths.is_empty() {
        return 0;
    }

    let label_to_idx = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_to_idx);
    let n = func.blocks.len();

    // Forward dataflow to a fixpoint. Entry blocks start empty; IN[b] is the
    // agreement-intersection of OUT over all predecessors. Because the maps
    // only shrink at joins and kills, and grow only through stores, the
    // lattice is finite and the worklist terminates.
    let mut in_map: Vec<FxHashMap<FieldPath, (Operand, i64)>> =
        (0..n).map(|_| FxHashMap::default()).collect();
    let mut out_map: Vec<FxHashMap<FieldPath, (Operand, i64)>> =
        (0..n).map(|_| FxHashMap::default()).collect();
    let mut computed = vec![false; n];
    let mut worklist: Vec<usize> = (0..n).collect();
    while let Some(b) = worklist.pop() {
        let mut acc: Option<FxHashMap<FieldPath, (Operand, i64)>> = None;
        for &p in preds.row(b).iter() {
            let p = p as usize;
            acc = Some(match acc {
                None => out_map[p].clone(),
                Some(mut a) => {
                    a.retain(|fp, v| out_map[p].get(fp) == Some(&*v));
                    a
                }
            });
        }
        let in_b = acc.unwrap_or_default();
        if computed[b] && in_b == in_map[b] {
            continue;
        }
        computed[b] = true;
        in_map[b] = in_b.clone();
        let mut m = in_b;
        let mut dummy = 0;
        for inst in &mut func.blocks[b].instructions {
            apply_inst(inst, &paths, &mut m, false, &mut dummy);
        }
        if m != out_map[b] {
            out_map[b] = m;
            for &s in succs.row(b).iter() {
                worklist.push(s as usize);
            }
        }
    }

    // Rewrite pass using the converged IN maps.
    let mut changes = 0;
    for b in 0..n {
        let mut m = in_map[b].clone();
        for inst in &mut func.blocks[b].instructions {
            apply_inst(inst, &paths, &mut m, true, &mut changes);
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::AddressSpace;
    use crate::ir::reexports::{BasicBlock, BlockId, IrConst, Terminator};

    fn mk_func(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::I64, vec![], false);
        f.blocks = blocks;
        f.next_value_id = 1000;
        f
    }

    fn block(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
        }
    }

    fn store(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            volatile: false,
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            volatile: false,
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        }
    }

    fn alloca(dest: u32) -> Instruction {
        Instruction::Alloca {
            dest: Value(dest),
            ty: IrType::Ptr,
            size: 16,
            align: 8,
            volatile: false,
            semantic_volatile: false,
        }
    }

    #[test]
    fn forwards_simple_store_to_load() {
        let mut f = mk_func(vec![block(
            0,
            vec![alloca(1), store(7, 1), load(2, 1)],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            f.blocks[0].instructions[2],
            Instruction::Copy {
                dest: Value(2),
                src: Operand::Value(Value(7))
            }
        ));
    }

    #[test]
    fn intrinsic_kills_map() {
        // Intrinsic may write through dest_ptr — the fail-open original
        // forwarded across it (the __m128i-init deletion bug shape).
        let mut f = mk_func(vec![block(
            0,
            vec![
                alloca(1),
                store(7, 1),
                Instruction::Intrinsic {
                    dest: None,
                    op: crate::ir::intrinsics::IntrinsicOp::AddPs128,
                    dest_ptr: Some(Value(1)),
                    args: vec![],
                },
                load(2, 1),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
        assert!(matches!(
            f.blocks[0].instructions[3],
            Instruction::Load { .. }
        ));
    }

    #[test]
    fn volatile_alloca_not_tracked() {
        let mut f = mk_func(vec![block(
            0,
            vec![
                Instruction::Alloca {
                    dest: Value(1),
                    ty: IrType::Ptr,
                    size: 16,
                    align: 8,
                    volatile: true,
                    semantic_volatile: false,
                },
                store(7, 1),
                load(2, 1),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn escaped_root_not_tracked() {
        // The alloca's address is itself stored to memory: a later reload
        // could write through an alias the map never sees.
        let mut f = mk_func(vec![block(
            0,
            vec![
                alloca(1),
                alloca(3),
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(1)), // address escapes!
                    ptr: Value(3),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                },
                store(7, 1),
                load(2, 1),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn call_kills_map() {
        use crate::ir::reexports::CallInfo;
        let mut f = mk_func(vec![block(
            0,
            vec![
                alloca(1),
                store(7, 1),
                Instruction::Call {
                    func: "opaque".into(),
                    info: CallInfo::default(),
                },
                load(2, 1),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn join_requires_agreement() {
        // Two predecessors store DIFFERENT values to the same field: the
        // load in the join block must not be forwarded.
        let gep = |dest: u32, base: u32| Instruction::GetElementPtr {
            dest: Value(dest),
            base: Value(base),
            offset: Operand::Const(IrConst::I64(8)),
            ty: IrType::I64,
        };
        let mut f = mk_func(vec![
            block(
                0,
                vec![alloca(1), gep(10, 1)],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I64(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(1, vec![store(7, 10)], Terminator::Branch(BlockId(3))),
            block(2, vec![store(8, 10)], Terminator::Branch(BlockId(3))),
            block(
                3,
                vec![load(2, 10)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
        ]);
        assert_eq!(run(&mut f), 0);
        assert!(matches!(
            f.blocks[3].instructions[0],
            Instruction::Load { .. }
        ));
    }

    #[test]
    fn join_forwards_agreeing_value() {
        let mut f = mk_func(vec![
            block(
                0,
                vec![alloca(1), store(7, 1)],
                Terminator::CondBranch {
                    cond: Operand::Const(IrConst::I64(1)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            block(1, vec![], Terminator::Branch(BlockId(3))),
            block(2, vec![], Terminator::Branch(BlockId(3))),
            block(
                3,
                vec![load(2, 1)],
                Terminator::Return(Some(Operand::Value(Value(2)))),
            ),
        ]);
        assert_eq!(run(&mut f), 1);
        assert!(matches!(
            f.blocks[3].instructions[0],
            Instruction::Copy {
                dest: Value(2),
                src: Operand::Value(Value(7))
            }
        ));
    }

    #[test]
    fn overlapping_narrow_store_kills_wide_entry() {
        // I64 store, then an I8 store into byte 3 of the same field, then an
        // I64 load: must NOT forward the stale wide value.
        let gep3 = Instruction::GetElementPtr {
            dest: Value(10),
            base: Value(1),
            offset: Operand::Const(IrConst::I64(3)),
            ty: IrType::I8,
        };
        let mut f = mk_func(vec![block(
            0,
            vec![
                alloca(1),
                store(7, 1),
                gep3,
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(9)),
                    ptr: Value(10),
                    ty: IrType::I8,
                    seg_override: AddressSpace::Default,
                },
                load(2, 1),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn size_mismatch_not_forwarded() {
        // I64 store, I32 load of the same offset: byte-exact subset but the
        // pass only forwards size-matched accesses.
        let mut f = mk_func(vec![block(
            0,
            vec![
                alloca(1),
                store(7, 1),
                Instruction::Load {
                    volatile: false,
                    dest: Value(2),
                    ptr: Value(1),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
    }

    #[test]
    fn variable_gep_escapes_root() {
        // A variable-offset GEP off the root makes ALL fields untrackable
        // (a store through it could hit any offset).
        let mut f = mk_func(vec![block(
            0,
            vec![
                alloca(1),
                store(7, 1),
                Instruction::GetElementPtr {
                    dest: Value(10),
                    base: Value(1),
                    offset: Operand::Value(Value(99)),
                    ty: IrType::I8,
                },
                Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(9)),
                    ptr: Value(10),
                    ty: IrType::I8,
                    seg_override: AddressSpace::Default,
                },
                load(2, 1),
            ],
            Terminator::Return(Some(Operand::Value(Value(2)))),
        )]);
        assert_eq!(run(&mut f), 0);
    }
}

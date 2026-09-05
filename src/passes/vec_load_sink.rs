//! Single-use vector-load sinking (x86 VLFOLD enabler).
//!
//! The x86 vector emitter can fold a 256-bit `VecLoad*` straight into the
//! memory operand of its consumer (`vaddps (%rdi,%r10), %ymm0, %ymm0`,
//! see `compute_vector_memfold_values` / `try_elide_vec_load`), but only
//! when the load is *adjacent* to that consumer (position `j-1`, or `j-2`
//! behind another pure load).  The vectorizer, however, emits every stream
//! load at the top of the body:
//!
//! ```text
//!   %y = VecLoadF32x8 y, iv        ; <- consumed three instructions later
//!   %x = VecLoadF32x8 x, iv
//!   %p = VecMulF32x8 %a, %x        ; folds %x (adjacent)
//!   %s = VecAddF32x8 %y, %p        ; %y NOT foldable: staged via %ymm0 -> slot
//!   VecStoreF32x8 %s -> y, iv
//! ```
//!
//! Without this pass the `y[i]` load is loaded into `%ymm0`, spilled to a
//! 32-byte stack slot (the scratch register is needed for the product),
//! and re-read as the `vaddps` memory operand: two extra memory µops and a
//! 136-byte frame in every `y[i] += a*x[i]`-shaped loop compiled at the
//! default `-ffp-contract=off` (GCC/ICX/Clang emit three instructions).
//!
//! The pass moves a pure, non-volatile vector load with exactly one use to
//! the instruction slot immediately before that use, when
//!
//! * the use is in the same basic block and is an `Intrinsic`
//!   (the only consumers that can take a memory operand), and
//! * every instruction strictly between the load and the use is free of
//!   memory writes and side effects: pure intrinsics (`IntrinsicOp::is_pure`
//!   with no `dest_ptr`), scalar arithmetic / compares / casts / GEPs /
//!   selects / copies, and **non-volatile** scalar loads.
//!
//! Sinking a read past other reads is always semantics-preserving; crossing
//! any store, call, atomic, memcpy, inline asm, volatile access or counter
//! update is refused fail-closed.  SSA guarantees the load's address
//! operands are still valid at the new position.  `source_spans` (parallel
//! to `instructions`) is kept in lock-step so `-g` line tables stay exact.
//!
//! Pass name for `CCC_DISABLE_PASSES`: `vec_load_sink`.  Kill switch:
//! `CCC_NO_VEC_LOAD_SINK=1`.  Trace: `CCC_DEBUG_VEC_LOAD_SINK=1`.

use crate::backend::liveness::{
    for_each_operand_in_instruction, for_each_operand_in_terminator,
    for_each_value_use_in_instruction,
};
use crate::common::fx_hash::FxHashMap;
use crate::ir::reexports::{Instruction, IrFunction, Operand};

/// One recorded use of an SSA value: (block index, instruction index).
/// `usize::MAX` as the instruction index denotes the block terminator.
type UseSite = (usize, usize);

/// Sinkable candidate: a pure, non-volatile vector load producing an SSA
/// value (no `dest_ptr`).
fn is_sinkable_vec_load(inst: &Instruction) -> bool {
    match inst {
        Instruction::Intrinsic {
            dest: Some(_),
            op,
            dest_ptr: None,
            ..
        } => {
            use crate::ir::intrinsics::IntrinsicOp as O;
            matches!(
                op,
                O::VecLoadF64x4
                    | O::VecLoadF32x8
                    | O::VecLoadI32x8
                    | O::VecLoadI64x4
                    | O::VecLoadF64x2
                    | O::VecLoadF32x4
                    | O::VecLoadI32x4
                    | O::VecLoadI64x2
            )
        }
        _ => false,
    }
}

/// May a vector load be moved *past* this instruction (i.e. may this
/// instruction execute before the load instead of after it)?  Only
/// instructions that neither write memory nor carry side effects qualify.
fn is_transparent_for_load(inst: &Instruction) -> bool {
    match inst {
        Instruction::Intrinsic {
            op, dest_ptr: None, ..
        } => op.is_pure(),
        Instruction::Load { volatile, .. } => !*volatile,
        Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Cast { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::Select { .. }
        | Instruction::Copy { .. } => true,
        _ => false,
    }
}

/// Collect every use site of every SSA value in `func` (operands, value
/// references such as store pointers / GEP bases, and terminator operands).
fn collect_use_sites(func: &IrFunction) -> FxHashMap<u32, Vec<UseSite>> {
    let mut uses: FxHashMap<u32, Vec<UseSite>> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    uses.entry(v.0).or_default().push((bi, ii));
                }
            });
            for_each_value_use_in_instruction(inst, |v| {
                uses.entry(v.0).or_default().push((bi, ii));
            });
        }
        for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                uses.entry(v.0).or_default().push((bi, usize::MAX));
            }
        });
    }
    uses
}

/// Sink single-use pure vector loads next to their consumer.  Returns the
/// number of loads moved.
pub(crate) fn sink_vector_loads(func: &mut IrFunction) -> usize {
    if std::env::var_os("CCC_NO_VEC_LOAD_SINK").is_some() {
        return 0;
    }
    if func.is_declaration || func.blocks.is_empty() {
        return 0;
    }
    let mut uses = collect_use_sites(func);
    let debug = std::env::var_os("CCC_DEBUG_VEC_LOAD_SINK").is_some();
    let mut moved = 0usize;

    for (bi, block) in func.blocks.iter_mut().enumerate() {
        // Walk backwards.  Moving the load at `i` to `uj-1` only permutes
        // instructions inside `[i, uj)`; every position `>= uj` and every
        // position `< i` is untouched.  A candidate still to be visited
        // (position `< i`) may have its consumer INSIDE the permuted window,
        // so the recorded use index can go stale — the consumer is therefore
        // re-located by value before each move (fail-closed).
        let mut i = block.instructions.len();
        while i > 0 {
            i -= 1;
            if !is_sinkable_vec_load(&block.instructions[i]) {
                continue;
            }
            let Instruction::Intrinsic { dest: Some(d), .. } = &block.instructions[i] else {
                continue;
            };
            let d = d.0;
            let Some(sites) = uses.get(&d) else {
                continue;
            };
            // Exactly one use total, in this block, not the terminator.
            let [(ub, uj)] = sites.as_slice() else {
                continue;
            };
            if *ub != bi || *uj == usize::MAX {
                continue;
            }
            // Earlier moves may have permuted positions inside `[i, uj)`, so
            // the recorded index is not trustworthy.  Re-locate the consumer
            // by value: the unique use of `d` is the first instruction after
            // the load that names it, and it must be an intrinsic (the only
            // consumer class able to take a memory operand).
            let Some(uj) = block.instructions[i + 1..]
                .iter()
                .position(|inst| {
                    matches!(inst, Instruction::Intrinsic { .. })
                        && {
                            let mut found = false;
                            for_each_operand_in_instruction(inst, |op| {
                                if matches!(op, Operand::Value(v) if v.0 == d) {
                                    found = true;
                                }
                            });
                            found
                        }
                })
                .map(|p| p + i + 1)
            else {
                continue;
            };
            // Not already adjacent (nothing to gain), not past the end.
            if uj <= i + 1 || uj >= block.instructions.len() {
                continue;
            }
            if !block.instructions[i + 1..uj]
                .iter()
                .all(is_transparent_for_load)
            {
                continue;
            }
            if debug {
                let consumer = match &block.instructions[uj] {
                    Instruction::Intrinsic { op, .. } => format!("{:?}", op),
                    _ => String::new(),
                };
                eprintln!(
                    "[VEC-LOAD-SINK] {} b{}: move %{} from i{} to i{} (consumer {})",
                    func.name,
                    bi,
                    d,
                    i,
                    uj - 1,
                    consumer
                );
            }
            let inst = block.instructions.remove(i);
            block.instructions.insert(uj - 1, inst);
            if block.source_spans.len() > uj - 1 {
                let span = block.source_spans.remove(i);
                block.source_spans.insert(uj - 1, span);
            }
            // The permutation `remove(i); insert(uj-1)` shifts every recorded
            // use site strictly inside (i, uj) down by one; sites at uj (the
            // moved load's own consumer) and outside the window are stable.
            // Without this remap, a still-unvisited load whose consumer sits
            // inside the permuted window fails the re-verification below and
            // its sink is forfeited (two independent streams in one block
            // sank only the higher-indexed load).
            for sites in uses.values_mut() {
                for (b, p) in sites.iter_mut() {
                    if *b == bi && *p > i && *p < uj {
                        *p -= 1;
                    }
                }
            }
            moved += 1;
        }
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes every test that reaches the pass entry: `sink_vector_loads`
    /// reads CCC_NO_VEC_LOAD_SINK and environment variables are
    /// PROCESS-GLOBAL, so the kill-switch test's set/remove window otherwise
    /// races with the other tests' env reads under cargo's default test
    /// parallelism.  Every call below goes through sink_locked.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sink_locked(f: &mut IrFunction) -> usize {
        let _g = ENV_LOCK.lock().unwrap();
        sink_vector_loads(f)
    }
    use crate::common::source::Span;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::intrinsics::IntrinsicOp as O;
    use crate::ir::reexports::{BasicBlock, BlockId, IrBinOp, IrConst, Terminator, Value};

    fn mkfunc() -> IrFunction {
        IrFunction::new("t".to_string(), IrType::Void, vec![], false)
    }

    fn vload(dest: u32, base: u32, idx: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op: O::VecLoadF32x8,
            dest_ptr: None,
            args: vec![Operand::Value(Value(base)), Operand::Value(Value(idx))],
        }
    }

    fn vbin(op: O, dest: u32, a: u32, b: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: Some(Value(dest)),
            op,
            dest_ptr: None,
            args: vec![Operand::Value(Value(a)), Operand::Value(Value(b))],
        }
    }

    fn vstore(val: u32, base: u32, idx: u32) -> Instruction {
        Instruction::Intrinsic {
            dest: None,
            op: O::VecStoreF32x8,
            dest_ptr: Some(Value(base)),
            args: vec![
                Operand::Value(Value(val)),
                Operand::Value(Value(base)),
                Operand::Value(Value(idx)),
            ],
        }
    }

    fn sload(dest: u32, ptr: u32, volatile: bool) -> Instruction {
        Instruction::Load {
            volatile,
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
        }
    }

    fn block(id: u32, instructions: Vec<Instruction>) -> BasicBlock {
        BasicBlock {
            label: BlockId(id),
            instructions,
            terminator: Terminator::Return(None),
            source_spans: Vec::new(),
        }
    }

    fn ops(f: &IrFunction, b: usize) -> Vec<String> {
        f.blocks[b]
            .instructions
            .iter()
            .map(|i| match i {
                Instruction::Intrinsic { dest, op, .. } => {
                    format!("{:?}:{}", op, dest.map(|v| v.0 as i64).unwrap_or(-1))
                }
                Instruction::Store { .. } => "store".to_string(),
                Instruction::Load { dest, .. } => format!("load:{}", dest.0),
                Instruction::BinOp { dest, .. } => format!("binop:{}", dest.0),
                _ => "other".to_string(),
            })
            .collect()
    }

    /// The saxpy shape: the `y` load sinks below the product so VLFOLD can
    /// fold it into `vaddps`.
    #[test]
    fn saxpy_y_load_sinks_to_add() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                vload(39, 5, 35),
                vbin(O::VecMulF32x8, 40, 38, 39),
                vbin(O::VecAddF32x8, 41, 37, 40),
                vstore(41, 4, 35),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 1);
        assert_eq!(
            ops(&f, 0),
            vec![
                "VecLoadF32x8:39",
                "VecMulF32x8:40",
                "VecLoadF32x8:37",
                "VecAddF32x8:41",
                "VecStoreF32x8:-1",
            ]
        );
        // Idempotent: nothing left to move.
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// `add(load_a, load_b)`: `load_b` is adjacent already; `load_a` sits at
    /// `j-2` and is moved to `j-1` (a harmless reorder among reads) — the
    /// exact contract is asserted so a change here is a conscious decision.
    #[test]
    fn adjacent_pair_contract() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(40, 5, 38),
                vload(41, 6, 38),
                vbin(O::VecAddI32x8, 42, 40, 41),
                vstore(42, 4, 38),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 1);
        assert_eq!(
            ops(&f, 0),
            vec!["VecLoadF32x8:41", "VecLoadF32x8:40", "VecAddI32x8:42", "VecStoreF32x8:-1"]
        );
    }

    /// A scalar store between the load and its use blocks the sink.
    #[test]
    fn intervening_store_blocks_sink() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                Instruction::Store {
                    volatile: false,
                    val: Operand::Const(IrConst::I32(1)),
                    ptr: Value(7),
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                },
                vload(39, 5, 35),
                vbin(O::VecMulF32x8, 40, 38, 39),
                vbin(O::VecAddF32x8, 41, 37, 40),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 0);
        assert_eq!(ops(&f, 0)[0], "VecLoadF32x8:37");
    }

    /// A vector store intrinsic (`dest_ptr` set) is a memory write: blocks.
    #[test]
    fn intervening_vector_store_blocks_sink() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                vstore(30, 9, 35),
                vbin(O::VecMulF32x8, 40, 38, 38),
                vbin(O::VecAddF32x8, 41, 37, 40),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// A call is opaque (may write the loaded memory): blocks.
    #[test]
    fn intervening_call_blocks_sink() {
        use crate::ir::reexports::CallInfo;
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                Instruction::Call {
                    func: "opaque".to_string(),
                    info: CallInfo::default(),
                },
                vbin(O::VecMulF32x8, 40, 38, 38),
                vbin(O::VecAddF32x8, 41, 37, 40),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// A volatile scalar load is an observable side effect: blocks.
    #[test]
    fn intervening_volatile_load_blocks_sink() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                sload(50, 7, true),
                vbin(O::VecMulF32x8, 40, 38, 38),
                vbin(O::VecAddF32x8, 41, 37, 40),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// Non-volatile scalar loads and scalar arithmetic are transparent.
    #[test]
    fn scalar_reads_and_arith_are_transparent() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                sload(50, 7, false),
                Instruction::BinOp {
                    dest: Value(51),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(50)),
                    rhs: Operand::Const(IrConst::I32(1)),
                    ty: IrType::I32,
                },
                vbin(O::VecMulF32x8, 40, 38, 38),
                vbin(O::VecAddF32x8, 41, 37, 40),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 1);
        assert_eq!(ops(&f, 0)[3], "VecLoadF32x8:37");
        assert_eq!(ops(&f, 0)[4], "VecAddF32x8:41");
    }

    /// Two uses (even in the same block) disqualify the load.
    #[test]
    fn multi_use_load_is_not_sunk() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                vbin(O::VecMulF32x8, 40, 38, 38),
                vbin(O::VecAddF32x8, 41, 37, 40),
                vbin(O::VecAddF32x8, 42, 37, 41),
            ],
        ));
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// A use in another block disqualifies the load (no cross-block motion).
    #[test]
    fn cross_block_use_is_not_sunk() {
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![vload(37, 4, 35), vbin(O::VecMulF32x8, 40, 38, 38)],
        ));
        f.blocks.push(block(1, vec![vbin(O::VecAddF32x8, 41, 37, 40)]));
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// A use by a non-intrinsic (e.g. a terminator operand) never sinks.
    #[test]
    fn terminator_use_is_not_sunk() {
        let mut f = mkfunc();
        let mut b = block(0, vec![vload(37, 4, 35), vbin(O::VecMulF32x8, 40, 38, 38)]);
        b.terminator = Terminator::Return(Some(Operand::Value(Value(37))));
        f.blocks.push(b);
        assert_eq!(sink_locked(&mut f), 0);
    }

    /// Kill switch honoured.
    #[test]
    fn kill_switch_disables_pass() {
        // Hold ENV_LOCK across the whole set/run/remove window: the env var
        // is process-global and every other sink_locked() caller reads it.
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("CCC_NO_VEC_LOAD_SINK", "1");
        let mut f = mkfunc();
        f.blocks.push(block(
            0,
            vec![
                vload(37, 4, 35),
                vbin(O::VecMulF32x8, 40, 38, 38),
                vbin(O::VecAddF32x8, 41, 37, 40),
            ],
        ));
        // Direct call: the guard above already serializes us.
        let n = sink_vector_loads(&mut f);
        std::env::remove_var("CCC_NO_VEC_LOAD_SINK");
        assert_eq!(n, 0);
    }

    /// Two independent sinks in one block keep their consumers correct and
    /// the parallel `source_spans` vector in lock-step.
    #[test]
    fn multiple_sinks_and_spans_stay_parallel() {
        let mut f = mkfunc();
        let mut b = block(
            0,
            vec![
                vload(10, 4, 35),                 // -> add %14
                vload(11, 5, 35),                 // -> add %15
                vbin(O::VecMulF32x8, 12, 38, 38), // pure filler
                vbin(O::VecMulF32x8, 13, 38, 38), // pure filler
                vbin(O::VecAddF32x8, 14, 10, 12),
                vbin(O::VecAddF32x8, 15, 11, 13),
            ],
        );
        b.source_spans = (0..6u32).map(|k| Span::new(100 + k, 100 + k, 0)).collect();
        f.blocks.push(b);
        assert_eq!(sink_locked(&mut f), 2);
        assert_eq!(
            ops(&f, 0),
            vec![
                "VecMulF32x8:12",
                "VecMulF32x8:13",
                "VecLoadF32x8:10",
                "VecAddF32x8:14",
                "VecLoadF32x8:11",
                "VecAddF32x8:15",
            ]
        );
        let starts: Vec<u32> = f.blocks[0].source_spans.iter().map(|s| s.start).collect();
        assert_eq!(starts, vec![102, 103, 100, 104, 101, 105]);
    }
}

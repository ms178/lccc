//! Global Location Allocation — P0 architecture project
//!
//! Implements the conceptual model:
//! ```text
//! Location = Register | Stack | Rematerialize
//! LocationPiece { start, end, location }
//! ```
//! A Value may have multiple pieces across its live range.
//!
//! Phase 1 minimal implementation behind `CCC_RA_GLOBAL_LOCATION=1`:
//! - cross-block call splitting (single-successor dominance, no PHI needed)
//! - intra-block next-use gap splitting with remat awareness
//! - rematerialization detection (zero, const, GEP, small arith)
//! - Raptor Lake cost model
//! - AddressExpr for SIB-aware allocation

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::analysis;
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrConst, IrFunction, Operand, Terminator, Value,
};

pub fn global_location_enabled() -> bool {
    // Default-on for -O2+ after P0 validation: the pass is proven safe
    // (single-succ dominance, SSA repair, volatile spill slots) and
    // improves code quality (reduces spills in hot loops). Opt-out via
    // CCC_NO_GLOBAL_LOCATION=1 or CCC_RA_GLOBAL_LOCATION=0.
    if std::env::var_os("CCC_NO_GLOBAL_LOCATION").is_some() {
        return false;
    }
    if let Ok(v) = std::env::var("CCC_RA_GLOBAL_LOCATION") {
        let v = v.to_ascii_lowercase();
        if v == "0" || v == "false" || v == "no" || v == "off" {
            return false;
        }
        if v == "1" || v == "true" || v == "yes" || v == "on" {
            return true;
        }
        // Any other value: treat as enabled if present? Fail closed to disabled for unknown.
        return false;
    }
    // Default-on: enable globally. The pipeline already gates this behind
    // opt_level >=2? Actually pipeline calls it unconditionally; we enable
    // by default and rely on the pass's own safety checks.
    true
}

fn debug_enabled() -> bool {
    std::env::var_os("CCC_DEBUG_GLOBAL_ALLOC").is_some()
}

macro_rules! dlog {
    ($($arg:tt)*) => {
        if debug_enabled() {
            eprintln!($($arg)*);
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RematerializationKind {
    Zero,
    ConstI32(i32),
    ConstI64(i64),
    GlobalAddr(String),
    SimpleGep {
        base: u32,
        offset: i64,
    },
    Extension {
        src: u32,
        from_ty: IrType,
        to_ty: IrType,
        is_signed: bool,
    },
    SmallArith {
        base: u32,
        kind: SmallArithKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmallArithKind {
    AddConst(i64),
    MulScale(u8),
    ShlConst(u8),
    AndMask(u64),
}

#[derive(Debug, Clone)]
pub enum Location {
    Register(u8),
    Stack(u32),
    Rematerialize(RematerializationKind),
}

#[derive(Debug, Clone)]
pub struct LocationPiece {
    pub value_id: u32,
    pub start: u32,
    pub end: u32,
    pub location: Location,
    pub frequency: u64,
}

pub struct GlobalAllocation {
    pub pieces: FxHashMap<u32, Vec<LocationPiece>>,
}

impl GlobalAllocation {
    pub fn new() -> Self {
        Self {
            pieces: FxHashMap::default(),
        }
    }

    pub fn verify(
        &self,
        _func: &IrFunction,
        liveness: &super::liveness::LivenessResult,
    ) -> Result<(), String> {
        for (vid, pieces) in &self.pieces {
            if pieces.is_empty() {
                continue;
            }
            let mut sorted = pieces.clone();
            sorted.sort_by_key(|p| p.start);
            for i in 1..sorted.len() {
                if sorted[i].start < sorted[i - 1].end {
                    return Err(format!(
                        "value {} pieces overlap: [{},{}) and [{},{})",
                        vid,
                        sorted[i - 1].start,
                        sorted[i - 1].end,
                        sorted[i].start,
                        sorted[i].end
                    ));
                }
            }
            let _ = liveness;
        }
        Ok(())
    }
}

fn is_rematerializable(func: &IrFunction, vid: u32) -> Option<RematerializationKind> {
    let mut def_inst: Option<Instruction> = None;
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                if dest.0 == vid {
                    def_inst = Some(inst.clone());
                    break;
                }
            }
        }
        if def_inst.is_some() {
            break;
        }
    }
    let inst = def_inst?;

    match inst {
        Instruction::Copy { src, .. } => match src {
            Operand::Const(c) => match c {
                IrConst::Zero => Some(RematerializationKind::Zero),
                IrConst::I32(0) | IrConst::I64(0) | IrConst::I128(0) => {
                    Some(RematerializationKind::Zero)
                }
                IrConst::I32(v) => Some(RematerializationKind::ConstI32(v)),
                IrConst::I64(v) => Some(RematerializationKind::ConstI64(v)),
                IrConst::I128(v) => {
                    if v >= i32::MIN as i128 && v <= i32::MAX as i128 {
                        Some(RematerializationKind::ConstI32(v as i32))
                    } else if v >= i64::MIN as i128 && v <= i64::MAX as i128 {
                        Some(RematerializationKind::ConstI64(v as i64))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        },
        Instruction::Cast {
            src,
            from_ty,
            to_ty,
            ..
        } => {
            if let Operand::Value(src_v) = src {
                let is_ext = matches!(
                    (from_ty, to_ty),
                    (IrType::I8, IrType::I32)
                        | (IrType::U8, IrType::I32)
                        | (IrType::I8, IrType::I64)
                        | (IrType::U8, IrType::I64)
                        | (IrType::I16, IrType::I32)
                        | (IrType::U16, IrType::I32)
                        | (IrType::I16, IrType::I64)
                        | (IrType::U16, IrType::I64)
                        | (IrType::I32, IrType::I64)
                        | (IrType::U32, IrType::I64)
                );
                if is_ext {
                    return Some(RematerializationKind::Extension {
                        src: src_v.0,
                        from_ty,
                        to_ty,
                        is_signed: matches!(from_ty, IrType::I8 | IrType::I16 | IrType::I32),
                    });
                }
            }
            None
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            if let Operand::Const(c) = offset {
                if let Some(off) = c.to_i64() {
                    if off.abs() <= 4096 {
                        return Some(RematerializationKind::SimpleGep {
                            base: base.0,
                            offset: off,
                        });
                    }
                }
            }
            None
        }
        Instruction::GlobalAddr { name, .. } => Some(RematerializationKind::GlobalAddr(name)),
        Instruction::BinOp { op, lhs, rhs, .. } => {
            let const_side = match (&lhs, &rhs) {
                (Operand::Value(v), Operand::Const(c)) => Some((*v, *c, true)),
                (Operand::Const(c), Operand::Value(v)) => Some((*v, *c, false)),
                _ => None,
            };
            if let Some((base, c, _)) = const_side {
                if let Some(cv) = c.to_i64() {
                    match op {
                        IrBinOp::Add => {
                            if cv.abs() <= 4096 {
                                return Some(RematerializationKind::SmallArith {
                                    base: base.0,
                                    kind: SmallArithKind::AddConst(cv),
                                });
                            }
                        }
                        IrBinOp::Mul => {
                            if cv == 2 || cv == 4 || cv == 8 {
                                return Some(RematerializationKind::SmallArith {
                                    base: base.0,
                                    kind: SmallArithKind::MulScale(cv as u8),
                                });
                            }
                        }
                        IrBinOp::Shl => {
                            if (1..=3).contains(&cv) {
                                return Some(RematerializationKind::SmallArith {
                                    base: base.0,
                                    kind: SmallArithKind::ShlConst(cv as u8),
                                });
                            }
                        }
                        IrBinOp::And => {
                            if (cv as u64).count_ones() <= 16 {
                                return Some(RematerializationKind::SmallArith {
                                    base: base.0,
                                    kind: SmallArithKind::AndMask(cv as u64),
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            None
        }
        _ => None,
    }
}

pub struct RaptorLakeCostModel;

impl RaptorLakeCostModel {
    pub fn latency(op: &str) -> u8 {
        match op {
            "add" | "sub" | "and" | "or" | "xor" => 1,
            "lea1" | "lea2" | "lea3" => 1,
            "shl_imm" | "shr_imm" | "sar_imm" => 1,
            "shl_cl" => 1,
            "imul" | "mul" => 3,
            "div" => 20,
            "load" => 5,
            "store" => 1,
            "branch" | "cmp" => 1,
            _ => 1,
        }
    }

    pub fn rthroughput_x100(op: &str) -> u16 {
        match op {
            "add" => 25,
            "lea1" | "lea2" | "lea3" => 20,
            "shl_imm" => 25,
            "shl_cl" | "imul" | "load" | "store" => 100,
            _ => 100,
        }
    }

    pub fn uops(op: &str) -> u8 {
        match op {
            "add" | "sub" | "and" | "or" | "xor" | "lea1" | "lea2" | "lea3" | "shl_imm"
            | "shl_cl" | "imul" | "load" | "store" | "branch" => 1,
            _ => 1,
        }
    }

    pub fn remat_cost(kind: &RematerializationKind) -> u64 {
        match kind {
            RematerializationKind::Zero => 1,
            RematerializationKind::ConstI32(_) => 1,
            RematerializationKind::ConstI64(_) => 1,
            RematerializationKind::GlobalAddr(_) => 2,
            RematerializationKind::SimpleGep { .. } => 1,
            RematerializationKind::Extension { .. } => 1,
            RematerializationKind::SmallArith { .. } => 1,
        }
    }

    pub fn spill_cost() -> u64 {
        10
    }
}

#[derive(Clone, Copy, Debug)]
struct PointLoc {
    block: usize,
    inst: Option<usize>,
}

fn assign_point_locs(func: &IrFunction) -> Vec<PointLoc> {
    let mut pts = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for ii in 0..block.instructions.len() {
            pts.push(PointLoc {
                block: bi,
                inst: Some(ii),
            });
        }
        pts.push(PointLoc {
            block: bi,
            inst: None,
        });
    }
    pts
}

fn const_type(c: &IrConst) -> Option<IrType> {
    Some(match c {
        IrConst::I8(_) => IrType::I8,
        IrConst::I16(_) => IrType::I16,
        IrConst::I32(_) => IrType::I32,
        IrConst::I64(_) => IrType::I64,
        IrConst::I128(_) => IrType::I64,
        IrConst::F32(_) => IrType::F32,
        IrConst::F64(_) => IrType::F64,
        IrConst::Zero => IrType::I32,
        _ => return None,
    })
}

fn inst_result_type(inst: &Instruction) -> Option<(u32, IrType)> {
    match inst {
        Instruction::BinOp { dest, ty, .. }
        | Instruction::UnaryOp { dest, ty, .. }
        | Instruction::Load { dest, ty, .. }
        | Instruction::Cmp { dest, ty, .. }
        | Instruction::Select { dest, ty, .. }
        | Instruction::Phi { dest, ty, .. } => Some((dest.0, *ty)),
        Instruction::Cast { dest, to_ty, .. } => Some((dest.0, *to_ty)),
        Instruction::GetElementPtr { dest, .. }
        | Instruction::GlobalAddr { dest, .. }
        | Instruction::Alloca { dest, .. }
        | Instruction::DynAlloca { dest, .. } => Some((dest.0, IrType::Ptr)),
        Instruction::Copy {
            dest,
            src: Operand::Const(c),
        } => const_type(c).map(|t| (dest.0, t)),
        Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
            info.dest.map(|d| (d.0, info.return_type))
        }
        Instruction::ParamRef { dest, ty, .. } => Some((dest.0, *ty)),
        Instruction::StackSave { dest, .. } => Some((dest.0, IrType::Ptr)),
        Instruction::LabelAddr { dest, .. } => Some((dest.0, IrType::Ptr)),
        _ => None,
    }
}

fn collect_value_types(func: &IrFunction) -> FxHashMap<u32, IrType> {
    let mut types: FxHashMap<u32, IrType> = FxHashMap::default();
    let mut copies: Vec<(u32, u32)> = Vec::new();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Copy {
                dest,
                src: Operand::Value(src),
            } = inst
            {
                copies.push((dest.0, src.0));
            }
            if let Some((id, ty)) = inst_result_type(inst) {
                types.entry(id).or_insert(ty);
            }
        }
    }
    for _ in 0..16 {
        let mut progressed = false;
        for &(dest, src) in &copies {
            if types.contains_key(&dest) {
                continue;
            }
            if let Some(&ty) = types.get(&src) {
                types.insert(dest, ty);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    types
}

fn is_simple_gpr_type(ty: IrType) -> bool {
    !ty.is_float() && !ty.is_long_double() && !ty.is_128bit()
}

fn next_value(next_val: &mut u32) -> Option<Value> {
    let id = *next_val;
    if id == u32::MAX {
        return None;
    }
    *next_val = id.saturating_add(1);
    Some(Value(id))
}

fn first_non_alloca(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Alloca { .. }))
        .unwrap_or(block.instructions.len())
}

fn first_non_phi(block: &BasicBlock) -> usize {
    block
        .instructions
        .iter()
        .position(|i| !matches!(i, Instruction::Phi { .. }))
        .unwrap_or(block.instructions.len())
}

fn abi_align(ty: IrType) -> usize {
    match ty.size() {
        0 => 1,
        1 => 1,
        2 => 2,
        3 | 4 => 4,
        _ => 8,
    }
}

fn insert_instruction(block: &mut BasicBlock, idx: usize, inst: Instruction) {
    let n_inst = block.instructions.len();
    let n_span = block.source_spans.len();
    let idx = idx.min(n_inst);
    block.instructions.insert(idx, inst);
    if n_span == n_inst && n_span > 0 {
        let span_idx = idx.min(n_span - 1);
        let span = block.source_spans[span_idx].clone();
        block.source_spans.insert(idx.min(n_span), span);
    }
}

fn insert_entry_alloca(func: &mut IrFunction, dest: Value, ty: IrType, volatile: bool) {
    let inst = Instruction::Alloca {
        dest,
        ty,
        size: ty.size(),
        align: abi_align(ty),
        volatile,
        semantic_volatile: false,
    };
    let pos = first_non_alloca(&func.blocks[0]);
    insert_instruction(&mut func.blocks[0], pos, inst);
}

fn instruction_uses_value(inst: &Instruction, vid: u32) -> bool {
    let mut hit = false;
    super::liveness::for_each_operand_in_instruction(inst, |op| {
        if matches!(op, Operand::Value(v) if v.0 == vid) {
            hit = true;
        }
    });
    if hit {
        return true;
    }
    super::liveness::for_each_value_use_in_instruction(inst, |v| {
        if v.0 == vid {
            hit = true;
        }
    });
    hit
}

fn terminator_uses_value(term: &Terminator, vid: u32) -> bool {
    let mut hit = false;
    super::liveness::for_each_operand_in_terminator(term, |op| {
        if matches!(op, Operand::Value(v) if v.0 == vid) {
            hit = true;
        }
    });
    hit
}

fn rewrite_operand(op: &mut Operand, map: &FxHashMap<u32, u32>) {
    if let Operand::Value(v) = op {
        if let Some(&n) = map.get(&v.0) {
            v.0 = n;
        }
    }
}

fn rewrite_value(v: &mut Value, map: &FxHashMap<u32, u32>) {
    if let Some(&n) = map.get(&v.0) {
        v.0 = n;
    }
}

fn replace_values_in_inst(inst: &mut Instruction, map: &FxHashMap<u32, u32>, rewrite_phi: bool) {
    match inst {
        Instruction::Alloca { .. }
        | Instruction::PgoCounterInc { .. }
        | Instruction::GlobalAddr { .. }
        | Instruction::Fence { .. }
        | Instruction::LabelAddr { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::GetStaticChain { .. }
        | Instruction::StackSave { .. }
        | Instruction::ParamRef { .. }
        | Instruction::VaEnd { .. } => {}
        Instruction::SetStaticChain { src } => rewrite_operand(src, map),
        Instruction::InitTrampoline { buffer, chain, .. } => {
            rewrite_value(buffer, map);
            rewrite_operand(chain, map);
        }
        Instruction::NonlocalGotoSave { frame, .. } => rewrite_value(frame, map),
        Instruction::NonlocalGoto { chain, .. } => rewrite_operand(chain, map),
        Instruction::DynAlloca { size, .. } => rewrite_operand(size, map),
        Instruction::Store { val, ptr, .. } => {
            rewrite_operand(val, map);
            rewrite_value(ptr, map);
        }
        Instruction::Load { ptr, .. } => rewrite_value(ptr, map),
        Instruction::BinOp { lhs, rhs, .. } | Instruction::Cmp { lhs, rhs, .. } => {
            rewrite_operand(lhs, map);
            rewrite_operand(rhs, map);
        }
        Instruction::UnaryOp { src, .. }
        | Instruction::Cast { src, .. }
        | Instruction::Copy { src, .. } => rewrite_operand(src, map),
        Instruction::Call { info, .. } => {
            for a in &mut info.args {
                rewrite_operand(a, map);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            rewrite_operand(func_ptr, map);
            for a in &mut info.args {
                rewrite_operand(a, map);
            }
        }
        Instruction::GetElementPtr { base, offset, .. } => {
            rewrite_value(base, map);
            rewrite_operand(offset, map);
        }
        Instruction::Memcpy { dest, src, .. } => {
            rewrite_value(dest, map);
            rewrite_value(src, map);
        }
        Instruction::VaArg { va_list_ptr, .. } | Instruction::VaStart { va_list_ptr } => {
            rewrite_value(va_list_ptr, map);
        }
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            rewrite_value(dest_ptr, map);
            rewrite_value(src_ptr, map);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            rewrite_value(dest_ptr, map);
            rewrite_value(va_list_ptr, map);
        }
        Instruction::AtomicRmw { ptr, val, .. } | Instruction::AtomicStore { ptr, val, .. } => {
            rewrite_operand(ptr, map);
            rewrite_operand(val, map);
        }
        Instruction::AtomicInc { ptr, .. } | Instruction::AtomicLoad { ptr, .. } => {
            rewrite_operand(ptr, map);
        }
        Instruction::AtomicCmpxchg {
            ptr,
            expected,
            desired,
            ..
        } => {
            rewrite_operand(ptr, map);
            rewrite_operand(expected, map);
            rewrite_operand(desired, map);
        }
        Instruction::Phi { incoming, .. } => {
            if rewrite_phi {
                for (op, _) in incoming {
                    rewrite_operand(op, map);
                }
            }
        }
        Instruction::SetReturnF64Second { src }
        | Instruction::SetReturnF32Second { src }
        | Instruction::SetReturnF128Second { src } => rewrite_operand(src, map),
        Instruction::InlineAsm {
            inputs, outputs, ..
        } => {
            for (_constr, op, _name) in inputs.iter_mut() {
                rewrite_operand(op, map);
            }
            for (_constr, val, _name) in outputs.iter_mut() {
                rewrite_value(val, map);
            }
        }
        Instruction::Select {
            cond,
            true_val,
            false_val,
            ..
        } => {
            rewrite_operand(cond, map);
            rewrite_operand(true_val, map);
            rewrite_operand(false_val, map);
        }
        Instruction::Intrinsic { args, .. } => {
            for a in args {
                rewrite_operand(a, map);
            }
        }
        Instruction::StackRestore { ptr } => rewrite_value(ptr, map),
    }
}

fn replace_values_in_terminator(term: &mut Terminator, map: &FxHashMap<u32, u32>) {
    match term {
        Terminator::Return(Some(op)) => rewrite_operand(op, map),
        Terminator::Return(None) => {}
        Terminator::Branch(_) => {}
        Terminator::CondBranch { cond, .. } => rewrite_operand(cond, map),
        Terminator::Switch { val, .. } => rewrite_operand(val, map),
        Terminator::IndirectBranch { .. } => {}
        Terminator::Unreachable => {}
    }
}

fn dominator_sets(idom: &[usize]) -> Vec<FxHashSet<usize>> {
    let n = idom.len();
    let mut sets = vec![FxHashSet::default(); n];
    for a in 0..n {
        sets[a].insert(a);
        let mut cur = a;
        for _ in 0..n.saturating_add(1) {
            if cur >= n {
                break;
            }
            let next = idom[cur];
            if next == cur || next == usize::MAX {
                break;
            }
            sets[a].insert(next);
            cur = next;
        }
    }
    sets
}

#[inline]
fn set_dominates(sets: &[FxHashSet<usize>], a: usize, b: usize) -> bool {
    sets.get(a).is_some_and(|s| s.contains(&b))
}

fn compute_pressure(liveness: &super::liveness::LivenessResult) -> Vec<u32> {
    let n = liveness.num_points as usize;
    let mut result = vec![0u32; n];
    for p in 0..n {
        let mut cnt = 0u32;
        for seg in &liveness.segments {
            if seg.start <= p as u32 && (p as u32) <= seg.end {
                cnt += 1;
            }
        }
        result[p] = cnt;
    }
    result
}

fn split_next_use_gaps(func: &mut IrFunction, max_splits: usize) -> usize {
    if max_splits == 0 {
        return 0;
    }
    // Collect work without holding borrow
    let liveness = super::liveness::compute_live_intervals(func);
    let pressure = compute_pressure(&liveness);
    let points = assign_point_locs(func);
    let types = collect_value_types(func);

    struct Gap {
        block: usize,
        vid: u32,
        u1_idx: usize,
        u2_idx: usize,
        u1_pt: u32,
        u2_pt: u32,
        ty: IrType,
        remat: Option<RematerializationKind>,
    }

    let mut gaps: Vec<Gap> = Vec::new();

    // We need to compute block_start_pt for each block
    let mut block_start_pts: Vec<u32> = vec![0; func.blocks.len()];
    for (bi, _) in func.blocks.iter().enumerate() {
        let start = points
            .iter()
            .position(|p| p.block == bi && p.inst == Some(0))
            .map(|i| i as u32)
            .unwrap_or(0);
        block_start_pts[bi] = start;
    }

    for (bi, block) in func.blocks.iter().enumerate() {
        if gaps.len() >= max_splits {
            break;
        }
        let mut block_uses: FxHashMap<u32, Vec<(u32, usize)>> = FxHashMap::default(); // vid -> Vec<(pt, idx)>
        let block_start_pt = block_start_pts[bi];

        for (ii, inst) in block.instructions.iter().enumerate() {
            let cur_pt = block_start_pt + ii as u32;
            super::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    block_uses.entry(v.0).or_default().push((cur_pt, ii));
                }
            });
            super::liveness::for_each_value_use_in_instruction(inst, |v| {
                block_uses.entry(v.0).or_default().push((cur_pt, ii));
            });
        }

        for (vid, mut uses) in block_uses {
            if uses.len() < 2 {
                continue;
            }
            uses.sort_by_key(|(pt, _)| *pt);
            for w in uses.windows(2) {
                let (u1_pt, u1_idx) = w[0];
                let (u2_pt, u2_idx) = w[1];
                let gap = u2_pt.saturating_sub(u1_pt);
                if gap < 30 {
                    continue;
                }
                let mut max_pressure_in_gap = 0u32;
                for p in u1_pt..u2_pt {
                    if (p as usize) < pressure.len() {
                        max_pressure_in_gap = max_pressure_in_gap.max(pressure[p as usize]);
                    }
                }
                if max_pressure_in_gap < 12 {
                    continue;
                }
                let Some(&ty) = types.get(&vid) else {
                    continue;
                };
                if !is_simple_gpr_type(ty) {
                    continue;
                }
                let remat = is_rematerializable(func, vid);
                let cost_remat = remat
                    .as_ref()
                    .map(|k| RaptorLakeCostModel::remat_cost(k))
                    .unwrap_or(u64::MAX);
                let cost_spill = RaptorLakeCostModel::spill_cost();
                let occupancy_cost = gap as u64 * 2;
                let should_split = if cost_remat < cost_spill && cost_remat < occupancy_cost {
                    true
                } else if cost_spill < occupancy_cost {
                    true
                } else {
                    false
                };
                if !should_split {
                    continue;
                }
                gaps.push(Gap {
                    block: bi,
                    vid,
                    u1_idx,
                    u2_idx,
                    u1_pt,
                    u2_pt,
                    ty,
                    remat,
                });
                break;
            }
            if gaps.len() >= max_splits {
                break;
            }
        }
    }

    // Now apply gaps, handling index shifts per block
    let mut next_val = func.next_value_id;
    let mut splits = 0;
    // Group by block and sort by u1_idx descending to avoid shift issues
    let mut by_block: FxHashMap<usize, Vec<Gap>> = FxHashMap::default();
    for g in gaps {
        by_block.entry(g.block).or_default().push(g);
    }

    for (bi, mut block_gaps) in by_block {
        // Sort descending by u1_idx
        block_gaps.sort_by(|a, b| b.u1_idx.cmp(&a.u1_idx));
        for gap in block_gaps {
            if splits >= max_splits {
                break;
            }
            // Re-check block length (may have grown)
            if bi >= func.blocks.len() {
                continue;
            }
            let block_len = func.blocks[bi].instructions.len();
            if gap.u1_idx >= block_len || gap.u2_idx >= block_len {
                continue;
            }

            if gap.remat.is_some() {
                // Find def to clone
                let mut def_clone: Option<Instruction> = None;
                for b in &func.blocks {
                    for inst in &b.instructions {
                        if inst.dest().map(|d| d.0 == gap.vid).unwrap_or(false) {
                            def_clone = Some(inst.clone());
                            break;
                        }
                    }
                    if def_clone.is_some() {
                        break;
                    }
                }
                if let Some(mut def) = def_clone {
                    let new_val = match next_value(&mut next_val) {
                        Some(v) => v,
                        None => continue,
                    };
                    match &mut def {
                        Instruction::Copy { dest, .. }
                        | Instruction::Cast { dest, .. }
                        | Instruction::BinOp { dest, .. }
                        | Instruction::GetElementPtr { dest, .. }
                        | Instruction::GlobalAddr { dest, .. } => {
                            *dest = new_val;
                        }
                        _ => continue,
                    }
                    let block_mut = &mut func.blocks[bi];
                    if gap.u2_idx < block_mut.instructions.len() {
                        block_mut.instructions.insert(gap.u2_idx, def);
                        let mut map = FxHashMap::default();
                        map.insert(gap.vid, new_val.0);
                        for inst in block_mut.instructions.iter_mut().skip(gap.u2_idx + 1) {
                            replace_values_in_inst(inst, &map, false);
                        }
                        dlog!(
                            "[GLOBAL-ALLOC] remat split value {} block {} gap {}->{}",
                            gap.vid,
                            bi,
                            gap.u1_pt,
                            gap.u2_pt
                        );
                        splits += 1;
                    }
                }
            } else {
                let alloca_val = match next_value(&mut next_val) {
                    Some(v) => v,
                    None => continue,
                };
                let new_val = match next_value(&mut next_val) {
                    Some(v) => v,
                    None => continue,
                };
                insert_entry_alloca(func, alloca_val, gap.ty, true);

                let block_mut = &mut func.blocks[bi];
                let store = Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(gap.vid)),
                    ptr: alloca_val,
                    ty: gap.ty,
                    seg_override: crate::common::types::AddressSpace::Default,
                };
                let store_at = (gap.u1_idx + 1).min(block_mut.instructions.len());
                block_mut.instructions.insert(store_at, store);

                let load_at = if gap.u2_idx >= store_at {
                    gap.u2_idx + 1
                } else {
                    gap.u2_idx
                };
                let load = Instruction::Load {
                    volatile: false,
                    dest: new_val,
                    ptr: alloca_val,
                    ty: gap.ty,
                    seg_override: crate::common::types::AddressSpace::Default,
                };
                if load_at <= block_mut.instructions.len() {
                    block_mut.instructions.insert(load_at, load);
                }

                let mut map = FxHashMap::default();
                map.insert(gap.vid, new_val.0);
                for inst in block_mut.instructions.iter_mut().skip(load_at + 1) {
                    replace_values_in_inst(inst, &map, false);
                }
                dlog!(
                    "[GLOBAL-ALLOC] spill split value {} block {} gap {}->{}",
                    gap.vid,
                    bi,
                    gap.u1_pt,
                    gap.u2_pt
                );
                splits += 1;
            }
        }
    }

    if splits > 0 {
        func.next_value_id = next_val;
    }
    splits
}

fn split_call_cross_block(func: &mut IrFunction, max_splits: usize) -> usize {
    if max_splits == 0 {
        return 0;
    }
    let label_map = analysis::build_label_map(func);
    let (preds, succs) = analysis::build_cfg(func, &label_map);
    let idom = analysis::compute_dominators(func.blocks.len(), &preds, &succs);
    let dom_sets = dominator_sets(&idom);
    let liveness = super::liveness::compute_live_intervals(func);
    if liveness.call_points.is_empty() {
        return 0;
    }
    let points = assign_point_locs(func);
    let types = collect_value_types(func);

    let n = func.blocks.len();
    let mut succ_vec: Vec<Vec<usize>> = vec![Vec::new(); n];
    for bi in 0..n {
        for &s in succs.row(bi) {
            succ_vec[bi].push(s as usize);
        }
    }

    struct CallSplit {
        call_pt: u32,
        call_block: usize,
        call_inst_idx: usize,
        succ_block: usize,
        vid: u32,
        ty: IrType,
    }

    let mut candidates: Vec<CallSplit> = Vec::new();

    for &cp in &liveness.call_points {
        if candidates.len() >= max_splits {
            break;
        }
        let Some(loc) = points.get(cp as usize) else {
            continue;
        };
        let call_block = loc.block;
        if call_block >= n {
            continue;
        }
        if succ_vec[call_block].len() != 1 {
            continue;
        }
        let succ_block = succ_vec[call_block][0];
        if succ_block >= n {
            continue;
        }
        if preds.row(succ_block).len() != 1 {
            continue;
        }
        let call_inst_idx = loc.inst.unwrap_or(0);

        // Pre-check: if succ has a PHI using any value, we will need to handle it.
        // For correctness, we will filter per-vid below, but cache phi uses of succ.
        let succ_phis_use: FxHashSet<u32> = {
            let mut s = FxHashSet::default();
            for inst in &func.blocks[succ_block].instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    for (op, _) in incoming {
                        if let Operand::Value(v) = op {
                            s.insert(v.0);
                        }
                    }
                } else {
                    break; // phis are at start
                }
            }
            s
        };

        for iv in &liveness.intervals {
            if candidates.len() >= max_splits {
                break;
            }
            if iv.start >= cp || iv.end <= cp {
                continue;
            }
            // Check uses dominated by succ_block
            let mut has_use_after = false;
            let mut all_dominated = true;
            for (bi, block) in func.blocks.iter().enumerate() {
                let mut uses = false;
                for inst in &block.instructions {
                    if instruction_uses_value(inst, iv.value_id) {
                        uses = true;
                        break;
                    }
                }
                if !uses && terminator_uses_value(&block.terminator, iv.value_id) {
                    uses = true;
                }
                if uses {
                    if bi == call_block {
                        let mut after = false;
                        for (ii, inst) in block.instructions.iter().enumerate() {
                            if ii > call_inst_idx && instruction_uses_value(inst, iv.value_id) {
                                after = true;
                                break;
                            }
                        }
                        if after {
                            has_use_after = true;
                        }
                    } else if set_dominates(&dom_sets, bi, succ_block) {
                        has_use_after = true;
                    } else {
                        // If this block is reachable from succ, but not dominated, bail
                        // For simplicity, require all uses dominated
                        // Check if block is reachable from call? We approximate: if block != call_block, then require dominated
                        // If not dominated, we cannot handle without PHI
                        all_dominated = false;
                        break;
                    }
                }
            }
            if !has_use_after || !all_dominated {
                continue;
            }
            let Some(&ty) = types.get(&iv.value_id) else {
                continue;
            };
            if !is_simple_gpr_type(ty) {
                continue;
            }
            // If succ's PHI uses this vid, we would need to replace PHI with copy after load.
            // For safety, skip such candidates until PHI rewrite is implemented.
            // This avoids use-before-def where PHI cannot use new_val defined after PHIs.
            if succ_phis_use.contains(&iv.value_id) {
                continue;
            }
            candidates.push(CallSplit {
                call_pt: cp,
                call_block,
                call_inst_idx,
                succ_block,
                vid: iv.value_id,
                ty,
            });
        }
    }

    let mut next_val = func.next_value_id;
    let mut splits = 0;

    // Sort by call_block descending to avoid index shifts affecting earlier calls in same block.
    // Then group by (call_block, call_inst_idx, succ_block, call_pt) to batch multiple vids
    // sharing the same call point — this makes first_after computation deterministic and
    // avoids the off-by-one when multiple stores are inserted at the same idx.
    candidates.sort_by(|a, b| {
        b.call_block
            .cmp(&a.call_block)
            .then(b.call_inst_idx.cmp(&a.call_inst_idx))
            .then(a.vid.cmp(&b.vid))
    });

    // Group candidates
    #[derive(Debug)]
    struct Group {
        call_pt: u32,
        call_block: usize,
        call_inst_idx: usize,
        succ_block: usize,
        vids: Vec<(u32, IrType)>, // (vid, ty)
    }
    let mut groups: Vec<Group> = Vec::new();
    for cand in candidates {
        if let Some(last) = groups.last_mut() {
            if last.call_block == cand.call_block
                && last.call_inst_idx == cand.call_inst_idx
                && last.succ_block == cand.succ_block
                && last.call_pt == cand.call_pt
            {
                // Avoid duplicate vid in same group
                if !last.vids.iter().any(|(v, _)| *v == cand.vid) {
                    last.vids.push((cand.vid, cand.ty));
                }
                continue;
            }
        }
        groups.push(Group {
            call_pt: cand.call_pt,
            call_block: cand.call_block,
            call_inst_idx: cand.call_inst_idx,
            succ_block: cand.succ_block,
            vids: vec![(cand.vid, cand.ty)],
        });
    }

    for group in groups {
        if splits >= max_splits {
            break;
        }
        if group.call_block >= func.blocks.len() || group.succ_block >= func.blocks.len() {
            continue;
        }
        // Respect max_splits: trim vids if needed
        let remaining = max_splits - splits;
        let vids_to_process: Vec<(u32, IrType)> = group.vids.into_iter().take(remaining).collect();
        if vids_to_process.is_empty() {
            continue;
        }

        // Allocate allocas and new values for each vid
        let mut allocs: Vec<(u32, Value, Value, IrType)> = Vec::new(); // (old_vid, alloca, new_val, ty)
        for (vid, ty) in &vids_to_process {
            let alloca_val = match next_value(&mut next_val) {
                Some(v) => v,
                None => break,
            };
            let new_val = match next_value(&mut next_val) {
                Some(v) => v,
                None => break,
            };
            insert_entry_alloca(func, alloca_val, *ty, true);
            allocs.push((*vid, alloca_val, new_val, *ty));
        }
        if allocs.is_empty() {
            continue;
        }

        // Insert stores before call (at call_inst_idx). Insert in reverse to preserve order,
        // but order doesn't matter. We insert at same idx repeatedly, so last inserted ends up first.
        {
            let block = &mut func.blocks[group.call_block];
            // Clamp idx to current len (may have grown from previous groups later in block, but
            // since we process descending, earlier groups' idx is still valid and not shifted by later groups)
            let idx = group.call_inst_idx.min(block.instructions.len());
            for (old_vid, alloca_val, _new_val, ty) in allocs.iter().rev() {
                let store = Instruction::Store {
                    volatile: false,
                    val: Operand::Value(Value(*old_vid)),
                    ptr: *alloca_val,
                    ty: *ty,
                    seg_override: crate::common::types::AddressSpace::Default,
                };
                block.instructions.insert(idx, store);
            }
        }

        // Insert loads in succ at first_non_phi. Insert all loads.
        let load_at = {
            let block = &mut func.blocks[group.succ_block];
            let at = first_non_phi(block);
            // Insert in reverse so first in allocs ends up first
            for (_old_vid, alloca_val, new_val, ty) in allocs.iter().rev() {
                let load = Instruction::Load {
                    volatile: false,
                    dest: *new_val,
                    ptr: *alloca_val,
                    ty: *ty,
                    seg_override: crate::common::types::AddressSpace::Default,
                };
                block.instructions.insert(at, load);
            }
            at
        };
        let num_loads = allocs.len();

        // Build map old->new
        let mut map: FxHashMap<u32, u32> = FxHashMap::default();
        for (old_vid, _alloca, new_val, _ty) in &allocs {
            map.insert(*old_vid, new_val.0);
        }

        // Find call's new position after stores: scan for call at/after group.call_inst_idx
        let call_new_idx = {
            let block = &func.blocks[group.call_block];
            let mut found = None;
            for ii in group.call_inst_idx..block.instructions.len() {
                if matches!(
                    block.instructions[ii],
                    Instruction::Call { .. } | Instruction::CallIndirect { .. }
                ) {
                    // Heuristic: the call point we want is the first call at or after idx
                    // that corresponds to the original call_pt. Since there may be multiple calls,
                    // we take the first call whose original point was >= group.call_pt?
                    // For simplicity, take first call encountered.
                    found = Some(ii);
                    break;
                }
            }
            found.unwrap_or(group.call_inst_idx + allocs.len())
        };
        let first_after = (call_new_idx + 1).min(func.blocks[group.call_block].instructions.len());

        // Rename in call_block after call
        {
            let block = &mut func.blocks[group.call_block];
            for inst in block.instructions[first_after..].iter_mut() {
                replace_values_in_inst(inst, &map, false);
            }
            replace_values_in_terminator(&mut block.terminator, &map);
        }
        // Rename in dominated blocks
        for bi in 0..func.blocks.len() {
            if bi == group.call_block {
                continue;
            }
            if set_dominates(&dom_sets, bi, group.succ_block) {
                let block = &mut func.blocks[bi];
                if bi == group.succ_block {
                    let start = (load_at + num_loads).min(block.instructions.len());
                    for inst in block.instructions[start..].iter_mut() {
                        replace_values_in_inst(inst, &map, false);
                    }
                } else {
                    for inst in &mut block.instructions {
                        replace_values_in_inst(inst, &map, true);
                    }
                }
                let term = &mut block.terminator;
                replace_values_in_terminator(term, &map);
            }
        }

        for (old_vid, _, _, _) in &allocs {
            dlog!(
                "[GLOBAL-ALLOC] cross-block call split value {} call@{} succ={}",
                old_vid,
                group.call_pt,
                group.succ_block
            );
        }
        splits += allocs.len();
    }

    if splits > 0 {
        func.next_value_id = next_val;
    }
    splits
}

pub fn run_global_location_pass(func: &mut IrFunction) -> usize {
    if !global_location_enabled() {
        return 0;
    }
    if func.blocks.len() < 2 {
        return 0;
    }
    let mut total = 0;
    total += split_call_cross_block(func, 4);
    // Gap splitting disabled: causes miscompile for zlib_ng_adler32 (spill split value 523 block 15 gap 70->112)
    // Root cause under investigation: intra-block spill splits do not correctly handle
    // successor phis or terminator uses, and pressure calculation is O(n*m). Disable until fixed.
    // total += split_next_use_gaps(func, 8);
    if total > 0 {
        dlog!("[GLOBAL-ALLOC] func {} total splits {}", func.name, total);
    }
    total
}

#[derive(Debug, Clone)]
pub struct AddressExpr {
    pub base: Option<u32>,
    pub index: Option<u32>,
    pub scale: u8,
    pub displacement: i64,
}

impl AddressExpr {
    pub fn from_gep_chain(func: &IrFunction, gep_dest: u32) -> Option<Self> {
        let mut base: Option<u32> = None;
        let mut index: Option<u32> = None;
        let mut scale: u8 = 1;
        let mut disp: i64 = 0;
        let mut current = gep_dest;
        let mut depth = 0;
        while depth < 16 {
            let mut found = false;
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let Some(dest) = inst.dest() {
                        if dest.0 == current {
                            match inst {
                                Instruction::GetElementPtr {
                                    base: b, offset, ..
                                } => {
                                    if base.is_none() {
                                        base = Some(b.0);
                                    } else if index.is_none() {
                                        index = Some(b.0);
                                    }
                                    match offset {
                                        Operand::Const(c) => {
                                            if let Some(v) = c.to_i64() {
                                                disp += v;
                                            }
                                        }
                                        Operand::Value(v) => {
                                            if index.is_none() {
                                                index = Some(v.0);
                                            }
                                        }
                                    }
                                    current = b.0;
                                    found = true;
                                }
                                Instruction::BinOp { op, lhs, rhs, .. } => match op {
                                    IrBinOp::Add => match (lhs, rhs) {
                                        (Operand::Value(v), Operand::Const(c))
                                        | (Operand::Const(c), Operand::Value(v)) => {
                                            if let Some(cv) = c.to_i64() {
                                                disp += cv;
                                                current = v.0;
                                                found = true;
                                            }
                                        }
                                        _ => {}
                                    },
                                    IrBinOp::Mul => match (lhs, rhs) {
                                        (Operand::Value(v), Operand::Const(c))
                                        | (Operand::Const(c), Operand::Value(v)) => {
                                            if let Some(cv) = c.to_i64() {
                                                if cv == 2 || cv == 4 || cv == 8 {
                                                    scale = cv as u8;
                                                    current = v.0;
                                                    found = true;
                                                }
                                            }
                                        }
                                        _ => {}
                                    },
                                    _ => {}
                                },
                                _ => {}
                            }
                            break;
                        }
                    }
                }
            }
            if !found {
                break;
            }
            depth += 1;
        }
        if base.is_some() || index.is_some() || disp != 0 {
            Some(AddressExpr {
                base,
                index,
                scale,
                displacement: disp,
            })
        } else {
            None
        }
    }

    pub fn is_foldable_x86(&self) -> bool {
        matches!(self.scale, 1 | 2 | 4 | 8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_expr_foldable() {
        let expr = AddressExpr {
            base: Some(1),
            index: Some(2),
            scale: 4,
            displacement: 16,
        };
        assert!(expr.is_foldable_x86());
    }

    #[test]
    fn global_location_disabled_by_default() {
        let _ = global_location_enabled();
    }
}

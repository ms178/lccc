//! Shared use-def information for optimization passes.
//!
//! Built once per function and consumed read-only by passes that need
//! use-counts, definition locations, or full use-chains. Sharing one analysis
//! eliminates the redundant whole-function scans that each pass would
//! otherwise perform.
//!
//! # Layout
//!
//! Use-chains use a **Compressed Sparse Row** encoding:
//!
//! ```text
//!   use_offsets: [0, 2, 2, 5, ...]        (len = size + 1)
//!   use_sites:   [u0 u1 | | u2 u3 u4 ...]
//!                  ^^^^^ uses of value 0
//!                          ^^^^^^^^ uses of value 2
//! ```
//!
//! `uses_of(v)` is a single slice index — no hashing, no per-value `Vec`, one
//! contiguous allocation for the whole function. Building it costs two linear
//! scans (count, then scatter) and is cache-friendly in both.
//!
//! # Validity
//!
//! `UseDefInfo` is **not** incrementally maintained. Block indices and
//! instruction indices are positional, so *any* IR mutation that inserts,
//! removes, or reorders instructions or blocks invalidates it. Passes that
//! mutate must rebuild (or finish consuming the info before mutating).
//! [`UseDefInfo::is_stale_for`] gives a cheap debug-time sanity check.
//!
//! # Conventions
//!
//! * Phi **self-references** (`%v = phi [.., (%v, L)]`) are excluded from both
//!   `use_count` and the use-chains. A self-referencing phi that has no other
//!   consumer is dead, and DCE relies on this exclusion to remove loop-carried
//!   phi cycles. Every other consumer of this analysis wants the same view.
//! * Terminator uses are included, encoded as `inst_idx == UseLoc::TERMINATOR`.
//! * A value used *n* times by one instruction appears *n* times in its chain,
//!   contiguously. Consumers that only care about *which* instructions read a
//!   value can use [`UseDefInfo::use_insts_of`], which collapses those runs.

use crate::ir::reexports::{Instruction, IrFunction, Operand, Terminator};

/// Compact use-site: identifies where a value is read.
///
/// `inst_idx == UseLoc::TERMINATOR` means the value is used by the block's
/// terminator rather than by an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UseLoc {
    pub block_idx: u32,
    pub inst_idx: u32,
}

impl UseLoc {
    /// Sentinel `inst_idx` marking a terminator use.
    pub const TERMINATOR: u32 = u32::MAX;

    #[inline]
    pub fn instruction(block: u32, inst: u32) -> Self {
        debug_assert_ne!(
            inst,
            Self::TERMINATOR,
            "instruction index collides with the terminator sentinel"
        );
        UseLoc {
            block_idx: block,
            inst_idx: inst,
        }
    }

    #[inline]
    pub fn terminator(block: u32) -> Self {
        UseLoc {
            block_idx: block,
            inst_idx: Self::TERMINATOR,
        }
    }

    #[inline]
    pub fn is_terminator(self) -> bool {
        self.inst_idx == Self::TERMINATOR
    }
}

/// Compact definition location packed into a single `u64`.
///
/// | encoding                          | meaning                        |
/// |-----------------------------------|--------------------------------|
/// | `u64::MAX`                        | no definition found            |
/// | `(1 << 63) \| param_idx`          | function parameter (`ParamRef`)|
/// | `(block_idx << 32) \| inst_idx`   | instruction at that position   |
///
/// The parameter bit is disjoint from the instruction encoding because a block
/// index cannot reach 2^31 (`u32::MAX` block indices would need more memory
/// than exists), so the two spaces never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefLoc(u64);

impl DefLoc {
    const NONE_SENTINEL: u64 = u64::MAX;
    const PARAM_BIT: u64 = 1 << 63;

    /// No definition found: an undefined value, or one whose defining
    /// instruction a previous pass removed.
    #[inline]
    pub fn none() -> Self {
        DefLoc(Self::NONE_SENTINEL)
    }

    /// Defined by the instruction at `(block_idx, inst_idx)`.
    #[inline]
    pub fn instruction(block: u32, inst: u32) -> Self {
        debug_assert!(
            block < (1 << 31),
            "block index {block} overflows the DefLoc instruction encoding"
        );
        DefLoc(((block as u64) << 32) | (inst as u64))
    }

    /// Defined as a function parameter (an `Instruction::ParamRef`).
    #[inline]
    pub fn parameter(idx: u32) -> Self {
        DefLoc(Self::PARAM_BIT | (idx as u64))
    }

    #[inline]
    pub fn is_none(self) -> bool {
        self.0 == Self::NONE_SENTINEL
    }

    /// True when this value is a function parameter.
    #[inline]
    pub fn is_parameter(self) -> bool {
        self.0 != Self::NONE_SENTINEL && (self.0 & Self::PARAM_BIT) != 0
    }

    /// `(block_idx, inst_idx)` if this is an instruction definition.
    #[inline]
    pub fn as_instruction(self) -> Option<(u32, u32)> {
        if self.0 == Self::NONE_SENTINEL || (self.0 & Self::PARAM_BIT) != 0 {
            None
        } else {
            Some(((self.0 >> 32) as u32, self.0 as u32))
        }
    }

    /// The block containing the defining instruction, if any.
    #[inline]
    pub fn block(self) -> Option<u32> {
        self.as_instruction().map(|(b, _)| b)
    }
}

/// Per-function use-def information.
///
/// All arrays are indexed by Value ID (`Value.0 as usize`) and have length
/// [`UseDefInfo::len`], which covers every value ID the function can mention.
pub struct UseDefInfo {
    /// `use_count[v]` = number of operand occurrences of `Value(v)`.
    /// Phi self-references are excluded; terminator uses are included.
    pub use_count: Vec<u32>,

    /// `def_loc[v]` = where `Value(v)` is defined.
    pub def_loc: Vec<DefLoc>,

    /// CSR row offsets, length `len() + 1`.
    pub use_offsets: Vec<u32>,

    /// CSR payload: use-sites grouped by value ID, in program order.
    pub use_sites: Vec<UseLoc>,

    /// Number of blocks the function had when this info was built. Used by
    /// [`UseDefInfo::is_stale_for`].
    block_count: u32,
}

impl UseDefInfo {
    /// Build use-def information for `func`.
    ///
    /// Two linear scans: count uses and record definitions, then prefix-sum the
    /// counts into row offsets and scatter the use-sites. Cost is
    /// `O(instructions x avg_operands)` with two allocations sized exactly.
    pub fn build(func: &IrFunction) -> Self {
        // `max_value_id()` trusts the cached `next_value_id`. If a pass ever
        // leaves a dest above that watermark, sizing from the cache alone would
        // silently drop that value's uses from every chain — which for SCCP
        // means a stale optimistic lattice value and a miscompile. Take the max
        // of the cache and what the IR actually contains.
        let mut max_id = func.max_value_id();
        for block in func.blocks.iter() {
            for inst in &block.instructions {
                if let Some(dest) = inst.dest() {
                    if dest.0 > max_id {
                        max_id = dest.0;
                    }
                }
                inst.for_each_used_value(|id| {
                    if id > max_id {
                        max_id = id;
                    }
                });
            }
            block.terminator.for_each_used_value(|id| {
                if id > max_id {
                    max_id = id;
                }
            });
        }
        let size = max_id as usize + 1;

        let mut use_count: Vec<u32> = vec![0; size];
        let mut def_loc: Vec<DefLoc> = vec![DefLoc::none(); size];

        // ---- pass 1: count uses, record definitions ------------------------
        for (bi, block) in func.blocks.iter().enumerate() {
            let bi32 = bi as u32;
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Some(dest) = inst.dest() {
                    let id = dest.0 as usize;
                    if let Instruction::ParamRef { param_idx, .. } = inst {
                        def_loc[id] = DefLoc::parameter(*param_idx as u32);
                    } else {
                        def_loc[id] = DefLoc::instruction(bi32, ii as u32);
                    }
                }
                count_uses(inst, |id| use_count[id as usize] += 1);
            }
            block
                .terminator
                .for_each_used_value(|id| use_count[id as usize] += 1);
        }

        // ---- pass 2: prefix-sum, then scatter ------------------------------
        let mut use_offsets: Vec<u32> = vec![0; size + 1];
        let mut running: u32 = 0;
        for i in 0..size {
            use_offsets[i] = running;
            running = running.saturating_add(use_count[i]);
        }
        use_offsets[size] = running;

        let mut use_sites: Vec<UseLoc> = vec![UseLoc::terminator(0); running as usize];
        // Write cursor per value, initialised to the row start.
        let mut cursor: Vec<u32> = use_offsets[..size].to_vec();

        for (bi, block) in func.blocks.iter().enumerate() {
            let bi32 = bi as u32;
            for (ii, inst) in block.instructions.iter().enumerate() {
                let loc = UseLoc::instruction(bi32, ii as u32);
                count_uses(inst, |id| {
                    let idx = id as usize;
                    let pos = cursor[idx] as usize;
                    use_sites[pos] = loc;
                    cursor[idx] += 1;
                });
            }
            let term_loc = UseLoc::terminator(bi32);
            block.terminator.for_each_used_value(|id| {
                let idx = id as usize;
                let pos = cursor[idx] as usize;
                use_sites[pos] = term_loc;
                cursor[idx] += 1;
            });
        }

        debug_assert!(
            cursor
                .iter()
                .enumerate()
                .all(|(i, &c)| c == use_offsets[i + 1]),
            "CSR scatter did not fill every row exactly; count and scatter \
             passes disagree about which operands are uses"
        );

        UseDefInfo {
            use_count,
            def_loc,
            use_offsets,
            use_sites,
            block_count: func.blocks.len() as u32,
        }
    }

    /// Number of value IDs covered (one past the highest ID seen).
    #[inline]
    pub fn len(&self) -> usize {
        self.use_count.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.use_count.is_empty()
    }

    /// True when `v` has no uses at all.
    #[inline]
    pub fn is_dead(&self, v: u32) -> bool {
        self.use_count.get(v as usize).is_none_or(|&c| c == 0)
    }

    /// Where `v` is defined. Out-of-range IDs report [`DefLoc::none`].
    #[inline]
    pub fn def_of(&self, v: u32) -> DefLoc {
        self.def_loc.get(v as usize).copied().unwrap_or(DefLoc::none())
    }

    /// The instruction defining `v`, or `None` for parameters, undefined
    /// values, and out-of-range IDs.
    #[inline]
    pub fn def_inst<'a>(&self, v: u32, func: &'a IrFunction) -> Option<&'a Instruction> {
        let (bi, ii) = self.def_of(v).as_instruction()?;
        func.blocks
            .get(bi as usize)
            .and_then(|b| b.instructions.get(ii as usize))
    }

    /// All use-sites of `v`, in program order. `O(1)`; empty for unknown IDs.
    ///
    /// A value read twice by one instruction yields two adjacent identical
    /// entries; see [`UseDefInfo::use_insts_of`] to collapse them.
    #[inline]
    pub fn uses_of(&self, v: u32) -> &[UseLoc] {
        let idx = v as usize;
        // `use_offsets` has len == size + 1, so row `idx` needs idx + 1 < len.
        if idx.saturating_add(1) >= self.use_offsets.len() {
            return &[];
        }
        let start = self.use_offsets[idx] as usize;
        let end = self.use_offsets[idx + 1] as usize;
        &self.use_sites[start..end]
    }

    /// The *distinct* sites reading `v`, in program order.
    ///
    /// Duplicate entries produced by an instruction reading `v` more than once
    /// (`%d = add %v, %v`) are adjacent by construction, so collapsing runs is
    /// an allocation-free `dedup` over the slice.
    #[inline]
    pub fn use_insts_of(&self, v: u32) -> impl Iterator<Item = UseLoc> + '_ {
        let sites = self.uses_of(v);
        let mut prev: Option<UseLoc> = None;
        sites.iter().copied().filter(move |&loc| {
            if prev == Some(loc) {
                false
            } else {
                prev = Some(loc);
                true
            }
        })
    }

    /// The instruction at `loc`, or `None` if `loc` names a terminator.
    #[inline]
    pub fn use_inst<'a>(&self, loc: UseLoc, func: &'a IrFunction) -> Option<&'a Instruction> {
        if loc.is_terminator() {
            return None;
        }
        func.blocks
            .get(loc.block_idx as usize)
            .and_then(|b| b.instructions.get(loc.inst_idx as usize))
    }

    /// The terminator at `loc`, or `None` if `loc` names an instruction.
    #[inline]
    pub fn use_terminator<'a>(&self, loc: UseLoc, func: &'a IrFunction) -> Option<&'a Terminator> {
        if !loc.is_terminator() {
            return None;
        }
        func.blocks
            .get(loc.block_idx as usize)
            .map(|b| &b.terminator)
    }

    /// Cheap staleness check for debug assertions: block count and value-ID
    /// watermark must still match the function this info was built from.
    ///
    /// This cannot detect every invalidating mutation (an in-place instruction
    /// replacement keeps both quantities), but it catches the common
    /// block-insertion and value-creation cases for free.
    #[inline]
    pub fn is_stale_for(&self, func: &IrFunction) -> bool {
        self.block_count != func.blocks.len() as u32
            || (func.max_value_id() as usize) >= self.len() + 1
    }
}

/// Visit every value read by `inst`, applying the analysis's phi convention.
///
/// Phi self-references are skipped so a loop-carried phi with no external
/// consumer reports zero uses and can be deleted.
#[inline]
fn count_uses(inst: &Instruction, mut f: impl FnMut(u32)) {
    if let Instruction::Phi { dest, incoming, .. } = inst {
        for (op, _) in incoming {
            if let Operand::Value(v) = op {
                if v.0 != dest.0 {
                    f(v.0);
                }
            }
        }
    } else {
        inst.for_each_used_value(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::reexports::*;

    fn make_func(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("test".into(), IrType::Void, vec![], false);
        let mut max = 0u32;
        for b in &blocks {
            for inst in &b.instructions {
                if let Some(v) = inst.dest() {
                    max = max.max(v.0);
                }
            }
        }
        f.blocks = blocks;
        f.next_value_id = max + 1;
        f
    }

    fn blk(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: vec![],
        }
    }

    fn copy_const(dest: u32, c: IrConst) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Const(c),
        }
    }

    #[test]
    fn def_loc_encoding_round_trips() {
        let none = DefLoc::none();
        assert!(none.is_none());
        assert!(!none.is_parameter());
        assert_eq!(none.as_instruction(), None);
        assert_eq!(none.block(), None);

        let inst = DefLoc::instruction(3, 7);
        assert!(!inst.is_none());
        assert!(!inst.is_parameter());
        assert_eq!(inst.as_instruction(), Some((3, 7)));
        assert_eq!(inst.block(), Some(3));

        let param = DefLoc::parameter(2);
        assert!(!param.is_none());
        assert!(param.is_parameter());
        assert_eq!(param.as_instruction(), None);

        // Boundary: the largest legal block/inst indices must not alias the
        // parameter bit or the "none" sentinel.
        let big = DefLoc::instruction((1 << 31) - 1, u32::MAX);
        assert!(!big.is_none());
        assert!(!big.is_parameter());
        assert_eq!(big.as_instruction(), Some(((1 << 31) - 1, u32::MAX)));
    }

    #[test]
    fn counts_uses_and_records_defs() {
        let func = make_func(vec![blk(
            0,
            vec![
                copy_const(0, IrConst::I64(42)),
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(0)),
                    ty: IrType::I64,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        )]);
        let info = UseDefInfo::build(&func);

        assert_eq!(info.use_count[0], 2);
        assert_eq!(info.use_count[1], 1);
        assert_eq!(info.def_of(0).as_instruction(), Some((0, 0)));
        assert_eq!(info.def_of(1).as_instruction(), Some((0, 1)));
        assert!(!info.is_dead(0));
        assert!(!info.is_dead(1));
    }

    #[test]
    fn use_chains_are_csr_indexed_in_program_order() {
        let func = make_func(vec![blk(
            0,
            vec![
                copy_const(0, IrConst::I64(42)),
                Instruction::BinOp {
                    dest: Value(1),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(0)),
                    rhs: Operand::Value(Value(0)),
                    ty: IrType::I64,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        )]);
        let info = UseDefInfo::build(&func);

        let uses0 = info.uses_of(0);
        assert_eq!(uses0, &[UseLoc::instruction(0, 1), UseLoc::instruction(0, 1)]);
        // The duplicate collapses for consumers that want distinct sites.
        assert_eq!(
            info.use_insts_of(0).collect::<Vec<_>>(),
            vec![UseLoc::instruction(0, 1)]
        );

        let uses1 = info.uses_of(1);
        assert_eq!(uses1, &[UseLoc::terminator(0)]);
        assert!(info.use_inst(uses0[0], &func).is_some());
        assert!(info.use_terminator(uses1[0], &func).is_some());
        assert!(info.use_inst(uses1[0], &func).is_none());
        assert!(info.use_terminator(uses0[0], &func).is_none());
    }

    #[test]
    fn phi_self_reference_is_excluded() {
        let func = make_func(vec![
            blk(
                0,
                vec![copy_const(0, IrConst::I64(0))],
                Terminator::Branch(BlockId(1)),
            ),
            blk(
                1,
                vec![Instruction::Phi {
                    dest: Value(1),
                    incoming: vec![
                        (Operand::Value(Value(0)), BlockId(0)),
                        (Operand::Value(Value(1)), BlockId(1)), // self-reference
                    ],
                    ty: IrType::I64,
                }],
                Terminator::Branch(BlockId(1)),
            ),
        ]);
        let info = UseDefInfo::build(&func);

        assert_eq!(info.use_count[0], 1);
        assert_eq!(info.uses_of(0), &[UseLoc::instruction(1, 0)]);
        // The self-reference is the phi's only operand use, so the phi is dead.
        assert_eq!(info.use_count[1], 0);
        assert!(info.is_dead(1));
        assert!(info.uses_of(1).is_empty());
    }

    #[test]
    fn out_of_range_queries_are_empty_not_panics() {
        let func = make_func(vec![blk(
            0,
            vec![copy_const(0, IrConst::I64(42))],
            Terminator::Return(None),
        )]);
        let info = UseDefInfo::build(&func);

        assert!(info.is_dead(0));
        assert!(info.uses_of(0).is_empty());
        assert!(info.uses_of(999).is_empty());
        assert!(info.uses_of(u32::MAX).is_empty());
        assert!(info.def_of(999).is_none());
        assert!(info.def_inst(999, &func).is_none());
    }

    /// A value whose ID exceeds the cached `next_value_id` watermark must still
    /// get a complete use-chain. Sizing the tables from the stale cache alone
    /// would silently drop its uses, which for SCCP means the consumer keeps an
    /// optimistic lattice value forever — a miscompile.
    #[test]
    fn tables_cover_values_above_a_stale_next_value_id() {
        let mut func = make_func(vec![blk(
            0,
            vec![
                copy_const(7, IrConst::I32(1)),
                Instruction::BinOp {
                    dest: Value(9),
                    op: IrBinOp::Add,
                    lhs: Operand::Value(Value(7)),
                    rhs: Operand::Const(IrConst::I32(2)),
                    ty: IrType::I32,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(9)))),
        )]);
        func.next_value_id = 1; // deliberately stale

        let info = UseDefInfo::build(&func);
        assert!(info.len() >= 10, "tables must cover value 9");
        assert_eq!(info.use_count[7], 1);
        assert_eq!(info.uses_of(7), &[UseLoc::instruction(0, 1)]);
        assert_eq!(info.uses_of(9), &[UseLoc::terminator(0)]);
        assert_eq!(info.def_of(9).as_instruction(), Some((0, 1)));
    }

    /// Pointer operands stored as bare `Value` fields (Load.ptr, GEP.base,
    /// Store.ptr) are real uses. Missing them would let DCE delete a live
    /// address computation.
    #[test]
    fn value_position_pointer_operands_count_as_uses() {
        let func = make_func(vec![blk(
            0,
            vec![
                Instruction::Alloca {
                    dest: Value(0),
                    ty: IrType::I64,
                    size: 8,
                    align: 8,
                    volatile: false,
                    semantic_volatile: false,
                },
                Instruction::Store {
                    val: Operand::Const(IrConst::I64(1)),
                    ptr: Value(0),
                    ty: IrType::I64,
                    seg_override: Default::default(),
                    volatile: false,
                },
                Instruction::Load {
                    dest: Value(1),
                    ptr: Value(0),
                    ty: IrType::I64,
                    seg_override: Default::default(),
                    volatile: false,
                },
            ],
            Terminator::Return(Some(Operand::Value(Value(1)))),
        )]);
        let info = UseDefInfo::build(&func);

        // Alloca result is read by both the Store and the Load.
        assert_eq!(info.use_count[0], 2);
        assert_eq!(
            info.uses_of(0),
            &[UseLoc::instruction(0, 1), UseLoc::instruction(0, 2)]
        );
    }

    #[test]
    fn parameters_are_tagged_as_parameters() {
        let func = make_func(vec![blk(
            0,
            vec![Instruction::ParamRef {
                dest: Value(0),
                param_idx: 3,
                ty: IrType::I32,
            }],
            Terminator::Return(Some(Operand::Value(Value(0)))),
        )]);
        let info = UseDefInfo::build(&func);

        assert!(info.def_of(0).is_parameter());
        assert_eq!(info.def_of(0).as_instruction(), None);
        assert!(info.def_inst(0, &func).is_none());
    }

    #[test]
    fn terminator_operands_are_recorded_per_block() {
        let func = make_func(vec![
            blk(
                0,
                vec![copy_const(0, IrConst::I32(1))],
                Terminator::CondBranch {
                    cond: Operand::Value(Value(0)),
                    true_label: BlockId(1),
                    false_label: BlockId(2),
                },
            ),
            blk(
                1,
                vec![],
                Terminator::Switch {
                    val: Operand::Value(Value(0)),
                    cases: vec![(1, BlockId(2))],
                    default: BlockId(2),
                    ty: IrType::I32,
                },
            ),
            blk(2, vec![], Terminator::Return(None)),
        ]);
        let info = UseDefInfo::build(&func);

        assert_eq!(info.use_count[0], 2);
        assert_eq!(
            info.uses_of(0),
            &[UseLoc::terminator(0), UseLoc::terminator(1)]
        );
    }

    #[test]
    fn staleness_check_notices_new_blocks() {
        let mut func = make_func(vec![blk(
            0,
            vec![copy_const(0, IrConst::I32(1))],
            Terminator::Return(None),
        )]);
        let info = UseDefInfo::build(&func);
        assert!(!info.is_stale_for(&func));
        func.blocks.push(blk(1, vec![], Terminator::Return(None)));
        assert!(info.is_stale_for(&func));
    }
}

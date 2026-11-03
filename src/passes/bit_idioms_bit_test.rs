//! Single-bit TESTS: every reader shape of `x & (1 << n)`, and the selects
//! and joins that pick between `x` and a bit operation of `x` under that
//! test.
//!
//! [`super::bit_select`] folds one instruction at a time and only answers the
//! `!= 0` direction of a test, because rewriting a test whose answer is
//! consumed as a VALUE needs the readers' cooperation.  The rewrites here look
//! at all readers of a value at once:
//!
//! * [`zero_tests_to_bit_tests`]: `x & (1 << n)` whose readers only ask "is it
//!   zero?" (`== 0`, `!= 0`, a branch, a select) becomes `BitTest(x, n)` IN
//!   PLACE.  `BitTest` is zero exactly when the AND is, so no reader changes.
//!   This is the shape of every `if (flags & (1u << bit))` in the browser
//!   corpora (dav1d `decode.c:1639`, `thread_task.c:649`/`820`, FreeType's
//!   `ttcmap.c` coverage bitmaps), which never contain a compare at all.  Two
//!   variants ride along: `~x & (1 << n)` tests the bit of `x` with every
//!   reader's polarity flipped, and FreeType's MSB-first `b & (0x80 >> k)` is
//!   `BitTest(b, 7 ^ k)` once `k <= 7` is proven.
//! * [`canonicalize_bit_test_branches`]: `if (BitTest(x, n) == 0)` branches on
//!   the test itself with the targets swapped, which every backend lowers as
//!   one `bt; jnc` / `tbz` instead of materializing the boolean first.
//! * [`fold_bit_test_selects`] and [`fold_bit_test_phis`]: a select (or a
//!   two-way join) between `x` and `BitSet`/`BitClear`/`BitToggle(x, n)`
//!   under a test of the SAME bit is itself a single bit operation of `x`.
//!   `if (!(M & (1 << I))) M |= 1 << I;` (SQLite's `DbMaskSet` guard) is just
//!   `M | (1 << I)`, and `if (x & m) { x &= ~m; ... }` is `x & ~m` on both
//!   paths.  Neither GCC 16.2 nor Clang 23.1.4 folds the first one.
//!
//! The x86 flag fusion of a test with a modify of the same bit lives in
//! [`schedule_bit_test_modify_pairs`]; it runs immediately before
//! instruction selection, because it only places instructions for the
//! backend's adjacency-based fusion and any later IR pass could undo that.

use super::bit_select::{
    SharedLoc, complement_of_one_hot, index_upper_bound, one_hot_at, only_zero_tested,
    strip_index_mask,
};
use super::*;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::ir::reexports::{BasicBlock, BlockId, Terminator};

/// Per-function def/use tables shared by the rewrites below.  `defs` is the
/// clone table the single-instruction matchers in `bit_idioms` read.
struct Tables {
    defs: Vec<Option<Instruction>>,
    def_loc: Vec<Option<(usize, usize)>>,
    uses: Vec<Vec<SharedLoc>>,
}

impl Tables {
    fn build(func: &IrFunction) -> Option<Tables> {
        let max_id = func.max_value_id();
        if func.next_value_id == u32::MAX || max_id > 1_000_000 {
            return None;
        }
        let len = max_id as usize + 1;
        let mut defs = vec![None; len];
        let mut def_loc = vec![None; len];
        let mut ambiguous = false;
        let mut uses = vec![Vec::new(); len];
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                if let Some(dest) = inst.dest() {
                    let id = dest.0 as usize;
                    if id >= len {
                        return None;
                    }
                    if def_loc[id].is_some() {
                        ambiguous = true;
                    }
                    defs[id] = Some(inst.clone());
                    def_loc[id] = Some((bi, ii));
                }
                let loc = SharedLoc {
                    block: bi,
                    inst: Some(ii),
                };
                let mut out_of_range = false;
                crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                    if let Operand::Value(v) = op {
                        match uses.get_mut(v.0 as usize) {
                            Some(list) => list.push(loc),
                            None => out_of_range = true,
                        }
                    }
                });
                crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                    match uses.get_mut(v.0 as usize) {
                        Some(list) => list.push(loc),
                        None => out_of_range = true,
                    }
                });
                if out_of_range {
                    return None;
                }
            }
            let loc = SharedLoc {
                block: bi,
                inst: None,
            };
            let mut out_of_range = false;
            crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
                if let Operand::Value(v) = op {
                    match uses.get_mut(v.0 as usize) {
                        Some(list) => list.push(loc),
                        None => out_of_range = true,
                    }
                }
            });
            if out_of_range {
                return None;
            }
        }
        // A value defined twice is not SSA; nothing here reasons about it.
        if ambiguous {
            return None;
        }
        Some(Tables {
            defs,
            def_loc,
            uses,
        })
    }

    fn use_count(&self, id: u32) -> usize {
        self.uses.get(id as usize).map_or(usize::MAX, Vec::len)
    }

    fn def(&self, op: Operand) -> Option<&Instruction> {
        match op {
            Operand::Value(v) => self.defs.get(v.0 as usize).and_then(Option::as_ref),
            Operand::Const(_) => None,
        }
    }

    fn chain_dies(&self, chain: &[u32]) -> bool {
        !chain.is_empty() && chain.iter().all(|id| self.use_count(*id) == 1)
    }
}

/// The widths a `BitTest`/`BitSet`/... may be created at by these rewrites.
/// Same rule as the single-instruction matcher: no 128-bit pair, and no 64-bit
/// operation on a 32-bit target.
fn creatable(ty: IrType) -> bool {
    ty.is_integer()
        && !ty.is_128bit()
        && (ty.size() <= 4 || !crate::common::types::target_is_32bit())
}

/// The integer type an operand is produced at, when its producer says so.
fn int_type_of(op: Operand, defs: &[Option<Instruction>]) -> Option<IrType> {
    let mut op = op;
    for _ in 0..32 {
        let ty = match op {
            Operand::Const(c) => match c {
                IrConst::I8(_) => IrType::I8,
                IrConst::I16(_) => IrType::I16,
                IrConst::I32(_) => IrType::I32,
                IrConst::I64(_) => IrType::I64,
                _ => return None,
            },
            Operand::Value(v) => match defs.get(v.0 as usize).and_then(Option::as_ref)? {
                Instruction::Copy { src, .. } => {
                    op = *src;
                    continue;
                }
                Instruction::BinOp { ty, .. }
                | Instruction::UnaryOp { ty, .. }
                | Instruction::Load { ty, .. }
                | Instruction::ParamRef { ty, .. }
                | Instruction::Phi { ty, .. }
                | Instruction::Select { ty, .. } => *ty,
                Instruction::Cast { to_ty, .. } => *to_ty,
                _ => return None,
            },
        };
        return (ty.is_integer() && !ty.is_128bit()).then_some(ty);
    }
    None
}

/// Whether the MSB-first rewrite below pays on this target: only where the
/// test of a register index is one instruction (x86 `bt`), so the inserted
/// `k ^ p` replaces the constant and the `%cl`-pinned shift.  AArch64 and
/// RV64 test `0x80 >> k` as a shift of a constant plus `tst`/`and`, exactly
/// as long as the `eor` + shift + `and` the rewrite would buy, and the extra
/// index computation measured one instruction longer per test there
/// (engineering/evidence/2026-11-02-bit-test-fusion).
fn msb_first_profitable() -> bool {
    matches!(crate::common::types::target_arch_name(), "x86-64" | "i686")
}

/// The MSB-first mask `0x80 >> k` (FreeType `sfnt/ttcmap.c:1868` and
/// `sfnt/sfwoff2.c:957`, LZMA SDK `7z.h:75`) at `bits` width: a power-of-two
/// constant `2^p` shifted RIGHT by a count provably at most `p`, so exactly
/// bit `p - k` is set.  Returns the count operand, `p`, and the mask path for
/// the single-use proof.
fn msb_first_one_hot(
    op: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    chain: &mut Vec<u32>,
) -> Option<(Operand, u64)> {
    let mut op = op;
    for _ in 0..16 {
        let Operand::Value(v) = op else {
            return None;
        };
        chain.push(v.0);
        match defs.get(v.0 as usize).and_then(Option::as_ref)? {
            Instruction::Copy { src, .. } => op = *src,
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer()
                && to_ty.is_integer()
                && from_ty.size() == to_ty.size()
                && to_ty.size() * 8 == bits =>
            {
                op = *src;
            }
            Instruction::BinOp {
                op: shift @ (IrBinOp::LShr | IrBinOp::AShr),
                lhs,
                rhs,
                ty,
                ..
            } if ty.is_integer() && ty.size() * 8 == bits && bits <= 64 => {
                let c = const_in_ring(*lhs, bits)?;
                if !c.is_power_of_two() {
                    return None;
                }
                let p = u64::from(c.trailing_zeros());
                // An arithmetic shift of a constant with the sign bit set
                // smears it; a non-negative one is a logical shift.
                if *shift == IrBinOp::AShr && p + 1 >= bits as u64 {
                    return None;
                }
                if index_upper_bound(*rhs, defs)? > p {
                    return None;
                }
                return Some((*rhs, p));
            }
            _ => return None,
        }
    }
    None
}

/// One planned in-place AND rewrite, with the reader edits it needs.
struct ZeroTestRewrite {
    at: (usize, usize),
    inst: Instruction,
    /// `Some((count, p))`: the index is a fresh `count ^ p` / `p - count`
    /// inserted right before the rewritten AND.
    msb_index: Option<(Operand, u64, IrType)>,
    /// Readers whose polarity flips (`~x & m` tests the bit of `x`).
    flip: Vec<SharedLoc>,
}

/// `x & (1 << n)` read only by zero tests -> `BitTest(x, n)` in place.
///
/// The mask path must die with the AND (a mask with other readers is the
/// shared-mask pass's business on x86, and on AArch64/RISC-V a live mask makes
/// the plain `tst` the cheaper test).  When `x` is a complement `~y`, the test
/// reads `y` instead and every reader's polarity is flipped; that needs direct
/// readers, so a copy between the AND and a reader refuses the flip.
fn zero_tests_to_bit_tests(func: &mut IrFunction) -> usize {
    let Some(t) = Tables::build(func) else {
        return 0;
    };
    let mut plans: Vec<ZeroTestRewrite> = Vec::new();
    let mut touched: FxHashSet<SharedLoc> = FxHashSet::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op: IrBinOp::And,
                lhs,
                rhs,
                ty,
            } = inst
            else {
                continue;
            };
            if !creatable(*ty) {
                continue;
            }
            let bits = ty.size() * 8;
            if !only_zero_tested(dest.0, bits, func, &t.uses) {
                continue;
            }
            let Some(plan) = plan_zero_test(func, &t, (bi, ii), *dest, *lhs, *rhs, *ty) else {
                continue;
            };
            let at = SharedLoc {
                block: bi,
                inst: Some(ii),
            };
            if touched.contains(&at) || plan.flip.iter().any(|l| touched.contains(l)) {
                continue;
            }
            touched.insert(at);
            touched.extend(plan.flip.iter().copied());
            plans.push(plan);
        }
    }
    apply_bit_op_plans(func, plans)
}

/// Apply planned in-place rewrites: insert each MSB-first index right before
/// its operation, replace the operation, flip the readers that need it.
fn apply_bit_op_plans(func: &mut IrFunction, plans: Vec<ZeroTestRewrite>) -> usize {
    if plans.is_empty() {
        return 0;
    }
    let changes = plans.len();
    let mut next_id = func.sound_next_value_id();
    let mut inserts: Vec<(usize, usize, Instruction)> = Vec::new();
    for plan in plans {
        let (bi, ii) = plan.at;
        let mut inst = plan.inst;
        if let Some((count, p, count_ty)) = plan.msb_index {
            let index = Value(next_id);
            next_id += 1;
            // `p - k` for `0 <= k <= p`; when `p` is all ones below its top
            // bit (7, 15, 31, 63) the subtraction never borrows and is `k ^ p`,
            // one two-operand instruction instead of a constant plus a `sub`.
            let new_index = if (p + 1).is_power_of_two() {
                Instruction::BinOp {
                    dest: index,
                    op: IrBinOp::Xor,
                    lhs: count,
                    rhs: Operand::Const(IrConst::from_i64(p as i64, count_ty)),
                    ty: count_ty,
                }
            } else {
                Instruction::BinOp {
                    dest: index,
                    op: IrBinOp::Sub,
                    lhs: Operand::Const(IrConst::from_i64(p as i64, count_ty)),
                    rhs: count,
                    ty: count_ty,
                }
            };
            if let Instruction::BinOp { rhs, .. } = &mut inst {
                *rhs = Operand::Value(index);
            }
            inserts.push((bi, ii, new_index));
        }
        func.blocks[bi].instructions[ii] = inst;
        for loc in plan.flip {
            flip_zero_test_reader(func, loc);
        }
    }
    // Insert highest index first so earlier coordinates stay valid.
    inserts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    for (bi, ii, inst) in inserts {
        // The index belongs to the test it feeds: same source construct.
        let block = &mut func.blocks[bi];
        let span = block.span_at(ii);
        block.insert_inst(ii, inst, span);
    }
    func.next_value_id = next_id;
    changes
}

/// FreeType `pshinter/pshrec.c:182` `p[0] | (0x80 >> (idx & 7))`: the
/// MSB-first mask under a set, toggle or clear.  Where the zero-test rewrite
/// pays ([`msb_first_profitable`]: x86, whose `bts`/`btc`/`btr` take a register
/// index) the same `k ^ p` index turns the constant, the `%cl`-pinned shift
/// and the logic operation into one bit instruction:
///
/// ```text
///     andl $7, %esi                   andl $7, %esi
///     movl $128, %r10d                xorl $7, %esi
///     movl %esi, %ecx           ->    btsl %esi, %r11d
///     sarl %cl, %r10d
///     orl %r10d, %r11d
/// ```
///
/// GCC 16.2 and Clang 23.1.4 emit the shift-and-`or` form (8 instructions for
/// `ps_mask_set_bit`); this is 8 as well, one register and the `%cl` pin
/// fewer.  `msb_first_one_hot` proves the count is at most `p`, so `k ^ p` is
/// `p - k` exactly and names a bit inside the operation's width; the mask path
/// (and the complement of a clear) must die with the rewritten operation.
///
/// x86-64 only: the i686 accumulator backend stages both the index and the
/// base through `%eax` around the `bts` and measured one instruction LONGER
/// than its shift form (21 vs 20 on `ft_mask_set`), unlike the zero test.
fn msb_first_modifies(func: &mut IrFunction) -> usize {
    if crate::common::types::target_arch_name() != "x86-64" {
        return 0;
    }
    let candidate = |i: &Instruction| {
        matches!(
            i,
            Instruction::BinOp {
                op: IrBinOp::Or | IrBinOp::Xor | IrBinOp::And,
                ..
            }
        )
    };
    if !func
        .blocks
        .iter()
        .any(|b| b.instructions.iter().any(candidate))
    {
        return 0;
    }
    let Some(t) = Tables::build(func) else {
        return 0;
    };
    let mut plans: Vec<ZeroTestRewrite> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                dest,
                op,
                lhs,
                rhs,
                ty,
            } = inst
            else {
                continue;
            };
            let bit_op = match op {
                IrBinOp::Or => IrBinOp::BitSet,
                IrBinOp::Xor => IrBinOp::BitToggle,
                IrBinOp::And => IrBinOp::BitClear,
                _ => continue,
            };
            if !creatable(*ty) {
                continue;
            }
            let bits = ty.size() * 8;
            let mut hits = Vec::new();
            for (mask, base) in [(*rhs, *lhs), (*lhs, *rhs)] {
                let mut chain = Vec::new();
                let mask = if bit_op == IrBinOp::BitClear {
                    match complement_base(mask, bits, &t.defs, &mut chain) {
                        Some(inner) => inner,
                        None => continue,
                    }
                } else {
                    mask
                };
                let Some((count, p)) = msb_first_one_hot(mask, bits, &t.defs, &mut chain) else {
                    continue;
                };
                let Some(count_ty) = int_type_of(count, &t.defs) else {
                    continue;
                };
                if t.chain_dies(&chain) {
                    hits.push((base, count, p, count_ty));
                }
            }
            // Both operands masks: either rewrite would keep the other live.
            let [(base, count, p, count_ty)] = hits[..] else {
                continue;
            };
            plans.push(ZeroTestRewrite {
                at: (bi, ii),
                inst: Instruction::BinOp {
                    dest: *dest,
                    op: bit_op,
                    lhs: base,
                    rhs: count,
                    ty: *ty,
                },
                msb_index: Some((count, p, count_ty)),
                flip: Vec::new(),
            });
        }
    }
    apply_bit_op_plans(func, plans)
}

fn plan_zero_test(
    func: &IrFunction,
    t: &Tables,
    at: (usize, usize),
    dest: Value,
    lhs: Operand,
    rhs: Operand,
    ty: IrType,
) -> Option<ZeroTestRewrite> {
    let bits = ty.size() * 8;
    let mut found: Option<(Operand, Operand, Option<(Operand, u64, IrType)>)> = None;
    for (mask, base) in [(rhs, lhs), (lhs, rhs)] {
        let mut chain = Vec::new();
        let hit = if let Some(index) = one_hot_at(mask, bits, &t.defs, &mut chain) {
            Some((base, index, None))
        } else {
            chain.clear();
            let msb = if msb_first_profitable() {
                msb_first_one_hot(mask, bits, &t.defs, &mut chain)
            } else {
                None
            };
            msb.and_then(|(count, p)| {
                let count_ty = int_type_of(count, &t.defs)?;
                Some((base, count, Some((count, p, count_ty))))
            })
        };
        let Some(hit) = hit else {
            continue;
        };
        if !t.chain_dies(&chain) {
            continue;
        }
        if found.is_some() {
            // Both sides are masks: either rewrite would keep the other.
            return None;
        }
        found = Some(hit);
    }
    let (base, index, msb_index) = found?;
    // `~y & m` tests bit `n` of `y` with the opposite answer.
    let mut flip = Vec::new();
    let mut test_base = base;
    let mut complement_chain = Vec::new();
    if let Some(inner) = complement_base(base, bits, &t.defs, &mut complement_chain) {
        let readers = t.uses.get(dest.0 as usize)?;
        let direct = readers.iter().all(|loc| match loc.inst {
            None => true,
            Some(ii) => !matches!(
                func.blocks[loc.block].instructions[ii],
                Instruction::Copy { .. }
            ),
        });
        if direct {
            flip = readers.clone();
            test_base = inner;
        }
    }
    Some(ZeroTestRewrite {
        at,
        inst: Instruction::BinOp {
            dest,
            op: IrBinOp::BitTest,
            lhs: test_base,
            rhs: index,
            ty,
        },
        msb_index,
        flip,
    })
}

/// `y` when `op` is `~y` (or `y ^ all_ones`) at `bits` width, read through
/// same-width integer casts on either side: a sign reinterpretation keeps
/// every bit, so `(unsigned)~prog` (dav1d `thread_task.c:820`, where `prog`
/// is an `int` and the mask unsigned) complements `prog` exactly.
fn complement_base(
    op: Operand,
    bits: usize,
    defs: &[Option<Instruction>],
    chain: &mut Vec<u32>,
) -> Option<Operand> {
    let mut op = op;
    for _ in 0..8 {
        let Operand::Value(v) = peel_copies(op, defs) else {
            return None;
        };
        chain.push(v.0);
        match defs.get(v.0 as usize).and_then(Option::as_ref)? {
            Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            } if from_ty.is_integer()
                && to_ty.is_integer()
                && !from_ty.is_128bit()
                && from_ty.size() == to_ty.size()
                && to_ty.size() * 8 == bits =>
            {
                op = *src;
            }
            Instruction::UnaryOp {
                op: IrUnaryOp::Not,
                src,
                ty,
                ..
            } if ty.is_integer() && ty.size() * 8 == bits => return Some(*src),
            Instruction::BinOp {
                op: IrBinOp::Xor,
                lhs,
                rhs,
                ty,
                ..
            } if ty.is_integer() && ty.size() * 8 == bits => {
                let all_ones = u64::MAX >> (64 - bits);
                return if const_in_ring(*rhs, bits) == Some(all_ones) {
                    Some(*lhs)
                } else if const_in_ring(*lhs, bits) == Some(all_ones) {
                    Some(*rhs)
                } else {
                    None
                };
            }
            _ => return None,
        }
    }
    None
}

/// The base of a `BitTest` read through an integer cast that cannot change
/// the tested bit, retargeted to the cast's source:
///
/// * a same-width cast (a sign reinterpretation keeps every bit) at any
///   index -- `p1 & (1U << k)` converts the `int` `p1` first (dav1d
///   `thread_task.c:649`), which otherwise costs a register copy;
/// * a 32 -> 64-bit extension, zero or sign, when the index is provably
///   below 32: bit `n < 32` of either extension is bit `n` of the source, so
///   the test runs at the source's width and an `& 31` that only kept the
///   64-bit index in range strips with it (brotli `dec/decode.c:1251-1252`,
///   `size_t trivial = trivial_literal_contexts[...]; (trivial >> (bt & 31))
///   & 1` -> one `btl`, Clang 23.1.4's spelling).
///
/// The cast itself stays for any other reader (DCE removes it otherwise).
fn retarget_bit_test_bases(func: &mut IrFunction, t: &Tables) -> usize {
    let counts: Vec<u32> = t.uses.iter().map(|u| u.len() as u32).collect();
    let mut edits: Vec<(usize, usize, Operand, IrType, Operand)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let Instruction::BinOp {
                op: IrBinOp::BitTest,
                lhs: Operand::Value(b),
                rhs,
                ty,
                ..
            } = inst
            else {
                continue;
            };
            let Some(Instruction::Cast {
                src,
                from_ty,
                to_ty,
                ..
            }) = t.defs.get(b.0 as usize).and_then(Option::as_ref)
            else {
                continue;
            };
            if !from_ty.is_integer()
                || !to_ty.is_integer()
                || from_ty.is_128bit()
                || to_ty.size() != ty.size()
                || matches!(src, Operand::Const(_))
            {
                continue;
            }
            if from_ty.size() == to_ty.size() {
                edits.push((bi, ii, *src, *ty, *rhs));
            } else if from_ty.size() == 4
                && to_ty.size() == 8
                && index_upper_bound(*rhs, &t.defs).is_some_and(|bound| bound < 32)
            {
                let index = strip_index_mask(*rhs, 32, &t.defs, &counts);
                edits.push((bi, ii, *src, *from_ty, index));
            }
        }
    }
    let changes = edits.len();
    for (bi, ii, base, width, index) in edits {
        if let Instruction::BinOp { lhs, rhs, ty, .. } = &mut func.blocks[bi].instructions[ii] {
            *lhs = base;
            *rhs = index;
            *ty = width;
        }
    }
    changes
}

/// Flip one zero-test reader: `== 0` <-> `!= 0`, swap a branch's targets or
/// a select's arms.
fn flip_zero_test_reader(func: &mut IrFunction, loc: SharedLoc) {
    let block = &mut func.blocks[loc.block];
    match loc.inst {
        None => {
            if let Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } = &mut block.terminator
            {
                std::mem::swap(true_label, false_label);
            }
        }
        Some(ii) => match &mut block.instructions[ii] {
            Instruction::Cmp { op, .. } => {
                *op = match *op {
                    IrCmpOp::Eq => IrCmpOp::Ne,
                    IrCmpOp::Ne => IrCmpOp::Eq,
                    other => other,
                };
            }
            Instruction::Select {
                true_val,
                false_val,
                ..
            } => std::mem::swap(true_val, false_val),
            _ => {}
        },
    }
}

/// `i & M` read ONLY as the index of bit operations whose width `W` has
/// `W - 1` inside `M` -> every reader reads `i` itself (the AND then dies).
/// The single-reader case is the scan's (`strip_index_mask`); this is the
/// shared one: `m = 1u << (i & 31)` feeding both the test and the set of a
/// "visited" bitmap (`ff_visit`), where the shared-mask pass hands both new
/// bit operations the same `i & 31`.  Every bit operation reads `index mod
/// W`, which only depends on the bits `M` keeps.
fn strip_shared_index_masks(func: &mut IrFunction, t: &Tables) -> usize {
    use super::bit_select::peel_index;
    let mut rewrites: Vec<(usize, usize, Operand)> = Vec::new();
    for (id, def) in t.defs.iter().enumerate() {
        let Some(Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
            ..
        }) = def
        else {
            continue;
        };
        if !ty.is_integer() || ty.is_128bit() {
            continue;
        }
        let bits = ty.size() * 8;
        let (inner, mask) = match (const_in_ring(*rhs, bits), const_in_ring(*lhs, bits)) {
            (Some(m), _) => (*lhs, m),
            (None, Some(m)) => (*rhs, m),
            _ => continue,
        };
        let readers = &t.uses[id];
        if readers.len() < 2 {
            continue;
        }
        let me = Operand::Value(Value(id as u32));
        let all_index_reads = readers.iter().all(|loc| {
            let Some(ii) = loc.inst else {
                return false;
            };
            match &func.blocks[loc.block].instructions[ii] {
                Instruction::BinOp {
                    op: IrBinOp::BitSet | IrBinOp::BitClear | IrBinOp::BitToggle | IrBinOp::BitTest,
                    lhs: base,
                    rhs: index,
                    ty: op_ty,
                    ..
                } => {
                    let low = (op_ty.size() * 8 - 1) as u64;
                    *index == me && *base != me && mask & low == low
                }
                _ => false,
            }
        });
        if !all_index_reads {
            continue;
        }
        let new_index = peel_index(inner, &t.defs);
        for loc in readers {
            if let Some(ii) = loc.inst {
                rewrites.push((loc.block, ii, new_index));
            }
        }
    }
    let changes = rewrites.len();
    for (bi, ii, new_index) in rewrites {
        if let Instruction::BinOp { rhs, .. } = &mut func.blocks[bi].instructions[ii] {
            *rhs = new_index;
        }
    }
    changes
}

/// `CondBranch(BitTest(x, n) ==/!= 0)` -> `CondBranch(BitTest(x, n))`, with
/// the targets swapped for `== 0`.  The test is already the 0/1 answer, so
/// the comparison only stands between it and a fused `bt; jc` / `tbnz`.
fn canonicalize_bit_test_branches(func: &mut IrFunction, t: &Tables) -> usize {
    let mut changes = 0;
    for block in &mut func.blocks {
        let Terminator::CondBranch {
            cond,
            true_label,
            false_label,
        } = &mut block.terminator
        else {
            continue;
        };
        let Some(Instruction::Cmp {
            op: op @ (IrCmpOp::Eq | IrCmpOp::Ne),
            lhs,
            rhs,
            ty,
            ..
        }) = t.def(peel_copies(*cond, &t.defs))
        else {
            continue;
        };
        if !ty.is_integer() || ty.is_128bit() {
            continue;
        }
        let bits = ty.size() * 8;
        let tested = if const_in_ring(*rhs, bits) == Some(0) {
            *lhs
        } else if const_in_ring(*lhs, bits) == Some(0) {
            *rhs
        } else {
            continue;
        };
        let tested = peel_copies(tested, &t.defs);
        let Some(Instruction::BinOp {
            op: IrBinOp::BitTest,
            ty: test_ty,
            ..
        }) = t.def(tested)
        else {
            continue;
        };
        if test_ty.size() != ty.size() {
            continue;
        }
        *cond = tested;
        if *op == IrCmpOp::Eq {
            std::mem::swap(true_label, false_label);
        }
        changes += 1;
    }
    changes
}

/// What a select arm does to bit `n` of `x` (every other bit is `x`'s).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Effect {
    Keep,
    Set,
    Clear,
    Toggle,
}

impl Effect {
    fn apply(self, bit: bool) -> bool {
        match self {
            Effect::Keep => bit,
            Effect::Set => true,
            Effect::Clear => false,
            Effect::Toggle => !bit,
        }
    }

    fn from_table(when_clear: bool, when_set: bool) -> Effect {
        match (when_clear, when_set) {
            (false, true) => Effect::Keep,
            (true, true) => Effect::Set,
            (false, false) => Effect::Clear,
            (true, false) => Effect::Toggle,
        }
    }

    fn op(self) -> Option<IrBinOp> {
        match self {
            Effect::Keep => None,
            Effect::Set => Some(IrBinOp::BitSet),
            Effect::Clear => Some(IrBinOp::BitClear),
            Effect::Toggle => Some(IrBinOp::BitToggle),
        }
    }
}

/// A test of bit `index` of `base` at `bits` width; `set_is_true` says
/// whether a nonzero condition means the bit is set.
#[derive(Clone, Copy)]
struct BitCond {
    base: Operand,
    index: Operand,
    bits: usize,
    set_is_true: bool,
}

/// Read a condition operand as a single-bit test: `BitTest(x, n)`, `x & (1 <<
/// n)` (the mask may have other readers -- nothing is rewritten here), or
/// either of them compared `==`/`!=` with zero.
fn bit_cond(cond: Operand, defs: &[Option<Instruction>]) -> Option<BitCond> {
    let op = peel_copies(cond, defs);
    let Operand::Value(v) = op else {
        return None;
    };
    let Some(Instruction::Cmp {
        op: cmp @ (IrCmpOp::Eq | IrCmpOp::Ne),
        lhs,
        rhs,
        ty,
        ..
    }) = defs.get(v.0 as usize).and_then(Option::as_ref)
    else {
        return bit_cond_value(op, defs);
    };
    if !ty.is_integer() || ty.is_128bit() {
        return None;
    }
    let bits = ty.size() * 8;
    let tested = if const_in_ring(*rhs, bits) == Some(0) {
        *lhs
    } else if const_in_ring(*lhs, bits) == Some(0) {
        *rhs
    } else {
        return None;
    };
    let found = bit_cond_value(peel_copies(tested, defs), defs)?;
    // The compare must read the test at its own width.
    if found.bits != bits {
        return None;
    }
    // `!= 0` keeps the test's polarity, `== 0` inverts it.
    Some(BitCond {
        set_is_true: found.set_is_true == (*cmp == IrCmpOp::Ne),
        ..found
    })
}

fn bit_cond_value(op: Operand, defs: &[Option<Instruction>]) -> Option<BitCond> {
    let Operand::Value(v) = op else {
        return None;
    };
    match defs.get(v.0 as usize).and_then(Option::as_ref)? {
        Instruction::BinOp {
            op: IrBinOp::BitTest,
            lhs,
            rhs,
            ty,
            ..
        } if ty.is_integer() && !ty.is_128bit() => Some(BitCond {
            base: *lhs,
            index: *rhs,
            bits: ty.size() * 8,
            set_is_true: true,
        }),
        Instruction::BinOp {
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
            ..
        } if ty.is_integer() && !ty.is_128bit() => {
            let bits = ty.size() * 8;
            let mut lc = Vec::new();
            let mut rc = Vec::new();
            let (base, index) = match (
                one_hot_at(*lhs, bits, defs, &mut lc),
                one_hot_at(*rhs, bits, defs, &mut rc),
            ) {
                (Some(index), None) => (*rhs, index),
                (None, Some(index)) => (*lhs, index),
                _ => return None,
            };
            Some(BitCond {
                base,
                index,
                bits,
                set_is_true: true,
            })
        }
        _ => None,
    }
}

/// Bases must be the same SSA value (copies only: `zext(x)` is not `x`).
fn same_base(a: Operand, b: Operand, defs: &[Option<Instruction>]) -> bool {
    match (peel_copies(a, defs), peel_copies(b, defs)) {
        (Operand::Value(a), Operand::Value(b)) => a == b,
        _ => false,
    }
}

/// Indices compare modulo integer casts: `index mod W` (W <= 64) only reads
/// the low six bits, which every cast preserves.
fn same_index(a: Operand, b: Operand, defs: &[Option<Instruction>]) -> bool {
    same(a, b, defs)
}

/// What `arm` does to bit `cond.index` of `cond.base`, if it is `x` itself or
/// a single-bit operation of `x` at the same width and index (`BitSet` & co.,
/// or the `|`, `& ~`, `^` spellings with a one-hot mask).
fn arm_effect(arm: Operand, cond: &BitCond, defs: &[Option<Instruction>]) -> Option<Effect> {
    if same_base(arm, cond.base, defs) {
        return Some(Effect::Keep);
    }
    let Operand::Value(v) = peel_copies(arm, defs) else {
        return None;
    };
    let Instruction::BinOp {
        op, lhs, rhs, ty, ..
    } = defs.get(v.0 as usize).and_then(Option::as_ref)?
    else {
        return None;
    };
    if !ty.is_integer() || ty.size() * 8 != cond.bits {
        return None;
    }
    let bits = cond.bits;
    let effect = match op {
        IrBinOp::BitSet => Effect::Set,
        IrBinOp::BitClear => Effect::Clear,
        IrBinOp::BitToggle => Effect::Toggle,
        IrBinOp::Or | IrBinOp::Xor | IrBinOp::And => {
            let mut chain = Vec::new();
            let effect = match op {
                IrBinOp::Or => Effect::Set,
                IrBinOp::Xor => Effect::Toggle,
                _ => Effect::Clear,
            };
            for (mask, base) in [(*rhs, *lhs), (*lhs, *rhs)] {
                chain.clear();
                let index = if effect == Effect::Clear {
                    complement_of_one_hot(mask, bits, defs, &mut chain)
                } else {
                    one_hot_at(mask, bits, defs, &mut chain)
                };
                if let Some(index) = index {
                    if same_base(base, cond.base, defs) && same_index(index, cond.index, defs) {
                        return Some(effect);
                    }
                }
            }
            return None;
        }
        _ => return None,
    };
    (same_base(*lhs, cond.base, defs) && same_index(*rhs, cond.index, defs)).then_some(effect)
}

/// The replacement for a choice between `when_set` (the arm taken when the
/// bit is set) and `when_clear`, or `None` when the arms are not both `x` or
/// bit operations of `x`.
fn resolve_choice(
    cond: &BitCond,
    when_set: Operand,
    when_clear: Operand,
    defs: &[Option<Instruction>],
) -> Option<(Effect, Option<Operand>)> {
    let set_arm = arm_effect(when_set, cond, defs)?;
    let clear_arm = arm_effect(when_clear, cond, defs)?;
    let effect = Effect::from_table(clear_arm.apply(false), set_arm.apply(true));
    // Reuse an arm that already computes the answer for every input bit.
    let reuse = [(when_set, set_arm), (when_clear, clear_arm)]
        .into_iter()
        .find(|(_, e)| *e == effect)
        .map(|(arm, _)| arm);
    Some((effect, reuse))
}

fn choice_instruction(
    dest: Value,
    cond: &BitCond,
    effect: Effect,
    reuse: Option<Operand>,
    ty: IrType,
) -> Option<Instruction> {
    if let Some(arm) = reuse {
        return Some(Instruction::Copy { dest, src: arm });
    }
    let op = effect.op()?;
    if !creatable(ty) {
        return None;
    }
    Some(Instruction::BinOp {
        dest,
        op,
        lhs: cond.base,
        rhs: cond.index,
        ty,
    })
}

/// `Select(test of bit n of x, A, B)` with `A`, `B` each `x` or a bit
/// operation of bit `n` of `x` -> that single operation (or `x`).
fn fold_bit_test_selects(func: &mut IrFunction, t: &Tables) -> usize {
    let mut changes = 0;
    for block in &mut func.blocks {
        for inst in &mut block.instructions {
            let Instruction::Select {
                dest,
                cond,
                true_val,
                false_val,
                ty,
            } = inst
            else {
                continue;
            };
            let Some(bc) = bit_cond(*cond, &t.defs) else {
                continue;
            };
            if !ty.is_integer() || ty.size() * 8 != bc.bits {
                continue;
            }
            let (when_set, when_clear) = if bc.set_is_true {
                (*true_val, *false_val)
            } else {
                (*false_val, *true_val)
            };
            let Some((effect, reuse)) = resolve_choice(&bc, when_set, when_clear, &t.defs) else {
                continue;
            };
            let reuse = if effect == Effect::Keep {
                Some(bc.base)
            } else {
                reuse
            };
            if let Some(new) = choice_instruction(*dest, &bc, effect, reuse, *ty) {
                *inst = new;
                changes += 1;
            }
        }
    }
    changes
}

/// The two-way join form of [`fold_bit_test_selects`]: a `Phi` in `J` whose
/// two predecessors are the arms of `B0`'s branch on a test of bit `n` of
/// `x` (a triangle `B0 -> P -> J`, `B0 -> J`, or a diamond).  The phi is
/// replaced by the single bit operation computed in `B0`, just before the
/// branch condition's own computation.  Every arm block must have `B0` as its
/// only predecessor and branch only to `J`, and `J` must have exactly those
/// two predecessors, so the incoming value of each edge is known to be taken
/// under one answer of the test.  The arm blocks keep their instructions;
/// only the phi's value is recomputed.
fn fold_bit_test_phis(func: &mut IrFunction, t: &Tables) -> usize {
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();
    let (preds, _succs) = crate::ir::analysis::build_cfg(func, &label_to_idx);
    // (join block, phi index, B0, instruction to insert in B0)
    let mut plans: Vec<(usize, usize, usize, Instruction)> = Vec::new();
    for (ji, join) in func.blocks.iter().enumerate() {
        let jp = preds.row(ji);
        if jp.len() != 2 || jp[0] == jp[1] {
            continue;
        }
        for (pi, inst) in join.instructions.iter().enumerate() {
            let Instruction::Phi { dest, ty, incoming } = inst else {
                continue;
            };
            if incoming.len() != 2 {
                continue;
            }
            let Some((b0, when_true, when_false)) =
                phi_edges(func, &preds, &label_to_idx, ji, incoming)
            else {
                continue;
            };
            let Terminator::CondBranch { cond, .. } = &func.blocks[b0].terminator else {
                continue;
            };
            let Some(bc) = bit_cond(*cond, &t.defs) else {
                continue;
            };
            if !ty.is_integer() || ty.size() * 8 != bc.bits {
                continue;
            }
            let (when_set, when_clear) = if bc.set_is_true {
                (when_true, when_false)
            } else {
                (when_false, when_true)
            };
            let Some((effect, _)) = resolve_choice(&bc, when_set, when_clear, &t.defs) else {
                continue;
            };
            // The arms live in blocks that do not dominate the join, so the
            // answer is recomputed in `B0` from the test's own operands (which
            // the branch condition already reads there).
            let reuse = (effect == Effect::Keep).then_some(bc.base);
            let Some(new) = choice_instruction(*dest, &bc, effect, reuse, *ty) else {
                continue;
            };
            plans.push((ji, pi, b0, new));
        }
    }
    if plans.is_empty() {
        return 0;
    }
    let changes = plans.len();
    // Remove the phis (highest index first per block), then insert into B0.
    let mut removals: Vec<(usize, usize)> = plans.iter().map(|p| (p.0, p.1)).collect();
    removals.sort_by(|a, b| b.cmp(a));
    for (ji, pi) in removals {
        func.blocks[ji].remove_inst(pi);
    }
    for (_, _, b0, new) in plans {
        let pos = insertion_point_before_condition(&func.blocks[b0], &new);
        // A synthesized value with no source construct of its own.
        func.blocks[b0].insert_inst(pos, new, Span::dummy());
    }
    changes
}

/// For a two-incoming phi in block `ji`, find the branch block `B0` and the
/// incoming values taken when its condition is true / false.
fn phi_edges(
    func: &IrFunction,
    preds: &crate::ir::analysis::FlatAdj,
    label_to_idx: &FxHashMap<BlockId, usize>,
    ji: usize,
    incoming: &[(Operand, BlockId)],
) -> Option<(usize, Operand, Operand)> {
    let (a0, l0) = incoming[0];
    let (a1, l1) = incoming[1];
    let p0 = *label_to_idx.get(&l0)?;
    let p1 = *label_to_idx.get(&l1)?;
    if p0 == p1 {
        return None;
    }
    // An arm block: a single predecessor and an unconditional branch to the
    // join; returns that predecessor.
    let arm_of = |p: usize| -> Option<usize> {
        let pp = preds.row(p);
        if pp.len() != 1 {
            return None;
        }
        match func.blocks[p].terminator {
            Terminator::Branch(l) if label_to_idx.get(&l) == Some(&ji) => Some(pp[0] as usize),
            _ => None,
        }
    };
    let branch_targets = |b0: usize| -> Option<(usize, usize)> {
        match func.blocks[b0].terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                ..
            } => {
                let t = *label_to_idx.get(&true_label)?;
                let f = *label_to_idx.get(&false_label)?;
                (t != f).then_some((t, f))
            }
            _ => None,
        }
    };
    // Diamond: both predecessors are arms of the same B0.
    if let (Some(b0), Some(b0b)) = (arm_of(p0), arm_of(p1)) {
        if b0 == b0b && b0 != ji {
            let (t, f) = branch_targets(b0)?;
            return if t == p0 && f == p1 {
                Some((b0, a0, a1))
            } else if t == p1 && f == p0 {
                Some((b0, a1, a0))
            } else {
                None
            };
        }
    }
    // Triangle: one predecessor is B0 itself, the other an arm of it.
    for (b0, arm, a_b0, a_arm) in [(p0, p1, a0, a1), (p1, p0, a1, a0)] {
        if b0 == ji || arm_of(arm) != Some(b0) {
            continue;
        }
        let Some((t, f)) = branch_targets(b0) else {
            continue;
        };
        if t == arm && f == ji {
            return Some((b0, a_arm, a_b0));
        }
        if f == arm && t == ji {
            return Some((b0, a_b0, a_arm));
        }
    }
    None
}

/// Where to insert `inst` in `block` (the branch block of a join): before the
/// tail that computes the branch condition -- backends fuse a compare or bit
/// test into the branch only when it is adjacent to it -- but never before a
/// definition `inst` reads.
fn insertion_point_before_condition(block: &BasicBlock, inst: &Instruction) -> usize {
    let n = block.instructions.len();
    let mut pos = n;
    if let Terminator::CondBranch {
        cond: Operand::Value(c),
        ..
    } = block.terminator
    {
        // Walk back over the condition's own chain while it is the tail of
        // the block: copies, casts, the compare and the bit test or AND.
        let mut wanted: FxHashSet<u32> = FxHashSet::default();
        wanted.insert(c.0);
        while pos > 0 {
            let tail = &block.instructions[pos - 1];
            let in_chain = matches!(
                tail,
                Instruction::Copy { .. }
                    | Instruction::Cast { .. }
                    | Instruction::Cmp { .. }
                    | Instruction::BinOp {
                        op: IrBinOp::BitTest | IrBinOp::And,
                        ..
                    }
            );
            match tail.dest() {
                Some(dest) if in_chain && wanted.contains(&dest.0) => {
                    crate::backend::liveness::for_each_operand_in_instruction(tail, |op| {
                        if let Operand::Value(v) = op {
                            wanted.insert(v.0);
                        }
                    });
                    pos -= 1;
                }
                _ => break,
            }
        }
    }
    // Never above a definition the new instruction reads.
    let mut reads: FxHashSet<u32> = FxHashSet::default();
    crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
        if let Operand::Value(v) = op {
            reads.insert(v.0);
        }
    });
    for (i, existing) in block.instructions.iter().enumerate() {
        if let Some(dest) = existing.dest() {
            if reads.contains(&dest.0) && i + 1 > pos {
                pos = i + 1;
            }
        }
    }
    pos
}

/// The function-level bit rewrites that run before `recognize_function`'s
/// single-instruction scan: shared one-hot masks first (x86/i686: every
/// reader of a multi-use mask becomes a bit instruction), then the
/// all-reader test rewrites below (every target).
pub(super) fn fold_shared_masks_then_tests(func: &mut IrFunction) -> usize {
    super::bit_select::fold_shared_bit_select(func) + fold_bit_tests(func)
}

/// Entry point for the function-level test rewrites (all targets).
pub(super) fn fold_bit_tests(func: &mut IrFunction) -> usize {
    let has_and = func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::And,
                    ..
                }
            )
        })
    });
    let mut changes = if has_and {
        zero_tests_to_bit_tests(func)
    } else {
        0
    };
    changes += msb_first_modifies(func);
    let worth_a_table = func.blocks.iter().any(|b| {
        matches!(b.terminator, Terminator::CondBranch { .. })
            || b.instructions.iter().any(|i| {
                matches!(
                    i,
                    Instruction::Select { .. }
                        | Instruction::BinOp {
                            op: IrBinOp::BitSet
                                | IrBinOp::BitClear
                                | IrBinOp::BitToggle
                                | IrBinOp::BitTest,
                            ..
                        }
                )
            })
    });
    if !worth_a_table {
        return changes;
    }
    let Some(t) = Tables::build(func) else {
        return changes;
    };
    // Branch retargeting edits terminators only, the index rewrite edits
    // bit-operation indices only and the select rewrite replaces selects in
    // place, so one table serves all of them (a stale entry can only hide a
    // match, never invent one).
    changes += retarget_bit_test_bases(func, &t);
    changes += strip_shared_index_masks(func, &t);
    changes += canonicalize_bit_test_branches(func, &t);
    changes += fold_bit_test_selects(func, &t);
    changes += fold_bit_test_phis(func, &t);
    changes
}

// ── x86 test + modify pairing ────────────────────────────────────────────

/// Whether `inst` modifies bit `index` of `base` at `width` bytes:
/// `BitSet`/`BitClear`/`BitToggle` with exactly these operands.
fn modify_partner(inst: &Instruction, base: Operand, index: Operand, width: usize) -> bool {
    matches!(
        inst,
        Instruction::BinOp {
            op: IrBinOp::BitSet | IrBinOp::BitClear | IrBinOp::BitToggle,
            lhs,
            rhs,
            ty,
            ..
        } if *lhs == base && *rhs == index && ty.size() == width
    )
}

/// x86 `bts`/`btr`/`btc` leave the OLD value of the bit in CF, which is
/// exactly `BitTest` of the same operands.  The backend fuses a `BitTest`
/// immediately followed by such a modify into one instruction
/// (`generation.rs`, `detect_bit_test_modify_fusions`): `btr %esi,%edi;
/// setc %al` instead of `bt; setc; ...; btr` (Clang 23.1.4's `ff_test_clear`
/// shape, which GCC 16.2 misses), or `btr %esi,%edi; jnc` for a branch.
///
/// This pass only places the pair: it moves the modify (or the test) so the
/// two are adjacent, test first, and for a test that feeds the block's branch
/// puts both last.  A modify may also be pulled out of the branch's
/// single-predecessor successor or out of the join the branch closes -- it
/// is pure and its operands are the test's, so it is valid there and costs
/// at most one register copy on the path that did not need it.  Runs right
/// before instruction selection (after every IR pass that could reorder or
/// sink instructions); AArch64 and RISC-V have no flags result to share.
pub(crate) fn schedule_bit_test_modify_pairs(func: &mut IrFunction) -> usize {
    use crate::common::types::target_arch_name;
    let wide_ok = match target_arch_name() {
        "x86-64" => true,
        "i686" => false,
        _ => return 0,
    };
    if super::facts::bit_select_disabled() || func.blocks.is_empty() {
        return 0;
    }
    let has_test = func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::BinOp {
                    op: IrBinOp::BitTest,
                    ..
                }
            )
        })
    });
    if !has_test {
        return 0;
    }
    let label_to_idx: FxHashMap<BlockId, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label, i))
        .collect();
    let (preds, succs) = crate::ir::analysis::build_cfg(func, &label_to_idx);
    let use_counts = value_use_counts(func);
    let mut changes = 0;
    for bi in 0..func.blocks.len() {
        changes += schedule_block(func, bi, &preds, &succs, &use_counts, wide_ok);
    }
    changes
}

fn schedule_block(
    func: &mut IrFunction,
    bi: usize,
    preds: &crate::ir::analysis::FlatAdj,
    succs: &crate::ir::analysis::FlatAdj,
    use_counts: &[u32],
    wide_ok: bool,
) -> usize {
    let tests: Vec<(Value, Operand, Operand, usize)> = func.blocks[bi]
        .instructions
        .iter()
        .filter_map(|inst| match inst {
            Instruction::BinOp {
                dest,
                op: IrBinOp::BitTest,
                lhs,
                rhs,
                ty,
            } if matches!(ty, IrType::I32 | IrType::U32)
                || (wide_ok && matches!(ty, IrType::I64 | IrType::U64)) =>
            {
                Some((*dest, *lhs, *rhs, ty.size()))
            }
            _ => None,
        })
        .collect();
    let mut changes = 0;
    let mut paired: FxHashSet<u32> = FxHashSet::default();
    for (test, base, index, width) in tests {
        if base == index || matches!(base, Operand::Const(_)) {
            continue;
        }
        let block = &func.blocks[bi];
        let Some(tpos) = block
            .instructions
            .iter()
            .position(|i| i.dest() == Some(test))
        else {
            continue;
        };
        let branch_form = matches!(
            block.terminator,
            Terminator::CondBranch { cond: Operand::Value(c), .. } if c == test
        ) && use_counts.get(test.0 as usize) == Some(&1);
        // The LAST modify of the bit pairs: it is the likeliest final reader
        // of `x`, which lets the backend modify `x`'s register in place.
        let local = block
            .instructions
            .iter()
            .enumerate()
            .rev()
            .find_map(|(p, i)| match i.dest() {
                Some(d) if modify_partner(i, base, index, width) && !paired.contains(&d.0) => {
                    Some((p, d))
                }
                _ => None,
            });
        if let Some((ppos, partner)) = local {
            if branch_form {
                // [.., test, partner] + branch: the partner moves to the end,
                // so nothing after it in this block may read it.
                let read_later = block.instructions[ppos + 1..].iter().any(|i| {
                    let mut hit = false;
                    crate::backend::liveness::for_each_operand_in_instruction(i, |op| {
                        hit |= matches!(op, Operand::Value(v) if *v == partner);
                    });
                    hit
                });
                if read_later {
                    continue;
                }
                let blk = &mut func.blocks[bi];
                let p = take(blk, ppos);
                // Removing the partner shifts the test down iff it came after.
                let t = take(blk, if tpos > ppos { tpos - 1 } else { tpos });
                put(blk, blk.instructions.len(), t);
                put(blk, blk.instructions.len(), p);
            } else {
                let blk = &mut func.blocks[bi];
                let read_between = ppos > tpos
                    && blk.instructions[tpos + 1..ppos].iter().any(|i| {
                        let mut hit = false;
                        crate::backend::liveness::for_each_operand_in_instruction(i, |op| {
                            hit |= matches!(op, Operand::Value(v) if *v == test);
                        });
                        hit
                    });
                if ppos > tpos && read_between {
                    // The modify moves up to just after the test: its
                    // operands are the test's, defined before it.
                    let p = take(blk, ppos);
                    put(blk, tpos + 1, p);
                } else if ppos > tpos {
                    // Nothing reads the test before the modify: sink the
                    // test instead, which shortens its live range rather
                    // than lengthening the modify's.
                    let t = take(blk, tpos);
                    put(blk, ppos - 1, t);
                } else {
                    // The test moves up to just before the modify: its
                    // operands are the modify's, defined earlier still.
                    let t = take(blk, tpos);
                    put(blk, ppos, t);
                }
            }
            paired.insert(partner.0);
            changes += 1;
            continue;
        }
        if !branch_form {
            continue;
        }
        let Some((src, spos, partner)) =
            hoistable_partner(func, bi, preds, succs, base, index, width)
        else {
            continue;
        };
        // `src` is never `bi`, so the test's position is unchanged.
        let moved = take(&mut func.blocks[src], spos);
        let blk = &mut func.blocks[bi];
        let t = take(blk, tpos);
        put(blk, blk.instructions.len(), t);
        put(blk, blk.instructions.len(), moved);
        paired.insert(partner.0);
        changes += 1;
    }
    changes
}

/// A modify of the same bit in a block the test's branch always or
/// exclusively leads to: a single-predecessor successor of `bi`, or the join
/// block of a triangle/diamond rooted at `bi` (which every path from `bi`
/// reaches).  Returns (block, index, the modify's dest).
fn hoistable_partner(
    func: &IrFunction,
    bi: usize,
    preds: &crate::ir::analysis::FlatAdj,
    succs: &crate::ir::analysis::FlatAdj,
    base: Operand,
    index: Operand,
    width: usize,
) -> Option<(usize, usize, Value)> {
    let out = succs.row(bi);
    if out.len() != 2 {
        return None;
    }
    let mut candidates: Vec<usize> = Vec::new();
    for &s in out {
        let s = s as usize;
        if s != bi && preds.row(s) == [bi as u32] {
            candidates.push(s);
        }
    }
    // The join: reached from both successors (diamond) or from `bi` and the
    // other successor (triangle), with no other predecessor.
    let (s0, s1) = (out[0] as usize, out[1] as usize);
    let single_exit = |b: usize| -> Option<usize> {
        let o = succs.row(b);
        (o.len() == 1 && preds.row(b) == [bi as u32]).then(|| o[0] as usize)
    };
    let join = match (single_exit(s0), single_exit(s1)) {
        (Some(j0), Some(j1)) if j0 == j1 => Some((j0, [s0, s1])),
        (Some(j0), _) if j0 == s1 => Some((j0, [bi, s0])),
        (_, Some(j1)) if j1 == s0 => Some((j1, [bi, s1])),
        _ => None,
    };
    if let Some((j, arms)) = join {
        let mut jp: Vec<u32> = preds.row(j).to_vec();
        jp.sort_unstable();
        let mut want = vec![arms[0] as u32, arms[1] as u32];
        want.sort_unstable();
        if j != bi && jp == want {
            candidates.push(j);
        }
    }
    candidates.into_iter().find_map(|b| {
        func.blocks[b]
            .instructions
            .iter()
            .enumerate()
            .find_map(|(p, i)| match i.dest() {
                Some(d) if modify_partner(i, base, index, width) => Some((b, p, d)),
                _ => None,
            })
    })
}

/// Reads of each value: operands, address uses and terminator operands.
fn value_use_counts(func: &IrFunction) -> Vec<u32> {
    let mut counts = vec![0u32; func.max_value_id() as usize + 1];
    let mut bump = |id: u32| {
        if let Some(c) = counts.get_mut(id as usize) {
            *c = c.saturating_add(1);
        }
    };
    for block in &func.blocks {
        for inst in &block.instructions {
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    bump(v.0);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| bump(v.0));
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                bump(v.0);
            }
        });
    }
    counts
}

fn take(block: &mut BasicBlock, at: usize) -> (Instruction, Span) {
    let span = block.span_at(at);
    (block.remove_inst(at), span)
}

fn put(block: &mut BasicBlock, at: usize, (inst, span): (Instruction, Span)) {
    block.insert_inst(at, inst, span);
}

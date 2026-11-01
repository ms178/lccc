//! Single-bit selection: `x | 1 << n`, `x & ~(1 << n)`, `x ^ 1 << n` and
//! `(x & 1 << n) != 0`.
//!
//! These four shapes are how portable C spells "set bit n", "clear bit n",
//! "toggle bit n" and "test bit n".  Spelled out they cost a materialized
//! constant, a variable
//! shift pinned to `%cl` and the logic op; every ISA this compiler targets has
//! a one- or two-instruction form of the *whole* idiom instead (x86
//! `bts`/`btr`/`btc`/`bt`, AArch64 `orr`/`bic`/`eor` with an `lsl`, RISC-V
//! `or`/`and`/`xor` against a shifted one).  The IR forms are
//! [`IrBinOp::BitSet`], [`IrBinOp::BitClear`], [`IrBinOp::BitToggle`] and the
//! already-existing [`IrBinOp::BitTest`], whose `index mod W` contract all
//! four share.
//!
//! Every rewrite here replaces only the instruction it matches and leaves the
//! producers (`1 << n`, and the `~` above it) to DCE, which is why each one
//! requires those producers to be single-use.  Nothing repurposes an earlier
//! instruction, so the fold needs no coherence bookkeeping: `defs` and
//! `use_counts` are read, never written.
//!
//! What is deliberately *not* matched, because the IR cannot express it more
//! cheaply than the shape already is:
//!
//! * `(x & 1 << n) == 0`, the equality direction of the test.  `BitTest` is
//!   defined as a 0/1 boolean, so the negated value needs a second instruction
//!   (`xor $1`) that the `setnc` GCC and Clang emit does not; folding the
//!   producer would still leave the compare.  The inequality direction folds
//!   because `BitTest`'s 0/1 result *is* the answer.
//! * `x ^ 1u << n` with the shifted operand used twice, and the seven other
//!   spellings the controls in `check_bit_select_fold.sh` pin.  The single-use
//!   rule is what keeps a toggle from duplicating the shift, exactly as it does
//!   for `BitSet`/`BitClear`.

use super::*;

/// The width an IR bit operation may be created at.  The 128-bit pair type has
/// no lowering, and on a 32-bit target neither does a 64-bit operation (its
/// "native" width is a register pair the bit emitters do not handle) -- the
/// same legality rule `simplify`'s `BitTest` producer applies, kept here in one
/// place for the four shapes below.
///
/// The backends also lower 8/16-bit bit operations (the C frontend never asks
/// for one, because the integer promotions widen both operands to `int`, but
/// the IR contract allows them and the ARM/RISC-V narrow paths are tested
/// directly).  This recognizer only creates what its own shapes produce, so the
/// narrow widths are simply out of reach here.
fn native_bit_width(ty: IrType) -> bool {
    ty.is_integer()
        && !ty.is_128bit()
        && (ty.size() <= 4 || !crate::common::types::target_is_32bit())
}

/// `1 << n` as an operand: the shift both the bit-set and the bit-test idioms
/// read their index from.  Returns the index operand and the shift's width.
fn shift_of_one(op: Operand, defs: &[Option<Instruction>]) -> Option<(Operand, IrType)> {
    let Operand::Value(v) = peel_copies(op, defs) else {
        return None;
    };
    let Some(Instruction::BinOp {
        op: IrBinOp::Shl,
        lhs,
        rhs,
        ty,
        ..
    }) = defs.get(v.0 as usize).and_then(Option::as_ref)
    else {
        return None;
    };
    if !ty.is_integer() || ty.size() == 0 || ty.size() > 8 {
        return None;
    }
    let bits = ty.size() * 8;
    if const_in_ring(*lhs, bits) != Some(1) {
        return None;
    }
    // The count is read through integer casts, because `index mod W` survives
    // any of them: a narrowing cast drops bits that are multiples of 2^8 and a
    // sign-extending one adds 2^(8k-1), and every W these operations are
    // created at (8, 16, 32, 64) divides those.  The cast is usually the C
    // frontend's count-to-int conversion on a `1ul << n`, and reading through
    // it lets the widened copy the backend would otherwise materialize die
    // with the shift (`movslq %esi, %rdx` on every 64-bit bit-set).
    Some((peel(*rhs, defs), *ty))
}

/// `~(1 << n)` as an operand, plus the shift operand underneath it so the
/// caller can require that the shift dies with the complement.  Both spellings
/// of the complement are accepted: `UnaryOp::Not` (what the frontend emits for
/// `~`) and `Xor` with an all-ones constant (what reassociation can leave).
fn not_of_shift_of_one(
    op: Operand,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, IrType)> {
    let Operand::Value(v) = peel_copies(op, defs) else {
        return None;
    };
    let shifted = match defs.get(v.0 as usize).and_then(Option::as_ref)? {
        Instruction::UnaryOp {
            op: IrUnaryOp::Not,
            src,
            ty,
            ..
        } if ty.is_integer() => (*src, *ty),
        Instruction::BinOp {
            op: IrBinOp::Xor,
            lhs,
            rhs,
            ty,
            ..
        } if ty.is_integer() && ty.size() > 0 && ty.size() <= 8 => {
            let bits = ty.size() * 8;
            let all_ones = u64::MAX >> (64 - bits);
            if const_in_ring(*rhs, bits) == Some(all_ones) {
                (*lhs, *ty)
            } else if const_in_ring(*lhs, bits) == Some(all_ones) {
                (*rhs, *ty)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let (index, shift_ty) = shift_of_one(shifted.0, defs)?;
    if shift_ty != shifted.1 {
        return None;
    }
    Some((index, shifted.0, shift_ty))
}

/// The inner base of `(x ^ m) ^ m`, i.e. `x` when `base` is a `BitToggle` (or
/// the pre-fold `Xor`) of the same index operand.  Used to keep the toggle's
/// involution from being *worse* than the shape it replaces: the two masks in
/// `x ^ (1u << n) ^ (1u << n)` are the same CSE'd value, so the outer XOR here
/// is `BitToggle(BitToggle(x, n), n)` and its answer is `x`.
fn toggle_involution(
    base: Operand,
    index: Operand,
    defs: &[Option<Instruction>],
) -> Option<Operand> {
    let Operand::Value(v) = peel_copies(base, defs) else {
        return None;
    };
    let (inner_base, inner_index) = match defs.get(v.0 as usize).and_then(Option::as_ref)? {
        Instruction::BinOp {
            op: IrBinOp::BitToggle,
            lhs,
            rhs,
            ..
        } => (*lhs, *rhs),
        _ => return None,
    };
    (peel_copies(inner_index, defs) == peel_copies(index, defs)).then_some(inner_base)
}

/// Whether an operand's producer is used exactly once, i.e. dies when the
/// matched instruction stops reading it.  `uses` is the running count at this
/// point of the scan, so a use by an *earlier* instruction is already gone.
fn single_use(op: Operand, defs: &[Option<Instruction>], uses: &[u32]) -> bool {
    matches!(peel_copies(op, defs), Operand::Value(v) if uses.get(v.0 as usize) == Some(&1))
}

/// Pick the shifted side of a two-operand instruction, returning the value
/// operand (`base`) and the shift's index.  Both operand orders are accepted;
/// a shape where *both* sides are `1 << n` is left alone, because either
/// rewrite would then have to duplicate the other shift.
fn split_shift(
    lhs: Operand,
    rhs: Operand,
    defs: &[Option<Instruction>],
) -> Option<(Operand, Operand, Operand, IrType)> {
    match (shift_of_one(lhs, defs), shift_of_one(rhs, defs)) {
        (Some((index, ty)), None) => Some((lhs, rhs, index, ty)),
        (None, Some((index, ty))) => Some((rhs, lhs, index, ty)),
        _ => None,
    }
}

pub(super) fn fold_bit_select(
    inst: &Instruction,
    defs: &[Option<Instruction>],
    uses: &[u32],
) -> Option<Instruction> {
    match inst {
        // `x | (1 << n)` -> `BitSet(x, n)`
        Instruction::BinOp {
            dest,
            op: IrBinOp::Or,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let (shifted, base, index, shift_ty) = split_shift(*lhs, *rhs, defs)?;
            if shift_ty != *ty || !single_use(shifted, defs, uses) {
                return None;
            }
            Some(Instruction::BinOp {
                dest: *dest,
                op: IrBinOp::BitSet,
                lhs: base,
                rhs: index,
                ty: *ty,
            })
        }
        // `x & ~(1 << n)` -> `BitClear(x, n)`
        Instruction::BinOp {
            dest,
            op: IrBinOp::And,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let (complement, base) = match (
                not_of_shift_of_one(*lhs, defs),
                not_of_shift_of_one(*rhs, defs),
            ) {
                (Some(_), None) => (*lhs, *rhs),
                (None, Some(_)) => (*rhs, *lhs),
                _ => return None,
            };
            let (index, shifted, complement_ty) = not_of_shift_of_one(complement, defs)?;
            if complement_ty != *ty
                || !single_use(complement, defs, uses)
                || !single_use(shifted, defs, uses)
            {
                return None;
            }
            Some(Instruction::BinOp {
                dest: *dest,
                op: IrBinOp::BitClear,
                lhs: base,
                rhs: index,
                ty: *ty,
            })
        }
        // `x ^ (1 << n)` -> `BitToggle(x, n)`
        //
        // The third member of the family: `btc` on x86, `eor` behind an `lsl`
        // elsewhere.  Accepted in both operand orders like `BitSet`, and the
        // shift must die here too.
        Instruction::BinOp {
            dest,
            op: IrBinOp::Xor,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let (shifted, base, index, shift_ty) = split_shift(*lhs, *rhs, defs)?;
            if shift_ty != *ty || !single_use(shifted, defs, uses) {
                return None;
            }
            // `(x ^ m) ^ m` is `x`, and the toggle is its own inverse.  The
            // earlier XOR of that pair has usually been folded to a `BitToggle`
            // by the time this one is reached, and the pass that would cancel
            // `a ^ a` sees two different operations afterwards -- so folding
            // BOTH would turn a 2-instruction identity into two `btc`s
            // (`x ^ (1u << n) ^ (1u << n)` measured 2 before this fold and 5
            // after).  Catch it here instead: the result is a plain copy of the
            // inner base, which DCE turns into the identity.
            if let Some(inner_base) = toggle_involution(base, index, defs) {
                return Some(Instruction::Copy {
                    dest: *dest,
                    src: inner_base,
                });
            }
            Some(Instruction::BinOp {
                dest: *dest,
                op: IrBinOp::BitToggle,
                lhs: base,
                rhs: index,
                ty: *ty,
            })
        }
        // `(x & (1 << n)) != 0` -> `BitTest(x, n)`
        //
        // The whole comparison is replaced: `BitTest`'s value is the 0/1 the
        // comparison computes, so nothing downstream changes type or carrier.
        Instruction::Cmp {
            dest,
            op: IrCmpOp::Ne,
            lhs,
            rhs,
            ty,
        } if native_bit_width(*ty) => {
            let bits = ty.size() * 8;
            let tested = if const_in_ring(*rhs, bits) == Some(0) {
                *lhs
            } else if const_in_ring(*lhs, bits) == Some(0) {
                *rhs
            } else {
                return None;
            };
            let Operand::Value(v) = peel_copies(tested, defs) else {
                return None;
            };
            let Some(Instruction::BinOp {
                op: IrBinOp::And,
                lhs: and_lhs,
                rhs: and_rhs,
                ty: and_ty,
                ..
            }) = defs.get(v.0 as usize).and_then(Option::as_ref)
            else {
                return None;
            };
            if *and_ty != *ty || !single_use(tested, defs, uses) {
                return None;
            }
            let (shifted, base, index, shift_ty) = split_shift(*and_lhs, *and_rhs, defs)?;
            if shift_ty != *ty || !single_use(shifted, defs, uses) {
                return None;
            }
            Some(Instruction::BinOp {
                dest: *dest,
                op: IrBinOp::BitTest,
                lhs: base,
                rhs: index,
                ty: *ty,
            })
        }
        _ => None,
    }
}

/// Target-aware shared-mask selection.
///
/// The single-use matcher above deliberately refuses a one-hot mask that has
/// several readers: rewriting just one reader can leave the mask live and turn
/// a cheap logic op into a bit instruction without a net win.  On x86, however,
/// `bts`/`btr`/`btc`/`bt` consume the index directly.  If *every* reader of the
/// mask (through copies, complements, and test-ands) is a member of this family,
/// the whole group can be rewritten atomically: the mask and its internal
/// producers die, and no reader is left paying for a materialized mask.
///
/// This is intentionally not done on AArch64/RISC-V: their lowerings rebuild a
/// one-hot mask for each bit operation, so sharing the mask is the better shape.
/// The pass is all-or-nothing per mask.  One escaping store, call, phi, inline
/// asm, or non-bit reader refuses the entire group rather than duplicating work.
pub(super) fn fold_shared_bit_select(func: &mut IrFunction) -> usize {
    use crate::common::types::target_arch_name;

    if !matches!(target_arch_name(), "x86-64" | "i686") {
        return 0;
    }
    let max_id = func.max_value_id();
    if func.next_value_id == u32::MAX || max_id > 1_000_000 {
        return 0;
    }
    let len = max_id as usize + 1;
    let mut def_loc = vec![None; len];
    let mut ambiguous = vec![false; len];
    let mut uses = vec![Vec::new(); len];
    let mut candidates = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, inst) in block.instructions.iter().enumerate() {
            let loc = SharedLoc {
                block: bi,
                inst: Some(ii),
            };
            if let Some(dest) = inst.dest() {
                let id = dest.0 as usize;
                if def_loc[id].is_some() {
                    ambiguous[id] = true;
                } else {
                    def_loc[id] = Some(loc);
                }
            }
            crate::backend::liveness::for_each_operand_in_instruction(inst, |op| {
                if let Operand::Value(v) = op {
                    uses[v.0 as usize].push(loc);
                }
            });
            crate::backend::liveness::for_each_value_use_in_instruction(inst, |v| {
                uses[v.0 as usize].push(loc);
            });
            if let Instruction::BinOp {
                dest,
                op: IrBinOp::Shl,
                lhs,
                rhs,
                ty,
            } = inst
            {
                if native_bit_width(*ty)
                    && const_in_ring(*lhs, ty.size() * 8) == Some(1)
                    && !ambiguous[dest.0 as usize]
                {
                    let index = forward_shared_index(*rhs, func, &def_loc);
                    candidates.push(SharedCandidate {
                        value: dest.0,
                        index,
                        ty: *ty,
                    });
                }
            }
        }
        crate::backend::liveness::for_each_operand_in_terminator(&block.terminator, |op| {
            if let Operand::Value(v) = op {
                uses[v.0 as usize].push(SharedLoc {
                    block: bi,
                    inst: None,
                });
            }
        });
    }

    let mut changes = 0;
    for cand in candidates {
        if ambiguous[cand.value as usize] {
            continue;
        }
        let Some(rewrites) = analyze_shared_candidate(func, &uses, &ambiguous, &cand) else {
            continue;
        };
        for (loc, inst) in rewrites {
            let Some(ii) = loc.inst else { continue };
            func.blocks[loc.block].instructions[ii] = inst;
            changes += 1;
        }
    }
    changes
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SharedLoc {
    block: usize,
    inst: Option<usize>,
}

#[derive(Clone, Copy)]
struct SharedCandidate {
    value: u32,
    index: Operand,
    ty: IrType,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SharedKind {
    Mask,
    Complement,
    TestAnd,
}

fn forward_shared_index(
    mut op: Operand,
    func: &IrFunction,
    def_loc: &[Option<SharedLoc>],
) -> Operand {
    for _ in 0..32 {
        let Operand::Value(v) = op else { break };
        let next = def_loc.get(v.0 as usize).and_then(|o| *o).and_then(|l| {
            let ii = l.inst?;
            match &func.blocks[l.block].instructions[ii] {
                Instruction::Cast {
                    src,
                    from_ty,
                    to_ty,
                    ..
                } if from_ty.is_integer() && to_ty.is_integer() => Some(*src),
                Instruction::Copy { src, .. } => Some(*src),
                _ => None,
            }
        });
        match next {
            Some(src) => op = src,
            None => break,
        }
    }
    op
}

fn shared_group_operand(op: Operand, kind: &[Option<SharedKind>]) -> bool {
    match op {
        Operand::Value(v) => kind.get(v.0 as usize).and_then(|k| *k).is_some(),
        Operand::Const(_) => false,
    }
}

/// Prove that every reader of `cand` is a bit-family consumer, and return the
/// consumer rewrites.  Internal copies/complements/test-ands are closed over;
/// any escaping or non-bit reader returns `None` for the whole group.
fn analyze_shared_candidate(
    func: &IrFunction,
    uses: &[Vec<SharedLoc>],
    ambiguous: &[bool],
    cand: &SharedCandidate,
) -> Option<Vec<(SharedLoc, Instruction)>> {
    use crate::common::fx_hash::FxHashMap;

    let len = ambiguous.len();
    let bits = cand.ty.size() * 8;
    let all_ones = u64::MAX >> (64 - bits);
    let mut kind = vec![None; len];
    let mut testand_base = vec![None; len];
    let mut queue = vec![cand.value];
    kind[cand.value as usize] = Some(SharedKind::Mask);
    let mut rewrites: FxHashMap<SharedLoc, Instruction> = FxHashMap::default();

    while let Some(v) = queue.pop() {
        let k = kind[v as usize]?;
        for &loc in &uses[v as usize] {
            let ii = loc.inst?;
            let inst = &func.blocks[loc.block].instructions[ii];
            if rewrites.contains_key(&loc) {
                return None;
            }
            macro_rules! enqueue {
                ($dest:expr, $new_kind:expr, $base:expr) => {{
                    let dest: Value = $dest;
                    let new_kind: SharedKind = $new_kind;
                    let base: Option<Operand> = $base;
                    let id = dest.0 as usize;
                    if ambiguous[id] {
                        return None;
                    }
                    match kind[id] {
                        Some(existing) if existing != new_kind => return None,
                        Some(_) => {}
                        None => {
                            kind[id] = Some(new_kind);
                            if new_kind == SharedKind::TestAnd {
                                testand_base[id] = base;
                            }
                            queue.push(dest.0);
                        }
                    }
                }};
            }
            match (k, inst) {
                (SharedKind::Mask, Instruction::Copy { dest, src }) if matches!(src, Operand::Value(x) if x.0 == v) =>
                {
                    enqueue!(*dest, SharedKind::Mask, None);
                }
                (SharedKind::Complement, Instruction::Copy { dest, src }) if matches!(src, Operand::Value(x) if x.0 == v) =>
                {
                    enqueue!(*dest, SharedKind::Complement, None);
                }
                (SharedKind::TestAnd, Instruction::Copy { dest, src }) if matches!(src, Operand::Value(x) if x.0 == v) =>
                {
                    let base = testand_base[v as usize];
                    enqueue!(*dest, SharedKind::TestAnd, base);
                }
                (
                    SharedKind::Mask,
                    Instruction::UnaryOp {
                        dest,
                        op: IrUnaryOp::Not,
                        src,
                        ty,
                    },
                ) if *ty == cand.ty && matches!(src, Operand::Value(x) if x.0 == v) => {
                    enqueue!(*dest, SharedKind::Complement, None);
                }
                (
                    SharedKind::Mask,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Xor,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if *ty == cand.ty => {
                    let other = if matches!(lhs, Operand::Value(x) if x.0 == v) {
                        rhs
                    } else if matches!(rhs, Operand::Value(x) if x.0 == v) {
                        lhs
                    } else {
                        return None;
                    };
                    if const_in_ring(*other, bits) == Some(all_ones) {
                        enqueue!(*dest, SharedKind::Complement, None);
                    } else if !shared_group_operand(*other, &kind) {
                        rewrites.insert(
                            loc,
                            Instruction::BinOp {
                                dest: *dest,
                                op: IrBinOp::BitToggle,
                                lhs: *other,
                                rhs: cand.index,
                                ty: cand.ty,
                            },
                        );
                    } else {
                        return None;
                    }
                }
                (
                    SharedKind::Mask,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::And,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if *ty == cand.ty => {
                    let other = if matches!(lhs, Operand::Value(x) if x.0 == v) {
                        rhs
                    } else if matches!(rhs, Operand::Value(x) if x.0 == v) {
                        lhs
                    } else {
                        return None;
                    };
                    if shared_group_operand(*other, &kind) {
                        return None;
                    }
                    enqueue!(*dest, SharedKind::TestAnd, Some(*other));
                }
                (
                    SharedKind::Mask,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::Or,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if *ty == cand.ty => {
                    let other = if matches!(lhs, Operand::Value(x) if x.0 == v) {
                        rhs
                    } else if matches!(rhs, Operand::Value(x) if x.0 == v) {
                        lhs
                    } else {
                        return None;
                    };
                    if shared_group_operand(*other, &kind) {
                        return None;
                    }
                    rewrites.insert(
                        loc,
                        Instruction::BinOp {
                            dest: *dest,
                            op: IrBinOp::BitSet,
                            lhs: *other,
                            rhs: cand.index,
                            ty: cand.ty,
                        },
                    );
                }
                (
                    SharedKind::Complement,
                    Instruction::BinOp {
                        dest,
                        op: IrBinOp::And,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if *ty == cand.ty => {
                    let other = if matches!(lhs, Operand::Value(x) if x.0 == v) {
                        rhs
                    } else if matches!(rhs, Operand::Value(x) if x.0 == v) {
                        lhs
                    } else {
                        return None;
                    };
                    if shared_group_operand(*other, &kind) {
                        return None;
                    }
                    rewrites.insert(
                        loc,
                        Instruction::BinOp {
                            dest: *dest,
                            op: IrBinOp::BitClear,
                            lhs: *other,
                            rhs: cand.index,
                            ty: cand.ty,
                        },
                    );
                }
                (
                    SharedKind::TestAnd,
                    Instruction::Cmp {
                        dest,
                        op: IrCmpOp::Ne,
                        lhs,
                        rhs,
                        ty,
                    },
                ) if *ty == cand.ty => {
                    let other = if matches!(lhs, Operand::Value(x) if x.0 == v) {
                        rhs
                    } else if matches!(rhs, Operand::Value(x) if x.0 == v) {
                        lhs
                    } else {
                        return None;
                    };
                    if const_in_ring(*other, bits) != Some(0) {
                        return None;
                    }
                    let base = testand_base[v as usize]?;
                    rewrites.insert(
                        loc,
                        Instruction::BinOp {
                            dest: *dest,
                            op: IrBinOp::BitTest,
                            lhs: base,
                            rhs: cand.index,
                            ty: cand.ty,
                        },
                    );
                }
                _ => return None,
            }
        }
    }
    if rewrites.len() < 2 {
        return None;
    }
    Some(rewrites.into_iter().collect())
}

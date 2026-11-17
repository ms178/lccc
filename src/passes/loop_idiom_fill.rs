//! Dual-bound buffer-fill widening: the streaming-update copy idiom
//!
//! ```c
//! while (src < src_end && H->p < buf_end)
//!     *H->p++ = *src++;
//! ```
//!
//! is the inner loop of every block-buffered streaming kernel — SipHash
//! (Expat's `sip24_update`), MD5/SHA-style `update` functions, stdio-like
//! buffer fills, ring buffers. The destination pointer lives in a memory
//! cell (`H->p`), there are TWO loop-invariant bounds, and the trip count is
//! `min(src_end - src, buf_end - dst)` — a shape the counted-copy idiom in
//! [`super::loop_idiom`] (one bound, SSA induction) can never match, and one
//! that GCC 16 and Clang 23 also leave as a byte-at-a-time loop (~7-9
//! instructions and two pointer-cell round-trips per byte).
//!
//! The rewrite versions the loop: a guard chain proves at runtime that an
//! 8-byte-chunk copy is observationally identical to the byte loop, a wide
//! loop moves whole chunks, and the ORIGINAL loop remains both the tail
//! (fewer than 8 bytes left on either bound) and the fail-closed fallback.
//! Nothing is deleted; every guard failure lands on the untouched byte loop.
//!
//! # Equivalence proof sketch
//!
//! Partition the byte loop's iterations into 8-byte blocks. The wide loop
//! executes block `k` as one load + one store, and enters it only when both
//! bounds admit 8 more bytes — exactly when the byte loop would copy those
//! same 8 bytes, so the wide form never touches memory the byte loop would
//! not. Byte and wide execution agree at every block boundary by induction
//! provided, within one block, no read observes a write of the same block
//! and no access touches the pointer cell:
//!
//! * **smear**: `d <= s` or `d - s >= 8` (unsigned). A forward byte copy and
//!   a chunk copy differ only when `0 < d - s < 8` (a store of byte `j`
//!   lands on the load of byte `i > j` within one block). `d - s` is
//!   constant — the pointers advance in lockstep — and overlap at distance
//!   `>= 8` smears identically in both forms, block by block.
//! * **cell-vs-dst**: stores span `[d0, de)` in both forms (every store is
//!   bound-checked first). The cell `[pd, pd + ptr_size)` must not
//!   intersect that span, or the byte loop rewrites its own induction
//!   pointer mid-copy. Discharged statically when `pd >= de` is a
//!   structural fact (Expat lays `buf[8]` directly before `p`, so
//!   `buf + 8 == &H->p`).
//! * **cell-vs-src**: reads span `[s0, se)`; a cell inside it would let the
//!   byte loop read back its own pointer-cell updates.
//!
//! The cell load is NOT speculated: the guards test `s < se` first, so the
//! fill-path cell access happens exactly when the original d-check block
//! would have performed it. The wide loop keeps the destination in a
//! register and publishes the cell once on exit; the byte loop reloads it.
//! On the Expat SipHash-2-4 workload this removes over half of all
//! dynamically executed instructions.
//!
//! # Scope (fail-closed)
//!
//! Exactly the three-block loop produced for the dual-bound source form:
//! header (one `Phi` for `src`, `Ult` bound test), d-check block (cell load,
//! `Ult` bound test, same exit target), latch (cell reload, cell bump store,
//! one U8 load from `src`, one U8 store to the loaded cell value,
//! `src + 1`, branch back). All accesses nonvolatile, default address
//! space; bounds and the cell address loop-invariant (`base + constant`
//! after peeling `Copy`/const-GEP chains); the preheader ends in a plain
//! `Branch`/`CondBranch`. Targets with valid unaligned 8-byte scalar access
//! only (x86-64, i686, AArch64 — the same gate as [`super::byte_load`]).
//!
//! Shares `CCC_NO_LOOP_IDIOM` / `CCC_DEBUG_LOOP_IDIOM` with the parent pass.

use crate::common::fx_hash::FxHashMap;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::analysis::{self, CfgAnalysis};
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, IrFunction, Operand, Terminator,
    Value,
};
use crate::passes::loop_analysis::NaturalLoop;

/// Chunk width in bytes. One general-purpose register on every admitted
/// target; `U64` accesses lower to `mov`/`ldr`/register pairs.
const CHUNK: i64 = 8;

/// Targets where an unaligned 8-byte scalar access is architecturally valid
/// and fast — the same set [`super::byte_load`] admits for widened loads.
pub(crate) fn target_admits_wide_fill(target: crate::backend::Target) -> bool {
    matches!(
        target,
        crate::backend::Target::X86_64
            | crate::backend::Target::I686
            | crate::backend::Target::Aarch64
    )
}

/// A loop-invariant pointer expression `base + offset`, the result of
/// peeling `Copy` chains and constant `GetElementPtr`s.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct PtrExpr {
    base: Value,
    offset: i64,
}

/// Everything the rewrite needs, gathered before any mutation.
struct FillLoop {
    header: usize,
    preheader: usize,
    header_label: BlockId,
    /// `src` induction: the preheader-edge value of the header phi.
    s_init: Operand,
    /// Loop-invariant source bound (`src_end`).
    se: Operand,
    /// The memory cell holding the destination pointer (`&H->p`).
    cell: PtrExpr,
    /// Loop-invariant destination bound (`buf_end`).
    de: PtrExpr,
    /// `cell >= de` holds structurally (same base, `cell.offset >=
    /// de.offset`): the cell can never lie inside the store region.
    cell_dst_static: bool,
}

/// Peel `Copy`, constant-offset `GetElementPtr` and pointer-width constant
/// `Add` chains (the fold of `buf + sizeof buf`) to a canonical
/// `base + offset`. Stops at phis, parameters and loads.
fn canon_ptr(defs: &FxHashMap<u32, &Instruction>, v: Value, ptr_ty: IrType) -> Option<PtrExpr> {
    let mut base = v;
    let mut offset: i64 = 0;
    for _ in 0..64 {
        match defs.get(&base.0) {
            Some(Instruction::Copy {
                src: Operand::Value(s),
                ..
            }) => base = *s,
            Some(Instruction::GetElementPtr {
                base: b,
                offset: Operand::Const(c),
                ..
            }) => {
                offset = offset.checked_add(c.to_i64()?)?;
                base = *b;
            }
            Some(Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(b),
                rhs: Operand::Const(c),
                ty,
                ..
            }) if *ty == ptr_ty => {
                offset = offset.checked_add(c.to_i64()?)?;
                base = *b;
            }
            _ => return Some(PtrExpr { base, offset }),
        }
    }
    None
}

fn resolve_copy(defs: &FxHashMap<u32, &Instruction>, mut v: Value) -> Value {
    for _ in 0..64 {
        match defs.get(&v.0) {
            Some(Instruction::Copy {
                src: Operand::Value(s),
                ..
            }) => v = *s,
            _ => break,
        }
    }
    v
}

/// The pointer-arithmetic integer type the frontend uses on this target
/// (`I64` on 64-bit, `I32` on i686) and its unsigned twin for guards.
fn ptr_types() -> Option<(IrType, IrType)> {
    match crate::common::types::target_int_ir_type() {
        IrType::I64 => Some((IrType::I64, IrType::U64)),
        IrType::I32 => Some((IrType::I32, IrType::U32)),
        _ => None,
    }
}

fn int_const(ty: IrType, v: i64) -> Operand {
    Operand::Const(match ty {
        IrType::I32 | IrType::U32 => IrConst::I32(v as i32),
        _ => IrConst::I64(v),
    })
}

/// Match the dual-bound byte-fill loop. Read-only; returns owned facts.
fn match_fill_loop(func: &IrFunction, lp: &NaturalLoop, cfg: &CfgAnalysis) -> Option<FillLoop> {
    if lp.body.len() != 3 {
        return None;
    }
    let (ptr_ty, _) = ptr_types()?;
    let label_to_idx = analysis::build_label_map(func);
    let header = lp.header;

    let mut defs: FxHashMap<u32, &Instruction> = FxHashMap::default();
    let mut def_blocks: FxHashMap<u32, usize> = FxHashMap::default();
    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(v) = inst.dest() {
                defs.insert(v.0, inst);
                def_blocks.insert(v.0, bi);
            }
        }
    }
    // Loop-invariant: defined outside the loop body (parameters and
    // constants have no definition and pass).
    let invariant = |v: Value| match def_blocks.get(&v.0) {
        Some(bi) => !lp.body.contains(bi),
        None => true,
    };
    let canon_invariant = |v: Value| -> Option<PtrExpr> {
        let e = canon_ptr(&defs, v, ptr_ty)?;
        invariant(e.base).then_some(e)
    };

    // --- Header: exactly one src phi + one Ult bound test. ---
    let hblock = &func.blocks[header];
    let [
        Instruction::Phi {
            dest: s_phi,
            ty: IrType::Ptr,
            incoming,
        },
        Instruction::Cmp {
            dest: hcmp_dest,
            op: IrCmpOp::Ult,
            lhs: Operand::Value(hcmp_lhs),
            rhs: se,
            ty: hcmp_ty,
        },
    ] = hblock.instructions.as_slice()
    else {
        return None;
    };
    let s_phi = *s_phi;
    if incoming.len() != 2 || *hcmp_ty != ptr_ty || resolve_copy(&defs, *hcmp_lhs) != s_phi {
        return None;
    }
    if let Operand::Value(v) = se {
        if !invariant(*v) {
            return None;
        }
    }
    let Terminator::CondBranch {
        cond: Operand::Value(hcond),
        true_label: h_true,
        false_label: exit_label,
    } = &hblock.terminator
    else {
        return None;
    };
    if *hcond != *hcmp_dest {
        return None;
    }
    let dcheck = *label_to_idx.get(h_true)?;
    if !lp.body.contains(&dcheck) || lp.body.contains(label_to_idx.get(exit_label)?) {
        return None;
    }

    // --- Preds: header = {preheader, latch}; dcheck/latch single-pred. ---
    let hpreds: Vec<usize> = cfg.preds.row(header).iter().map(|&p| p as usize).collect();
    if hpreds.len() != 2 {
        return None;
    }
    let latch = lp.single_latch(&cfg.preds)?;
    let preheader = if hpreds[0] == latch {
        hpreds[1]
    } else {
        hpreds[0]
    };
    if preheader == latch
        || lp.body.contains(&preheader)
        || latch == header
        || dcheck == latch
        || dcheck == header
    {
        return None;
    }
    if cfg.preds.row(dcheck) != [header as u32] || cfg.preds.row(latch) != [dcheck as u32] {
        return None;
    }
    // The commit retargets the preheader's header edge; only plain branch
    // terminators are supported (a jump table edge stays unmatched).
    if !matches!(
        func.blocks[preheader].terminator,
        Terminator::Branch(_) | Terminator::CondBranch { .. }
    ) {
        return None;
    }
    let preheader_label = func.blocks[preheader].label;
    let latch_label = func.blocks[latch].label;
    let mut s_init = None;
    let mut s_next = None;
    for (op, b) in incoming {
        if *b == preheader_label {
            s_init = Some(op.clone());
        } else if *b == latch_label {
            s_next = match op {
                Operand::Value(v) => Some(resolve_copy(&defs, *v)),
                Operand::Const(_) => None,
            };
        }
    }
    let s_init = s_init?;
    let s_next = s_next?;

    // --- D-check block: cell load, Ult bound test, same exit target. ---
    let dblock = &func.blocks[dcheck];
    let mut d_hdr: Option<(Value, PtrExpr)> = None;
    let mut dcmp: Option<(Value, PtrExpr)> = None;
    for inst in &dblock.instructions {
        match inst {
            Instruction::GetElementPtr {
                offset: Operand::Const(_),
                ..
            } => {}
            Instruction::BinOp {
                op: IrBinOp::Add,
                lhs: Operand::Value(_),
                rhs: Operand::Const(c),
                ty,
                ..
            } if *ty == ptr_ty && c.to_i64() != Some(1) => {}
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            } if d_hdr.is_none() => {
                d_hdr = Some((*dest, canon_invariant(*ptr)?));
            }
            Instruction::Cmp {
                dest,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(l),
                rhs: Operand::Value(r),
                ty,
            } if dcmp.is_none() && *ty == ptr_ty => {
                let (dh, _) = d_hdr?;
                if resolve_copy(&defs, *l) != dh {
                    return None;
                }
                dcmp = Some((*dest, canon_invariant(*r)?));
            }
            _ => return None,
        }
    }
    let (_, cell) = d_hdr?;
    let (dcmp_dest, de) = dcmp?;
    let Terminator::CondBranch {
        cond: Operand::Value(dcond),
        true_label: d_true,
        false_label: d_false,
    } = &dblock.terminator
    else {
        return None;
    };
    if *dcond != dcmp_dest || d_false != exit_label || *label_to_idx.get(d_true)? != latch {
        return None;
    }

    // --- Latch: cell reload+bump, one byte copy, src bump, branch back. ---
    let lblock = &func.blocks[latch];
    if !matches!(lblock.terminator, Terminator::Branch(l) if l == hblock.label) {
        return None;
    }
    let mut d2: Option<Value> = None; // cell reload
    let mut d3: Option<Value> = None; // d2 + 1
    let mut cell_stored = false;
    let mut s_add: Option<Value> = None;
    let mut byte: Option<Value> = None; // loaded byte
    let mut byte_stored = false;
    for inst in &lblock.instructions {
        match inst {
            Instruction::GetElementPtr {
                offset: Operand::Const(_),
                ..
            } => {}
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            } if d2.is_none() => {
                if canon_invariant(*ptr)? != cell {
                    return None;
                }
                d2 = Some(*dest);
            }
            Instruction::BinOp {
                dest,
                op: IrBinOp::Add,
                lhs: Operand::Value(l),
                rhs: Operand::Const(c),
                ty,
            } if *ty == ptr_ty && c.to_i64() == Some(1) => {
                let lr = resolve_copy(&defs, *l);
                if d2 == Some(lr) && d3.is_none() {
                    d3 = Some(*dest);
                } else if lr == s_phi && s_add.is_none() {
                    s_add = Some(*dest);
                } else {
                    return None;
                }
            }
            Instruction::Store {
                val: Operand::Value(v),
                ptr,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
                volatile: false,
            } if !cell_stored => {
                if canon_invariant(*ptr)? != cell || d3 != Some(resolve_copy(&defs, *v)) {
                    return None;
                }
                cell_stored = true;
            }
            Instruction::Load {
                dest,
                ptr,
                ty: IrType::U8,
                seg_override: AddressSpace::Default,
                volatile: false,
            } if byte.is_none() => {
                if resolve_copy(&defs, *ptr) != s_phi {
                    return None;
                }
                byte = Some(*dest);
            }
            Instruction::Store {
                val: Operand::Value(v),
                ptr,
                ty: IrType::U8,
                seg_override: AddressSpace::Default,
                volatile: false,
            } if !byte_stored => {
                if byte != Some(resolve_copy(&defs, *v)) || d2 != Some(resolve_copy(&defs, *ptr)) {
                    return None;
                }
                byte_stored = true;
            }
            _ => return None,
        }
    }
    if !(cell_stored && byte_stored) || s_add != Some(s_next) {
        return None;
    }

    // Static discharge of the cell-vs-dst guard: same base and the cell
    // starts at or beyond the store bound (`buf[N]` directly before `p`
    // gives `cell.offset == de.offset`).
    let cell_dst_static = cell.base == de.base && cell.offset >= de.offset;

    Some(FillLoop {
        header,
        preheader,
        header_label: hblock.label,
        s_init,
        se: se.clone(),
        cell,
        de,
        cell_dst_static,
    })
}

/// Build the guard chain + wide loop and reroute the preheader. Pure
/// addition: the original loop is kept as tail and fallback.
/// Shared labels and SSA ids for the emitted guard chain and wide loop.
/// Allocated up front in `rewrite_fill_loop` so every block can reference
/// them directly.
struct FillEmit {
    ga: BlockId,
    gb: BlockId,
    gc: Option<BlockId>,
    gd: Option<BlockId>,
    ge: BlockId,
    geb: BlockId,
    gs: BlockId,
    gsb: BlockId,
    gf: BlockId,
    wh: BlockId,
    wh2: BlockId,
    wb: BlockId,
    wx: BlockId,
    cellp: Value,
    dep: Value,
    c1: Value,
    cell_end: Value,
    d0: Value,
    c2: Value,
    gc_vals: Option<(Value, Value)>,
    c5: Value,
    c6: Value,
    c7: Value,
    di: Value,
    si: Value,
    diff: Value,
    c8: Value,
    s_w: Value,
    d_w: Value,
    s8: Value,
    c9: Value,
    d8: Value,
    c10: Value,
    chunk: Value,
}

fn push_block(
    blocks: &mut Vec<BasicBlock>,
    label: BlockId,
    instructions: Vec<Instruction>,
    terminator: Terminator,
) {
    blocks.push(BasicBlock {
        label,
        instructions,
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
        terminator,
    });
}

fn cond_br(cond: Value, t: BlockId, f: BlockId) -> Terminator {
    Terminator::CondBranch {
        cond: Operand::Value(cond),
        true_label: t,
        false_label: f,
    }
}

/// GA..GF: the runtime guard chain. Every failure edge lands on GF, which
/// falls back to the untouched byte loop.
fn emit_fill_guards(
    blocks: &mut Vec<BasicBlock>,
    m: &FillLoop,
    e: &FillEmit,
    ptr_ty: IrType,
    guard_ty: IrType,
    ptr_size: i64,
) {
    let seg = AddressSpace::Default;

    // GA: address computations (non-trapping, safe to speculate) and the
    // entry bound `s < se`. The cell is NOT loaded here: the original loop
    // dereferences it only after this check passes.
    push_block(
        blocks,
        e.ga,
        vec![
            Instruction::GetElementPtr {
                dest: e.cellp,
                base: m.cell.base,
                offset: int_const(ptr_ty, m.cell.offset),
                ty: IrType::Ptr,
            },
            Instruction::GetElementPtr {
                dest: e.dep,
                base: m.de.base,
                offset: int_const(ptr_ty, m.de.offset),
                ty: IrType::Ptr,
            },
            Instruction::Cmp {
                dest: e.c1,
                op: IrCmpOp::Ult,
                lhs: m.s_init.clone(),
                rhs: m.se.clone(),
                ty: ptr_ty,
            },
        ],
        cond_br(e.c1, e.gb, e.gf),
    );

    // GB: load the destination pointer, entry bound `d0 < de`.
    push_block(
        blocks,
        e.gb,
        vec![
            Instruction::BinOp {
                dest: e.cell_end,
                op: IrBinOp::Add,
                lhs: Operand::Value(e.cellp),
                rhs: int_const(ptr_ty, ptr_size),
                ty: ptr_ty,
            },
            Instruction::Load {
                dest: e.d0,
                ptr: e.cellp,
                ty: IrType::Ptr,
                seg_override: seg,
                volatile: false,
            },
            Instruction::Cmp {
                dest: e.c2,
                op: IrCmpOp::Ult,
                lhs: Operand::Value(e.d0),
                rhs: Operand::Value(e.dep),
                ty: ptr_ty,
            },
        ],
        cond_br(e.c2, e.gc.unwrap_or(e.ge), e.gf),
    );

    // GC/GD: cell-vs-dst — `cell >= de || cell_end <= d0` (skipped when the
    // struct layout proves it).
    if let (Some(gc), Some(gd), Some((c3, c4))) = (e.gc, e.gd, e.gc_vals) {
        push_block(
            blocks,
            gc,
            vec![Instruction::Cmp {
                dest: c3,
                op: IrCmpOp::Uge,
                lhs: Operand::Value(e.cellp),
                rhs: Operand::Value(e.dep),
                ty: ptr_ty,
            }],
            cond_br(c3, e.ge, gd),
        );
        push_block(
            blocks,
            gd,
            vec![Instruction::Cmp {
                dest: c4,
                op: IrCmpOp::Ule,
                lhs: Operand::Value(e.cell_end),
                rhs: Operand::Value(e.d0),
                ty: ptr_ty,
            }],
            cond_br(c4, e.ge, e.gf),
        );
    }

    // GE/GEb: cell-vs-src — `cell >= se || cell_end <= s0`.
    push_block(
        blocks,
        e.ge,
        vec![Instruction::Cmp {
            dest: e.c5,
            op: IrCmpOp::Uge,
            lhs: Operand::Value(e.cellp),
            rhs: m.se.clone(),
            ty: ptr_ty,
        }],
        cond_br(e.c5, e.gs, e.geb),
    );
    push_block(
        blocks,
        e.geb,
        vec![Instruction::Cmp {
            dest: e.c6,
            op: IrCmpOp::Ule,
            lhs: Operand::Value(e.cell_end),
            rhs: m.s_init.clone(),
            ty: ptr_ty,
        }],
        cond_br(e.c6, e.gs, e.gf),
    );

    // GS/GSb: smear — `d0 <= s0 || d0 - s0 >= CHUNK` (unsigned).
    push_block(
        blocks,
        e.gs,
        vec![Instruction::Cmp {
            dest: e.c7,
            op: IrCmpOp::Ule,
            lhs: Operand::Value(e.d0),
            rhs: m.s_init.clone(),
            ty: ptr_ty,
        }],
        cond_br(e.c7, e.wh, e.gsb),
    );
    push_block(
        blocks,
        e.gsb,
        vec![
            Instruction::Cast {
                dest: e.di,
                src: Operand::Value(e.d0),
                from_ty: IrType::Ptr,
                to_ty: guard_ty,
            },
            Instruction::Cast {
                dest: e.si,
                src: m.s_init.clone(),
                from_ty: IrType::Ptr,
                to_ty: guard_ty,
            },
            Instruction::BinOp {
                dest: e.diff,
                op: IrBinOp::Sub,
                lhs: Operand::Value(e.di),
                rhs: Operand::Value(e.si),
                ty: guard_ty,
            },
            Instruction::Cmp {
                dest: e.c8,
                op: IrCmpOp::Uge,
                lhs: Operand::Value(e.diff),
                rhs: int_const(guard_ty, CHUNK),
                ty: guard_ty,
            },
        ],
        cond_br(e.c8, e.wh, e.gf),
    );

    // GF: every guard failure falls back to the untouched byte loop.
    push_block(blocks, e.gf, Vec::new(), Terminator::Branch(m.header_label));
}

/// WH..WX: the wide 8-byte-chunk loop plus the exit block that publishes
/// the destination pointer back to the cell before handing the tail to the
/// original byte loop.
fn emit_wide_loop(blocks: &mut Vec<BasicBlock>, m: &FillLoop, e: &FillEmit, ptr_ty: IrType) {
    let seg = AddressSpace::Default;

    // WH: wide header — `s + 8 <= se`. `s <= se` is a loop invariant here
    // (entry checked `s < se`; the body advances by 8 only when
    // `s + 8 <= se`), so the bump cannot wrap past a valid one-past-the-end
    // bound.
    push_block(
        blocks,
        e.wh,
        vec![
            Instruction::Phi {
                dest: e.s_w,
                ty: IrType::Ptr,
                incoming: vec![
                    (m.s_init.clone(), e.gs),
                    (m.s_init.clone(), e.gsb),
                    (Operand::Value(e.s8), e.wb),
                ],
            },
            Instruction::Phi {
                dest: e.d_w,
                ty: IrType::Ptr,
                incoming: vec![
                    (Operand::Value(e.d0), e.gs),
                    (Operand::Value(e.d0), e.gsb),
                    (Operand::Value(e.d8), e.wb),
                ],
            },
            Instruction::BinOp {
                dest: e.s8,
                op: IrBinOp::Add,
                lhs: Operand::Value(e.s_w),
                rhs: int_const(ptr_ty, CHUNK),
                ty: ptr_ty,
            },
            Instruction::Cmp {
                dest: e.c9,
                op: IrCmpOp::Ule,
                lhs: Operand::Value(e.s8),
                rhs: m.se.clone(),
                ty: ptr_ty,
            },
        ],
        cond_br(e.c9, e.wh2, e.wx),
    );

    // WH2: `d + 8 <= de`.
    push_block(
        blocks,
        e.wh2,
        vec![
            Instruction::BinOp {
                dest: e.d8,
                op: IrBinOp::Add,
                lhs: Operand::Value(e.d_w),
                rhs: int_const(ptr_ty, CHUNK),
                ty: ptr_ty,
            },
            Instruction::Cmp {
                dest: e.c10,
                op: IrCmpOp::Ule,
                lhs: Operand::Value(e.d8),
                rhs: Operand::Value(e.dep),
                ty: ptr_ty,
            },
        ],
        cond_br(e.c10, e.wb, e.wx),
    );

    // WB: one chunk.
    push_block(
        blocks,
        e.wb,
        vec![
            Instruction::Load {
                dest: e.chunk,
                ptr: e.s_w,
                ty: IrType::U64,
                seg_override: seg,
                volatile: false,
            },
            Instruction::Store {
                val: Operand::Value(e.chunk),
                ptr: e.d_w,
                ty: IrType::U64,
                seg_override: seg,
                volatile: false,
            },
        ],
        Terminator::Branch(e.wh),
    );

    // WX: publish the destination pointer; the byte loop finishes the tail.
    push_block(
        blocks,
        e.wx,
        vec![Instruction::Store {
            val: Operand::Value(e.d_w),
            ptr: e.cellp,
            ty: IrType::Ptr,
            seg_override: seg,
            volatile: false,
        }],
        Terminator::Branch(m.header_label),
    );
}

fn rewrite_fill_loop(func: &mut IrFunction, m: &FillLoop) -> bool {
    let Some((ptr_ty, guard_ty)) = ptr_types() else {
        return false;
    };
    let ptr_size = crate::common::types::target_int_ir_type().size() as i64;
    let Some(max_label) = func.blocks.iter().map(|b| b.label.0).max() else {
        return false;
    };
    let Some(first) = func.next_label.max(max_label + 1).checked_add(13) else {
        return false;
    };
    let mut next = first - 13;
    let mut fresh_label = || {
        let l = BlockId(next);
        next += 1;
        l
    };
    let ga = fresh_label();
    let gb = fresh_label();
    let (gc, gd) = if m.cell_dst_static {
        (None, None)
    } else {
        (Some(fresh_label()), Some(fresh_label()))
    };
    let ge = fresh_label();
    let geb = fresh_label();
    let gs = fresh_label();
    let gsb = fresh_label();
    let gf = fresh_label();
    let wh = fresh_label();
    let wh2 = fresh_label();
    let wb = fresh_label();
    let wx = fresh_label();
    let end_label = next;

    let mut fresh_value = || {
        let v = Value(func.next_value_id);
        func.next_value_id += 1;
        v
    };
    // All SSA ids up front so every block can reference them directly.
    let e = FillEmit {
        ga,
        gb,
        gc,
        gd,
        ge,
        geb,
        gs,
        gsb,
        gf,
        wh,
        wh2,
        wb,
        wx,
        cellp: fresh_value(),
        dep: fresh_value(),
        c1: fresh_value(),
        cell_end: fresh_value(),
        d0: fresh_value(),
        c2: fresh_value(),
        gc_vals: gc.map(|_| (fresh_value(), fresh_value())),
        c5: fresh_value(),
        c6: fresh_value(),
        c7: fresh_value(),
        di: fresh_value(),
        si: fresh_value(),
        diff: fresh_value(),
        c8: fresh_value(),
        s_w: fresh_value(),
        d_w: fresh_value(),
        s8: fresh_value(),
        c9: fresh_value(),
        d8: fresh_value(),
        c10: fresh_value(),
        chunk: fresh_value(),
    };

    let mut blocks: Vec<BasicBlock> = Vec::new();
    emit_fill_guards(&mut blocks, m, &e, ptr_ty, guard_ty, ptr_size);
    emit_wide_loop(&mut blocks, m, &e, ptr_ty);

    // --- Commit: extend the src phi, reroute the preheader. ---
    func.next_label = end_label;
    let preheader_label = func.blocks[m.preheader].label;
    match func.blocks[m.header].instructions.first_mut() {
        Some(Instruction::Phi { incoming, .. }) => {
            incoming.retain(|(_, b)| *b != preheader_label);
            incoming.push((m.s_init.clone(), gf));
            incoming.push((Operand::Value(e.s_w), wx));
        }
        _ => return false,
    }
    match &mut func.blocks[m.preheader].terminator {
        Terminator::Branch(l) if *l == m.header_label => *l = ga,
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            if *true_label == m.header_label {
                *true_label = ga;
            }
            if *false_label == m.header_label {
                *false_label = ga;
            }
        }
        _ => return false,
    }
    func.blocks.extend(blocks);
    true
}

/// Entry point, called from the parent idiom fixpoint. Returns `true` when
/// the loop was versioned (the caller registers the header so the kept
/// fallback loop is not versioned again).
pub(crate) fn try_rewrite(
    func: &mut IrFunction,
    lp: &NaturalLoop,
    cfg: &CfgAnalysis,
    target: crate::backend::Target,
) -> bool {
    if !target_admits_wide_fill(target) {
        return false;
    }
    let Some(m) = match_fill_loop(func, lp, cfg) else {
        return false;
    };
    rewrite_fill_loop(func, &m)
}

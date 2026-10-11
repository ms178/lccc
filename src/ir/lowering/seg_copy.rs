//! Segment-aware aggregate transfers.
//!
//! An aggregate (struct, union, vector, complex) stored in a named address
//! space (`__seg_gs`/`__seg_fs`) must be read and written through the segment
//! prefix. The IR's `Memcpy` carries no address space, and an aggregate
//! rvalue is represented by the address of its storage, so a plain copy of a
//! segment object silently reads or writes generic memory (oracle: GCC emits
//! `%gs:` on every access to `gobj`, lccc emitted none).
//!
//! The fix is lowering-only and keeps segment information where it already
//! lives, on each scalar `Load`/`Store`. Passes already preserve those, and no
//! optimizer pass has to learn about a segment-carrying `Memcpy`:
//!
//! * `emit_segment_copy` copies `size` bytes between two addresses, each with
//!   its own segment. Transfers up to `SEG_COPY_UNROLL_MAX` bytes are a
//!   straight-line sequence of the widest aligned-or-not integer chunks
//!   (8/4/2/1 bytes). Larger transfers use one counted loop over 8-byte words
//!   followed by an unrolled tail, so code size stays bounded.
//! * `materialize_segment_aggregate` copies a segment-resident aggregate that
//!   is used as an rvalue (argument, return value, initializer, assignment
//!   source) into a generic temporary, so every downstream consumer sees
//!   ordinary memory. Lvalue chains (`gobj.inner.v[0] = 1`) never go through
//!   that path; they resolve addresses through `get_struct_base_addr`.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{
    BlockId, Instruction, IrBinOp, IrCmpOp, IrConst, Operand, Terminator, Value,
};

/// Largest transfer emitted as straight-line code. The counted loop keeps its
/// counter in memory (a load and a store per 8-byte word), which measured 2.8x
/// to 3.5x slower than straight-line code for a 320-byte copy (34-42 ns against
/// 12 ns on an Intel 14700KF, bench/bench_seg.c). 512 bytes (64 qword pairs,
/// about 0.5 KiB of code per copy site) covers that band. Transfers above it
/// keep the loop, where its per-iteration cost is amortized.
pub(super) const SEG_COPY_UNROLL_MAX: usize = 512;

impl Lowerer {
    /// Copy `size` bytes from `src` (addressing memory in `src_seg`) to `dst`
    /// (addressing memory in `dst_seg`). Both values are segment-relative
    /// offsets; `AddressSpace::Default` means ordinary memory.
    pub(super) fn emit_segment_copy(
        &mut self,
        dst: Value,
        dst_seg: AddressSpace,
        src: Value,
        src_seg: AddressSpace,
        size: usize,
        volatile: bool,
    ) {
        if size == 0 {
            return;
        }
        if size <= SEG_COPY_UNROLL_MAX {
            self.emit_segment_copy_span(dst, dst_seg, src, src_seg, 0, size, volatile);
            return;
        }

        // Counted loop over whole 8-byte words; the tail (< 8 bytes) is
        // unrolled after the loop.
        let words_bytes = size & !7;
        let counter = self.emit_entry_alloca(IrType::I64, 8, 8, false);
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Const(IrConst::I64(0)),
            ptr: counter,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        let head: BlockId = self.fresh_label();
        let body: BlockId = self.fresh_label();
        let exit: BlockId = self.fresh_label();

        self.terminate(Terminator::Branch(head));

        self.start_block(head);
        let idx = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: idx,
            ptr: counter,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        let more = self.fresh_value();
        self.emit(Instruction::Cmp {
            dest: more,
            op: IrCmpOp::Ult,
            lhs: Operand::Value(idx),
            rhs: Operand::Const(IrConst::I64(words_bytes as i64)),
            ty: IrType::I64,
        });
        self.terminate(Terminator::CondBranch {
            cond: Operand::Value(more),
            true_label: body,
            false_label: exit,
        });

        self.start_block(body);
        let src_at = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: src_at,
            base: src,
            offset: Operand::Value(idx),
            ty: IrType::I8,
        });
        let dst_at = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: dst_at,
            base: dst,
            offset: Operand::Value(idx),
            ty: IrType::I8,
        });
        let word = self.fresh_value();
        self.emit(Instruction::Load {
            volatile,
            dest: word,
            ptr: src_at,
            ty: IrType::I64,
            seg_override: src_seg,
        });
        self.emit(Instruction::Store {
            volatile,
            val: Operand::Value(word),
            ptr: dst_at,
            ty: IrType::I64,
            seg_override: dst_seg,
        });
        let next = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: next,
            op: IrBinOp::Add,
            lhs: Operand::Value(idx),
            rhs: Operand::Const(IrConst::I64(8)),
            ty: IrType::I64,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Value(next),
            ptr: counter,
            ty: IrType::I64,
            seg_override: AddressSpace::Default,
        });
        self.terminate(Terminator::Branch(head));

        self.start_block(exit);
        if words_bytes < size {
            self.emit_segment_copy_span(dst, dst_seg, src, src_seg, words_bytes, size, volatile);
        }
    }

    /// Load the `len` bytes (1..=8) at `base + off` as one zero-extended I64,
    /// reading exactly `len` bytes of memory.
    ///
    /// A whole-word load of a 1..7-byte tail reads past the end of the object.
    /// That is harmless for a local, but for a segment object it reads unrelated
    /// per-CPU or TLS data, and at the end of a mapped page it faults. Widths
    /// 1, 2, 4 and 8 are one exact load. Widths 3, 5, 6 and 7 are two overlapping
    /// loads of the narrower width: the low load covers bytes `[0, W)`, the high
    /// load covers bytes `[len - W, len)`, and the bytes they share agree, so
    /// `low | high << 8 * (len - W)` is exact.
    pub(super) fn load_bytes_exact(
        &mut self,
        base: Value,
        off: usize,
        len: usize,
        seg: AddressSpace,
    ) -> Value {
        debug_assert!((1..=8).contains(&len), "load_bytes_exact: width {len}");
        if len == 8 {
            return self.load_word_at(base, off, IrType::I64, seg);
        }
        if matches!(len, 1 | 2 | 4) {
            let ty = match len {
                1 => IrType::U8,
                2 => IrType::U16,
                _ => IrType::U32,
            };
            let narrow = self.load_word_at(base, off, ty, seg);
            return self.zext_to_i64(narrow, ty);
        }
        let (ty, width) = if len == 3 {
            (IrType::U16, 2)
        } else {
            (IrType::U32, 4)
        };
        let low = self.load_word_at(base, off, ty, seg);
        let low = self.zext_to_i64(low, ty);
        let high = self.load_word_at(base, off + len - width, ty, seg);
        let high = self.zext_to_i64(high, ty);
        let shifted = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: shifted,
            op: IrBinOp::Shl,
            lhs: Operand::Value(high),
            rhs: Operand::Const(IrConst::I64(8 * (len - width) as i64)),
            ty: IrType::I64,
        });
        let packed = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: packed,
            op: IrBinOp::Or,
            lhs: Operand::Value(low),
            rhs: Operand::Value(shifted),
            ty: IrType::I64,
        });
        packed
    }

    /// One scalar load of type `ty` at `base + off`, carrying `seg`.
    fn load_word_at(&mut self, base: Value, off: usize, ty: IrType, seg: AddressSpace) -> Value {
        let ptr = if off == 0 {
            base
        } else {
            let at = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: at,
                base,
                offset: Operand::Const(IrConst::ptr_int(off as i64)),
                ty: IrType::I8,
            });
            at
        };
        let dest = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest,
            ptr,
            ty,
            seg_override: seg,
        });
        dest
    }

    /// Zero-extend a narrow unsigned value to I64.
    fn zext_to_i64(&mut self, v: Value, from: IrType) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Cast {
            dest,
            src: Operand::Value(v),
            from_ty: from,
            to_ty: IrType::I64,
        });
        dest
    }

    /// Straight-line copy of bytes `[from, to)`, one chunk at a time, using the
    /// widest integer width that fits the remaining bytes.
    fn emit_segment_copy_span(
        &mut self,
        dst: Value,
        dst_seg: AddressSpace,
        src: Value,
        src_seg: AddressSpace,
        from: usize,
        to: usize,
        volatile: bool,
    ) {
        let mut off = from;
        while off < to {
            let (ty, width) = match to - off {
                n if n >= 8 => (IrType::I64, 8),
                n if n >= 4 => (IrType::I32, 4),
                n if n >= 2 => (IrType::I16, 2),
                _ => (IrType::I8, 1),
            };
            let src_at = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: src_at,
                base: src,
                offset: Operand::Const(IrConst::ptr_int(off as i64)),
                ty: IrType::I8,
            });
            let chunk = self.fresh_value();
            self.emit(Instruction::Load {
                volatile,
                dest: chunk,
                ptr: src_at,
                ty,
                seg_override: src_seg,
            });
            let dst_at = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: dst_at,
                base: dst,
                offset: Operand::Const(IrConst::ptr_int(off as i64)),
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile,
                val: Operand::Value(chunk),
                ptr: dst_at,
                ty,
                seg_override: dst_seg,
            });
            off += width;
        }
    }

    /// Does this expression denote an aggregate object whose storage is in a
    /// named address space? Only lvalue-shaped expressions qualify: a
    /// segment-resident aggregate produced by a call or a cast already lives
    /// in generic memory.
    pub(super) fn aggregate_lvalue_space(&self, expr: &Expr) -> AddressSpace {
        // Comma and assignment forward the storage of their operand (see
        // `get_addr_space_of_struct_expr`). A conditional is not listed: its
        // arms are materialized inside the branches (`lower_conditional`), so
        // its result is already a generic address.
        if !matches!(
            expr,
            Expr::Identifier(..)
                | Expr::MemberAccess(..)
                | Expr::PointerMemberAccess(..)
                | Expr::ArraySubscript(..)
                | Expr::Deref(..)
                | Expr::Comma(..)
                | Expr::Assign(..)
                | Expr::CompoundAssign(..)
        ) {
            return AddressSpace::Default;
        }
        let space = self.get_addr_space_of_struct_expr(expr);
        if space == AddressSpace::Default {
            return AddressSpace::Default;
        }
        let ct = self.expr_ctype(expr);
        if matches!(ct, CType::Struct(_) | CType::Union(_)) || ct.is_vector() || ct.is_complex() {
            space
        } else {
            AddressSpace::Default
        }
    }

    /// Lower an aggregate expression whose value is about to be COPIED (struct
    /// or vector assignment source, initializer, argument, return value).
    /// Segment-resident storage is copied into a generic temporary first, so
    /// the copy consumer can use ordinary memory operations.
    ///
    /// Do not use this for lvalue contexts (assignment destinations, member
    /// bases, `&`): those need the object's own address and must stay on
    /// `lower_expr`, which returns the storage address unchanged.
    pub(super) fn lower_aggregate_rvalue(&mut self, expr: &Expr) -> Operand {
        let result = self.lower_expr(expr);
        self.materialize_segment_aggregate(expr, result)
    }

    /// Copy `size` bytes from `src_addr` (the storage of `src_expr`) into the
    /// generic address `dest`, reading through the segment when `src_expr`
    /// lives in a named address space.
    pub(super) fn copy_aggregate_from_expr(
        &mut self,
        dest: Value,
        src_expr: &Expr,
        src_addr: Value,
        size: usize,
    ) {
        let volatile = self.expr_access_is_volatile(src_expr);
        let src_space = self.aggregate_lvalue_space(src_expr);
        if src_space == AddressSpace::Default {
            self.emit(Instruction::Memcpy {
                dest,
                src: src_addr,
                size,
            });
        } else {
            self.emit_segment_copy(
                dest,
                AddressSpace::Default,
                src_addr,
                src_space,
                size,
                volatile,
            );
        }
    }

    /// Complex rvalue materialization. Complex values are pointers to a
    /// two-component object, so only the complex case needs the copy here;
    /// struct and vector rvalues keep their direct segment access.
    pub(super) fn materialize_segment_complex(&mut self, expr: &Expr, result: Operand) -> Operand {
        if self.aggregate_lvalue_space(expr) == AddressSpace::Default {
            return result;
        }
        if !self.expr_ctype(expr).is_complex() {
            return result;
        }
        self.materialize_segment_aggregate(expr, result)
    }

    /// Copy `size` bytes between two places, each in its own address space.
    /// Generic-to-generic stays a plain memcpy, so non-segment code is unchanged.
    pub(super) fn copy_bytes_across_spaces(
        &mut self,
        dst: Value,
        dst_space: AddressSpace,
        src: Value,
        src_space: AddressSpace,
        size: usize,
        volatile: bool,
    ) {
        if dst_space == AddressSpace::Default && src_space == AddressSpace::Default {
            self.emit(Instruction::Memcpy {
                dest: dst,
                src,
                size,
            });
        } else {
            self.emit_segment_copy(dst, dst_space, src, src_space, size, volatile);
        }
    }

    /// If `expr` is a segment-resident aggregate, copy it into a generic
    /// temporary and return that temporary's address. For every other
    /// expression the operand is returned unchanged.
    pub(super) fn materialize_segment_aggregate(
        &mut self,
        expr: &Expr,
        result: Operand,
    ) -> Operand {
        let space = self.aggregate_lvalue_space(expr);
        if space == AddressSpace::Default {
            return result;
        }
        let ct = self.expr_ctype(expr);
        let size = match ct {
            CType::Struct(_) | CType::Union(_) => self.struct_value_size(expr),
            _ => Some(ct.size()),
        };
        let Some(size) = size else {
            // Dynamic (VLA-member) aggregates in a segment are not supported
            // yet; reject loudly rather than copy from generic memory.
            self.diagnostics.borrow_mut().error(
                "segment-qualified aggregate with a variable-length member is not supported as an rvalue",
                expr.span(),
            );
            return result;
        };
        let align = self.alignof_expr(expr).max(8);
        let src = self.operand_to_value(result);
        let tmp = self.emit_entry_alloca(IrType::I8, size, align, false);
        let volatile = self.expr_access_is_volatile(expr);
        self.emit_segment_copy(tmp, AddressSpace::Default, src, space, size, volatile);
        Operand::Value(tmp)
    }
}

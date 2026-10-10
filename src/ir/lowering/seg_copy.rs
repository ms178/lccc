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

/// Largest transfer emitted as straight-line code. 256 bytes is 32 qword
/// load/store pairs (64 instructions), the point where a counted loop costs
/// fewer bytes of code than the unrolled form and no more cycles per byte.
pub(super) const SEG_COPY_UNROLL_MAX: usize = 256;

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
    ) {
        if size == 0 {
            return;
        }
        if size <= SEG_COPY_UNROLL_MAX {
            self.emit_segment_copy_span(dst, dst_seg, src, src_seg, 0, size);
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
            volatile: false,
            dest: word,
            ptr: src_at,
            ty: IrType::I64,
            seg_override: src_seg,
        });
        self.emit(Instruction::Store {
            volatile: false,
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
            self.emit_segment_copy_span(dst, dst_seg, src, src_seg, words_bytes, size);
        }
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
                volatile: false,
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
                volatile: false,
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
        if !matches!(
            expr,
            Expr::Identifier(..)
                | Expr::MemberAccess(..)
                | Expr::PointerMemberAccess(..)
                | Expr::ArraySubscript(..)
                | Expr::Deref(..)
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
        let src_space = self.aggregate_lvalue_space(src_expr);
        if src_space == AddressSpace::Default {
            self.emit(Instruction::Memcpy {
                dest,
                src: src_addr,
                size,
            });
        } else {
            self.emit_segment_copy(dest, AddressSpace::Default, src_addr, src_space, size);
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
        self.emit_segment_copy(tmp, AddressSpace::Default, src, space, size);
        Operand::Value(tmp)
    }
}

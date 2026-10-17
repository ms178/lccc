//! Clang 23.1.3's i386 by-value convention for `ext_vector_type` values:
//! the 16-byte XMM-piece images (with `_Bool` lanes widened to one lane per
//! 16/N bytes), the non-GP register-return carriers, and the diagnostic for
//! shapes lccc has no carrier for (on both x86 targets).
//!
//! Classification lives in `CType::i386_ext_vector_arg`/`i386_ext_vector_ret`
//! (see `I386ExtVectorCarrier`); scalarized lanes and the EAX:EDX:ECX lane
//! words keep their decomposition in `expr_calls`, `func_lowering` and
//! `stmt_return`.

use super::lower::Lowerer;
use crate::common::source::Span;
use crate::common::types::{AddressSpace, CType, I386ExtVectorCarrier, IrType};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, Operand, Value};

/// IR type of one widened `_Bool` lane of an N-lane vector in its XMM piece.
fn xmm_bool_lane_ty(lanes: usize) -> IrType {
    match 16 / lanes {
        1 => IrType::U8,
        2 => IrType::U16,
        4 => IrType::U32,
        _ => IrType::U64,
    }
}

impl Lowerer {
    /// The value of a received `_Bool` ext-vector lane is its bit 0 (other
    /// lanes pass through): Clang 23.1.3 passes a one-lane `_Bool` vector as
    /// its zero-extended storage byte, padding bits included, and leaves the
    /// high bits of widened and returned lanes undefined.
    pub(super) fn i386_bool_lane_bit(
        &mut self,
        vector_ct: &CType,
        value: Operand,
        ty: IrType,
    ) -> Operand {
        if !vector_ct.is_bitpacked_bool_vector() {
            return value;
        }
        Operand::Value(self.emit_binop_val(
            IrBinOp::And,
            value,
            Operand::Const(IrConst::I64(1)),
            ty,
        ))
    }

    /// Copy the vector at `src` into a fresh 16-byte-aligned image of
    /// `16 * pieces` bytes laid out as LLVM's widened register type: the
    /// object bytes (never reading past the object; the tail is zeroed), or
    /// for `_Bool` lanes one 0/1 integer of 16/N bytes per lane.
    pub(super) fn i386_ext_vector_xmm_image(
        &mut self,
        src: Value,
        vector_ct: &CType,
        pieces: usize,
        space: AddressSpace,
        volatile: bool,
    ) -> Value {
        let image_size = 16 * pieces;
        let image = self.emit_entry_alloca(IrType::Ptr, image_size, 16, false);
        let (elem_ct, lanes) = vector_ct
            .vector_info()
            .expect("ExtVector must carry its logical lane count");
        let size = vector_ct.size();
        if vector_ct.is_bitpacked_bool_vector() || space != AddressSpace::Default || volatile {
            self.zero_init_alloca(image, image_size);
            let bool_lane_ty = vector_ct
                .is_bitpacked_bool_vector()
                .then(|| xmm_bool_lane_ty(lanes));
            let elem_ty = IrType::from_ctype(elem_ct);
            for lane in 0..lanes {
                let value = self.load_vector_lane(src, vector_ct, lane, space, volatile);
                if let Some(lane_ty) = bool_lane_ty {
                    let value = self.emit_implicit_cast(value, IrType::I32, lane_ty);
                    let ptr = self.emit_gep_offset(image, lane * lane_ty.size(), lane_ty);
                    self.emit(Instruction::Store {
                        volatile: false,
                        val: value,
                        ptr,
                        ty: lane_ty,
                        seg_override: AddressSpace::Default,
                    });
                } else {
                    self.store_vector_lane(
                        image,
                        vector_ct,
                        lane,
                        value,
                        elem_ty,
                        AddressSpace::Default,
                        false,
                    );
                }
            }
        } else {
            if size < image_size {
                self.zero_init_alloca(image, image_size);
            }
            self.emit(Instruction::Memcpy {
                dest: image,
                src,
                size: size.min(image_size),
            });
        }
        image
    }

    /// Rebuild a vector object from its XMM image (the inverse of
    /// `i386_ext_vector_xmm_image`). `_Bool` lanes take bit 0 of their
    /// widened lane, as Clang's callee does.
    fn i386_ext_vector_from_xmm_image(&mut self, image: Value, vector_ct: &CType) -> Value {
        let size = vector_ct.size();
        let align = self.ctype_align(vector_ct);
        let vector = self.emit_entry_alloca(IrType::Ptr, size, align, false);
        if vector_ct.is_bitpacked_bool_vector() {
            let (_, lanes) = vector_ct
                .vector_info()
                .expect("ExtVector must carry its logical lane count");
            let lane_ty = xmm_bool_lane_ty(lanes);
            self.zero_init_alloca(vector, size);
            for lane in 0..lanes {
                let ptr = self.emit_gep_offset(image, lane * lane_ty.size(), lane_ty);
                let wide = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: wide,
                    ptr,
                    ty: lane_ty,
                    seg_override: AddressSpace::Default,
                });
                let bit = self.i386_bool_lane_bit(vector_ct, Operand::Value(wide), lane_ty);
                self.store_vector_lane(
                    vector,
                    vector_ct,
                    lane,
                    bit,
                    lane_ty,
                    AddressSpace::Default,
                    false,
                );
            }
        } else {
            self.emit(Instruction::Memcpy {
                dest: vector,
                src: image,
                size: size.min(16),
            });
        }
        vector
    }

    /// Rebuild an XMM-carried ext-vector parameter from its 16-byte piece
    /// parameters (each already spilled to its own 16-byte slot).
    pub(super) fn i386_ext_vector_param_from_pieces(
        &mut self,
        vector_ct: &CType,
        piece_slots: &[Value],
    ) -> Value {
        if let [single] = piece_slots {
            return self.i386_ext_vector_from_xmm_image(*single, vector_ct);
        }
        // Multi-piece vectors are power-of-two sized non-bool vectors whose
        // object is exactly the concatenation of the pieces.
        let size = vector_ct.size();
        debug_assert_eq!(size, 16 * piece_slots.len());
        let align = self.ctype_align(vector_ct);
        let vector = self.emit_entry_alloca(IrType::Ptr, size, align, false);
        for (k, &slot) in piece_slots.iter().enumerate() {
            self.emit_memcpy_at_offset(vector, 16 * k, slot, 16);
        }
        vector
    }

    /// Lower `return e;` for an i386 ext vector returned in `%xmm0` (an I128
    /// image tagged `[Sse, SseUp]`) or, with one floating lane, as that x87
    /// or SSE scalar. GP-word returns are handled by
    /// `try_i686_ext_vector_gp_return`.
    pub(super) fn try_i686_ext_vector_xmm_or_fp_return(&mut self, e: &Expr) -> Option<Operand> {
        if !crate::common::types::target_is_32bit() {
            return None;
        }
        let vector_ct = self.func().return_ctype.clone()?;
        let (carrier_ty, classes) = vector_ct.i686_ext_vector_ret_carrier()?;
        if vector_ct.i686_ext_vector_gp_word_count().is_some() {
            return None;
        }
        let source_operand = self.lower_aggregate_rvalue(e);
        let source = self.operand_to_value(source_operand);
        let space = self.aggregate_lvalue_space(e);
        let volatile = self.expr_access_is_volatile(e);
        if classes.is_empty() {
            return Some(self.load_vector_lane(source, &vector_ct, 0, space, volatile));
        }
        let image = self.i386_ext_vector_xmm_image(source, &vector_ct, 1, space, volatile);
        let packed = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: packed,
            ptr: image,
            ty: carrier_ty,
            seg_override: AddressSpace::Default,
        });
        Some(Operand::Value(packed))
    }

    /// Materialize the vector object of an i386 ext-vector call result from
    /// its register carrier (see `CType::i686_ext_vector_ret_carrier`) and
    /// return the object's address.
    pub(super) fn unpack_i686_ext_vector_reg_return(
        &mut self,
        dest: Value,
        return_type: IrType,
        vector_ct: &CType,
    ) -> Operand {
        if vector_ct.i686_ext_vector_gp_word_count().is_some() {
            return self.unpack_i686_ext_vector_gp_return(dest, return_type, vector_ct);
        }
        if matches!(
            vector_ct.i386_ext_vector_ret(),
            Some(I386ExtVectorCarrier::Xmm { .. })
        ) {
            let image = self.emit_entry_alloca(IrType::Ptr, 16, 16, false);
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(dest),
                ptr: image,
                ty: IrType::I128,
                seg_override: AddressSpace::Default,
            });
            return Operand::Value(self.i386_ext_vector_from_xmm_image(image, vector_ct));
        }
        // One floating lane returned as its scalar.
        let size = vector_ct.size();
        let align = self.ctype_align(vector_ct);
        let vector = self.emit_entry_alloca(IrType::Ptr, size, align, false);
        self.zero_init_alloca(vector, size);
        self.store_vector_lane(
            vector,
            vector_ct,
            0,
            Operand::Value(dest),
            return_type,
            AddressSpace::Default,
            false,
        );
        Operand::Value(vector)
    }

    /// `va_arg` of an i386 ext vector passed as XMM pieces: a variadic caller
    /// stores them in 16-byte-aligned stack slots (Clang 23.1.3 and lccc;
    /// Clang's own `va_arg` reads small widened vectors at their object size
    /// instead and so disagrees with its caller; matching the callers keeps
    /// lccc's `va_arg` correct for both).
    pub(super) fn lower_i386_va_arg_ext_vector_xmm(
        &mut self,
        ap_expr: &Expr,
        vector_ct: &CType,
        pieces: usize,
    ) -> Operand {
        let ap_operand = self.lower_va_list_pointer(ap_expr);
        let ap_addr = self.operand_to_value(ap_operand);
        let cur = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: cur,
            ptr: ap_addr,
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
        });
        let cur_int = self.emit_implicit_cast(Operand::Value(cur), IrType::Ptr, IrType::U32);
        let bumped = self.emit_binop_val(
            IrBinOp::Add,
            cur_int,
            Operand::Const(IrConst::I64(15)),
            IrType::U32,
        );
        let aligned = self.emit_binop_val(
            IrBinOp::And,
            Operand::Value(bumped),
            Operand::Const(IrConst::I64(!15)),
            IrType::U32,
        );
        let next = self.emit_binop_val(
            IrBinOp::Add,
            Operand::Value(aligned),
            Operand::Const(IrConst::I64(16 * pieces as i64)),
            IrType::U32,
        );
        let next_ptr = self.emit_implicit_cast(Operand::Value(next), IrType::U32, IrType::Ptr);
        self.emit(Instruction::Store {
            volatile: false,
            val: next_ptr,
            ptr: ap_addr,
            ty: IrType::Ptr,
            seg_override: AddressSpace::Default,
        });
        let base = self.emit_implicit_cast(Operand::Value(aligned), IrType::U32, IrType::Ptr);
        let base = self.operand_to_value(base);
        let slots: Vec<Value> = (0..pieces)
            .map(|k| {
                if k == 0 {
                    base
                } else {
                    self.emit_gep_offset(base, 16 * k, IrType::Ptr)
                }
            })
            .collect();
        Operand::Value(self.i386_ext_vector_param_from_pieces(vector_ct, &slots))
    }

    /// `va_arg` of a scalarized i386 ext vector: one ordinary variadic slot
    /// per lane, as its callers pass it.
    pub(super) fn lower_i386_va_arg_ext_vector_lanes(
        &mut self,
        ap_expr: &Expr,
        vector_ct: &CType,
    ) -> Operand {
        let (elem_ct, lanes) = vector_ct
            .vector_info()
            .expect("ExtVector must carry its logical lane count");
        // Sub-int lanes occupy (and are read from) a full 4-byte slot.
        let lane_ty =
            if matches!(elem_ct, CType::Bool) || (elem_ct.is_integer() && elem_ct.size() < 4) {
                IrType::I32
            } else {
                IrType::from_ctype(elem_ct)
            };
        let ap_operand = self.lower_va_list_pointer(ap_expr);
        let va_list_ptr = self.operand_to_value(ap_operand);
        let size = vector_ct.size();
        let align = self.ctype_align(vector_ct);
        let vector = self.emit_entry_alloca(IrType::Ptr, size, align, false);
        self.zero_init_alloca(vector, size);
        for lane in 0..lanes {
            let dest = self.fresh_value();
            self.emit(Instruction::VaArg {
                dest,
                va_list_ptr,
                result_ty: lane_ty,
            });
            let value = self.i386_bool_lane_bit(vector_ct, Operand::Value(dest), lane_ty);
            self.store_vector_lane(
                vector,
                vector_ct,
                lane,
                value,
                lane_ty,
                AddressSpace::Default,
                false,
            );
        }
        Operand::Value(vector)
    }

    /// Whether `name` is a function with internal linkage in this
    /// translation unit (its by-value convention never meets foreign code).
    pub(super) fn is_internal_function(&self, name: &str) -> bool {
        self.static_functions.contains(name)
    }

    /// Diagnose a call that may reach code compiled by another compiler
    /// (anything but a direct call to an internal-linkage function) when it
    /// passes or returns an ext vector whose Clang convention lccc has no
    /// carrier for. Internal calls keep lccc's own convention on both sides.
    pub(super) fn diagnose_call_ext_vector_abi(
        &self,
        callee: &Expr,
        return_ct: Option<&CType>,
        param_ctypes: &Option<Vec<CType>>,
        args: &[Expr],
    ) {
        if let Expr::Identifier(name, _) = callee {
            if !self.is_func_ptr_variable(name) && self.is_internal_function(name) {
                return;
            }
        }
        if let Some(ct) = return_ct {
            self.diagnose_unsupported_ext_vector_abi(ct, true, callee.span());
        }
        for (arg_idx, arg) in args.iter().enumerate() {
            if let Some(ct) = self.call_arg_ctype(param_ctypes, arg_idx, arg) {
                self.diagnose_unsupported_ext_vector_abi(&ct, false, arg.span());
            }
        }
    }

    /// Reject passing or returning an ext vector by value when Clang
    /// 23.1.3's convention for it on the current x86 target has no lccc
    /// carrier (see `I386ExtVectorCarrier::Unsupported` and
    /// `CType::x86_64_ext_vector_unsupported`), instead of silently
    /// producing an incompatible call. Returns whether it was rejected.
    pub(super) fn diagnose_unsupported_ext_vector_abi(
        &self,
        ct: &CType,
        is_return: bool,
        span: Span,
    ) -> bool {
        let unsupported = if crate::common::types::target_is_32bit() {
            let carrier = if is_return {
                ct.i386_ext_vector_ret()
            } else {
                ct.i386_ext_vector_arg()
            };
            carrier == Some(I386ExtVectorCarrier::Unsupported)
        } else {
            matches!(ct, CType::ExtVector(..)) && ct.x86_64_ext_vector_unsupported(is_return)
        };
        if unsupported {
            let what = if is_return { "returning" } else { "passing" };
            self.emit_error(
                format!(
                    "{what} '{ct}' by value is not supported: the Clang ext_vector_type calling \
                     convention for this vector uses a register sequence lccc does not implement; \
                     pass it through a pointer instead"
                ),
                span,
            );
        }
        unsupported
    }
}

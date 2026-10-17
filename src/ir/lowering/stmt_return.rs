//! Return statement lowering.
//!
//! Extracted from stmt.rs to reduce that file's complexity. The `Stmt::Return`
//! match arm was 194 lines handling sret, two-register returns, complex returns,
//! and scalar-to-complex conversions. This module breaks that into focused helpers.

use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType};
use crate::frontend::parser::ast::{BinOp, Expr};
use crate::ir::reexports::{Instruction, IrBinOp, IrConst, Operand, Value};

impl Lowerer {
    /// Check if an expression is a BinaryOp with an arithmetic/bitwise operator.
    /// Such expressions never produce struct/union types, so struct-return checks can be skipped.
    fn expr_is_arithmetic_binop(e: &Expr) -> bool {
        matches!(e, Expr::BinaryOp(op, _, _, _) if matches!(op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
            | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor
            | BinOp::Shl | BinOp::Shr
            | BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
            | BinOp::LogicalAnd | BinOp::LogicalOr))
    }

    /// Lower a return statement's expression to an operand.
    /// Handles all return conventions: sret, two-register, complex decomposition,
    /// and scalar returns with implicit casts.
    pub(super) fn lower_return_expr(&mut self, e: &Expr) -> Operand {
        if let Some(op) = self.try_i686_ext_vector_gp_return(e) {
            return op;
        }
        if let Some(op) = self.try_i686_ext_vector_xmm_or_fp_return(e) {
            return op;
        }

        // Try sret return first (large structs and complex types via hidden pointer)
        if let Some(sret_alloca) = self.func().sret_ptr {
            if let Some(op) = self.try_sret_return(e, sret_alloca) {
                return op;
            }
        }

        // X87-class aggregate (`struct { long double }`): the value leaves in
        // %st(0), so the IR returns the long double read out of the object.
        let fname = self.func().name.clone();
        if self
            .func_meta
            .sigs
            .get(fname.as_str())
            .is_some_and(|s| s.ret_x87_aggregate)
        {
            let seg = self.aggregate_lvalue_space(e);
            let addr = self.get_struct_base_addr(e);
            let val = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: val,
                ptr: addr,
                ty: IrType::F128,
                seg_override: seg,
            });
            return Operand::Value(val);
        }

        // Two-register struct return (9-16 bytes packed into I128)
        if let Some(op) = self.try_two_reg_struct_return(e) {
            return op;
        }

        // Small struct return (<= 8 bytes loaded as I64)
        if let Some(op) = self.try_small_struct_return(e) {
            return op;
        }

        // Complex expression returned from complex-returning function
        if let Some(op) = self.try_complex_return(e) {
            return op;
        }

        // Non-complex expression returned from complex-returning function
        if let Some(op) = self.try_scalar_to_complex_return(e) {
            return op;
        }

        // Default: scalar return with implicit cast
        let val = self.lower_expr(e);
        let ret_ty = self.func_mut().return_type;
        let expr_ty = self.value_ir_type(e);
        let expr_ct = self.expr_ctype(e);
        if self.func_mut().return_is_bool {
            // For _Bool return, normalize at the source type before any truncation.
            self.emit_bool_normalize_typed(val, expr_ty)
        } else if let Some(ret_ct) = self.func_mut().return_ctype.clone() {
            if ret_ct == CType::Float128
                || expr_ct == CType::Float128
                || ret_ct.is_decimal()
                || expr_ct.is_decimal()
            {
                // _Float128 return conversions route through the soft-float
                // helpers (__extenddftf2/__trunctfdf2), not the bit-cast IR path.
                // C23 decimals likewise route through libbid (__bid_float*).
                self.convert_scalar_ctype(val, expr_ty, &expr_ct, &ret_ct)
            } else {
                self.emit_implicit_cast(val, expr_ty, ret_ty)
            }
        } else {
            self.emit_implicit_cast(val, expr_ty, ret_ty)
        }
    }

    /// True when `e` is a call to a `__builtin_*` name.
    ///
    /// The signature fallback in `try_two_reg_struct_return` exists for
    /// expressions whose size the CType cannot express — intrinsic wrappers
    /// around the raw vector builtins, where only the function's signature
    /// knows the object is returned by value. A builtin call is not that case:
    /// `struct_value_size` answers for the raw vector builtins
    /// authoritatively, and its two answers mean opposite things — `None` for
    /// a value that IS already the packed register pair (the
    /// `__builtin_ia32_*` 128-bit family, `__builtin_shufflevector`), `Some(n)`
    /// for a pointer to n bytes of data (`__builtin_shuffle` >8 bytes). Letting
    /// the signature override the `None` answer made the return path load the
    /// packed value as an aggregate, i.e. dereference the vector data as a
    /// pointer.
    fn expr_is_builtin_call(e: &Expr) -> bool {
        matches!(e, Expr::FunctionCall(callee, _, _)
            if matches!(&**callee, Expr::Identifier(n, _) if n.starts_with("__builtin_")))
    }

    /// Pack an i686 Clang integer ExtVector return into its scalar GP-register
    /// bundle. The third word is carried in the I128 stack slot and emitted by
    /// the backend in ECX, while the object representation remains lane-based.
    fn try_i686_ext_vector_gp_return(&mut self, e: &Expr) -> Option<Operand> {
        if !crate::common::types::target_is_32bit() {
            return None;
        }
        let vector_ct = self.func().return_ctype.clone()?;
        let words = vector_ct.i686_ext_vector_gp_word_count()?;
        let (elem_ct, lanes) = vector_ct.vector_info()?;
        let elem_size = elem_ct.size();
        // Invariant: the GP return bundle is bounded at 12 bytes (three 32-bit
        // words) for sub-i32 lanes and at 8 bytes (one 64-bit word) for a
        // single 8-byte lane. Any future widening must update the I128 bundle
        // store and the ECX word spill in the backend before reaching here.
        debug_assert!(
            words <= 3 && ((elem_size <= 4 && words == lanes) || (elem_size == 8 && lanes == 1)),
            "i686 ExtVector GP return: words={words} elem_size={elem_size} lanes={lanes} exceeds the EAX:EDX:ECX bundle",
        );
        let bundle_ty = match words {
            1 => IrType::I32,
            2 => IrType::I64,
            3 => IrType::I128,
            _ => return None,
        };
        let source_operand = self.lower_aggregate_rvalue(e);
        let source = self.operand_to_value(source_operand);
        let source_space = self.aggregate_lvalue_space(e);
        let source_volatile = self.expr_access_is_volatile(e);
        let bundle = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: bundle,
            ty: IrType::Ptr,
            size: 16,
            align: 4,
            volatile: false,
            semantic_volatile: false,
        });
        self.zero_init_alloca(bundle, 16);

        if elem_size <= 4 {
            debug_assert_eq!(words, lanes);
            for lane in 0..lanes {
                let value =
                    self.load_vector_lane(source, &vector_ct, lane, source_space, source_volatile);
                let source_ty = if matches!(elem_ct, CType::Bool) {
                    IrType::I32
                } else {
                    IrType::from_ctype(elem_ct)
                };
                let word = self.emit_implicit_cast(value, source_ty, IrType::I32);
                let ptr = self.emit_gep_offset(bundle, lane * 4, IrType::I32);
                self.emit(Instruction::Store {
                    volatile: false,
                    val: word,
                    ptr,
                    ty: IrType::I32,
                    seg_override: AddressSpace::Default,
                });
            }
        } else {
            debug_assert_eq!(elem_size, 8);
            debug_assert_eq!(lanes, 1);
            let value = self.load_vector_lane(source, &vector_ct, 0, source_space, source_volatile);
            let ptr = self.emit_gep_offset(bundle, 0, IrType::I64);
            self.emit(Instruction::Store {
                volatile: false,
                val: value,
                ptr,
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            });
        }

        let packed = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: packed,
            ptr: bundle,
            ty: bundle_ty,
            seg_override: AddressSpace::Default,
        });
        Some(Operand::Value(packed))
    }

    /// Try to handle return via sret (hidden pointer for large structs/complex).
    /// Returns Some(operand) if this is an sret return, None otherwise.
    fn try_sret_return(&mut self, e: &Expr, sret_alloca: Value) -> Option<Operand> {
        // _Float128/_Decimal128 returns on i686 (TFmode/TDmode = class MEMORY):
        // the caller receives the 16-byte carrier through the hidden pointer.
        // Store the carrier value into the sret buffer and return the pointer
        // (eax holds it for the caller, matching GCC's i386 convention).
        let fname0 = self.func().name.clone();
        let is_f128_sret = self
            .func_meta
            .sigs
            .get(fname0.as_str())
            .is_some_and(|s| s.ret_is_f128_sse && s.sret_size.is_some());
        if is_f128_sret {
            let val = self.lower_expr(e);
            let expr_ty = self.value_ir_type(e);
            let expr_ct = self.expr_ctype(e);
            let ret_ct = self
                .func_mut()
                .return_ctype
                .clone()
                .unwrap_or(CType::Float128);
            // Implicit conversions (e.g. `return 7;` / `return d;`) route
            // through the same soft-float helper path the scalar return uses.
            let val = if expr_ct != ret_ct {
                self.convert_scalar_ctype(val, expr_ty, &expr_ct, &ret_ct)
            } else {
                val
            };
            let sret_ptr = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: sret_ptr,
                ptr: sret_alloca,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val,
                ptr: sret_ptr,
                ty: IrType::U128,
                seg_override: AddressSpace::Default,
            });
            return Some(Operand::Value(sret_ptr));
        }
        // Large struct return via hidden pointer (sret).
        // Threshold is target-dependent: > 8 bytes on 32-bit, > 16 bytes on 64-bit.
        // struct_value_size may return Some(0) for FunctionCall/Conditional expressions
        // where CType::size() returns 0 for Struct/Union types. Fall back to the
        // current function's sret_size from sig metadata.
        let mut struct_size = self.struct_value_size(e).unwrap_or(0);
        if struct_size == 0 && !Self::expr_is_arithmetic_binop(e) && !Self::expr_is_builtin_call(e)
        {
            if let Some(ctype) = self.get_expr_ctype(e) {
                if ctype.is_struct_or_union() || ctype.is_vector() {
                    let fname = self.func().name.clone();
                    if let Some(size) = self
                        .func_meta
                        .sigs
                        .get(fname.as_str())
                        .and_then(|s| s.sret_size)
                    {
                        struct_size = size;
                    }
                }
            }
        }
        // On i686, the i386 SysV ABI uses sret for ALL struct sizes (threshold 0).
        // On 64-bit targets, structs > 16 bytes use sret -- and so does an
        // x86-64 MEMORY-class aggregate of any size (misaligned fields,
        // BLKmode vectors): the function's own signature is authoritative.
        let sret_threshold = if crate::common::types::target_is_32bit() {
            0
        } else {
            16
        };
        let fname = self.func().name.clone();
        let fn_returns_aggregate_in_memory = self
            .func_meta
            .sigs
            .get(fname.as_str())
            .is_some_and(|s| s.sret_size.is_some());
        if struct_size > sret_threshold || (struct_size > 0 && fn_returns_aggregate_in_memory) {
            let src_addr = self.get_struct_base_addr(e);
            let sret_ptr = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: sret_ptr,
                ptr: sret_alloca,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
            });
            self.copy_aggregate_from_expr(sret_ptr, e, src_addr, struct_size);
            return Some(Operand::Value(sret_ptr));
        }

        // Complex expression returned via sret
        let expr_ct = self.expr_ctype(e);
        let fname = self.func().name.clone();
        let ret_ct = self.types.func_return_ctypes.get(&fname).cloned();
        if expr_ct.is_complex() {
            let val = self.lower_expr(e);
            let src_addr = if let Some(ref rct) = ret_ct {
                if rct.is_complex() && expr_ct != *rct {
                    let ptr = self.operand_to_value(val);
                    let converted = self.complex_to_complex(ptr, &expr_ct, rct);
                    self.operand_to_value(converted)
                } else {
                    self.operand_to_value(val)
                }
            } else {
                self.operand_to_value(val)
            };
            let complex_size = ret_ct.as_ref().unwrap_or(&expr_ct).size();
            let sret_ptr = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: sret_ptr,
                ptr: sret_alloca,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
            });
            self.emit(Instruction::Memcpy {
                dest: sret_ptr,
                src: src_addr,
                size: complex_size,
            });
            return Some(Operand::Value(sret_ptr));
        }

        // Non-complex expression returned from sret complex function
        if let Some(ref rct) = ret_ct {
            if rct.is_complex() {
                let val = self.lower_expr(e);
                let src_ir_ty = self.get_expr_type(e);
                let rct_clone = rct.clone();
                let complex_val = self.scalar_to_complex(val, src_ir_ty, &rct_clone);
                let src_addr = self.operand_to_value(complex_val);
                let complex_size = rct_clone.size();
                let sret_ptr = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: sret_ptr,
                    ptr: sret_alloca,
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                });
                self.emit(Instruction::Memcpy {
                    dest: sret_ptr,
                    src: src_addr,
                    size: complex_size,
                });
                return Some(Operand::Value(sret_ptr));
            }
        }

        None
    }

    /// Try two-register struct return (9-16 bytes packed into I128).
    fn try_two_reg_struct_return(&mut self, e: &Expr) -> Option<Operand> {
        // Complex types are handled by try_complex_return, not struct return paths.
        // Packed complex float params have is_struct=true but must not be treated as
        // struct returns -- they need proper complex-to-scalar conversion.
        let expr_ct = self.expr_ctype(e);
        if expr_ct.is_complex() {
            return None;
        }
        // Every load below reads the source object; carry its segment (if any).
        let seg = self.aggregate_lvalue_space(e);
        // Get the struct size. struct_value_size may return Some(0) for expressions
        // where CType::size() returns 0 (Struct/Union types without resolved size).
        // In that case, fall back to the function's own two_reg_ret_size from sig
        // metadata, since the return type size is reliably known.
        let mut struct_size = self.struct_value_size(e).unwrap_or(0);
        if struct_size == 0 && !Self::expr_is_arithmetic_binop(e) {
            if let Some(ctype) = self.get_expr_ctype(e) {
                if ctype.is_struct_or_union() || ctype.is_vector() {
                    let fname = self.func().name.clone();
                    if let Some(size) = self
                        .func_meta
                        .sigs
                        .get(fname.as_str())
                        .and_then(|s| s.two_reg_ret_size)
                    {
                        struct_size = size;
                    }
                }
            }
        }
        // Two-register return: 9-16 bytes on 64-bit targets only.
        // On 32-bit targets, there is no two-register (I128) return path.
        if crate::common::types::target_is_32bit() {
            return None;
        }
        if struct_size <= 8 || struct_size > 16 {
            return None;
        }
        // x86-64: an all-SSE two-eightbyte struct return (e.g.
        // `struct { double, double }`, `struct { float x4 }`) is returned in
        // xmm0/xmm1 per the SysV psABI. When both eightbytes are fully
        // occupied (size 16), load them as two F64 values and use the
        // SetReturnF64Second path instead of packing into I128 — the I128
        // route materializes Cast/Shl/Or chains that round-trip the FP bits
        // through integer registers and the stack (the "rax shuttle",
        // IS-02/AB-01). Mixed [Integer, Sse] classifications and 9-15-byte
        // tails keep the I128 path (partial eightbyte loads would read
        // past the object).
        if struct_size == 16 && self.target == crate::backend::Target::X86_64 {
            // Vector types (__m128/__m128d/…) are NOT two-eightbyte structs:
            // the vector-ABI convention returns the full 16-byte value in a
            // single XMM register. Check BOTH the function's declared return
            // ctype and the expression's ctype — intrinsic wrapper bodies
            // sometimes lack a resolvable expression ctype, and the function
            // declaration is the authoritative ABI source.
            //
            // INLINE-CANDIDATE functions keep the I128 path: the inliner
            // merges only the Return operand into its phi — a value split
            // across Return + SetReturnF64Second cannot be reassembled
            // inline, so emitting the split form in an inlinable body loses
            // the high eightbyte at every inline site.
            let fname = self.func().name.clone();
            let ret_is_vector = self
                .types
                .func_return_ctypes
                .get(&fname)
                .map(|ct| ct.is_vector())
                .unwrap_or(false)
                || self
                    .get_expr_ctype(e)
                    .map(|ct| ct.is_vector())
                    .unwrap_or(false);
            let all_sse = !ret_is_vector
                && !self.func().is_inline_candidate
                && self
                    .func_meta
                    .sigs
                    .get(fname.as_str())
                    .map(|s| {
                        s.ret_eightbyte_classes.len() == 2
                            && s.ret_eightbyte_classes
                                .iter()
                                .all(|c| *c == crate::common::types::EightbyteClass::Sse)
                    })
                    .unwrap_or(false);
            if all_sse {
                let addr = self.get_struct_base_addr(e);
                let hi_ptr = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: hi_ptr,
                    base: addr,
                    offset: Operand::Const(IrConst::I64(8)),
                    ty: IrType::F64,
                });
                let hi = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: hi,
                    ptr: hi_ptr,
                    ty: IrType::F64,
                    seg_override: seg,
                });
                self.emit(Instruction::SetReturnF64Second {
                    src: Operand::Value(hi),
                });
                let lo = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: lo,
                    ptr: addr,
                    ty: IrType::F64,
                    seg_override: seg,
                });
                return Some(Operand::Value(lo));
            }
        }
        let addr = self.get_struct_base_addr(e);
        // Load low 8 bytes
        let lo = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: lo,
            ptr: addr,
            ty: IrType::I64,
            seg_override: seg,
        });
        // High bytes: exactly `struct_size - 8` of them (1..=7 below 16), so
        // no load reads past the object.
        let hi = self.load_bytes_exact(addr, 8, struct_size - 8, seg);
        // Pack into I128: (hi << 64) | lo (zero-extend both halves)
        let hi_wide = self.fresh_value();
        self.emit(Instruction::Cast {
            dest: hi_wide,
            src: Operand::Value(hi),
            from_ty: IrType::U64,
            to_ty: IrType::I128,
        });
        let lo_wide = self.fresh_value();
        self.emit(Instruction::Cast {
            dest: lo_wide,
            src: Operand::Value(lo),
            from_ty: IrType::U64,
            to_ty: IrType::I128,
        });
        let shifted = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: shifted,
            op: IrBinOp::Shl,
            lhs: Operand::Value(hi_wide),
            rhs: Operand::Const(IrConst::I64(64)),
            ty: IrType::I128,
        });
        let packed = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest: packed,
            op: IrBinOp::Or,
            lhs: Operand::Value(shifted),
            rhs: Operand::Value(lo_wide),
            ty: IrType::I128,
        });
        Some(Operand::Value(packed))
    }

    /// Try small struct return (<= 8 bytes loaded as I64).
    /// Only used on 64-bit targets; on i686, ALL structs use sret.
    fn try_small_struct_return(&mut self, e: &Expr) -> Option<Operand> {
        // On i686, the i386 SysV ABI uses sret for ALL struct returns.
        // Small structs must not be packed into eax:edx registers.
        if crate::common::types::target_is_32bit() {
            return None;
        }
        // Complex types are handled by try_complex_return, not struct return paths.
        // Packed complex float params have is_struct=true but must not be treated as
        // struct returns -- they need proper complex-to-scalar conversion.
        let expr_ct = self.expr_ctype(e);
        if expr_ct.is_complex() {
            return None;
        }
        let struct_size = self.struct_value_size(e)?;
        // struct_value_size returns Some(0) for FunctionCall/Conditional expressions
        // where CType::size() returns 0 for Struct/Union types. Don't handle those here;
        // they should be caught by try_sret_return or try_two_reg_struct_return fallbacks.
        if struct_size == 0 || struct_size > 8 {
            return None;
        }
        let addr = self.get_struct_base_addr(e);
        // The segment rides on the load itself; no temporary is needed. Read
        // exactly the object's bytes: a whole-word load would over-read 1..7
        // bytes past a small struct (see `load_bytes_exact`).
        let seg = self.aggregate_lvalue_space(e);
        // One-lane ≤8-byte vectors are scalar-carrier values per the psABI
        // (I8/I16/I32/I64/U8/.../F32/F64), not aggregates, but their lvalue
        // is still a by-reference vector object (identifiers return the
        // alloca address). Handle them here so the load uses the correctly
        // signed integer type: `load_bytes_exact` always zexts from U8/U16/
        // U32, which flips i8x1=-1 to 255 on return.
        //
        // For FP one-lanes (double/float) use the dedicated F64/F32 loads
        // below; for integer one-lanes load with signed or unsigned byte/
        // word/long/doubleword matching the element type. Multi-lane and
        // >8-byte vectors continue through load_bytes_exact.
        if expr_ct.is_vector() {
            if let Some((elem, n)) = expr_ct.vector_info() {
                if n == 1 && struct_size <= 8 {
                    let ty = match elem.size() {
                        4 if elem.is_floating() => IrType::F32,
                        8 if elem.is_floating() => IrType::F64,
                        1 if elem.is_signed() => IrType::I8,
                        2 if elem.is_signed() => IrType::I16,
                        4 if elem.is_signed() => IrType::I32,
                        8 if elem.is_signed() => IrType::I64,
                        1 => IrType::U8,
                        2 => IrType::U16,
                        4 => IrType::U32,
                        _ => IrType::U64,
                    };
                    let dest = self.fresh_value();
                    self.emit(Instruction::Load {
                        volatile: false,
                        dest,
                        ptr: addr,
                        ty,
                        seg_override: seg,
                    });
                    return Some(Operand::Value(dest));
                }
            }
        }
        // For one-lane FP vectors (double1/float1) the IR return type is F64/F32
        // so the value stays in the SSE domain. Load as the FP type directly
        // instead of via an I64 bit-pattern load, which would force an XMM→GPR
        // movq + reverse movq bounce and a ~3-cycle cross-domain stall on
        // Intel uarchs (Raptor Lake/Cove). Narrow integer one-lanes (1/2/4-byte)
        // still use the integer load path which the return-type cast then
        // zexts/sexts to match the caller's parameter register class.
        let ret_ty = self.func().return_type;
        if struct_size == 8 && ret_ty == IrType::F64 {
            let dest = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest,
                ptr: addr,
                ty: IrType::F64,
                seg_override: seg,
            });
            return Some(Operand::Value(dest));
        }
        if struct_size == 4 && ret_ty == IrType::F32 {
            let dest = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest,
                ptr: addr,
                ty: IrType::F32,
                seg_override: seg,
            });
            return Some(Operand::Value(dest));
        }
        let dest = self.load_bytes_exact(addr, 0, struct_size, seg);
        Some(Operand::Value(dest))
    }

    /// Try returning a complex expression from a complex-returning function.
    /// Handles decomposition into register pairs or I64 packing.
    fn try_complex_return(&mut self, e: &Expr) -> Option<Operand> {
        let expr_ct = self.expr_ctype(e);
        if !expr_ct.is_complex() {
            return None;
        }

        let fname = self.func().name.clone();
        let ret_ct = self.types.func_return_ctypes.get(&fname).cloned();
        if let Some(ref rct) = ret_ct {
            let val = self.lower_expr(e);
            let src_ptr = if rct.is_complex() && expr_ct != *rct {
                let ptr = self.operand_to_value(val);
                let converted = self.complex_to_complex(ptr, &expr_ct, rct);
                self.operand_to_value(converted)
            } else {
                self.operand_to_value(val)
            };

            // _Complex double: return real in xmm0, imag in xmm1
            if *rct == CType::ComplexDouble
                && self
                    .func_meta
                    .sigs
                    .get(&fname)
                    .and_then(|s| s.sret_size)
                    .is_none()
            {
                let real = self.load_complex_real(src_ptr, rct);
                let imag = self.load_complex_imag(src_ptr, rct);
                self.emit(Instruction::SetReturnF64Second { src: imag });
                return Some(real);
            }

            // _Complex long double on x86-64: return real in st(0), imag in st(1)
            if *rct == CType::ComplexLongDouble
                && self.returns_complex_long_double_in_regs()
                && self
                    .func_meta
                    .sigs
                    .get(&fname)
                    .and_then(|s| s.sret_size)
                    .is_none()
            {
                let real = self.load_complex_real(src_ptr, rct);
                let imag = self.load_complex_imag(src_ptr, rct);
                self.emit(Instruction::SetReturnF128Second { src: imag });
                return Some(real);
            }

            // _Complex float: platform-specific return convention
            if *rct == CType::ComplexFloat
                && self
                    .func_meta
                    .sigs
                    .get(&fname)
                    .and_then(|s| s.sret_size)
                    .is_none()
            {
                if self.uses_packed_complex_float() {
                    // x86-64: load packed 8 bytes as F64 for one XMM register return
                    let packed = self.fresh_value();
                    self.emit(Instruction::Load {
                        volatile: false,
                        dest: packed,
                        ptr: src_ptr,
                        ty: IrType::F64,
                        seg_override: AddressSpace::Default,
                    });
                    return Some(Operand::Value(packed));
                } else if !self.decomposes_complex_float() {
                    // i686: load packed 8 bytes as I64 for eax:edx register return
                    let packed = self.fresh_value();
                    self.emit(Instruction::Load {
                        volatile: false,
                        dest: packed,
                        ptr: src_ptr,
                        ty: IrType::I64,
                        seg_override: AddressSpace::Default,
                    });
                    return Some(Operand::Value(packed));
                } else {
                    // ARM/RISC-V: return real in first FP reg (F32), imag in second FP reg (F32)
                    let real = self.load_complex_real(src_ptr, rct);
                    let imag = self.load_complex_imag(src_ptr, rct);
                    self.emit(Instruction::SetReturnF32Second { src: imag });
                    return Some(real);
                }
            }

            // _Complex long double on x86-64/ARM64: return real in FP reg, imag in second FP reg
            if *rct == CType::ComplexLongDouble
                && self.returns_complex_long_double_in_regs()
                && self
                    .func_meta
                    .sigs
                    .get(&fname)
                    .and_then(|s| s.sret_size)
                    .is_none()
            {
                let real = self.load_complex_real(src_ptr, rct);
                let imag = self.load_complex_imag(src_ptr, rct);
                self.emit(Instruction::SetReturnF128Second { src: imag });
                return Some(real);
            }
        }

        // Complex expression returned from non-complex function: extract real part
        let ret_ty = self.func_mut().return_type;
        if ret_ty != IrType::Ptr {
            let val2 = self.lower_expr(e);
            let ptr = self.operand_to_value(val2);
            let val2 = if self.func_mut().return_is_bool {
                // For _Bool return, check both real and imag parts per C11 6.3.1.2
                self.lower_complex_to_bool(ptr, &expr_ct)
            } else {
                let real_part = self.load_complex_real(ptr, &expr_ct);
                let from_ty = Self::complex_component_ir_type(&expr_ct);
                self.emit_implicit_cast(real_part, from_ty, ret_ty)
            };
            return Some(val2);
        }

        None
    }

    /// Try converting a non-complex scalar to complex for return from a complex function.
    fn try_scalar_to_complex_return(&mut self, e: &Expr) -> Option<Operand> {
        let expr_ct = self.expr_ctype(e);
        let fname = self.func().name.clone();
        let ret_ct = self.types.func_return_ctypes.get(&fname).cloned();
        let rct = ret_ct.as_ref()?;
        if !rct.is_complex() || expr_ct.is_complex() {
            return None;
        }

        let val = self.lower_expr(e);
        let src_ir_ty = self.get_expr_type(e);
        let rct_clone = rct.clone();
        let complex_val = self.scalar_to_complex(val, src_ir_ty, &rct_clone);

        // _Complex double: decompose into two FP return registers
        if rct_clone == CType::ComplexDouble
            && self
                .func_meta
                .sigs
                .get(&fname)
                .and_then(|s| s.sret_size)
                .is_none()
        {
            let src_ptr = self.operand_to_value(complex_val);
            let real = self.load_complex_real(src_ptr, &rct_clone);
            let imag = self.load_complex_imag(src_ptr, &rct_clone);
            self.emit(Instruction::SetReturnF64Second { src: imag });
            return Some(real);
        }

        // _Complex long double on x86-64: return real in st(0), imag in st(1)
        if rct_clone == CType::ComplexLongDouble
            && self.returns_complex_long_double_in_regs()
            && self
                .func_meta
                .sigs
                .get(&fname)
                .and_then(|s| s.sret_size)
                .is_none()
        {
            let src_ptr = self.operand_to_value(complex_val);
            let real = self.load_complex_real(src_ptr, &rct_clone);
            let imag = self.load_complex_imag(src_ptr, &rct_clone);
            self.emit(Instruction::SetReturnF128Second { src: imag });
            return Some(real);
        }

        // For sret returns, copy to the hidden pointer
        if let Some(sret_alloca) = self.func().sret_ptr {
            let src_addr = self.operand_to_value(complex_val);
            let complex_size = rct_clone.size();
            let sret_ptr = self.fresh_value();
            self.emit(Instruction::Load {
                volatile: false,
                dest: sret_ptr,
                ptr: sret_alloca,
                ty: IrType::Ptr,
                seg_override: AddressSpace::Default,
            });
            self.emit(Instruction::Memcpy {
                dest: sret_ptr,
                src: src_addr,
                size: complex_size,
            });
            return Some(Operand::Value(sret_ptr));
        }

        // For non-sret complex float: platform-specific return convention
        if rct_clone == CType::ComplexFloat
            && self
                .func_meta
                .sigs
                .get(&fname)
                .and_then(|s| s.sret_size)
                .is_none()
        {
            let ptr = self.operand_to_value(complex_val);
            if self.uses_packed_complex_float() {
                // x86-64: pack into F64 for one XMM register return
                let packed = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: packed,
                    ptr,
                    ty: IrType::F64,
                    seg_override: AddressSpace::Default,
                });
                return Some(Operand::Value(packed));
            } else if !self.decomposes_complex_float() {
                // i686: pack into I64 for eax:edx register return
                let packed = self.fresh_value();
                self.emit(Instruction::Load {
                    volatile: false,
                    dest: packed,
                    ptr,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                });
                return Some(Operand::Value(packed));
            } else {
                // ARM/RISC-V: return real in first FP reg, imag in second
                let real = self.load_complex_real(ptr, &rct_clone);
                let imag = self.load_complex_imag(ptr, &rct_clone);
                self.emit(Instruction::SetReturnF32Second { src: imag });
                return Some(real);
            }
        }

        // _Complex long double on x86-64/ARM64: return real in FP reg, imag in second FP reg
        if rct_clone == CType::ComplexLongDouble
            && self.returns_complex_long_double_in_regs()
            && self
                .func_meta
                .sigs
                .get(&fname)
                .and_then(|s| s.sret_size)
                .is_none()
        {
            let ptr = self.operand_to_value(complex_val);
            let real = self.load_complex_real(ptr, &rct_clone);
            let imag = self.load_complex_imag(ptr, &rct_clone);
            self.emit(Instruction::SetReturnF128Second { src: imag });
            return Some(real);
        }

        Some(complex_val)
    }
}

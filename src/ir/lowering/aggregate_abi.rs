//! Lowering-side view of the by-value aggregate calling convention.
//!
//! Every place that needs to know how a struct, union or GNU vector crosses
//! a call boundary -- callee parameters, call arguments, signature return
//! metadata, indirect calls, `va_arg` -- asks here. On x86-64 the answer
//! comes from the psABI classifier (`crate::common::sysv_classify`); other
//! targets keep their size-based conventions.

use super::lower::Lowerer;
use crate::common::sysv_classify::SysvPassing;
use crate::common::types::{CType, EightbyteClass};
use crate::frontend::parser::ast::Expr;

/// Return convention of a function returning an aggregate by value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct AggregateReturn {
    /// Returned through a hidden pointer (MEMORY class, or i686).
    pub sret_size: Option<usize>,
    /// Returned in a register pair (9..=16 bytes), packed as I128 in the IR.
    pub two_reg_size: Option<usize>,
    /// x86-64 eightbyte classes of a register return (empty otherwise).
    pub classes: Vec<EightbyteClass>,
    /// SSE + SSEUP: the 16-byte value comes back in ONE `%xmm0`.
    pub one_xmm: bool,
    /// X87 + X87UP: the value comes back in `%st(0)` as a `long double`.
    pub x87: bool,
    /// Two `long double` lanes come back in `%st(0)` and `%st(1)` (the
    /// `_Complex long double` register pair); see `SysvPassing::X87Pair`.
    pub x87_pair: bool,
}

impl Lowerer {
    /// x86-64 psABI convention of a by-value struct, union or vector;
    /// `None` on other targets and for anything else.
    pub(super) fn sysv_passing(&self, ct: &CType) -> Option<SysvPassing> {
        if crate::common::types::target_elf_machine() != 62 {
            return None;
        }
        ct.sysv_passing(&*self.types.borrow_struct_layouts())
    }

    /// The return-position counterpart of `sysv_passing`.
    pub(super) fn sysv_return_passing(&self, ct: &CType) -> Option<SysvPassing> {
        if crate::common::types::target_elf_machine() != 62 {
            return None;
        }
        ct.sysv_return_passing(&*self.types.borrow_struct_layouts())
    }

    /// Eightbyte classes and the one-`%xmm` flag of a by-value aggregate
    /// argument on x86-64 (`None` elsewhere, or for non-aggregates). Records
    /// over 16 bytes keep an empty class list (MEMORY by size) unless they
    /// are the AVX one-register shape (SSE + SSEUPs, see
    /// `SysvPassing::vector_reg_bytes`), and so do
    /// records of zero static size: they have no eightbyte to classify
    /// (`StructLayout::classify_sysv_eightbytes` answers the same), and a
    /// VLA-containing struct -- static size zero, passed by reference in a
    /// variadic call -- must reach the backend as a plain pointer. The
    /// classifier's `[NoClass]` for an empty record is its merge identity,
    /// not an argument carrier.
    pub(super) fn sysv_arg_abi(&self, ct: &CType) -> Option<(Vec<EightbyteClass>, bool)> {
        let passing = self.sysv_passing(ct)?;
        let size = ct.size_ctx(&*self.types.borrow_struct_layouts());
        // Over 16 bytes only the AVX vector-register shape (SSE + SSEUPs,
        // one `%ymm`/`%zmm`) is a register carrier; every other wide
        // object is MEMORY by size.
        if size == 0 || (size > 16 && passing.vector_reg_bytes().is_none()) {
            return Some((Vec::new(), false));
        }
        let one_xmm = passing.one_xmm();
        Some((passing.arg_classes(), one_xmm))
    }

    /// Return convention for a function returning `ret_ct` (a struct,
    /// union or vector of `size` bytes) by value.
    pub(super) fn aggregate_return(&self, ret_ct: &CType, size: usize) -> AggregateReturn {
        let (sret_size, two_reg_size) = Self::classify_struct_return(size);
        let mut r = AggregateReturn {
            sret_size,
            two_reg_size,
            ..AggregateReturn::default()
        };
        match self.sysv_return_passing(ret_ct) {
            None => {}
            Some(SysvPassing::Memory) => {
                r.sret_size = Some(size);
                r.two_reg_size = None;
            }
            Some(SysvPassing::X87) => {
                r.sret_size = None;
                r.two_reg_size = None;
                r.x87 = true;
            }
            Some(SysvPassing::X87Pair) => {
                r.sret_size = None;
                r.two_reg_size = None;
                r.x87_pair = true;
            }
            Some(passing @ SysvPassing::Regs(_)) => {
                r.one_xmm = passing.one_xmm();
                r.classes = passing.arg_classes();
                // One `%ymm0` / `%zmm0` (SSE + 3 or 7 SSEUPs, the AVX ABI
                // level): the IR has no 32/64-byte value type, so the value
                // keeps the hidden-pointer SHAPE in the IR (`sret_size`
                // stays `Some(size)`: the caller allocates the buffer and
                // passes its address first, the callee's returns copy into
                // it) and the backend applies the register convention at
                // the boundary — the hidden pointer consumes no argument
                // register, the callee homes it in a private frame buffer,
                // and the bytes cross the call in the vector register. See
                // `backend::call_abi::hidden_vector_return_bytes`, which
                // recognises exactly this `sret + wide SSE classes` pair.
                debug_assert!(
                    passing.vector_reg_bytes().is_some_and(|b| b > 16) == (r.sret_size.is_some()),
                    "register-returned aggregate classes and the size rule disagree"
                );
            }
        }
        r
    }

    /// The callee a call to `func` targets, seen through `(*f)(...)` on a
    /// function pointer or designator and `(&g)(...)` on a function name.
    pub(super) fn resolve_callee<'e>(&self, func: &'e Expr) -> &'e Expr {
        let mut callee = func;
        while let Expr::Deref(inner, _) = callee {
            if !self.is_function_pointer_deref(inner) {
                break;
            }
            callee = inner;
        }
        if let Expr::AddressOf(inner, _) = callee {
            if let Expr::Identifier(name, _) = inner.as_ref() {
                if !self.is_func_ptr_variable(name) {
                    callee = inner;
                }
            }
        }
        callee
    }

    /// The return convention of a call to the resolved `callee`: the
    /// signature of a named function; for anything else (function pointers,
    /// computed callees) the callee expression's function type, classified
    /// exactly like a definition.
    pub(super) fn call_aggregate_return(&self, callee: &Expr) -> AggregateReturn {
        match callee {
            Expr::Identifier(name, _) if !self.is_func_ptr_variable(name) => self
                .func_meta
                .sigs
                .get(name.as_str())
                .map(|s| AggregateReturn {
                    sret_size: s.sret_size,
                    two_reg_size: s.two_reg_ret_size,
                    classes: s.ret_eightbyte_classes.clone(),
                    one_xmm: s.ret_is_f128_sse,
                    x87: s.ret_x87_aggregate,
                    x87_pair: s.ret_x87_pair,
                })
                .unwrap_or_default(),
            _ => match self.call_return_ctype(callee) {
                Some(ct) if ct.is_struct_or_union() || ct.is_vector() => {
                    let size = self.resolve_ctype_size(&ct);
                    self.aggregate_return(&ct, size)
                }
                _ => AggregateReturn::default(),
            },
        }
    }

    /// Whether a call to `func` returning a `size`-byte aggregate yields the
    /// value itself, packed in an integer register value (<= 8 bytes back in
    /// registers), rather than an address: an sret result is the buffer's
    /// address, and two-register, one-XMM and x87 results are unpacked into
    /// a temporary by the call lowering. i686 returns every aggregate via sret.
    pub(super) fn call_result_is_packed(&self, func: &Expr, size: usize) -> bool {
        let callee = self.resolve_callee(func);
        // A named callee without a seeded signature (an SSE/AVX builtin whose
        // system-header declaration is ISA-gated out of the build) is sized by
        // its recorded return type: `_mm256_clmulepi64_epi128` returns 32
        // bytes by address, whatever size the caller inferred.
        let size = match callee {
            Expr::Identifier(name, _) if !self.func_meta.sigs.contains_key(name.as_str()) => self
                .types
                .func_return_ctypes
                .get(name.as_str())
                .filter(|c| c.is_struct_or_union() || c.is_vector())
                .map_or(size, |c| self.ctype_size(c)),
            _ => size,
        };
        if crate::common::types::target_is_32bit() || size > 8 {
            return false;
        }
        let ret = self.call_aggregate_return(callee);
        ret.sret_size.is_none()
            && ret.two_reg_size.is_none()
            && !ret.one_xmm
            && !ret.x87
            && !ret.x87_pair
    }
}

//! Core lowering logic: AST -> alloca-based IR.
//!
//! This file contains the `Lowerer` struct and its primary methods:
//! - Construction and top-level orchestration (`lower()`)
//! - IR emission helpers (fresh_value, emit, terminate, etc.)
//! - Scope management delegation
//! - Enum/label helpers and string/array init helpers
//!
//! Function lowering pipeline is in `func_lowering.rs`, global declaration
//! lowering and declaration analysis are in `global_decl.rs`.
//!
//! Data structure definitions are in `definitions.rs`, per-function state in
//! `func_state.rs`, and type-system state in `frontend::sema::type_context`.

use super::definitions::*;
use super::func_state::FunctionBuildState;
use crate::backend::Target;
use crate::common::error::DiagnosticEngine;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::{AddressSpace, CType, IrType, SsoMode};
use crate::frontend::parser::ast::{
    self, DerivedDeclarator, Expr, ExprId, ExternalDecl, ParamDecl, TranslationUnit, TypeSpecifier,
};
use crate::frontend::sema::type_context::TypeContext;
use crate::frontend::sema::{ConstMap, ExprTypeMap, FunctionInfo};
use crate::ir::reexports::{
    BasicBlock, BlockId, CallInfo, Instruction, IrBinOp, IrCmpOp, IrConst, IrModule, IrUnaryOp,
    Operand, Terminator, Value,
};
use std::cell::RefCell;
use std::mem::Discriminant;

/// Lowers AST to IR (alloca-based, not yet SSA).
pub struct Lowerer {
    /// Target architecture, used for ABI-specific lowering decisions
    pub(super) target: Target,
    pub(super) next_label: u32,
    pub(super) next_string: u32,
    pub(super) next_anon_struct: u32,
    /// Counter for unique static local variable names
    pub(super) next_static_local: u32,
    pub(super) module: IrModule,
    /// Per-function build state. None between functions, Some during lowering.
    pub(super) func_state: Option<FunctionBuildState>,
    /// FP expression tags for the function being lowered (OP-36). Attached
    /// to the IrFunction at finalization; see [`Self::tag_fp_expr`].
    pub(super) fp_expr_tags: crate::common::fp_contract::FpExprTags,
    // Global variable tracking
    pub(super) globals: FxHashMap<String, GlobalInfo>,
    // Set of known function names
    pub(super) known_functions: FxHashSet<String>,
    // Set of already-defined function bodies
    pub(super) defined_functions: FxHashSet<String>,
    // Set of function names declared with static linkage
    pub(super) static_functions: FxHashSet<String>,
    /// Set of function names declared with __attribute__((error("..."))) or __attribute__((warning("..."))).
    /// Calls to these functions should be treated as unreachable (they are compile-time assertion traps).
    pub(super) error_functions: FxHashSet<String>,
    /// Set of function names declared with __attribute__((noreturn)) or _Noreturn.
    /// After calls to these functions, emit Unreachable to avoid generating dead epilogue code.
    pub(super) noreturn_functions: FxHashSet<String>,
    /// Set of function names declared with __attribute__((fastcall)).
    /// On i386, these use ecx/edx for the first two integer/pointer args.
    pub(super) fastcall_functions: FxHashSet<String>,
    /// Set of function names declared with __attribute__((pure)).
    pub pure_functions: FxHashSet<String>,
    /// Set of function names declared with __attribute__((const)).
    pub const_functions: FxHashSet<String>,
    /// Set while lowering a top-level expression whose result is discarded
    /// (expression statement, for-increment), so a discarded post-inc/dec can
    /// skip its volatile old-value spill.
    pub(super) discard_expr_result: bool,
    /// Set of function names that have at least one file-scope declaration
    /// without the `inline` specifier OR with `extern`. Per C99 6.7.4p7,
    /// an inline definition is only an "inline definition" (no external def)
    /// if ALL file-scope declarations include `inline` WITHOUT `extern`.
    /// If any declaration lacks `inline` or has `extern`, the definition
    /// provides an external definition.
    pub(super) has_non_inline_decl: FxHashSet<String>,
    /// Type-system state (struct layouts, typedefs, enum constants, type caches)
    pub(super) types: TypeContext,
    /// Metadata about known functions (consolidated FuncSig)
    pub(super) func_meta: FunctionMeta,
    /// Set of emitted global variable names (O(1) dedup)
    pub(super) emitted_global_names: FxHashSet<String>,
    /// String literal deduplication: maps string content → label.
    /// Identical string literals share the same .rodata entry so that
    /// pointer equality (`"hello" == "hello"`) holds, matching GCC behavior.
    pub(super) string_dedup: FxHashMap<String, String>,
    /// Wide and UTF-16 literals have different element widths and therefore
    /// need independent intern tables from ordinary byte strings.
    pub(super) wide_string_dedup: FxHashMap<String, String>,
    pub(super) char16_string_dedup: FxHashMap<String, String>,
    /// Function signatures from semantic analysis.
    /// Used as authoritative source for function return types and parameter types,
    /// reducing the lowerer's need to re-derive type information from the raw AST.
    pub(super) sema_functions: FxHashMap<String, FunctionInfo>,
    /// Expression type annotations from semantic analysis.
    /// Maps `ExprId` keys to their sema-inferred CTypes.
    /// Consulted as a fast O(1) fallback in get_expr_ctype() before the lowerer
    /// does its own (more expensive) type inference using lowering-specific state.
    pub(super) sema_expr_types: ExprTypeMap,
    /// Pre-computed constant expression values from semantic analysis.
    /// Maps `ExprId` keys to their IrConst values.
    /// Consulted as an O(1) fast path in eval_const_expr() before the lowerer
    /// falls back to its own evaluation (which handles lowering-specific cases
    /// like global addresses, func_state const locals, etc.).
    pub(super) sema_const_values: ConstMap,
    /// Stack of local label scopes from GNU __label__ declarations.
    /// Each entry maps a label name to its scope-qualified name.
    /// When resolving a label, the stack is searched top-down so that
    /// inner __label__ declarations shadow outer ones.
    pub(super) local_label_scopes: Vec<FxHashMap<String, String>>,
    /// Counter for generating unique local label scope IDs.
    pub(super) next_local_label_scope: u32,
    /// Maps C function/variable names to linker symbol overrides from __asm__("label").
    /// E.g., `extern int strerror_r(...) __asm__("__xpg_strerror_r")` maps
    /// "strerror_r" -> "__xpg_strerror_r". Used to redirect calls/references at IR emission.
    pub(super) asm_label_map: FxHashMap<String, String>,
    /// Versioned symbol references from top-level `.symver real, name@VER`
    /// where `real` is NOT defined in this TU. GCC semantics: all references
    /// to `real` become references to `name@VER` (glibc
    /// `compat_symbol_reference`; GNU ld 2.47 refuses to bind unversioned
    /// refs to `foo@VER` definitions under --whole-archive).
    pub(super) symver_ref_map: FxHashMap<String, String>,
    /// Memoization cache for get_expr_ctype().
    /// Maps `ExprId` keys to their resolved CType plus the Expr discriminant at
    /// insertion time. The discriminant is checked on cache hit to detect address
    /// reuse (ABA): expressions inside TypeSpecifier trees (typeof, _Generic) can
    /// share addresses with different Expr variants in the main AST, so a stale
    /// hit from a prior discriminant must be treated as a miss to avoid returning
    /// the wrong type.
    ///
    /// TODO: Once ExprId is backed by a counter-based scheme instead of pointer
    /// addresses, the ABA discriminant check can be removed entirely.
    pub(super) expr_ctype_cache: RefCell<FxHashMap<ExprId, (Discriminant<Expr>, Option<CType>)>>,
    /// Diagnostic engine for emitting structured errors and warnings during lowering.
    /// Threaded from the driver through the compilation pipeline so that lowering-phase
    /// diagnostics get the same source location rendering and warning control as
    /// parser/sema diagnostics.
    /// Wrapped in RefCell because many lowering methods take &self (not &mut self)
    /// but still need to emit diagnostics (same pattern as expr_ctype_cache).
    pub(super) diagnostics: RefCell<DiagnosticEngine>,
    /// Maps `*const Expr` identity of compound literal nodes to their materialized
    /// anonymous global names. Populated by `materialize_compound_literals_in_expr()`
    /// before `eval_global_addr_expr()` is called, so that compound literals
    /// embedded in pointer arithmetic (e.g., `(char*)(int[]){1,2,3} + 8`)
    /// can be resolved as global addresses.
    ///
    /// Lifetime: entries persist for the entire lowering session since the AST
    /// (and therefore Expr node addresses) lives through the full lowering pass.
    /// If the same compound literal appears in multiple globals, it will be
    /// materialized once and reused via this cache.
    ///
    /// Precondition: callers of `eval_global_addr_expr()` that may encounter
    /// compound literals in arithmetic contexts must first call
    /// `materialize_compound_literals_in_expr()` to populate this map.
    pub(super) materialized_compound_literals: FxHashMap<usize, String>,
    /// Whether GNU89 inline semantics are in effect (-fgnu89-inline, -std=c89, etc).
    /// When true, `extern inline` without `__attribute__((gnu_inline__))` is treated
    /// as an inline-only definition (no external def emitted), matching the behaviour
    /// of GCC in GNU89 mode. The default (false) uses C99 inline semantics.
    pub(super) gnu89_inline: bool,
}

/// Largest zero-initialised aggregate region lowered as an explicit store
/// ladder (see `zero_init_region`).  16 eight-byte stores: Clang's
/// `MaxStoresPerMemset` at the scalar width; GCC's `clear_by_pieces` gives up
/// earlier (CLEAR_RATIO 6 at generic tuning), so this is the *larger* of the
/// two oracles' ladders and never trades a store the passes could fold for a
/// call they cannot see through.
pub(crate) const ZERO_INIT_STORE_LADDER_MAX: usize = 128;

impl Lowerer {
    /// Create a new Lowerer with pre-populated state from semantic analysis.
    ///
    /// The sema-provided TypeContext contains typedefs, enum constants, struct layouts,
    /// and function typedef info collected during the sema pass.
    ///
    /// The sema-provided function map contains function signatures (return types,
    /// parameter types, variadic status) collected during sema. This is used to:
    /// - Pre-populate `known_functions` so function names are recognized immediately
    /// - Pre-populate `func_return_ctypes` for function call return type resolution
    /// - Provide authoritative CType info for `get_expr_ctype` fallback
    ///
    /// The sema-provided expr_types map contains CType annotations for AST expression
    /// nodes, keyed by their pointer address. The lowerer consults this as a fast
    /// fallback in get_expr_ctype() before doing its own type inference.
    pub fn with_type_context(
        target: Target,
        type_context: TypeContext,
        sema_functions: FxHashMap<String, FunctionInfo>,
        sema_expr_types: ExprTypeMap,
        sema_const_values: ConstMap,
        diagnostics: DiagnosticEngine,
        gnu89_inline: bool,
    ) -> Self {
        // Pre-populate known_functions from sema's function map.
        // This means the lowerer knows about all functions before the first pass,
        // which helps with early identifier resolution.
        let mut known_functions = FxHashSet::default();
        for name in sema_functions.keys() {
            known_functions.insert(name.clone());
        }

        Self {
            target,
            next_label: 0,
            next_string: 0,
            next_anon_struct: 0,
            next_static_local: 0,
            module: IrModule::new(),
            func_state: None,
            fp_expr_tags: Default::default(),
            globals: FxHashMap::default(),
            known_functions,
            defined_functions: FxHashSet::default(),
            static_functions: FxHashSet::default(),
            error_functions: FxHashSet::default(),
            noreturn_functions: FxHashSet::default(),
            fastcall_functions: FxHashSet::default(),
            pure_functions: FxHashSet::default(),
            const_functions: FxHashSet::default(),
            discard_expr_result: false,
            has_non_inline_decl: FxHashSet::default(),
            types: type_context,
            func_meta: FunctionMeta::default(),
            emitted_global_names: FxHashSet::default(),
            string_dedup: FxHashMap::default(),
            wide_string_dedup: FxHashMap::default(),
            char16_string_dedup: FxHashMap::default(),
            sema_functions,
            sema_expr_types,
            sema_const_values,
            local_label_scopes: Vec::new(),
            next_local_label_scope: 0,
            asm_label_map: FxHashMap::default(),
            symver_ref_map: FxHashMap::default(),
            expr_ctype_cache: RefCell::new(FxHashMap::default()),
            diagnostics: RefCell::new(diagnostics),
            materialized_compound_literals: FxHashMap::default(),
            gnu89_inline,
        }
    }

    // --- State accessors ---

    /// Access the current function build state (panics if not inside a function).
    #[inline]
    pub(super) fn func(&self) -> &FunctionBuildState {
        self.func_state.as_ref().expect("not inside a function")
    }

    /// Mutably access the current function build state (panics if not inside a function).
    #[inline]
    pub(super) fn func_mut(&mut self) -> &mut FunctionBuildState {
        self.func_state.as_mut().expect("not inside a function")
    }

    /// Returns true if the target is x86 (either x86-64 or i686).
    /// Used to select x87 80-bit extended precision format for long double memory layout.
    pub(super) fn is_x86(&self) -> bool {
        self.target == Target::X86_64 || self.target == Target::I686
    }

    /// Returns true if the target is RISC-V 64.
    /// RISC-V stores full IEEE binary128 long doubles in memory.
    pub(super) fn is_riscv(&self) -> bool {
        self.target == Target::Riscv64
    }

    /// Returns true if the target is AArch64.
    /// ARM64 stores full IEEE binary128 long doubles in memory.
    pub(super) fn is_arm(&self) -> bool {
        self.target == Target::Aarch64
    }

    /// Returns true if the function has `extern inline` semantics that do NOT
    /// emit an external definition. This covers two cases:
    ///
    ///   1. `extern inline` + `__attribute__((gnu_inline__))` (explicit GNU inline attr)
    ///   2. `extern inline` in GNU89 mode (`-std=c89`, `-fgnu89-inline`) without the attr
    ///
    /// In both cases, the function body is available for inlining but the compiler
    /// must not emit a standalone global symbol.
    pub(super) fn is_gnu_inline_no_extern_def(
        &self,
        attrs: &crate::frontend::parser::ast::FunctionAttributes,
    ) -> bool {
        attrs.is_inline() && attrs.is_extern() && (attrs.is_gnu_inline() || self.gnu89_inline)
    }

    /// Returns true if the target uses x86-64 style packed _Complex float ABI
    /// (two F32s packed into a single F64/xmm register).
    /// Returns false for ARM/RISC-V which pass _Complex float as two separate F32 registers.
    /// Returns false for i686 where _Complex float is passed/returned as a struct (8 bytes
    /// in eax:edx or on the stack), not packed into an XMM register.
    pub(super) fn uses_packed_complex_float(&self) -> bool {
        self.target == Target::X86_64
    }

    /// Returns true if the target packs _Complex float into a single 8-byte value
    /// for variadic function calls. On RISC-V, variadic float args go through GP
    /// registers, so _Complex float (2xF32) must be packed into one I64 GP register.
    pub(super) fn packs_complex_float_variadic(&self) -> bool {
        self.target == Target::Riscv64
    }

    /// Returns true if the target decomposes _Complex long double into 2 F128 scalar
    /// components for function argument/parameter passing.
    /// On ARM64 (AAPCS64): _Complex long double is an HFA passed in Q0/Q1 registers,
    ///   so we decompose into 2 F128 values.
    /// On x86-64: _Complex long double is passed on the stack (MEMORY class), not decomposed.
    /// On RISC-V: _Complex long double is passed by reference (pointer), not decomposed.
    pub(super) fn decomposes_complex_long_double(&self) -> bool {
        self.target == Target::Aarch64
    }

    /// Returns true if the target returns _Complex long double via register pairs
    /// rather than via sret hidden pointer.
    /// On x86-64: true (COMPLEX_X87 class, returned in st(0) and st(1)).
    /// On ARM64: true (HFA with 2 quad-precision members, returned in q0/q1).
    /// On i686: false (24 bytes > 8-byte reg pair; uses sret hidden pointer like GCC).
    /// On RISC-V: false (returned via sret hidden pointer).
    pub(super) fn returns_complex_long_double_in_regs(&self) -> bool {
        self.target == Target::X86_64 || self.target == Target::Aarch64
    }

    /// Returns true if the target decomposes _Complex double into two separate
    /// FP scalar arguments/returns (real, imag) rather than passing as a struct.
    /// On x86-64: true (xmm0/xmm1 for return, separate XMM regs for params).
    /// On ARM64: true (d0/d1 for return, separate FP regs for params).
    /// On RISC-V: true (fa0/fa1 for return, separate FP regs for params).
    /// On i686: false (16 bytes > 8; uses sret for return, struct-on-stack for params).
    pub(super) fn decomposes_complex_double(&self) -> bool {
        self.target != Target::I686
    }

    /// Returns true if the target decomposes _Complex float into two separate
    /// FP scalar arguments rather than passing as a struct.
    /// On i686: false (8 bytes fits in eax:edx for return, passed as struct on stack).
    /// On all 64-bit targets: true (decomposed into two FP register args).
    pub(super) fn decomposes_complex_float(&self) -> bool {
        self.target != Target::I686
    }

    // --- Diagnostic helpers ---

    /// Emit a warning diagnostic with a source span.
    /// Uses RefCell for interior mutability so this can be called from &self methods.
    pub(super) fn emit_warning(&self, message: impl Into<String>, span: Span) {
        self.diagnostics.borrow_mut().warning(message, span);
    }

    /// Look up the shared type metadata for a variable by name.
    ///
    /// Checks locals first, then globals. Returns `&VarInfo` which provides
    /// access to the shared fields (ty, elem_size, is_array, pointee_type,
    /// struct_layout, is_struct, array_dim_strides, c_type).
    pub(super) fn lookup_var_info(&self, name: &str) -> Option<&VarInfo> {
        if let Some(ref fs) = self.func_state {
            if let Some(info) = fs.locals.get(name) {
                return Some(&info.var);
            }
        }
        if let Some(ginfo) = self.globals.get(name) {
            return Some(&ginfo.var);
        }
        None
    }

    /// Resolve the CType of a struct/union field, handling both direct member access
    /// (s.field) and pointer member access (p->field) through a single entry point.
    pub(super) fn resolve_field_ctype(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer_access: bool,
    ) -> Option<CType> {
        self.resolve_member_field_ctype_impl(base_expr, field_name, is_pointer_access)
    }

    // --- Scope management delegation ---

    /// Push a new scope frame onto both TypeContext and FunctionBuildState scope stacks.
    /// Call this at the start of a compound statement or function body.
    pub(super) fn push_scope(&mut self) {
        self.types.push_scope();
        self.func_mut().push_scope();
    }

    /// Pop the top scope frame from both TypeContext and FunctionBuildState,
    /// undoing all scoped changes made in that scope.
    /// Emits cleanup function calls for variables with __attribute__((cleanup(func)))
    /// in reverse declaration order, then restores the stack pointer for VLAs.
    pub(super) fn pop_scope(&mut self) {
        let (scope_stack_save, cleanup_vars) = self.func_mut().pop_scope();
        self.types.pop_scope();
        // Emit cleanup calls in reverse declaration order (C semantics: last declared = first cleaned up)
        self.emit_cleanup_calls(&cleanup_vars);
        // If this scope contained VLA declarations, restore the stack pointer
        // to reclaim the dynamically-allocated stack space.
        if let Some(save_val) = scope_stack_save {
            self.emit(Instruction::StackRestore { ptr: save_val });
        }
    }

    /// Emit cleanup function calls for variables with __attribute__((cleanup(func))).
    /// Calls func(&var) for each cleanup variable, in reverse declaration order.
    pub(super) fn emit_cleanup_calls(&mut self, cleanup_vars: &[(String, Value)]) {
        for (func_name, alloca_val) in cleanup_vars.iter().rev() {
            let dest = Some(self.fresh_value());
            self.emit(Instruction::Call {
                func: func_name.clone(),
                info: CallInfo {
                    dest,
                    args: vec![Operand::Value(*alloca_val)],
                    arg_types: vec![IrType::Ptr],
                    return_type: IrType::Void,
                    is_variadic: false,
                    num_fixed_args: 1,
                    struct_arg_sizes: vec![None],
                    struct_arg_aligns: vec![],
                    struct_arg_classes: Vec::new(),
                    struct_arg_riscv_float_classes: Vec::new(),
                    struct_arg_is_f128_sse: Vec::new(),
                    is_sret: false,
                    is_fastcall: false,
                    is_pure: false,
                    is_const: false,
                    ret_eightbyte_classes: Vec::new(),
                    ret_is_f128_sse: false,
                },
            });
        }
    }

    /// Collect all cleanup variables from all active scopes (for return statements).
    /// Returns cleanup vars from innermost scope to outermost scope, each scope's
    /// vars in reverse declaration order.
    pub(super) fn collect_all_scope_cleanup_vars(&self) -> Vec<(String, Value)> {
        self.collect_scope_cleanup_vars_above_depth(0)
    }

    /// Collect cleanup variables from scopes above `target_depth` (for break/continue).
    /// This collects from the innermost scope down to (but not including) the scope at
    /// target_depth, with each scope's vars in reverse declaration order.
    pub(super) fn collect_scope_cleanup_vars_above_depth(
        &self,
        target_depth: usize,
    ) -> Vec<(String, Value)> {
        let func = self.func();
        let mut all_cleanups = Vec::new();
        // Walk scopes from innermost to outermost, stopping at target_depth
        for frame in func.scope_stack[target_depth..].iter().rev() {
            for (func_name, alloca_val) in frame.cleanup_vars.iter().rev() {
                all_cleanups.push((func_name.clone(), *alloca_val));
            }
        }
        all_cleanups
    }

    /// Remove a local variable, tracking the removal in the current scope frame.
    pub(super) fn shadow_local_for_scope(&mut self, name: &str) {
        self.func_mut().shadow_local_for_scope(name);
    }

    /// Remove a static local name, tracking the removal in the current scope frame.
    pub(super) fn shadow_static_for_scope(&mut self, name: &str) {
        self.func_mut().shadow_static_for_scope(name);
    }

    /// Insert a local variable, tracking the change in the current scope frame.
    pub(super) fn insert_local_scoped(&mut self, name: String, info: LocalInfo) {
        self.func_mut().insert_local_scoped(name, info);
    }

    /// Insert an enum constant, tracking the change in the current scope frame.
    pub(super) fn insert_enum_scoped(&mut self, name: String, value: i64) {
        self.types.insert_enum_scoped(name, value);
    }

    /// Insert a static local name, tracking the change in the current scope frame.
    pub(super) fn insert_static_local_scoped(&mut self, name: String, mangled: String) {
        self.func_mut().insert_static_local_scoped(name, mangled);
    }

    /// Insert a const local value, tracking the change in the current scope frame.
    pub(super) fn insert_const_local_scoped(&mut self, name: String, value: i64) {
        self.func_mut().insert_const_local_scoped(name, value);
    }

    // --- Top-level orchestration ---

    pub fn lower(mut self, tu: &TranslationUnit) -> (IrModule, DiagnosticEngine) {
        // Sema has already populated TypeContext with typedefs, enum constants,
        // struct/union layouts, function typedefs, and function pointer typedefs.
        // We only need to seed target-dependent builtin typedefs (va_list, size_t, etc.)
        // and libc math function signatures that sema doesn't know about.
        self.seed_builtin_typedefs();
        self.seed_libc_math_functions();

        // Mark transparent_union on union StructLayouts before the first pass,
        // so that register_function_meta can exclude them from param_struct_sizes.
        for decl in &tu.decls {
            if let ExternalDecl::Declaration(decl) = decl {
                if decl.is_transparent_union() {
                    self.mark_transparent_union(decl);
                }
            }
        }

        // Pre-pass: update typedefs that have __attribute__((vector_size(N))) or
        // __attribute__((ext_vector_type(N))).
        // Sema doesn't handle vector_size, so the typedef map has the unwrapped base type.
        // We need to apply vector_size wrapping before registering function metadata,
        // so that function return types using vector typedefs are correctly resolved.
        let mut has_vector_typedefs = false;
        for decl in &tu.decls {
            if let ExternalDecl::Declaration(decl) = decl {
                if decl.is_typedef() {
                    for declarator in &decl.declarators {
                        if !declarator.name.is_empty() {
                            let base_ctype =
                                self.build_full_ctype(&decl.type_spec, &declarator.derived);
                            let elem_size = base_ctype.size();
                            if let Some(vs) = decl.resolve_vector_size(elem_size) {
                                has_vector_typedefs = true;
                                let vec_ctype = CType::Vector(Box::new(base_ctype), vs);
                                self.types
                                    .typedefs
                                    .insert(declarator.name.clone(), vec_ctype);
                            }
                        }
                    }
                }
            }
        }

        // After updating vector typedefs, re-compute struct/union layouts that may
        // contain vector typedef fields. Sema computed these layouts before vector_size
        // was applied, so their field sizes may be wrong (e.g., float4 was treated as
        // float instead of vector(float, 16)). We must update the EXISTING layout
        // entries (by key) rather than creating new ones, because CType::Union/Struct
        // references use the key assigned by sema.
        if has_vector_typedefs {
            self.recompute_vector_struct_layouts(tu);
        }

        // Pre-pass: register struct/union layouts from all declarations and function
        // definitions so that function signature registration (below) can compute
        // correct SysV eightbyte classification for struct return types.
        // Without this, structs/unions defined via typedef before a function that
        // returns them would not have their layout available during classify_sysv_eightbytes.
        for decl in &tu.decls {
            match decl {
                ExternalDecl::Declaration(decl) => {
                    self.register_struct_type(&decl.type_spec);
                }
                ExternalDecl::FunctionDef(func) => {
                    self.register_struct_type(&func.return_type);
                    for p in &func.params {
                        self.register_struct_type(&p.type_spec);
                    }
                }
                ExternalDecl::TopLevelAsm(_) => {}
            }
        }

        // First pass: collect all function signatures (return types, param types,
        // variadic status, sret) so we can distinguish functions from globals and
        // insert proper casts/ABI handling during lowering.
        for decl in &tu.decls {
            if let ExternalDecl::FunctionDef(func) = decl {
                if func.attrs.is_pure() {
                    self.pure_functions.insert(func.name.clone());
                }
                if func.attrs.is_const_attr() {
                    self.const_functions.insert(func.name.clone());
                }
                self.register_function_meta(
                    &func.name,
                    &func.return_type,
                    0,
                    &func.params,
                    func.variadic,
                    func.attrs.is_static(),
                    func.is_kr,
                );
                // GNU C nested functions: register their signatures under
                // the parent-mangled name (recursively for deeper nesting)
                // so call sites find ABI metadata during lowering.
                self.register_nested_function_metas(&func.name, &func.body);
            }
            if let ExternalDecl::Declaration(decl) = decl {
                for declarator in &decl.declarators {
                    // Collect __asm__("label") linker symbol redirects on function declarations.
                    // When a function declaration has __asm__("symbol"), calls to that function
                    // should emit the asm label as the linker symbol instead of the C name.
                    // E.g.: extern int strerror_r(...) __asm__("__xpg_strerror_r");
                    // Note: register variables also use asm_register for register pinning
                    // (e.g., `register int x __asm__("rbx")`), which is handled separately
                    // in lower_global_decl. We only redirect function declarations here.
                    if let Some(ref asm_label) = declarator.attrs.asm_register {
                        let is_function_decl = declarator
                            .derived
                            .iter()
                            .any(|d| matches!(d, DerivedDeclarator::Function(_, _)));
                        if is_function_decl || !is_x86_register_name(asm_label) {
                            self.asm_label_map
                                .insert(declarator.name.clone(), asm_label.clone());
                            self.module
                                .asm_labels
                                .insert(declarator.name.clone(), asm_label.clone());
                        }
                    }

                    // Collect __asm__("label") linker symbol redirects on FILE-SCOPE
                    // variable declarations too (`extern int dfoo __asm("abccb");`).
                    // GCC emits the definition/aliases under the asm name; glibc's
                    // configure probes this exact pattern (`working alias attribute
                    // support`). Global register variables are unaffected: their
                    // references take the read_global_register path before the
                    // asm_label_map lookup, and they never carry a definition.
                    if let Some(ref asm_label) = declarator.attrs.asm_register {
                        let is_function_decl = declarator
                            .derived
                            .iter()
                            .any(|d| matches!(d, DerivedDeclarator::Function(_, _)));
                        if !is_function_decl
                            && !is_x86_register_name(asm_label)
                            && !decl.is_typedef()
                            && !declarator.name.is_empty()
                            && !self.asm_label_map.contains_key(&declarator.name)
                        {
                            self.asm_label_map
                                .insert(declarator.name.clone(), asm_label.clone());
                            self.module
                                .asm_labels
                                .insert(declarator.name.clone(), asm_label.clone());
                        }
                    }

                    // Find the Function derived declarator and count preceding Pointer derivations
                    let mut ptr_count = 0;
                    let mut func_info = None;
                    for d in &declarator.derived {
                        match d {
                            DerivedDeclarator::Pointer => ptr_count += 1,
                            DerivedDeclarator::Function(p, v) => {
                                func_info = Some((p.clone(), *v));
                                break;
                            }
                            _ => {}
                        }
                    }
                    if let Some((params, variadic)) = func_info {
                        self.register_function_meta(
                            &declarator.name,
                            &decl.type_spec,
                            ptr_count,
                            &params,
                            variadic,
                            decl.is_static(),
                            false,
                        );
                        // C99 6.7.4p7: Track function declarations that would make
                        // an inline definition provide an external definition.
                        // An inline definition is "inline only" (no external def) ONLY
                        // if ALL file-scope declarations include `inline` WITHOUT `extern`.
                        // So: if any declaration lacks `inline` OR has `extern`, mark it.
                        if !declarator.name.is_empty() && (!decl.is_inline() || decl.is_extern()) {
                            self.has_non_inline_decl.insert(declarator.name.clone());
                        }
                    } else if declarator.derived.is_empty() {
                        // Check if the base type is a function typedef
                        // (e.g., `func_t add;` where func_t is typedef int func_t(int);)
                        // Only when derived is empty — `func_t *callback;` is a variable,
                        // not a function declaration.
                        if let TypeSpecifier::TypedefName(tname) = &decl.type_spec {
                            if let Some(fti) = self.types.function_typedefs.get(tname).cloned() {
                                self.register_function_meta(
                                    &declarator.name,
                                    &fti.return_type,
                                    0,
                                    &fti.params,
                                    fti.variadic,
                                    false,
                                    false,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Pre-pass: versioned .symver references (GCC semantics). glibc's
        // compat_symbol_reference emits `.symver real, name@VER` as top-level
        // asm; when `real` is NOT defined in this TU, every reference to it
        // must become a versioned reference `name@VER` (GNU ld 2.47 refuses
        // to bind unversioned refs to `foo@VER` under --whole-archive).
        // Must run BEFORE body lowering so references lower correctly.
        {
            let mut defined: crate::common::fx_hash::FxHashSet<String> =
                crate::common::fx_hash::FxHashSet::default();
            // Function DEFINITIONS (a mere declaration does not count: glibc's
            // compat_symbol_reference versionizes references to declared-only
            // functions like matherr).
            for decl in &tu.decls {
                if let ExternalDecl::FunctionDef(func) = decl {
                    defined.insert(func.name.clone());
                }
            }
            for decl in &tu.decls {
                if let ExternalDecl::Declaration(decl) = decl {
                    for declarator in &decl.declarators {
                        // An `__attribute__((alias("target")))` declarator IS a
                        // definition in this TU, even though it parses as an
                        // extern declaration without an initializer. glibc's
                        // libm_alias_finite hits exactly this:
                        //   strong_alias (__j0f, __ieee754_j0f)      // alias def
                        //   .symver __ieee754_j0f,__j0f_finite@GLIBC_2.15
                        // GAS semantics for a symbol DEFINED in the object are
                        // "create versioned alias for export"; references to
                        // the plain name stay bound to the local definition.
                        // Misclassifying the alias as undefined applied the
                        // reference-versioning flavour instead and rewrote
                        // `call __ieee754_j0f` into a dangling reference to
                        // `__j0f_finite` (libm.so: undefined __j0f_finite).
                        if !declarator.name.is_empty()
                            && (declarator.init.is_some()
                                || !decl.is_extern()
                                || declarator.attrs.alias_target.is_some())
                        {
                            defined.insert(declarator.name.clone());
                        }
                    }
                }
            }
            for decl in &tu.decls {
                if let ExternalDecl::TopLevelAsm(asm_str) = decl {
                    for line in asm_str.lines() {
                        let t = line.trim();
                        if let Some(rest) = t.strip_prefix(".symver") {
                            let rest = rest.trim();
                            // Optional third operand (GAS 2.35+):
                            // `.symver real, name@VER, {local|hidden|remove}`.
                            // The flag governs the DEFINED plain symbol and is
                            // handled by the assembler; for the undefined-
                            // reference case handled here it changes nothing
                            // (verified against GAS: `.symver und, und@V,
                            // remove` still just versionizes references), so
                            // strip it rather than corrupt the version string.
                            let mut fields = rest.split(',');
                            let real = fields.next().unwrap_or("").trim().to_string();
                            let alias_ver = fields.next().unwrap_or("").trim().to_string();
                            if !real.is_empty()
                                && alias_ver.contains('@')
                                && !defined.contains(&real)
                            {
                                self.symver_ref_map.insert(real, alias_ver);
                            }
                        }
                    }
                }
            }
        }

        // Collect constructor/destructor/alias attributes from function definitions and declarations
        for decl in &tu.decls {
            match decl {
                ExternalDecl::FunctionDef(func) => {
                    if func.attrs.is_constructor() && !self.module.constructors.contains(&func.name)
                    {
                        self.module.constructors.push(func.name.clone());
                    }
                    if func.attrs.is_destructor() && !self.module.destructors.contains(&func.name) {
                        self.module.destructors.push(func.name.clone());
                    }
                    if func.attrs.is_fastcall() {
                        self.fastcall_functions.insert(func.name.clone());
                    }
                }
                ExternalDecl::Declaration(decl) => {
                    for declarator in &decl.declarators {
                        if declarator.attrs.is_constructor()
                            && !declarator.name.is_empty()
                            && !self.module.constructors.contains(&declarator.name)
                        {
                            self.module.constructors.push(declarator.name.clone());
                        }
                        if declarator.attrs.is_destructor()
                            && !declarator.name.is_empty()
                            && !self.module.destructors.contains(&declarator.name)
                        {
                            self.module.destructors.push(declarator.name.clone());
                        }
                        // Collect __attribute__((alias("target"))) declarations.
                        // The alias name must honour the declaration's asm label
                        // (`extern int foo __asm("xyzzy") __attribute__((alias("bar")))`
                        // emits `.set xyzzy,bar`, matching GCC; glibc's configure
                        // depends on this exact behaviour).
                        if let Some(ref target) = declarator.attrs.alias_target {
                            if !declarator.name.is_empty() {
                                let asm_name = self
                                    .asm_label_map
                                    .get(&declarator.name)
                                    .cloned()
                                    .unwrap_or_else(|| declarator.name.clone());
                                self.module.aliases.push((
                                    asm_name,
                                    target.clone(),
                                    declarator.attrs.is_weak(),
                                ));
                            }
                        }
                        // Collect __attribute__((symver("..."))) declarations
                        if let Some(ref sv) = declarator.attrs.symver {
                            if !declarator.name.is_empty() {
                                self.module
                                    .symver_directives
                                    .push((declarator.name.clone(), sv.clone()));
                            }
                        }
                        // Collect __attribute__((error("..."))) / __attribute__((warning("...")))
                        if declarator.attrs.is_error_attr() && !declarator.name.is_empty() {
                            self.error_functions.insert(declarator.name.clone());
                        }
                        // Collect __attribute__((noreturn)) / _Noreturn
                        if declarator.attrs.is_noreturn() && !declarator.name.is_empty() {
                            self.noreturn_functions.insert(declarator.name.clone());
                        }
                        // Collect __attribute__((fastcall))
                        if declarator.attrs.is_fastcall() && !declarator.name.is_empty() {
                            self.fastcall_functions.insert(declarator.name.clone());
                        }
                        // Collect __attribute__((pure))
                        if declarator.attrs.is_pure() && !declarator.name.is_empty() {
                            self.pure_functions.insert(declarator.name.clone());
                        }
                        // Collect __attribute__((const))
                        if declarator.attrs.is_const_attr() && !declarator.name.is_empty() {
                            self.const_functions.insert(declarator.name.clone());
                        }
                        // Collect weak/visibility attributes on extern declarations (not aliases).
                        // Skip typedefs: they are not linker symbols and should never get
                        // .weak/.hidden directives. This matters when #pragma GCC visibility
                        // push(hidden) is active (e.g., kernel EFI stub) because it would
                        // otherwise emit .hidden for thousands of typedef names.
                        if !decl.is_typedef()
                            && declarator.attrs.alias_target.is_none()
                            && !declarator.name.is_empty()
                            && (declarator.attrs.is_weak() || declarator.attrs.visibility.is_some())
                        {
                            self.module.symbol_attrs.push((
                                declarator.name.clone(),
                                declarator.attrs.is_weak(),
                                declarator.attrs.visibility.clone(),
                            ));
                        }
                    }
                }
                ExternalDecl::TopLevelAsm(_) => {
                    // Handled in the third pass
                }
            }
        }

        // Pass 2.5: collect referenced static functions so we can skip unreferenced ones.
        // Static/inline functions from headers that are never called don't need to be lowered.
        let referenced_statics = self.collect_referenced_static_functions(tu);

        // Third pass: lower everything
        for decl in tu.decls.iter() {
            match decl {
                ExternalDecl::FunctionDef(func) => {
                    // Skip unreferenced static/static-inline functions (e.g., static inline
                    // from headers that are never called).
                    //
                    // GNU89/gnu_inline semantics (explicit __attribute__((gnu_inline)) OR
                    // -fgnu89-inline / -std=c89 mode):
                    //   extern inline = inline definition only, no external def
                    //   → treat like static inline (can skip if unreferenced, local if emitted)
                    //   inline (no extern) = external definition (must emit, global)
                    //
                    // C99 semantics (default for -std=c99 and later):
                    //   extern inline = provides external definition (must emit, global)
                    //   inline (alone) = inline definition only (no external def)
                    let is_gnu_inline_no_extern_def = self.is_gnu_inline_no_extern_def(&func.attrs);
                    // C99 6.7.4p7: A plain `inline` definition (without `extern`)
                    // does not provide an external definition. We lower these as
                    // static functions so their bodies are available for inlining.
                    // If unreferenced, they are skipped. If referenced but all calls
                    // are inlined, dead code elimination removes them.
                    // Note: this only applies in C99 mode; in GNU89 mode, `inline`
                    // without `extern` provides the external definition.
                    let is_c99_inline_only = !self.gnu89_inline
                        && func.attrs.is_inline()
                        && !func.attrs.is_extern()
                        && !func.attrs.is_static()
                        && !func.attrs.is_gnu_inline()
                        && !self.has_non_inline_decl.contains(&func.name);
                    let can_skip = if is_c99_inline_only {
                        // C99 inline-only functions don't provide an external definition.
                        // Lower them as static so their bodies are available for inlining.
                        // If all call sites are inlined, dead code elimination removes them.
                        // If not inlined, they're emitted as local symbols (safe fallback).
                        true
                    } else if func.attrs.is_static() {
                        // static or static inline: internal linkage, safe to skip if unreferenced
                        true
                    } else if is_gnu_inline_no_extern_def {
                        // extern inline (gnu89 or gnu_inline attr): no external def, skip if unreferenced
                        true
                    } else {
                        false
                    };
                    if can_skip
                        && !func.attrs.is_used()
                        && !func.attrs.is_constructor()
                        && !self.module.constructors.contains(&func.name)
                        && !func.attrs.is_destructor()
                        && !self.module.destructors.contains(&func.name)
                        && !referenced_statics.contains(&func.name)
                    {
                        continue;
                    }
                    self.lower_function(func);
                }
                ExternalDecl::Declaration(decl) => {
                    self.lower_global_decl(decl);
                }
                ExternalDecl::TopLevelAsm(asm_str) => {
                    self.module.toplevel_asm.push(asm_str.clone());
                }
            }
        }

        // Preserve function-vs-data identity through IR. Calls already carry a
        // dedicated Call instruction, but taking a function address lowers to
        // the same GlobalAddr shape as data. In PIE, ordinary extern data may
        // use a copy relocation while an interposable extern function pointer
        // must remain GOT-indirect. Resolve asm labels / symver aliases now so
        // this set uses exactly the names GlobalAddr emission will see.
        let extern_function_symbols: Vec<String> = self
            .known_functions
            .iter()
            .filter(|name| !self.defined_functions.contains(*name))
            .map(|name| self.resolve_ref_name(name))
            .collect();
        self.module
            .extern_function_symbols
            .extend(extern_function_symbols);

        (self.module, self.diagnostics.into_inner())
    }

    /// Resolve a symbol reference: versioned .symver references first (glibc
    /// compat_symbol_reference), then __asm__("label") redirects, else the
    /// plain name.
    pub(super) fn resolve_ref_name(&self, name: &str) -> String {
        if let Some(v) = self.symver_ref_map.get(name) {
            return v.clone();
        }
        self.asm_label_map
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Register function metadata (return type, param types, variadic, sret) for
    /// a function name. This shared helper eliminates the triplicated pattern in `lower()`
    /// where function definitions, extern declarations, and typedef-based declarations
    /// all needed to register the same metadata fields.
    ///
    /// When sema has already collected the function's CType info (via FunctionInfo),
    /// this method uses sema's return CType as source-of-truth instead of re-computing
    /// it from the AST TypeSpecifier. This reduces duplicated work and establishes
    /// sema as the authority on function type information.
    pub(super) fn register_function_meta(
        &mut self,
        name: &str,
        ret_type_spec: &TypeSpecifier,
        ptr_count: usize,
        params: &[ParamDecl],
        variadic: bool,
        is_static: bool,
        is_kr: bool,
    ) {
        self.known_functions.insert(name.to_string());
        if is_static {
            self.static_functions.insert(name.to_string());
        }

        // Compute the return CType once. Prefer sema's authoritative CType if available,
        // falling back to re-computing from the AST TypeSpecifier.
        // When sema provides the return type, it already includes pointer levels from
        // the declarator chain, so we must NOT add ptr_count layers again.
        // Only the AST fallback path needs ptr_count wrapping, since type_spec_to_ctype
        // only resolves the base type specifier without pointer derivators.
        let full_ret_ctype = if let Some(func_info) = self.sema_functions.get(name) {
            let sema_ct = func_info.return_type.clone();
            // Sema doesn't resolve vector_size attributes, so if the lowerer's
            // type_spec_to_ctype produces a Vector but sema doesn't, prefer the
            // lowerer's result which correctly includes vector information.
            if !sema_ct.is_vector() && ptr_count == 0 {
                let lowerer_ct = self.type_spec_to_ctype(ret_type_spec);
                if lowerer_ct.is_vector() {
                    lowerer_ct
                } else {
                    sema_ct
                }
            } else {
                sema_ct
            }
        } else {
            let mut ct = self.type_spec_to_ctype(ret_type_spec);
            for _ in 0..ptr_count {
                ct = CType::Pointer(Box::new(ct), AddressSpace::Default);
            }
            ct
        };

        // Compute return type, wrapping with pointer levels if needed
        let mut ret_ty = IrType::from_ctype(&full_ret_ctype);
        // Complex return types need special IR type overrides based on target ABI:
        //
        // x86-64:
        //   _Complex double: real in xmm0 (F64), imag in xmm1 -> IR ret type F64
        //   _Complex float: packed two F32 in xmm0 as F64 -> IR ret type F64
        //   _Complex long double: real in x87 st(0) -> IR ret type F128
        //
        // ARM64/RISC-V:
        //   _Complex double: real in d0 (F64), imag in d1 -> IR ret type F64
        //   _Complex float: real in s0 (F32), imag in s1 -> IR ret type F32
        //
        // i686:
        //   _Complex float (8 bytes): packed in eax:edx -> IR ret type I64
        //   _Complex double (16 bytes): sret hidden pointer (ret_ty stays Ptr)
        //   _Complex long double (24 bytes): sret hidden pointer (ret_ty stays Ptr)
        if ptr_count == 0 {
            if matches!(full_ret_ctype, CType::ComplexLongDouble)
                && self.returns_complex_long_double_in_regs()
            {
                // x86-64: real part in x87 st(0); ARM64: real part in q0
                ret_ty = IrType::F128;
            } else if matches!(full_ret_ctype, CType::ComplexDouble)
                && self.decomposes_complex_double()
            {
                // x86-64/ARM64/RISC-V: _Complex double returns real in FP reg
                ret_ty = IrType::F64;
            } else if matches!(full_ret_ctype, CType::ComplexFloat) {
                if self.uses_packed_complex_float() {
                    // x86-64: packed two F32 in one xmm register as F64
                    ret_ty = IrType::F64;
                } else if self.decomposes_complex_float() {
                    // ARM64/RISC-V: real in first FP register as F32
                    ret_ty = IrType::F32;
                } else {
                    // i686: _Complex float (8 bytes) fits in eax:edx, pack as I64
                    ret_ty = IrType::I64;
                }
            }
            // i686: ComplexDouble (16 bytes) and ComplexLongDouble (24 bytes) keep
            // ret_ty = IrType::Ptr (the default from_ctype mapping for complex types).
            // They will be handled via sret below.
            // i686: _Float128/_Decimal128 (TFmode/TDmode, 16 bytes) is class
            // MEMORY — GCC returns it through a hidden pointer (first stack
            // arg, callee-popped with `ret $4`). The IR return becomes Ptr and
            // the body stores the 16-byte carrier through the sret buffer
            // (try_sret_return). x86-64 keeps the U128/xmm0:xmm1 return.
            else if (matches!(full_ret_ctype, CType::Float128)
                || full_ret_ctype == CType::Decimal128)
                && crate::common::types::target_is_32bit()
            {
                ret_ty = IrType::Ptr;
            }
        }

        // Track CType for pointer-returning functions
        let return_ctype = if ret_ty == IrType::Ptr {
            Some(full_ret_ctype.clone())
        } else {
            None
        };

        // Record complex return types for expr_ctype resolution
        if ptr_count == 0 && full_ret_ctype.is_complex() {
            self.types
                .func_return_ctypes
                .insert(name.to_string(), full_ret_ctype.clone());
        }

        // Detect struct/complex/vector returns that need special ABI handling.
        let mut sret_size = None;
        let mut two_reg_ret_size = None;
        if ptr_count == 0 {
            if full_ret_ctype.is_struct_or_union() {
                let size = self.sizeof_type(ret_type_spec);
                let (s, t) = Self::classify_struct_return(size);
                sret_size = s;
                two_reg_ret_size = t;
            }
            // Vector types use the same by-value return convention as structs:
            // small vectors (<=8 bytes) are packed into a register, larger ones use sret.
            if full_ret_ctype.is_vector() {
                let size = self.sizeof_type(ret_type_spec);
                let (s, t) = Self::classify_struct_return(size);
                sret_size = s;
                two_reg_ret_size = t;
                // Small vector returns (<=8 bytes, no sret/two_reg) need I64 return type
                // so the packed vector data is returned in a register, not as a pointer.
                if s.is_none() && t.is_none() {
                    ret_ty = IrType::I64;
                }
            }
            if matches!(full_ret_ctype, CType::ComplexLongDouble) {
                // On x86-64, _Complex long double is returned via x87 st(0)/st(1),
                // not via hidden pointer (sret). On i686 and other targets, use sret.
                if !self.returns_complex_long_double_in_regs() {
                    let size = self.sizeof_type(ret_type_spec);
                    sret_size = Some(size);
                }
            }
            // On i686, _Complex double (16 bytes) exceeds the 8-byte register pair
            // (eax:edx), so it must use sret (hidden pointer return).
            // On 64-bit targets, _Complex double is returned in two FP registers.
            if matches!(full_ret_ctype, CType::ComplexDouble) && !self.decomposes_complex_double() {
                let size = full_ret_ctype.size();
                sret_size = Some(size);
            }
            // On i686, _Float128/_Decimal128 (TFmode/TDmode) is class MEMORY —
            // hidden-pointer return, matching GCC's i386 convention.
            if (matches!(full_ret_ctype, CType::Float128) || full_ret_ctype == CType::Decimal128)
                && crate::common::types::target_is_32bit()
            {
                sret_size = Some(16);
            }
        }

        // Compute SysV eightbyte classification for two-register struct returns.
        // _Float128 returns are 16-byte SSE-class values (SysV psABI): ONE XMM
        // register (xmm0).
        let mut ret_is_f128_sse = false;
        let ret_eightbyte_classes =
            if matches!(full_ret_ctype, CType::Float128) || full_ret_ctype == CType::Decimal128 {
                ret_is_f128_sse = true;
                vec![
                    crate::common::types::EightbyteClass::Sse,
                    crate::common::types::EightbyteClass::Sse,
                ]
            } else if two_reg_ret_size.is_some() && ptr_count == 0 {
                if full_ret_ctype.is_struct_or_union() || full_ret_ctype.is_vector() {
                    if let Some(layout) = self.get_struct_layout_for_type(ret_type_spec) {
                        layout.classify_sysv_eightbytes(&*self.types.borrow_struct_layouts())
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

        // Collect parameter types, with K&R default argument promotions.
        // Use sema's param CTypes when available to avoid re-computing from AST.
        let sema_param_ctypes = self.sema_functions.get(name).map(|fi| {
            fi.params
                .iter()
                .map(|(ct, _)| ct.clone())
                .collect::<Vec<_>>()
        });

        let param_tys: Vec<IrType> = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = if let Some(ref sema_cts) = sema_param_ctypes {
                    if let Some(ct) = sema_cts.get(i) {
                        IrType::from_ctype(ct)
                    } else {
                        self.type_spec_to_ir(&p.type_spec)
                    }
                } else {
                    self.type_spec_to_ir(&p.type_spec)
                };
                if is_kr {
                    match ty {
                        IrType::F32 => IrType::F64,
                        IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 => IrType::I32,
                        other => other,
                    }
                } else {
                    ty
                }
            })
            .collect();
        let param_bool_flags: Vec<bool> = params
            .iter()
            .map(|p| self.is_type_bool(&p.type_spec))
            .collect();
        // Collect parameter CTypes for complex argument conversion.
        // Prefer sema's authoritative CTypes when available.
        let param_ctypes: Vec<CType> = if let Some(sema_cts) = sema_param_ctypes {
            params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if let Some(ct) = sema_cts.get(i) {
                        ct.clone()
                    } else {
                        self.type_spec_to_ctype(&p.type_spec)
                    }
                })
                .collect()
        } else {
            params
                .iter()
                .map(|p| self.type_spec_to_ctype(&p.type_spec))
                .collect()
        };

        // Collect per-parameter struct sizes for by-value struct passing ABI.
        // ComplexLongDouble is included as a struct on platforms that don't decompose it
        // (x86-64, RISC-V) since it's passed like a struct (on stack / by reference).
        // On i686, ComplexDouble (16 bytes) and ComplexFloat (8 bytes) are also passed
        // as structs on the stack since they aren't decomposed into separate FP args.
        // Transparent unions are excluded — they are passed as their first member.
        let decomposes_cld = self.decomposes_complex_long_double();
        let decomposes_cd = self.decomposes_complex_double();
        let decomposes_cf = self.decomposes_complex_float();
        let param_struct_sizes: Vec<Option<usize>> = params
            .iter()
            .map(|p| {
                let ctype = self.type_spec_to_ctype(&p.type_spec);
                if self.is_type_struct_or_union(&p.type_spec)
                    && !self.is_transparent_union(&p.type_spec)
                {
                    Some(self.sizeof_type(&p.type_spec))
                } else if ctype.is_vector() {
                    // Vector types are passed by value like structs
                    Some(self.sizeof_type(&p.type_spec))
                } else if !decomposes_cld && matches!(ctype, CType::ComplexLongDouble) {
                    Some(self.sizeof_type(&p.type_spec))
                } else if !decomposes_cd && matches!(ctype, CType::ComplexDouble) {
                    Some(CType::ComplexDouble.size())
                } else if !decomposes_cf && matches!(ctype, CType::ComplexFloat) {
                    Some(CType::ComplexFloat.size())
                } else if matches!(ctype, CType::Float128) || ctype == CType::Decimal128 {
                    // _Float128 is passed as a 16-byte SSE-class value (SysV psABI).
                    Some(16)
                } else {
                    None
                }
            })
            .collect();

        // Compute per-eightbyte SysV ABI classification for struct params
        let param_struct_classes: Vec<Vec<crate::common::types::EightbyteClass>> = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if param_struct_sizes.get(i).copied().flatten().is_some() {
                    if matches!(self.type_spec_to_ctype(&p.type_spec), CType::Float128)
                        || self.type_spec_to_ctype(&p.type_spec) == CType::Decimal128
                    {
                        vec![
                            crate::common::types::EightbyteClass::Sse,
                            crate::common::types::EightbyteClass::Sse,
                        ]
                    } else if let Some(layout) = self.get_struct_layout_for_type(&p.type_spec) {
                        layout.classify_sysv_eightbytes(&*self.types.borrow_struct_layouts())
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            })
            .collect();

        // _Float128 params: ONE 16-byte XMM register per arg (SysV psABI).
        let param_is_f128_sse: Vec<bool> = params
            .iter()
            .map(|p| {
                matches!(self.type_spec_to_ctype(&p.type_spec), CType::Float128)
                    || self.type_spec_to_ctype(&p.type_spec) == CType::Decimal128
            })
            .collect();

        // Compute RISC-V LP64D float field classification for struct params
        let param_riscv_float_classes: Vec<Option<crate::common::types::RiscvFloatClass>> = params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if param_struct_sizes.get(i).copied().flatten().is_some() {
                    if let Some(layout) = self.get_struct_layout_for_type(&p.type_spec) {
                        layout.classify_riscv_float_fields(&*self.types.borrow_struct_layouts())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        let sig = if !variadic || !param_tys.is_empty() {
            FuncSig {
                return_type: ret_ty,
                return_ctype,
                param_types: param_tys,
                param_ctypes,
                param_bool_flags,
                is_variadic: variadic,
                sret_size,
                two_reg_ret_size,
                ret_eightbyte_classes: ret_eightbyte_classes.clone(),
                ret_is_f128_sse,
                param_struct_sizes,
                param_struct_classes,
                param_riscv_float_classes,
                param_is_f128_sse: param_is_f128_sse.clone(),
            }
        } else {
            FuncSig {
                return_type: ret_ty,
                return_ctype,
                param_types: Vec::new(),
                param_ctypes: Vec::new(),
                param_bool_flags: Vec::new(),
                is_variadic: variadic,
                sret_size,
                two_reg_ret_size,
                ret_eightbyte_classes,
                ret_is_f128_sse: false,
                param_struct_sizes: Vec::new(),
                param_struct_classes: Vec::new(),
                param_riscv_float_classes: Vec::new(),
                param_is_f128_sse,
            }
        };
        self.func_meta.sigs.insert(name.to_string(), sig);
    }

    // --- IR emission helpers ---

    pub(super) fn fresh_value(&mut self) -> Value {
        let v = Value(self.func_mut().next_value);
        self.func_mut().next_value += 1;
        v
    }

    pub(super) fn fresh_label(&mut self) -> BlockId {
        let l = BlockId(self.next_label);
        self.next_label += 1;
        if std::env::var("LCCC_DEBUG_LABELS").is_ok() {
            eprintln!(
                "[LABEL] Allocated BlockId({}) in function {:?}, next_label now {}",
                l.0,
                self.func_state.as_ref().map(|f| &f.name),
                self.next_label
            );
        }
        l
    }

    /// Intern a string literal: add it to the module's .rodata string table and
    /// return its unique label. Identical strings are deduplicated so that
    /// pointer equality holds (matching GCC behavior).
    pub(super) fn intern_string_literal(&mut self, s: &str) -> String {
        if let Some(existing) = self.string_dedup.get(s) {
            return existing.clone();
        }
        let label = format!(".Lstr{}", self.next_string);
        self.next_string += 1;
        self.module
            .string_literals
            .push((label.clone(), s.to_string()));
        self.string_dedup.insert(s.to_string(), label.clone());
        label
    }

    /// Intern a wide string literal (L"...") and return its label.
    /// Each character is stored as a u32 (wchar_t), plus a null terminator.
    pub(super) fn intern_wide_string_literal(&mut self, s: &str) -> String {
        if let Some(existing) = self.wide_string_dedup.get(s) {
            return existing.clone();
        }
        let label = format!(".Lwstr{}", self.next_string);
        self.next_string += 1;
        let mut chars: Vec<u32> = s.chars().map(|c| c as u32).collect();
        chars.push(0); // null terminator
        self.module
            .wide_string_literals
            .push((label.clone(), chars));
        self.wide_string_dedup.insert(s.to_string(), label.clone());
        label
    }

    /// Intern a char16_t string literal (u"...") and return its label.
    /// Each character is stored as a u16 (char16_t), plus a null terminator.
    pub(super) fn intern_char16_string_literal(&mut self, s: &str) -> String {
        if let Some(existing) = self.char16_string_dedup.get(s) {
            return existing.clone();
        }
        let label = format!(".Lc16str{}", self.next_string);
        self.next_string += 1;
        let mut chars: Vec<u16> = s.chars().map(|c| c as u16).collect();
        chars.push(0); // null terminator
        self.module
            .char16_string_literals
            .push((label.clone(), chars));
        self.char16_string_dedup
            .insert(s.to_string(), label.clone());
        label
    }

    pub(super) fn emit(&mut self, inst: Instruction) {
        let span = self.func_mut().current_span;
        self.func_mut().instrs.push(inst);
        self.func_mut().instr_spans.push(span);
    }

    /// Record the FP expression tag for `dest` (OP-36). Tags accumulate in
    /// a side buffer owned by the lowering state and are attached to the
    /// `IrFunction` when the function body is finalized.
    pub(super) fn tag_fp_expr(&mut self, dest: Value) {
        let root = self.func_mut().fp_expr_root;
        self.fp_expr_tags.entry(dest.0).or_insert(root);
    }

    /// Emit an alloca into the entry block buffer.
    /// Used for local variable declarations so that variables whose
    /// declarations are skipped by `goto` still have valid stack slots.
    pub(super) fn emit_entry_alloca(
        &mut self,
        ty: IrType,
        size: usize,
        align: usize,
        volatile: bool,
    ) -> Value {
        self.emit_entry_alloca_with_flags(ty, size, align, volatile, volatile)
    }

    /// Emit an entry alloca with independent SSA-promotion and C-volatile
    /// semantics.  Compiler-created spill homes may need to remain in memory
    /// without turning their ordinary loads/stores into observable volatile
    /// accesses.
    pub(super) fn emit_entry_alloca_with_flags(
        &mut self,
        ty: IrType,
        size: usize,
        align: usize,
        volatile: bool,
        semantic_volatile: bool,
    ) -> Value {
        let dest = self.fresh_value();
        self.func_mut().entry_allocas.push(Instruction::Alloca {
            dest,
            ty,
            size,
            align,
            volatile,
            semantic_volatile,
        });
        dest
    }

    /// Emit a binary operation and return the result Value.
    pub(super) fn emit_binop_val(
        &mut self,
        op: IrBinOp,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
    ) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::BinOp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        });
        dest
    }

    /// Emit a comparison and return the result Value (I32: 0 or 1).
    pub(super) fn emit_cmp_val(
        &mut self,
        op: IrCmpOp,
        lhs: Operand,
        rhs: Operand,
        ty: IrType,
    ) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Cmp {
            dest,
            op,
            lhs,
            rhs,
            ty,
        });
        dest
    }

    /// Emit a type cast and return the result Value.
    pub(super) fn emit_cast_val(&mut self, src: Operand, from_ty: IrType, to_ty: IrType) -> Value {
        let dest = self.fresh_value();
        self.emit(Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        });
        dest
    }

    // === Reverse scalar_storage_order helpers ===
    //
    // Fields of records with `__attribute__((scalar_storage_order("big-endian")))`
    // (on little-endian targets) are stored byte-reversed. The layout records the
    // per-field SsoMode; these helpers emit the byte-manipulation at access time.

    /// Byteswap the raw representation of an integer `val` at the width of `ty`.
    pub(super) fn emit_sso_bswap_int(&mut self, val: Operand, ty: IrType) -> Operand {
        let dest = self.fresh_value();
        self.emit(Instruction::UnaryOp {
            dest,
            op: IrUnaryOp::Bswap,
            src: val,
            ty,
        });
        Operand::Value(dest)
    }

    /// Bit-reinterpret `val` between two same-size types via a temporary
    /// alloca (no value conversion — IR Cast between int/float is a VALUE
    /// conversion, so raw reinterpretation must go through memory).
    pub(super) fn emit_bit_reinterpret(
        &mut self,
        val: Operand,
        from: IrType,
        to: IrType,
    ) -> Operand {
        debug_assert_eq!(from.size(), to.size(), "bit reinterpret requires same size");
        let tmp = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: tmp,
            ty: from,
            size: from.size(),
            align: from.size(),
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val,
            ptr: tmp,
            ty: from,
            seg_override: AddressSpace::Default,
        });
        let dest = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest,
            ptr: tmp,
            ty: to,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(dest)
    }

    /// Reverse the bits of each BYTE of `val` (an integer of type `ty`),
    /// preserving byte order. Used for packed bitfields under reverse SSO,
    /// where the bit stream fills each byte MSB-first. Implemented with the
    /// classic 3-step shift/mask ladder (BitReverse is not available on i686).
    pub(super) fn emit_sso_bitrev_bytes(&mut self, val: Operand, ty: IrType) -> Operand {
        let uty = ty.to_unsigned();
        let start = self.emit_implicit_cast(val, ty, uty);
        let mut cur = self.operand_to_value(start);
        let bytes = uty.size();
        for i in 0..bytes {
            let shift = 8 * i as i64;
            // Extract byte i to bit position 0.
            let byte = if shift > 0 {
                let shifted = self.emit_binop_val(
                    IrBinOp::LShr,
                    Operand::Value(cur),
                    Operand::Const(IrConst::I64(shift)),
                    uty,
                );
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(shifted),
                    Operand::Const(IrConst::I64(0xFF)),
                    uty,
                )
            } else {
                self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(cur),
                    Operand::Const(IrConst::I64(0xFF)),
                    uty,
                )
            };
            // 3-step bit-reverse of the isolated byte.
            let b = |s: &mut Self, x: Value, op: IrBinOp, k: i64, m: i64| -> (Value, Value) {
                let a =
                    s.emit_binop_val(op, Operand::Value(x), Operand::Const(IrConst::I64(k)), uty);
                let b = s.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(a),
                    Operand::Const(IrConst::I64(m)),
                    uty,
                );
                (a, b)
            };
            // rev = ((byte & 0x55) << 1) | ((byte >> 1) & 0x55)
            let (a1, m1) = b(self, byte, IrBinOp::And, 1, 0x55);
            let (t1, m2) = b(self, m1, IrBinOp::LShr, 1, 0x55);
            let _ = t1;
            let mut rev = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(m1),
                Operand::Const(IrConst::I64(1)),
                uty,
            );
            rev = self.emit_binop_val(IrBinOp::Or, Operand::Value(rev), Operand::Value(m2), uty);
            let _ = a1;
            // rev = ((rev & 0x33) << 2) | ((rev >> 2) & 0x33)
            let (t2, m3) = b(self, rev, IrBinOp::And, 2, 0x33);
            let _ = t2;
            let sh2 = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(m3),
                Operand::Const(IrConst::I64(2)),
                uty,
            );
            let m4 = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(sh2),
                Operand::Const(IrConst::I64(0x33)),
                uty,
            );
            let sh2b = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(m3),
                Operand::Const(IrConst::I64(2)),
                uty,
            );
            rev = self.emit_binop_val(IrBinOp::Or, Operand::Value(sh2b), Operand::Value(m4), uty);
            // rev = ((rev & 0x0F) << 4) | ((rev >> 4) & 0x0F)
            let (t3, m5) = b(self, rev, IrBinOp::And, 4, 0x0F);
            let _ = t3;
            let sh3 = self.emit_binop_val(
                IrBinOp::LShr,
                Operand::Value(m5),
                Operand::Const(IrConst::I64(4)),
                uty,
            );
            let m6 = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(sh3),
                Operand::Const(IrConst::I64(0x0F)),
                uty,
            );
            let sh3b = self.emit_binop_val(
                IrBinOp::Shl,
                Operand::Value(m5),
                Operand::Const(IrConst::I64(4)),
                uty,
            );
            rev = self.emit_binop_val(IrBinOp::Or, Operand::Value(sh3b), Operand::Value(m6), uty);
            // Re-insert at byte i.
            if shift > 0 {
                let rev_shifted = self.emit_binop_val(
                    IrBinOp::Shl,
                    Operand::Value(rev),
                    Operand::Const(IrConst::I64(shift)),
                    uty,
                );
                let clear = self.emit_binop_val(
                    IrBinOp::And,
                    Operand::Value(cur),
                    Operand::Const(IrConst::I64(!(0xFFi64 << shift))),
                    uty,
                );
                cur = self.emit_binop_val(
                    IrBinOp::Or,
                    Operand::Value(clear),
                    Operand::Value(rev_shifted),
                    uty,
                );
            } else {
                cur = rev;
            }
        }
        Operand::Value(cur)
    }

    /// Apply the reverse-SSO storage transformation to a whole storage-unit
    /// value: byteswap (and, for 16-byte units, half-swap) the raw bits.
    /// Float types go through a bit-exact reinterpret first.
    pub(super) fn emit_sso_swap_unit(
        &mut self,
        val: Operand,
        ty: IrType,
        unit_bytes: u32,
    ) -> Operand {
        let (int_ty, val_int) = if ty.is_integer() {
            (ty.to_unsigned(), val.clone())
        } else {
            // Float members: reinterpret the raw bits as an integer of the
            // same size, swap, and reinterpret back.
            match (unit_bytes as usize, ty) {
                (4, IrType::F32) => {
                    let as_int = self.emit_bit_reinterpret(val, IrType::F32, IrType::U32);
                    let swapped = self.emit_sso_bswap_int(as_int, IrType::U32);
                    return self.emit_bit_reinterpret(swapped, IrType::U32, IrType::F32);
                }
                (8, IrType::F64) => {
                    let as_int = self.emit_bit_reinterpret(val, IrType::F64, IrType::U64);
                    let swapped = self.emit_sso_bswap_int(as_int, IrType::U64);
                    return self.emit_bit_reinterpret(swapped, IrType::U64, IrType::F64);
                }
                _ => {
                    // Unhandled float width (e.g. F128/x87 long double):
                    // conservative fallback: two 8-byte halves.
                    return self.emit_sso_swap_unit_16(val, ty);
                }
            }
            #[allow(unreachable_code)]
            (ty.to_unsigned(), val.clone())
        };
        match unit_bytes {
            2 => self.emit_sso_bswap_int(val_int, IrType::U16),
            4 => self.emit_sso_bswap_int(val_int, IrType::U32),
            8 => {
                let casted = self.emit_cast_val(val_int, int_ty, IrType::U64);
                Operand::Value(casted)
            }
            16 => self.emit_sso_swap_unit_16(val_int, int_ty),
            _ => val,
        }
    }

    /// 16-byte unit swap: bswap each 8-byte half and exchange the halves.
    fn emit_sso_swap_unit_16(&mut self, val: Operand, ty: IrType) -> Operand {
        // Round-trip through a 16-byte temp so we can address both halves.
        let tmp = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: tmp,
            ty: IrType::U128,
            size: 16,
            align: 16,
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val,
            ptr: tmp,
            ty,
            seg_override: AddressSpace::Default,
        });
        // Reinterpretation note: for 16-byte float carriers the raw bytes are
        // what matters, so load as U64 halves directly.
        let lo_ptr = self.emit_gep_offset(tmp, 0, IrType::U64);
        let hi_ptr = self.emit_gep_offset(tmp, 8, IrType::U64);
        let lo = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: lo,
            ptr: lo_ptr,
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
        });
        let hi = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest: hi,
            ptr: hi_ptr,
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
        });
        let lo_swapped = self.emit_sso_bswap_int(Operand::Value(lo), IrType::U64);
        let hi_swapped = self.emit_sso_bswap_int(Operand::Value(hi), IrType::U64);
        self.emit(Instruction::Store {
            volatile: false,
            val: hi_swapped,
            ptr: lo_ptr,
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: lo_swapped,
            ptr: hi_ptr,
            ty: IrType::U64,
            seg_override: AddressSpace::Default,
        });
        let dest = self.fresh_value();
        self.emit(Instruction::Load {
            volatile: false,
            dest,
            ptr: tmp,
            ty,
            seg_override: AddressSpace::Default,
        });
        Operand::Value(dest)
    }

    /// Fix up a loaded storage unit according to the field's SsoMode before
    /// bit extraction / value use.
    pub(super) fn emit_sso_load_fixup(
        &mut self,
        loaded: Operand,
        ty: IrType,
        sso: SsoMode,
    ) -> Operand {
        match sso {
            SsoMode::None => loaded,
            SsoMode::ByteSwapUnit(u) => self.emit_sso_swap_unit(loaded, ty, u),
            SsoMode::ByteBitReverse => self.emit_sso_bitrev_bytes(loaded, ty),
        }
    }

    /// Inverse fixup on a unit value about to be stored (same transform —
    /// bswap and per-byte bit-reverse are involutions).
    pub(super) fn emit_sso_store_fixup(
        &mut self,
        val: Operand,
        ty: IrType,
        sso: SsoMode,
    ) -> Operand {
        self.emit_sso_load_fixup(val, ty, sso)
    }

    // === _Float128 (binary128) soft-float call helpers ===
    //
    // _Float128 arithmetic/comparison/conversion is lowered to calls of the
    // libgcc/compiler-rt TF helpers (__addtf3, __extendsftf2, ...), exactly
    // like GCC. _Float128 values follow the SysV psABI: each _Float128 arg is
    // passed in ONE 16-byte XMM register; returns come back in xmm0.

    /// Emit a call to a libgcc soft-float helper. `struct_info[i]` is
    /// Some((size, align, classes)) for SSE-class 16-byte args, None for
    /// scalar args. `ret_classes` is the return eightbyte classification.
    /// `ret_is_f128` marks a single-XMM _Float128 return.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_softfloat_call(
        &mut self,
        name: &str,
        args: Vec<Operand>,
        arg_types: Vec<IrType>,
        struct_info: Vec<Option<(usize, usize, Vec<crate::common::types::EightbyteClass>)>>,
        ret: IrType,
        ret_classes: Vec<crate::common::types::EightbyteClass>,
        ret_is_f128: bool,
    ) -> Value {
        // _Float128 (16-byte carrier) arguments are passed like SSE-class
        // values: the backend emits 16-byte XMM loads. Spill each carrier arg
        // into a fresh 16-byte temp and pass its address.
        let mut args = args;
        let mut arg_types = arg_types;
        let mut arg_is_f128 = vec![false; args.len()];
        for (i, info) in struct_info.iter().enumerate() {
            if info.is_some() {
                let tmp = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: tmp,
                    ty: IrType::U128,
                    size: 16,
                    align: 16,
                    volatile: false,
                    semantic_volatile: false,
                });
                let store_ty = arg_types.get(i).copied().unwrap_or(IrType::U128);
                self.emit(Instruction::Store {
                    volatile: false,
                    val: args[i].clone(),
                    ptr: tmp,
                    ty: store_ty,
                    seg_override: crate::common::types::AddressSpace::Default,
                });
                args[i] = Operand::Value(tmp);
                arg_types[i] = IrType::U128;
                arg_is_f128[i] = true;
            }
        }
        // ILP32: TFmode/TDmode is class MEMORY — GCC i386 returns it through
        // a HIDDEN POINTER passed as the first stack argument, and the callee
        // writes the 16 bytes through it and pops the pointer with `ret $4`
        // (verified against GCC 14.2 i386 output for
        // `_Float128 f(_Float128,_Float128){return a+b;}` and libgcc's
        // __addtf3 epilogue). Model exactly that: alloca a 16-byte result
        // buffer, prepend its address as the leading Ptr argument, set
        // is_sret so the backend accounts for the callee-popped 4 bytes, and
        // load the result value out of the buffer afterwards.  Every
        // remaining argument KEEPS its own struct size/alignment/class
        // metadata (shifted by one) so 16-byte TFmode args continue to be
        // staged by value — zeroing that metadata made the backend pass
        // bare pointers instead and corrupted every soft-float call.
        // (The x86-64 path below returns in xmm0:xmm1 and does not apply.)
        if self.target == crate::backend::Target::I686 && ret == IrType::U128 && ret_is_f128 {
            let buf = self.fresh_value();
            self.emit(Instruction::Alloca {
                dest: buf,
                ty: IrType::Ptr,
                size: 16,
                align: 0,
                volatile: false,
                semantic_volatile: false,
            });
            let dest = self.fresh_value();
            let mut full_args = Vec::with_capacity(args.len() + 1);
            let mut full_types = Vec::with_capacity(arg_types.len() + 1);
            let mut full_sizes: Vec<Option<usize>> = Vec::with_capacity(args.len() + 1);
            let mut full_aligns = Vec::with_capacity(args.len() + 1);
            let mut full_classes: Vec<Vec<crate::common::types::EightbyteClass>> =
                Vec::with_capacity(args.len() + 1);
            let mut full_f128 = Vec::with_capacity(args.len() + 1);
            // Hidden return-buffer pointer: first stack argument, not a
            // struct arg, no eightbyte classes (same shape as the proven
            // struct-sret path in expr_calls.rs).
            full_args.push(Operand::Value(buf));
            full_types.push(IrType::Ptr);
            full_sizes.push(None);
            full_aligns.push(None);
            full_classes.push(Vec::new());
            full_f128.push(false);
            full_args.extend(args.iter().cloned());
            full_types.extend(arg_types.iter().cloned());
            // Preserve each original argument's own by-value staging info.
            for (i, info) in struct_info.iter().enumerate() {
                match info {
                    Some((size, align, classes)) => {
                        full_sizes.push(Some(*size));
                        full_aligns.push(Some(*align));
                        full_classes.push(classes.clone());
                    }
                    None => {
                        full_sizes.push(None);
                        full_aligns.push(None);
                        full_classes.push(Vec::new());
                    }
                }
                full_f128.push(arg_is_f128[i]);
            }
            let n_full = full_args.len();
            self.emit(Instruction::Call {
                func: name.to_string(),
                info: CallInfo {
                    dest: None,
                    args: full_args,
                    arg_types: full_types,
                    return_type: IrType::Void,
                    is_variadic: false,
                    num_fixed_args: n_full,
                    struct_arg_sizes: full_sizes,
                    struct_arg_aligns: full_aligns,
                    struct_arg_classes: full_classes,
                    struct_arg_riscv_float_classes: Vec::new(),
                    struct_arg_is_f128_sse: full_f128,
                    is_sret: true,
                    is_fastcall: false,
                    ret_eightbyte_classes: Vec::new(),
                    ret_is_f128_sse: false,
                    is_const: false,
                    is_pure: false,
                },
            });
            self.emit(Instruction::Load {
                volatile: false,
                dest,
                ptr: buf,
                ty: IrType::U128,
                seg_override: crate::common::types::AddressSpace::Default,
            });
            return dest;
        }
        let dest = self.fresh_value();
        let n = args.len();
        let mut struct_arg_sizes = Vec::with_capacity(n);
        let mut struct_arg_aligns = Vec::with_capacity(n);
        let mut struct_arg_classes: Vec<Vec<crate::common::types::EightbyteClass>> =
            Vec::with_capacity(n);
        for info in &struct_info {
            match info {
                Some((size, align, classes)) => {
                    struct_arg_sizes.push(Some(*size));
                    struct_arg_aligns.push(Some(*align));
                    struct_arg_classes.push(classes.clone());
                }
                None => {
                    struct_arg_sizes.push(None);
                    struct_arg_aligns.push(None);
                    struct_arg_classes.push(Vec::new());
                }
            }
        }
        self.emit(Instruction::Call {
            func: name.to_string(),
            info: CallInfo {
                dest: Some(dest),
                args,
                arg_types,
                return_type: ret,
                is_variadic: false,
                num_fixed_args: n,
                struct_arg_sizes,
                struct_arg_aligns,
                struct_arg_classes,
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: arg_is_f128,
                is_sret: false,
                is_fastcall: false,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: ret_classes,
                ret_is_f128_sse: ret_is_f128,
            },
        });
        dest
    }

    /// _Float128 call argument marker: 16-byte all-SSE (single XMM in psABI).
    pub(super) fn f128_arg_info(
    ) -> Option<(usize, usize, Vec<crate::common::types::EightbyteClass>)> {
        Some((
            16,
            16,
            vec![
                crate::common::types::EightbyteClass::Sse,
                crate::common::types::EightbyteClass::Sse,
            ],
        ))
    }

    pub(crate) fn f128_ret_classes() -> Vec<crate::common::types::EightbyteClass> {
        vec![
            crate::common::types::EightbyteClass::Sse,
            crate::common::types::EightbyteClass::Sse,
        ]
    }

    /// Emit a binary128 arithmetic call: __addtf3 etc. (single-XMM ABI).
    pub(super) fn emit_f128_binop_call(
        &mut self,
        helper: &str,
        lhs: Operand,
        rhs: Operand,
    ) -> Value {
        self.emit_softfloat_call(
            helper,
            vec![lhs, rhs],
            vec![IrType::U128, IrType::U128],
            vec![Self::f128_arg_info(), Self::f128_arg_info()],
            IrType::U128,
            Self::f128_ret_classes(),
            true,
        )
    }

    /// Emit a binary128 comparison call: __lttf2/__eqtf2/... These return an
    /// ordinary `int` (not _Float128), so the call is a plain I32 return with
    /// NO SSE eightbyte classification and NO f128 return marker.
    pub(super) fn emit_f128_cmp_call(&mut self, helper: &str, lhs: Operand, rhs: Operand) -> Value {
        self.emit_softfloat_call(
            helper,
            vec![lhs, rhs],
            vec![IrType::U128, IrType::U128],
            vec![Self::f128_arg_info(), Self::f128_arg_info()],
            IrType::I32,
            Vec::new(),
            false,
        )
    }

    /// Convert `val` (of C type `from_ct`) to _Float128 via the libgcc
    /// widening helpers, matching GCC's codegen.
    /// Convert a scalar operand of C type `src_ct` to C type `target_ct`.
    ///
    /// This is the CType-aware sibling of `emit_implicit_cast`: when either
    /// side is `_Float128` (IEEE binary128), the conversion must go through the
    /// libgcc soft-float helpers (`__extenddftf2`, `__trunctfdf2`, …), because
    /// the IR carrier type (`U128`) is ambiguous with `unsigned __int128` and
    /// the bit-cast machinery would produce garbage bit patterns
    /// (`_Float128 a = 100.0` became `{100,0}` instead of binary128 100.0).
    pub(super) fn convert_scalar_ctype(
        &mut self,
        src: Operand,
        src_ty: IrType,
        src_ct: &CType,
        target_ct: &CType,
    ) -> Operand {
        use crate::common::types::CType;
        // C23 decimal floating point: any conversion touching a decimal type
        // is a genuine value conversion through the libbid helpers — bit-cast
        // machinery must never see decimal carriers.
        if target_ct.is_decimal() || src_ct.is_decimal() {
            return self.convert_decimal_scalar(src, src_ty, src_ct, target_ct);
        }
        if *target_ct == CType::Float128 {
            // A source that is ALREADY the 128-bit carrier — an I128 constant
            // payload (a `1.5F128` literal or a `__builtin_*f128` result) or a
            // U128-typed value (a _Float128 variable) — must be copied
            // bit-exactly. Anything else is a genuine value conversion
            // (double/int/... -> binary128) via the soft-float helpers.
            // (An unsigned __int128 constant would also take the identity path
            // instead of __floatuntitf — a rare corner.)
            let already_carrier =
                matches!(&src, Operand::Const(IrConst::I128(_))) || src_ty == IrType::U128;
            if already_carrier {
                return src;
            }
            return self.convert_to_f128(src, src_ct);
        }
        if *src_ct == CType::Float128 {
            return self.convert_from_f128(src, target_ct);
        }
        let target_ty = IrType::from_ctype(target_ct);
        self.emit_implicit_cast(src, src_ty, target_ty)
    }

    pub(super) fn convert_to_f128(
        &mut self,
        val: Operand,
        from_ct: &crate::common::types::CType,
    ) -> Operand {
        use crate::common::types::CType;
        let (helper, arg_ty, struct_info): (
            &str,
            IrType,
            Option<(usize, usize, Vec<crate::common::types::EightbyteClass>)>,
        ) = match from_ct {
            CType::Float => ("__extendsftf2", IrType::F32, None),
            CType::Double => ("__extenddftf2", IrType::F64, None),
            // x87 long double: passed per SysV (16-byte stack slot, x87 class).
            CType::LongDouble => ("__extendxftf2", IrType::F128, None),
            CType::Bool
            | CType::Char
            | CType::UChar
            | CType::Short
            | CType::UShort
            | CType::Int
            | CType::UInt => {
                let signed = matches!(
                    from_ct,
                    CType::Bool | CType::Char | CType::Short | CType::Int
                );
                (
                    if signed {
                        "__floatsitf"
                    } else {
                        "__floatunsitf"
                    },
                    IrType::I32,
                    None,
                )
            }
            CType::Long | CType::ULong | CType::LongLong | CType::ULongLong => {
                let signed = matches!(from_ct, CType::Long | CType::LongLong);
                (
                    if signed {
                        "__floatditf"
                    } else {
                        "__floatunditf"
                    },
                    IrType::I64,
                    None,
                )
            }
            CType::Int128 | CType::UInt128 => {
                let signed = matches!(from_ct, CType::Int128);
                (
                    if signed {
                        "__floattitf"
                    } else {
                        "__floatuntitf"
                    },
                    IrType::I128,
                    None,
                )
            }
            CType::Float128 => return val,
            _ => return val,
        };
        let dest = self.emit_softfloat_call(
            helper,
            vec![val],
            vec![arg_ty],
            vec![struct_info],
            IrType::U128,
            Self::f128_ret_classes(),
            true,
        );
        Operand::Value(dest)
    }

    /// Convert `val` (_Float128) to C type `to_ct` via the libgcc narrowing
    /// helpers, matching GCC's codegen.
    pub(super) fn convert_from_f128(
        &mut self,
        val: Operand,
        to_ct: &crate::common::types::CType,
    ) -> Operand {
        use crate::common::types::CType;
        let (helper, ret_ty, ret_classes): (
            &str,
            IrType,
            Vec<crate::common::types::EightbyteClass>,
        ) = match to_ct {
            CType::Float => ("__trunctfsf2", IrType::F32, Vec::new()),
            CType::Double => ("__trunctfdf2", IrType::F64, Vec::new()),
            // x87 long double: returned in st(0) (the backend's F128 return path).
            CType::LongDouble => ("__trunctfxf2", IrType::F128, Vec::new()),
            CType::Bool
            | CType::Char
            | CType::UChar
            | CType::Short
            | CType::UShort
            | CType::Int
            | CType::UInt => {
                let signed = matches!(to_ct, CType::Bool | CType::Char | CType::Short | CType::Int);
                (
                    if signed { "__fixtfsi" } else { "__fixunstfsi" },
                    IrType::I32,
                    Vec::new(),
                )
            }
            CType::Long | CType::ULong | CType::LongLong | CType::ULongLong => {
                let signed = matches!(to_ct, CType::Long | CType::LongLong);
                (
                    if signed { "__fixtfdi" } else { "__fixunstfdi" },
                    IrType::I64,
                    Vec::new(),
                )
            }
            CType::Int128 | CType::UInt128 => {
                let signed = matches!(to_ct, CType::Int128);
                (
                    if signed { "__fixtfti" } else { "__fixunstfti" },
                    IrType::I128,
                    Vec::new(),
                )
            }
            CType::Float128 => return val,
            _ => return val,
        };
        let dest = self.emit_softfloat_call(
            helper,
            vec![val],
            vec![IrType::U128],
            vec![Self::f128_arg_info()],
            ret_ty,
            ret_classes,
            false,
        );
        Operand::Value(dest)
    }

    /// Emit a GEP + Store: store `val` at `base + byte_offset` with the given type.
    pub(super) fn emit_store_at_offset(
        &mut self,
        base: Value,
        byte_offset: usize,
        val: Operand,
        ty: IrType,
    ) {
        let addr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: addr,
            base,
            offset: Operand::Const(IrConst::ptr_int(byte_offset as i64)),
            ty,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val,
            ptr: addr,
            ty,
            seg_override: AddressSpace::Default,
        });
    }

    /// Lower an expression, cast to target type, then store at base + byte_offset.
    /// When target_is_bool is true, normalizes the value (any nonzero -> 1) per C11 6.3.1.2.
    pub(super) fn emit_init_expr_to_offset_bool(
        &mut self,
        e: &Expr,
        base: Value,
        byte_offset: usize,
        target_ty: IrType,
        target_is_bool: bool,
    ) {
        let expr_ty = self.get_expr_type(e);
        let val = self.lower_expr(e);
        let val = if target_is_bool {
            self.emit_bool_normalize_typed(val, expr_ty)
        } else {
            self.emit_implicit_cast(val, expr_ty, target_ty)
        };
        self.emit_store_at_offset(base, byte_offset, val, target_ty);
    }

    /// Emit a GEP to compute base + byte_offset and return the address Value.
    pub(super) fn emit_gep_offset(&mut self, base: Value, byte_offset: usize, ty: IrType) -> Value {
        let addr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: addr,
            base,
            offset: Operand::Const(IrConst::ptr_int(byte_offset as i64)),
            ty,
        });
        addr
    }

    /// Emit memcpy from src to base + byte_offset.
    pub(super) fn emit_memcpy_at_offset(
        &mut self,
        base: Value,
        byte_offset: usize,
        src: Value,
        size: usize,
    ) {
        let dest = self.emit_gep_offset(base, byte_offset, IrType::Ptr);
        self.emit(Instruction::Memcpy { dest, src, size });
    }

    /// Emit a libc `memcpy(dest, src, size)` call for runtime-sized aggregate
    /// copies.  The fixed-size IR `Memcpy` instruction intentionally keeps a
    /// compile-time byte count so the backend can inline small copies; VLA
    /// structs need the dynamic path instead.
    pub(super) fn emit_dynamic_memcpy(&mut self, dest: Value, src: Value, size: Value) -> Value {
        let ret = self.fresh_value();
        let ptr_ty = IrType::Ptr;
        let size_ty = crate::common::types::target_int_ir_type();
        self.emit(Instruction::Call {
            func: "memcpy".to_string(),
            info: CallInfo {
                dest: Some(ret),
                args: vec![
                    Operand::Value(dest),
                    Operand::Value(src),
                    Operand::Value(size),
                ],
                arg_types: vec![ptr_ty, ptr_ty, size_ty],
                return_type: ptr_ty,
                is_variadic: false,
                num_fixed_args: 3,
                struct_arg_sizes: vec![None, None, None],
                struct_arg_aligns: vec![None, None, None],
                struct_arg_classes: vec![Vec::new(), Vec::new(), Vec::new()],
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: vec![false, false, false],
                is_sret: false,
                is_fastcall: false,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
            },
        });
        ret
    }

    /// Emit `memset(dest, 0, size)` for a large zero-initialised region.
    /// The x86 backend expands the fixed-size call per CPU tuning row
    /// (`X86Tune::memset_strategy`: straight-line vector stores, counted
    /// loop, `rep stosb`); other backends keep the libc call, which is also
    /// what GCC/Clang emit for aggregates of this size.
    pub(super) fn emit_memset_zero(&mut self, dest: Value, size: usize) -> Value {
        let ret = self.fresh_value();
        let ptr_ty = IrType::Ptr;
        let size_ty = crate::common::types::target_int_ir_type();
        self.emit(Instruction::Call {
            func: "memset".to_string(),
            info: CallInfo {
                dest: Some(ret),
                args: vec![
                    Operand::Value(dest),
                    Operand::Const(IrConst::I32(0)),
                    Operand::Const(IrConst::ptr_int(size as i64)),
                ],
                arg_types: vec![ptr_ty, IrType::I32, size_ty],
                return_type: ptr_ty,
                is_variadic: false,
                num_fixed_args: 3,
                struct_arg_sizes: vec![None, None, None],
                struct_arg_aligns: vec![None, None, None],
                struct_arg_classes: vec![Vec::new(), Vec::new(), Vec::new()],
                struct_arg_riscv_float_classes: Vec::new(),
                struct_arg_is_f128_sse: vec![false, false, false],
                is_sret: false,
                is_fastcall: false,
                is_pure: false,
                is_const: false,
                ret_eightbyte_classes: Vec::new(),
                ret_is_f128_sse: false,
            },
        });
        ret
    }

    pub(super) fn terminate(&mut self, term: Terminator) {
        let block = BasicBlock {
            label: self.func_mut().current_label,
            instructions: std::mem::take(&mut self.func_mut().instrs),
            source_spans: std::mem::take(&mut self.func_mut().instr_spans),
            terminator: term,
        };
        self.func_mut().blocks.push(block);
    }

    pub(super) fn start_block(&mut self, label: BlockId) {
        self.func_mut().current_label = label;
        self.func_mut().instrs.clear();
        self.func_mut().instr_spans.clear();
    }

    // --- Enum and label helpers ---

    /// Collect enum constants from a type specifier.
    fn collect_enum_constants_impl(&mut self, ts: &TypeSpecifier, scoped: bool) {
        match ts {
            TypeSpecifier::Enum(name, Some(variants), is_packed) => {
                let mut next_val: i64 = 0;
                let mut variant_values = Vec::new();
                for variant in variants {
                    if let Some(ref expr) = variant.value {
                        if let Some(val) = self.eval_const_expr(expr) {
                            if let Some(v) = self.const_to_i64(&val) {
                                next_val = v;
                            }
                        }
                    }
                    if scoped {
                        self.insert_enum_scoped(variant.name.clone(), next_val);
                    } else {
                        self.types
                            .enum_constants
                            .insert(variant.name.clone(), next_val);
                    }
                    variant_values.push((variant.name.clone(), next_val));
                    next_val += 1;
                }
                // Store packed enum info for forward-reference lookups
                if *is_packed {
                    if let Some(tag) = name {
                        self.types.packed_enum_types.insert(
                            tag.clone(),
                            crate::common::types::EnumType {
                                name: Some(tag.clone()),
                                variants: variant_values,
                                is_packed: true,
                            },
                        );
                    }
                }
            }
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => {
                for field in fields {
                    self.collect_enum_constants_impl(&field.type_spec, scoped);
                }
            }
            TypeSpecifier::Array(inner, _) | TypeSpecifier::Pointer(inner, _) => {
                self.collect_enum_constants_impl(inner, scoped);
            }
            _ => {}
        }
    }

    /// Collect enum constants from a type specifier (file-scope, direct insertion).
    pub(super) fn collect_enum_constants(&mut self, ts: &TypeSpecifier) {
        self.collect_enum_constants_impl(ts, false);
    }

    /// Collect enum constants from a type specifier, using scoped insertion.
    pub(super) fn collect_enum_constants_scoped(&mut self, ts: &TypeSpecifier) {
        self.collect_enum_constants_impl(ts, true);
    }

    /// Check if a TypeSpecifier refers to an enum type (directly or via typedef).
    pub(super) fn is_enum_type_spec(&self, ts: &TypeSpecifier) -> bool {
        match ts {
            TypeSpecifier::Enum(..) => true,
            TypeSpecifier::TypedefName(name) => self.types.enum_typedefs.contains(name),
            TypeSpecifier::TypeofType(inner) => self.is_enum_type_spec(inner),
            _ => false,
        }
    }

    /// Check if a user-defined goto label has already been defined (i.e., the
    /// label statement was already lowered). Used to determine backward vs forward gotos.
    /// This checks `defined_user_labels` (set when `label:` is lowered), NOT `user_labels`
    /// (which is also populated by forward `goto` statements creating placeholder blocks).
    pub(super) fn user_label_exists(&self, name: &str) -> bool {
        let resolved_name = self.resolve_local_label(name);
        let func_name = &self.func().name;
        let key = format!("{}::{}", func_name, resolved_name);
        self.func().defined_user_labels.contains(&key)
    }

    /// Get or create a unique IR label for a user-defined goto label.
    /// If the label name is declared via __label__ in a local scope,
    /// uses the scope-qualified name so that different invocations of
    /// a macro with __label__ don't collide.
    pub(super) fn get_or_create_user_label(&mut self, name: &str) -> BlockId {
        // Check local label scopes from innermost to outermost
        let resolved_name = self.resolve_local_label(name);
        let func_name = self.func_mut().name.clone();
        let key = format!("{}::{}", func_name, resolved_name);
        if let Some(&label) = self.func_mut().user_labels.get(&key) {
            label
        } else {
            let label = self.fresh_label();
            self.func_mut().user_labels.insert(key, label);
            label
        }
    }

    /// Resolve a label name through the local label scope stack.
    /// Returns a scope-qualified name if the label is declared via __label__,
    /// or the original name if not.
    pub(super) fn resolve_local_label(&self, name: &str) -> String {
        // Search scopes from innermost to outermost
        for scope in self.local_label_scopes.iter().rev() {
            if let Some(qualified) = scope.get(name) {
                return qualified.clone();
            }
        }
        name.to_string()
    }

    // --- String and array init helpers ---

    /// Copy a string literal's bytes into an alloca at a given byte offset,
    /// followed by a null terminator (if it fits within `max_bytes`).
    ///
    /// Per C11 6.7.9 p14, when a char array is initialized from a string literal
    /// that is exactly one byte longer than the array (due to the implicit NUL
    /// terminator), the trailing NUL is silently dropped. This function respects
    /// that rule by only writing up to `max_bytes` total bytes.
    pub(super) fn emit_string_to_alloca(
        &mut self,
        alloca: Value,
        s: &str,
        base_offset: usize,
        max_bytes: usize,
    ) {
        let str_bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        let bytes_to_copy = str_bytes.len().min(max_bytes);
        for j in 0..bytes_to_copy {
            let byte = str_bytes[j];
            let val = Operand::Const(IrConst::I8(byte as i8));
            let offset = Operand::Const(IrConst::ptr_int((base_offset + j) as i64));
            let addr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: addr,
                base: alloca,
                offset,
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val,
                ptr: addr,
                ty: IrType::I8,
                seg_override: AddressSpace::Default,
            });
        }
        // Null terminator -- only write if there's room within max_bytes
        let null_pos = str_bytes.len();
        if null_pos < max_bytes {
            let null_offset = Operand::Const(IrConst::ptr_int((base_offset + null_pos) as i64));
            let null_addr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: null_addr,
                base: alloca,
                offset: null_offset,
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I8(0)),
                ptr: null_addr,
                ty: IrType::I8,
                seg_override: AddressSpace::Default,
            });
        }
    }

    /// Emit a wide string (L"...") to a local alloca. Each character is stored as I32 (wchar_t).
    pub(super) fn emit_wide_string_to_alloca(
        &mut self,
        alloca: Value,
        s: &str,
        base_offset: usize,
    ) {
        for (j, ch) in s.chars().enumerate() {
            let val = Operand::Const(IrConst::I32(ch as i32));
            let byte_offset = base_offset + j * 4;
            let offset = Operand::Const(IrConst::ptr_int(byte_offset as i64));
            let addr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: addr,
                base: alloca,
                offset,
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val,
                ptr: addr,
                ty: IrType::I32,
                seg_override: AddressSpace::Default,
            });
        }
        // Null terminator
        let null_byte_offset = base_offset + s.chars().count() * 4;
        let null_offset = Operand::Const(IrConst::ptr_int(null_byte_offset as i64));
        let null_addr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: null_addr,
            base: alloca,
            offset: null_offset,
            ty: IrType::I8,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Const(IrConst::I32(0)),
            ptr: null_addr,
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
        });
    }

    /// Emit a char16_t string (u"...") to a local alloca. Each character is stored as U16.
    pub(super) fn emit_char16_string_to_alloca(
        &mut self,
        alloca: Value,
        s: &str,
        base_offset: usize,
    ) {
        for (j, ch) in s.chars().enumerate() {
            let val = Operand::Const(IrConst::I16(ch as u16 as i16));
            let byte_offset = base_offset + j * 2;
            let offset = Operand::Const(IrConst::ptr_int(byte_offset as i64));
            let addr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: addr,
                base: alloca,
                offset,
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val,
                ptr: addr,
                ty: IrType::U16,
                seg_override: AddressSpace::Default,
            });
        }
        // Null terminator
        let null_byte_offset = base_offset + s.chars().count() * 2;
        let null_offset = Operand::Const(IrConst::ptr_int(null_byte_offset as i64));
        let null_addr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: null_addr,
            base: alloca,
            offset: null_offset,
            ty: IrType::I8,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val: Operand::Const(IrConst::I16(0)),
            ptr: null_addr,
            ty: IrType::U16,
            seg_override: AddressSpace::Default,
        });
    }

    /// Emit a single element store at a given byte offset in an alloca.
    pub(super) fn emit_array_element_store(
        &mut self,
        alloca: Value,
        val: Operand,
        offset: usize,
        ty: IrType,
    ) {
        let offset_val = Operand::Const(IrConst::ptr_int(offset as i64));
        let elem_addr = self.fresh_value();
        self.emit(Instruction::GetElementPtr {
            dest: elem_addr,
            base: alloca,
            offset: offset_val,
            ty,
        });
        self.emit(Instruction::Store {
            volatile: false,
            val,
            ptr: elem_addr,
            ty,
            seg_override: AddressSpace::Default,
        });
    }

    /// Zero-initialize a region of memory within an alloca at the given byte offset.
    ///
    /// Regions up to `ZERO_INIT_STORE_LADDER_MAX` bytes are lowered as a
    /// ladder of 8-byte (then 1-byte) stores: the IR-level passes (SROA,
    /// store→load forwarding, DSE of fields that are immediately
    /// overwritten) see every store.  Larger regions become one
    /// `memset(p, 0, n)` call.  Before this cut-over a 4096-byte
    /// `struct big b = {0};` emitted 512 × `movq $0, d(%rsp)` (≈5.5 KiB of
    /// code, one µop each); the tuned memset expansion is 3–8 instructions.
    pub(super) fn zero_init_region(
        &mut self,
        alloca: Value,
        base_offset: usize,
        region_size: usize,
    ) {
        if region_size > ZERO_INIT_STORE_LADDER_MAX {
            let dest = if base_offset == 0 {
                alloca
            } else {
                let addr = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: addr,
                    base: alloca,
                    offset: Operand::Const(IrConst::ptr_int(base_offset as i64)),
                    ty: IrType::I8,
                });
                addr
            };
            self.emit_memset_zero(dest, region_size);
            return;
        }
        let mut offset = base_offset;
        let end = base_offset + region_size;
        while offset + 8 <= end {
            let addr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: addr,
                base: alloca,
                offset: Operand::Const(IrConst::ptr_int(offset as i64)),
                ty: IrType::I64,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I64(0)),
                ptr: addr,
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            });
            offset += 8;
        }
        while offset < end {
            let addr = self.fresh_value();
            self.emit(Instruction::GetElementPtr {
                dest: addr,
                base: alloca,
                offset: Operand::Const(IrConst::ptr_int(offset as i64)),
                ty: IrType::I8,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Const(IrConst::I8(0)),
                ptr: addr,
                ty: IrType::I8,
                seg_override: AddressSpace::Default,
            });
            offset += 1;
        }
    }

    /// Zero-initialize an entire alloca.
    pub(super) fn zero_init_alloca(&mut self, alloca: Value, total_size: usize) {
        self.zero_init_region(alloca, 0, total_size);
    }
}

/// True if `name` is an x86-64 register name. Distinguishes
/// `register int x __asm__("rbx")` (register pinning) from
/// `extern int x __asm__("symbol")` (linker symbol rename) on variable
/// declarations — both are stored in the same attribute field by the parser.
/// A symbol-rename label that is also a register spelling must never be
/// interpreted as a register variable (previously emitted `movzbl %sym, %sym`
/// self-moves, e.g. `__GI__libc_intl_domainname`).
pub(super) fn is_x86_register_name(name: &str) -> bool {
    const REGS: &[&str] = &[
        "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp", "rip", "r8", "r9", "r10", "r11",
        "r12", "r13", "r14", "r15", "eax", "ebx", "ecx", "edx", "esi", "edi", "ebp", "esp", "r8d",
        "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d", "ax", "bx", "cx", "dx", "si", "di",
        "bp", "sp", "al", "bl", "cl", "dl", "sil", "dil", "bpl", "spl", "r8b", "r9b", "r10b",
        "r11b", "r12b", "r13b", "r14b", "r15b", "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5",
        "xmm6", "xmm7", "xmm8", "xmm9", "xmm10", "xmm11", "xmm12", "xmm13", "xmm14", "xmm15",
        "ymm0", "ymm1", "ymm2", "ymm3", "ymm4", "ymm5", "ymm6", "ymm7", "ymm8", "ymm9", "ymm10",
        "ymm11", "ymm12", "ymm13", "ymm14", "ymm15",
    ];
    REGS.contains(&name)
}

// ─────────────────────────────────────────────────────────────────────────────
// C23 decimal floating point (_Decimal32/_Decimal64/_Decimal128, IEEE 754-2008
// BID encoding). Every operation is lowered to the libbid helper family that
// GCC's own DFP lowering calls, so lccc is ABI- and semantics-compatible with
// GCC-compiled DFP code and links against the same libgcc.a archives.
//
// Carrier mapping (mirrors the _Float128 strategy):
//   _Decimal32  -> IrType::D32  (SSE-class bit container on x86-64)
//   _Decimal64  -> IrType::D64  (SSE-class bit container on x86-64)
//   _Decimal128 -> IrType::U128 carrier with the f128_sse ABI flags
//                  (16-byte single-XMM class, identical to GCC's TDmode)
// ─────────────────────────────────────────────────────────────────────────────
impl Lowerer {
    /// Width (32/64/128) of a decimal C type, or None for non-decimal types.
    pub(super) fn decimal_width(ct: &CType) -> Option<u8> {
        match ct {
            CType::Decimal32 => Some(32),
            CType::Decimal64 => Some(64),
            CType::Decimal128 => Some(128),
            _ => None,
        }
    }

    /// libbid three-letter suffix for a decimal width: sd / dd / td.
    fn dec_suffix(width: u8) -> &'static str {
        match width {
            32 => "sd",
            64 => "dd",
            _ => "td",
        }
    }

    /// IR carrier type for a decimal width.
    fn dec_ty(width: u8) -> IrType {
        match width {
            32 => IrType::D32,
            64 => IrType::D64,
            _ => IrType::U128,
        }
    }

    /// libbid suffix for an integer width: si (i32) / di (i64) / ti (i128).
    fn int_suffix(ty: IrType) -> &'static str {
        match ty {
            IrType::I8 | IrType::U8 | IrType::I16 | IrType::U16 | IrType::I32 | IrType::U32 => "si",
            IrType::I64 | IrType::U64 => "di",
            _ => "ti",
        }
    }

    /// Emit a libbid helper call with decimal-aware ABI marking.
    /// `args`: (value, IR carrier type, is_decimal128) triples. 128-bit
    /// decimal operands are spilled to 16-byte temps exactly like _Float128
    /// (single-XMM class); D32/D64 operands pass directly.
    pub(super) fn emit_decimal_call(
        &mut self,
        helper: &str,
        args: Vec<(Operand, IrType, bool)>,
        ret: IrType,
        ret_is_decimal128: bool,
    ) -> Value {
        let mut vals = Vec::with_capacity(args.len());
        let mut tys = Vec::with_capacity(args.len());
        let mut infos = Vec::with_capacity(args.len());
        for (op, ty, is128) in args {
            if is128 {
                infos.push(Self::f128_arg_info());
            } else {
                infos.push(None);
            }
            vals.push(op);
            tys.push(ty);
        }
        let ret_classes = if ret_is_decimal128 {
            Self::f128_ret_classes()
        } else {
            Vec::new()
        };
        self.emit_softfloat_call(
            helper,
            vals,
            tys,
            infos,
            ret,
            ret_classes,
            ret_is_decimal128,
        )
    }

    /// Convert an operand of C type `src_ct` into decimal type `dst_ct`
    /// (width `dst_w`) via the libbid conversion helpers.
    fn convert_to_decimal(&mut self, src: Operand, src_ct: &CType, dst_w: u8) -> Operand {
        use crate::common::types::target_ptr_size;
        let d = Self::dec_suffix(dst_w);
        let dst_ty = Self::dec_ty(dst_w);
        // Identity: same-width decimal carrier (constants or variables).
        if Self::decimal_width(src_ct) == Some(dst_w) {
            return src;
        }
        // Integer source: __bid_float{si,di,ti}{sd,dd,td} / floatuns*.
        if src_ct.is_integer() {
            let src_ty = IrType::from_ctype(src_ct);
            // Narrow integer constants ride as I32/I64; use the operand's IR
            // width for the helper family so char/short widen correctly.
            let eff_ty = match src {
                Operand::Const(IrConst::I64(_)) => IrType::I64,
                Operand::Const(IrConst::I128(_)) => IrType::I128,
                _ => src_ty,
            };
            let is128 = matches!(eff_ty, IrType::I128 | IrType::U128);
            let s = Self::int_suffix(eff_ty);
            let uns = if src_ct.is_unsigned() { "uns" } else { "" };
            let helper = format!("__bid_float{uns}{s}{d}");
            let v =
                self.emit_decimal_call(&helper, vec![(src, eff_ty, false)], dst_ty, dst_w == 128);
            return Operand::Value(v);
        }
        // Decimal source (different width): extend/trunc family.
        if let Some(sw) = Self::decimal_width(src_ct) {
            let s = Self::dec_suffix(sw);
            let helper = if sw < dst_w {
                format!("__bid_extend{s}{d}2")
            } else {
                format!("__bid_trunc{s}{d}2")
            };
            let v = self.emit_decimal_call(
                &helper,
                vec![(src, Self::dec_ty(sw), sw == 128)],
                dst_ty,
                dst_w == 128,
            );
            return Operand::Value(v);
        }
        // Binary float source: sf/df/xf/tf -> {sd,dd,td}.
        let s = match src_ct {
            CType::Float => "sf",
            CType::Double => "df",
            CType::LongDouble => "xf",
            CType::Float128 => "tf",
            _ => "df",
        };
        // Ground-truth helper names from the libbid symbol table (GCC 16):
        // widening vs narrowing follows GCC's own naming.
        let helper = match (s, d) {
            ("sf", "sd") => "__bid_extendsfsd".to_string(),
            ("df", "sd") => "__bid_truncdfsd".to_string(),
            ("xf", "sd") => "__bid_truncxfsd".to_string(),
            ("tf", "sd") => "__bid_trunctfsd".to_string(),
            ("sf", "dd") => "__bid_extendsfdd".to_string(),
            ("df", "dd") => "__bid_extenddfdd".to_string(),
            ("xf", "dd") => "__bid_truncxfdd".to_string(),
            ("tf", "dd") => "__bid_trunctfdd".to_string(),
            ("sf", "td") => "__bid_extendsftd".to_string(),
            ("df", "td") => "__bid_extenddftd".to_string(),
            ("xf", "td") => "__bid_extendxftd".to_string(),
            ("tf", "td") => "__bid_extendtftd".to_string(),
            _ => format!("__bid_extend{s}{d}2"),
        };
        let src_ty = match src_ct {
            CType::Float => IrType::F32,
            CType::Double => IrType::F64,
            CType::LongDouble => IrType::F128,
            CType::Float128 => IrType::U128,
            _ => IrType::F64,
        };
        // xf (x87 long double) args/rets use the F128 memory class.
        let arg_is_128 = src_ty == IrType::U128;
        let v = self.emit_decimal_call(
            &helper,
            vec![(src, src_ty, arg_is_128)],
            dst_ty,
            dst_w == 128,
        );
        let _ = target_ptr_size();
        Operand::Value(v)
    }

    /// Convert a decimal operand (width `src_w`) to a non-decimal C type.
    fn convert_from_decimal(&mut self, src: Operand, src_w: u8, target_ct: &CType) -> Operand {
        let s = Self::dec_suffix(src_w);
        let src_ty = Self::dec_ty(src_w);
        // Decimal -> integer: __bid_fix{,uns}{sd,dd,td}{si,di,ti}.
        if target_ct.is_integer() {
            let dst_ty = IrType::from_ctype(target_ct);
            let d = Self::int_suffix(dst_ty);
            let uns = if target_ct.is_unsigned() { "uns" } else { "" };
            let helper = format!("__bid_fix{uns}{s}{d}");
            let ret128 = matches!(dst_ty, IrType::I128 | IrType::U128);
            let v =
                self.emit_decimal_call(&helper, vec![(src, src_ty, src_w == 128)], dst_ty, false);
            return Operand::Value(v);
        }
        // Decimal -> binary float: {sd,dd,td} -> sf/df/xf/tf.
        let d = match target_ct {
            CType::Float => "sf",
            CType::Double => "df",
            CType::LongDouble => "xf",
            CType::Float128 => "tf",
            _ => "df",
        };
        let helper = match (s, d) {
            ("sd", "sf") => "__bid_truncsdsf".to_string(),
            ("sd", "df") => "__bid_truncsddf".to_string(),
            ("sd", "xf") => "__bid_extendsdxf".to_string(),
            ("sd", "tf") => "__bid_extendsdtf".to_string(),
            ("dd", "sf") => "__bid_truncddsf".to_string(),
            ("dd", "df") => "__bid_truncdddf".to_string(),
            ("dd", "xf") => "__bid_extendddxf".to_string(),
            ("dd", "tf") => "__bid_extendddtf".to_string(),
            ("td", "sf") => "__bid_trunctdsf".to_string(),
            ("td", "df") => "__bid_trunctddf".to_string(),
            ("td", "xf") => "__bid_trunctdxf".to_string(),
            ("td", "tf") => "__bid_trunctdtf".to_string(),
            _ => format!("__bid_trunc{s}{d}"),
        };
        let ret_ty = match target_ct {
            CType::Float => IrType::F32,
            CType::Double => IrType::F64,
            CType::LongDouble => IrType::F128,
            CType::Float128 => IrType::U128,
            _ => IrType::F64,
        };
        let ret_is_128 = ret_ty == IrType::U128;
        let v = self.emit_decimal_call(
            &helper,
            vec![(src, src_ty, src_w == 128)],
            ret_ty,
            ret_is_128,
        );
        Operand::Value(v)
    }

    /// Full decimal-aware scalar conversion dispatcher (CType-level).
    pub(super) fn convert_decimal_scalar(
        &mut self,
        src: Operand,
        src_ty: IrType,
        src_ct: &CType,
        target_ct: &CType,
    ) -> Operand {
        // Identity within the same decimal type: bit-exact move.
        if Self::decimal_width(src_ct).is_some() && src_ct == target_ct {
            return src;
        }
        if let Some(dw) = Self::decimal_width(target_ct) {
            if Self::decimal_width(src_ct).is_none()
                && !src_ct.is_integer()
                && !src_ct.is_floating()
            {
                // Pointer/aggregate source: not a valid decimal conversion;
                // fall through to the generic bit-cast machinery, which
                // produces the appropriate diagnostic downstream.
                let target_ty = IrType::from_ctype(target_ct);
                return self.emit_implicit_cast(src, src_ty, target_ty);
            }
            return self.convert_to_decimal(src, src_ct, dw);
        }
        if let Some(sw) = Self::decimal_width(src_ct) {
            return self.convert_from_decimal(src, sw, target_ct);
        }
        unreachable!("convert_decimal_scalar requires a decimal side")
    }

    /// Lower a decimal arithmetic/comparison binary operation. `lhs`/`rhs`
    /// have already been checked to involve at least one decimal operand.
    pub(super) fn lower_decimal_binop(
        &mut self,
        op: &ast::BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Operand {
        let lhs_ct = self.expr_ctype(lhs);
        let rhs_ct = self.expr_ctype(rhs);
        // Common type per the C23 usual arithmetic conversions (decimal
        // hierarchy; integers convert to the decimal operand's type).
        let common_ct = CType::usual_arithmetic_conversion(&lhs_ct, &rhs_ct);
        let Some(w) = Self::decimal_width(&common_ct) else {
            // Decimal mixed with binary float: binary float wins per our UAC
            // rule; fall back to the generic arithmetic lowering, which
            // performs the binary-float conversions through convert_scalar_ctype.
            return self.lower_arithmetic_binop(op, lhs, rhs);
        };
        let lval = self.lower_expr(lhs);
        let rval = self.lower_expr(rhs);
        let l = self.convert_decimal_scalar(lval, self.get_expr_type(lhs), &lhs_ct, &common_ct);
        let r = self.convert_decimal_scalar(rval, self.get_expr_type(rhs), &rhs_ct, &common_ct);
        let s = Self::dec_suffix(w);
        let arg_l = (l.clone(), Self::dec_ty(w), w == 128);
        let arg_r = (r, Self::dec_ty(w), w == 128);
        match op {
            ast::BinOp::Add => {
                let v = self.emit_decimal_call(
                    &format!("__bid_add{s}3"),
                    vec![arg_l, arg_r],
                    Self::dec_ty(w),
                    w == 128,
                );
                Operand::Value(v)
            }
            ast::BinOp::Sub => {
                let v = self.emit_decimal_call(
                    &format!("__bid_sub{s}3"),
                    vec![arg_l, arg_r],
                    Self::dec_ty(w),
                    w == 128,
                );
                Operand::Value(v)
            }
            ast::BinOp::Mul => {
                let v = self.emit_decimal_call(
                    &format!("__bid_mul{s}3"),
                    vec![arg_l, arg_r],
                    Self::dec_ty(w),
                    w == 128,
                );
                Operand::Value(v)
            }
            ast::BinOp::Div => {
                let v = self.emit_decimal_call(
                    &format!("__bid_div{s}3"),
                    vec![arg_l, arg_r],
                    Self::dec_ty(w),
                    w == 128,
                );
                Operand::Value(v)
            }
            ast::BinOp::Eq
            | ast::BinOp::Ne
            | ast::BinOp::Lt
            | ast::BinOp::Le
            | ast::BinOp::Gt
            | ast::BinOp::Ge => {
                // libbid comparisons return a three-way int (-1/0/1); normalize
                // against zero exactly like GCC's boolean lowering.
                let name = match op {
                    ast::BinOp::Eq => format!("__bid_eq{s}2"),
                    ast::BinOp::Ne => format!("__bid_ne{s}2"),
                    ast::BinOp::Lt => format!("__bid_lt{s}2"),
                    ast::BinOp::Le => format!("__bid_le{s}2"),
                    ast::BinOp::Gt => format!("__bid_gt{s}2"),
                    _ => format!("__bid_ge{s}2"),
                };
                let v = self.emit_decimal_call(&name, vec![arg_l, arg_r], IrType::I32, false);
                let cmp_op = match op {
                    ast::BinOp::Eq => IrCmpOp::Eq,
                    ast::BinOp::Ne => IrCmpOp::Ne,
                    ast::BinOp::Lt => IrCmpOp::Slt,
                    ast::BinOp::Le => IrCmpOp::Sle,
                    ast::BinOp::Gt => IrCmpOp::Sgt,
                    _ => IrCmpOp::Sge,
                };
                let z = self.emit_cmp_val(
                    cmp_op,
                    Operand::Value(v),
                    Operand::Const(IrConst::I64(0)),
                    IrType::I32,
                );
                Operand::Value(z)
            }
            _ => {
                // Remainder/bitwise on decimals is invalid C; defer to the
                // generic path so sema diagnostics surface unchanged.
                self.lower_arithmetic_binop(op, lhs, rhs)
            }
        }
    }

    /// Truthiness test for a decimal operand (`x != 0` under decimal
    /// semantics, where +0 == -0). Used by conditions and logical operators.
    pub(super) fn lower_decimal_truthiness(&mut self, val: Operand, ct: &CType) -> Operand {
        let Some(w) = Self::decimal_width(ct) else {
            return val;
        };
        let s = Self::dec_suffix(w);
        let zero: Operand = match w {
            32 => Operand::Const(IrConst::D32(Self::dec_zero32())),
            64 => Operand::Const(IrConst::D64(Self::dec_zero64())),
            _ => Operand::Const(IrConst::I128(Self::dec_zero128())),
        };
        let v = self.emit_decimal_call(
            &format!("__bid_ne{s}2"),
            vec![
                (val, Self::dec_ty(w), w == 128),
                (zero, Self::dec_ty(w), w == 128),
            ],
            IrType::I32,
            false,
        );
        let z = self.emit_cmp_val(
            IrCmpOp::Ne,
            Operand::Value(v),
            Operand::Const(IrConst::I64(0)),
            IrType::I32,
        );
        Operand::Value(z)
    }

    /// Canonical +0 encodings (biased exponent, zero coefficient) — the
    /// exact bit patterns GCC emits for `0.DF/0.DD/0.DL` literals.
    fn dec_zero32() -> u32 {
        (96u32) << 23 // bias 96
    }
    fn dec_zero64() -> u64 {
        (398u64) << 53 // bias 398
    }
    fn dec_zero128() -> i128 {
        ((6176u128) << 49) as i128 // bias 6176
    }
}

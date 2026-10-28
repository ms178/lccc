//! Local variable initialization lowering.
//!
//! Extracted from stmt.rs to reduce that file's size. Contains the logic for
//! lowering `Initializer::Expr` and `Initializer::List` for local variable
//! declarations, plus helpers for registering block-scope function declarations.

use super::definitions::{DeclAnalysis, FuncSig, GlobalInfo};
use super::lower::Lowerer;
use super::vector_array_init::VectorArrayInitPlan;
use crate::common::type_builder::TypeConvertContext;
use crate::common::types::{AddressSpace, CType, IrType};
use crate::frontend::parser::ast::{
    Declaration, DerivedDeclarator, Designator, Expr, InitDeclarator, Initializer, InitializerItem,
    ParamDecl, TypeSpecifier,
};
use crate::ir::reexports::{Instruction, IrConst, Operand, Value};

impl Lowerer {
    /// Handle extern declarations inside function bodies.
    /// They reference a global symbol, not a local variable.
    pub(super) fn lower_extern_decl(
        &mut self,
        decl: &Declaration,
        declarator: &InitDeclarator,
    ) -> bool {
        // Remove from locals, but track in scope frame so pop_scope()
        // restores the local when this block exits.
        self.shadow_local_for_scope(&declarator.name);
        // Also remove from static_local_names so that the extern name
        // resolves to the true global, not a same-named static local
        self.shadow_static_for_scope(&declarator.name);

        // Propagate __attribute__((weak)) / visibility from ANY block-scope
        // extern declaration — including `extern typeof(f) f
        // __attribute__((weak, visibility("hidden")))`, where typeof supplies
        // the function type and `derived` is empty so the function-declaration
        // handler below never sees it. The kernel's !CONFIG_MODULES
        // symbol_get() expands to exactly that shape (module.h:835); dropping
        // the attributes left ips_link_to_i915_driver a STB_GLOBAL undef and
        // the vmlinux link failed instead of resolving it to NULL.
        if !declarator.name.is_empty()
            && (declarator.attrs.is_weak() || declarator.attrs.visibility.is_some())
        {
            self.module.symbol_attrs.push((
                declarator.name.clone(),
                declarator.attrs.is_weak(),
                declarator.attrs.visibility.clone(),
            ));
        }

        // Check if this is a function declaration (extern int f(int))
        let is_func_decl = DerivedDeclarator::declares_function(&declarator.derived);
        if is_func_decl {
            return false; // Fall through to the function declaration handler
        }

        if !self.globals.contains_key(&declarator.name) {
            let mut ext_da = self.analyze_declaration(&decl.type_spec, &declarator.derived);
            self.apply_declaration_qualifiers(&mut ext_da, decl, declarator);
            ext_da.apply_decl_vector_type(decl);
            // A block-scope `extern` names an object with linkage, so it is
            // placed exactly like its file-scope declaration (global_decl.rs).
            // Skipping this left `extern long __seg_gs x;` in generic memory.
            let mut ginfo = GlobalInfo::from_analysis(&ext_da);
            ginfo.var.address_space = crate::common::type_builder::place_declared_space(
                ginfo.var.c_type.as_mut(),
                self.declaration_address_space(decl.address_space, &decl.type_spec),
                &decl.type_spec,
                &declarator.derived,
                declarator.attrs.object_space,
            );
            self.globals.insert(declarator.name.clone(), ginfo);
        }
        self.emit_extern_tls_global(decl, declarator);
        if declarator.attrs.tls_model.is_some() || decl.is_thread_local() {
            let ir_name = self.global_object_ir_name(declarator);
            self.record_tls_model_attr(
                ir_name,
                declarator,
                super::global_decl::TlsAttrSubject::object(decl.is_thread_local()),
            );
        }
        true // Handled, caller should continue to next declarator
    }

    /// Handle block-scope function declarations: `int f(int);`
    /// These declare an external function, not a local variable.
    /// Returns true if this was a function declaration (caller should continue to next declarator).
    pub(super) fn try_lower_block_func_decl(
        &mut self,
        decl: &Declaration,
        declarator: &InitDeclarator,
    ) -> bool {
        // If there's an initializer, this is a variable declaration, not a function declaration
        if declarator.init.is_some() {
            return false;
        }

        // Block-scope function declaration: `type name(params);`, including
        // pointer returns (`int *f(int)`) and functions returning function
        // pointers (`void (*f(int))(void)`, derived
        // `[Pointer, FunctionPointer(void), Function(int)]`).  The canonical
        // predicate keys on the declarator's OWN parameter list being last;
        // function-pointer VARIABLES (`int (*(*p)(int))(int)`) end in
        // FunctionPointer and fall through to the variable path.  Everything
        // before that parameter list is the return type; every such prefix
        // makes the return a pointer (array/function returns are invalid C),
        // so the pointer count is the ABI-relevant summary the meta needs.
        let func_info = if DerivedDeclarator::declares_function(&declarator.derived) {
            match declarator.derived.last() {
                Some(DerivedDeclarator::Function(p, v, _)) => Some((p.clone(), *v)),
                _ => None,
            }
        } else {
            None
        };
        let ptr_count = DerivedDeclarator::return_pointer_depth(&declarator.derived);
        if let Some((params, variadic)) = func_info {
            let entity =
                self.block_scope_entity_key(&declarator.name, declarator.attrs.is_nested_fn());
            self.register_block_func_meta(
                entity.as_deref(),
                &declarator.name,
                &decl.type_spec,
                ptr_count,
                &params,
                variadic,
            );
            self.shadow_local_for_scope(&declarator.name);
            // Propagate __attribute__((error("..."))) and __attribute__((noreturn)) / _Noreturn
            // from block-scope function declarations. Without this, the kernel's BUILD_BUG_ON
            // macro (which uses block-scope `extern void __compiletime_assert_NNN(void)
            // __attribute__((error(...)))`) would emit real calls to undefined symbols
            // instead of replacing them with Unreachable.
            // Same scoping law as the signature table above: these sets are
            // keyed by symbol, so an `auto` declaration must not attach its
            // attributes to the file-scope entity of the same basename -- an
            // `error` would reject a legal call elsewhere in the TU, and a
            // `noreturn` would let code after an unrelated call be dropped as
            // unreachable. When the entity is a nested one (mangled key
            // available) the fact is recorded there; when it is a nested
            // prototype with no definition in this TU, nothing can call it, so
            // the fact has no carrier and is dropped.
            let attr_key =
                self.block_scope_entity_key(&declarator.name, declarator.attrs.is_nested_fn());
            let attr_key = attr_key.filter(|_| !declarator.name.is_empty());
            if declarator.attrs.is_error_attr() {
                if let Some(k) = &attr_key {
                    self.error_functions.insert(k.clone());
                }
            }
            if declarator.attrs.is_noreturn() {
                if let Some(k) = &attr_key {
                    self.noreturn_functions.insert(k.clone());
                }
            }
            if declarator.attrs.is_pure() {
                if let Some(k) = &attr_key {
                    self.pure_functions.insert(k.clone());
                }
            }
            if declarator.attrs.is_const_attr() {
                if let Some(k) = &attr_key {
                    self.const_functions.insert(k.clone());
                }
            }
            // Propagate __attribute__((weak)) and __attribute__((visibility(...)))
            // from block-scope extern function declarations.
            // `weak`/`visibility` are properties of the *linker symbol*: a
            // nested function is not the file-scope symbol of that basename, so
            // an `auto` declaration must not retag it. The nested definition
            // carries its own attributes for its own symbol.
            if !declarator.attrs.is_nested_fn()
                && !declarator.name.is_empty()
                && (declarator.attrs.is_weak() || declarator.attrs.visibility.is_some())
            {
                self.module.symbol_attrs.push((
                    declarator.name.clone(),
                    declarator.attrs.is_weak(),
                    declarator.attrs.visibility.clone(),
                ));
            }
            return true;
        }

        // Also handle typedef-based function declarations in block scope
        // (e.g., `func_t add;` where func_t is `typedef int func_t(int);`)
        if declarator.derived.is_empty() && declarator.init.is_none() {
            if let TypeSpecifier::TypedefName(tname) = &decl.type_spec {
                if let Some(fti) = self.types.function_typedefs.get(tname).cloned() {
                    let entity = self
                        .block_scope_entity_key(&declarator.name, declarator.attrs.is_nested_fn());
                    self.register_block_func_meta(
                        entity.as_deref(),
                        &declarator.name,
                        &fti.return_type,
                        0,
                        &fti.params,
                        fti.variadic,
                    );
                    self.shadow_local_for_scope(&declarator.name);
                    if declarator.attrs.is_error_attr() && !declarator.name.is_empty() {
                        self.error_functions.insert(declarator.name.clone());
                    }
                    if declarator.attrs.is_noreturn() && !declarator.name.is_empty() {
                        self.noreturn_functions.insert(declarator.name.clone());
                    }
                    if declarator.attrs.is_weak() || declarator.attrs.visibility.is_some() {
                        self.module.symbol_attrs.push((
                            declarator.name.clone(),
                            declarator.attrs.is_weak(),
                            declarator.attrs.visibility.clone(),
                        ));
                    }
                    return true;
                }
            }
        }

        false
    }

    /// Register function metadata for a block-scope function declaration.
    /// Must compute the same ABI metadata as `register_function_meta` so that
    /// member access on struct return values (e.g., `stfunc1().field`) works
    /// correctly even when the function is only forward-declared at block scope.
    /// The symbol under which a block-scope function declaration's module-level
    /// facts (signature, `error`/`noreturn`/`pure`/`const`, weak/visibility) are
    /// recorded -- or `None` when there is no module-level fact to record.
    ///
    /// C11 6.7.1p5: a block-scope declaration of a function *without* `auto`
    /// declares the externally-visible identifier, so its facts belong to that
    /// symbol and any function in the translation unit may legitimately read
    /// them. GNU `auto` is the one spelling that says otherwise -- it names a
    /// nested function, a distinct entity that merely shares a basename, is
    /// registered as `parent.name`, and is visible only in that parent's scope.
    /// Writing its facts under the plain basename reaches functions that never
    /// see the declaration: measured on
    ///
    /// ```c
    /// int f(int x) { return x + 1; }
    /// int g(void) { auto double f(double); return 0; }
    /// int main(void) { return f(41) != 42; }
    /// ```
    ///
    /// `main`'s call was emitted as a *double* call with `vcvtsi2sd` on the
    /// argument (lccc exited 1, gcc 0) because the signature table is keyed per
    /// module. The same mechanism through `__attribute__((noreturn))` would
    /// delete `main`'s code after the call as unreachable, and through
    /// `__attribute__((error("...")))` would reject a call that is perfectly
    /// legal. A nested entity gets its facts from its own definition, which is
    /// emitted under the mangled symbol; a nested *prototype* with no definition
    /// in this translation unit cannot be called at all (GCC rejects it), so
    /// dropping the entry loses nothing.
    fn block_scope_entity_key(&self, name: &str, is_auto: bool) -> Option<String> {
        if !is_auto {
            return Some(name.to_string());
        }
        self.func()
            .nested_functions
            .get(name)
            .map(|e| e.mangled.clone())
    }

    fn register_block_func_meta(
        &mut self,
        entity_key: Option<&str>,
        name: &str,
        ret_type_spec: &TypeSpecifier,
        ptr_count: usize,
        params: &[ParamDecl],
        variadic: bool,
    ) {
        self.known_functions.insert(name.to_string());
        // The signature table is per-module, so a nested declaration is
        // recorded under its mangled entity name -- the same key
        // `callee_signature_key` resolves a call through -- and not at all when
        // no nested entity of that name is visible here.
        let Some(entity_key) = entity_key else {
            return;
        };
        let name = entity_key;
        let sig = self.build_function_sig(name, ret_type_spec, ptr_count, params, variadic, false);
        // Don't overwrite an existing, more complete sig from the first pass
        // (e.g., a K&R definition) with a less complete block-scope declaration
        // that has empty/unspecified parameters like `int *f();`.
        if let Some(existing) = self.func_meta.sigs.get(name) {
            if sig.param_types.is_empty() && !existing.param_types.is_empty() {
                return;
            }
            if sig.param_struct_sizes.is_empty() && !existing.param_struct_sizes.is_empty() {
                return;
            }
        }
        self.func_meta.sigs.insert(name.to_string(), sig);
    }

    /// Lower an `Initializer::Expr` for a local variable declaration.
    /// Handles char arrays from string literals, struct copy-init, complex init,
    /// and scalar init with implicit casts.
    pub(super) fn lower_local_init_expr(
        &mut self,
        expr: &Expr,
        alloca: Value,
        da: &DeclAnalysis,
        is_complex: bool,
        decl: &Declaration,
    ) {
        if da.is_array && (da.base_ty == IrType::I8 || da.base_ty == IrType::U8) {
            self.lower_char_array_init_expr(expr, alloca, da);
        } else if da.is_array && (da.base_ty == IrType::I32 || da.base_ty == IrType::U32) {
            self.lower_wchar_array_init_expr(expr, alloca, da);
        } else if da.is_array && (da.base_ty == IrType::I16 || da.base_ty == IrType::U16) {
            self.lower_char16_array_init_expr(expr, alloca, da);
        } else if da.is_struct {
            self.lower_struct_copy_init(expr, alloca, da);
        } else if is_complex {
            self.lower_complex_var_init(expr, alloca, da, decl);
        } else if let Some(ref ct) = da.c_type {
            if ct.is_vector() {
                self.copy_vector_expr_to_alloca(expr, alloca, ct);
            } else {
                self.lower_scalar_init_expr(expr, alloca, da, decl);
            }
        } else {
            self.lower_scalar_init_expr(expr, alloca, da, decl);
        }
    }

    /// Copy a vector expression into its destination object. Small vector
    /// calls return packed register data rather than an address; spill those
    /// values using the same size-correct path used for ordinary vector copy
    /// initialization.
    fn copy_vector_expr_to_alloca(&mut self, expr: &Expr, destination: Value, vector_ct: &CType) {
        // Scalar right operand: Clang splats it over every lane
        // (`ext_vector_type` only; plain GNU vectors are rejected by sema
        // for scalar initializers). Lower the scalar, convert it to the
        // lane type and store it into each lane — the splat is a value
        // operation, NOT a whole-object copy, so the generic Memcpy path
        // below (which treats the scalar's register value as the source
        // address) must never see it. Mirrors the scalar splat in
        // `try_lower_swizzle_assign`.
        let expr_ct = self.expr_ctype(expr);
        if !expr_ct.is_vector() {
            if let CType::ExtVector(elem_ct, layout) = vector_ct {
                let elem_ir_ty = Self::vector_elem_ir_type(elem_ct);
                let raw = self.lower_expr(expr);
                let raw_ty = self.value_ir_type(expr);
                let value = self.emit_implicit_cast(raw, raw_ty, elem_ir_ty);
                for lane in 0..layout.lanes {
                    self.store_vector_lane(
                        destination,
                        vector_ct,
                        lane,
                        value.clone(),
                        elem_ir_ty,
                        AddressSpace::Default,
                        false,
                    );
                }
                return;
            }
        }
        // Packed whole-object initialization (`vector_simd.rs`): one load,
        // one store, no Memcpy.  Total fallback below.
        if self.try_lower_vector_init_simd(expr, destination, vector_ct) {
            return;
        }
        let total_size = vector_ct.size();
        let is_small_vec_call = self.rhs_is_small_vector_call(expr, total_size);
        let src = self.lower_aggregate_rvalue(expr);
        let src_val = self.operand_to_value(src);
        if is_small_vec_call {
            let tmp_alloca = self.fresh_value();
            let store_ty = Self::packed_store_type(total_size);
            self.emit(Instruction::Alloca {
                dest: tmp_alloca,
                size: total_size,
                ty: store_ty,
                align: 0,
                volatile: false,
                semantic_volatile: false,
            });
            self.emit(Instruction::Store {
                volatile: false,
                val: Operand::Value(src_val),
                ptr: tmp_alloca,
                ty: store_ty,
                seg_override: AddressSpace::Default,
            });
            self.emit(Instruction::Memcpy {
                dest: destination,
                src: tmp_alloca,
                size: total_size,
            });
        } else {
            self.emit(Instruction::Memcpy {
                dest: destination,
                src: src_val,
                size: total_size,
            });
        }
    }

    /// Char array from string literal: `char s[] = "hello"`
    fn lower_char_array_init_expr(&mut self, expr: &Expr, alloca: Value, da: &DeclAnalysis) {
        match expr {
            Expr::StringLiteral(s, _)
            | Expr::WideStringLiteral(s, _)
            | Expr::Char16StringLiteral(s, _) => {
                let arr_size = da.alloc_size;
                self.emit_string_to_alloca(alloca, s, 0, arr_size);
                // Zero-fill remaining bytes if string is shorter than array
                let str_len = s.chars().count() + 1; // +1 for null terminator
                self.zero_fill_after_string(alloca, str_len, arr_size);
            }
            _ => {
                let val = self.lower_expr(expr);
                self.emit(Instruction::Store {
                    volatile: false,
                    val,
                    ptr: alloca,
                    ty: da.var_ty,
                    seg_override: AddressSpace::Default,
                });
            }
        }
    }

    /// wchar_t/int array from wide string: `wchar_t s[] = L"hello"`
    fn lower_wchar_array_init_expr(&mut self, expr: &Expr, alloca: Value, da: &DeclAnalysis) {
        match expr {
            Expr::WideStringLiteral(s, _)
            | Expr::StringLiteral(s, _)
            | Expr::Char16StringLiteral(s, _) => {
                let arr_size = da.alloc_size;
                self.emit_wide_string_to_alloca(alloca, s, 0, arr_size);
                // The elements after the terminator are zero (C11 6.7.9p21);
                // nothing wrote them before this fix.
                let str_len = (s.chars().count() + 1) * 4;
                self.zero_fill_after_string(alloca, str_len, arr_size);
            }
            _ => {
                let val = self.lower_expr(expr);
                self.emit(Instruction::Store {
                    volatile: false,
                    val,
                    ptr: alloca,
                    ty: da.var_ty,
                    seg_override: AddressSpace::Default,
                });
            }
        }
    }

    /// char16_t array from u"..." string literal: `char16_t s[] = u"hello"`
    fn lower_char16_array_init_expr(&mut self, expr: &Expr, alloca: Value, da: &DeclAnalysis) {
        match expr {
            Expr::Char16StringLiteral(s, _)
            | Expr::StringLiteral(s, _)
            | Expr::WideStringLiteral(s, _) => {
                let arr_size = da.alloc_size;
                self.emit_char16_string_to_alloca(alloca, s, 0, arr_size);
                // Zero-fill remaining bytes if string is shorter than array
                let str_len = (s.encode_utf16().count() + 1) * 2; // UTF-16 units + NUL
                self.zero_fill_after_string(alloca, str_len, arr_size);
            }
            _ => {
                let val = self.lower_expr(expr);
                self.emit(Instruction::Store {
                    volatile: false,
                    val,
                    ptr: alloca,
                    ty: da.var_ty,
                    seg_override: AddressSpace::Default,
                });
            }
        }
    }

    /// Struct copy-initialization: `struct Point b = a;`
    fn lower_struct_copy_init(&mut self, expr: &Expr, alloca: Value, da: &DeclAnalysis) {
        // For expressions producing packed struct data (small struct
        // function call returns, ternaries over them, etc.), the value
        // IS the struct data, not an address. Store directly.
        if self.expr_produces_packed_struct_data(expr) && da.actual_alloc_size <= 8 {
            let val = self.lower_expr(expr);
            self.emit(Instruction::Store {
                volatile: false,
                val,
                ptr: alloca,
                ty: Self::packed_store_type(da.actual_alloc_size),
                seg_override: AddressSpace::Default,
            });
        } else {
            let src_addr = self.get_struct_base_addr(expr);
            self.copy_aggregate_from_expr(alloca, expr, src_addr, da.actual_alloc_size);
        }
    }

    /// Complex variable initialization: `_Complex double z = expr;`
    fn lower_complex_var_init(
        &mut self,
        expr: &Expr,
        alloca: Value,
        da: &DeclAnalysis,
        decl: &Declaration,
    ) {
        let complex_ctype = self.type_spec_to_ctype(&decl.type_spec);
        let src = self.lower_expr_to_complex(expr, &complex_ctype);
        self.emit(Instruction::Memcpy {
            dest: alloca,
            src,
            size: da.actual_alloc_size,
        });
    }

    /// Scalar variable initialization with implicit casts and const tracking.
    fn lower_scalar_init_expr(
        &mut self,
        expr: &Expr,
        alloca: Value,
        da: &DeclAnalysis,
        decl: &Declaration,
    ) {
        // Track const-qualified integer variable values for compile-time
        // array size evaluation (e.g., const int len = 5000; int arr[len];)
        if decl.is_const() && !da.is_pointer && !da.is_array && !da.is_struct {
            if let Some(const_val) = self.eval_const_expr(expr) {
                if let Some(_ival) = self.const_to_i64(&const_val) {
                    // Use declarator name from the declaration context
                    // (passed via da which carries the name context)
                    // Actually we need the name; we'll handle this at the call site
                }
            }
        }
        // Check if RHS is complex but LHS is non-complex:
        // extract real part first, then convert to target type.
        let rhs_ctype = self.expr_ctype(expr);
        let val = if rhs_ctype.is_complex() {
            let complex_val = self.lower_expr(expr);
            let ptr = self.operand_to_value(complex_val);
            if da.is_bool {
                // For _Bool targets, check both real and imag parts per C11 6.3.1.2
                self.lower_complex_to_bool(ptr, &rhs_ctype)
            } else {
                let real_part = self.load_complex_real(ptr, &rhs_ctype);
                let from_ty = Self::complex_component_ir_type(&rhs_ctype);
                self.emit_implicit_cast(real_part, from_ty, da.var_ty)
            }
        } else if da.is_bool {
            // For _Bool targets, normalize at the source type before any truncation.
            // Truncating first (e.g. 0x100 -> U8 = 0) then normalizing gives wrong results.
            let val = self.lower_expr(expr);
            let expr_ty = self.value_ir_type(expr);
            self.emit_expr_bool_conversion(val, expr_ty, expr)
        } else {
            let val = self.lower_expr(expr);
            let expr_ty = self.value_ir_type(expr);
            match &da.c_type {
                Some(target_ct) => {
                    self.convert_assign_value(val, expr_ty, Some(&rhs_ctype), target_ct, da.var_ty)
                }
                None => self.emit_implicit_cast(val, expr_ty, da.var_ty),
            }
        };
        self.emit(Instruction::Store {
            volatile: false,
            val,
            ptr: alloca,
            ty: da.var_ty,
            seg_override: AddressSpace::Default,
        });
    }

    /// Lower `Initializer::List` for a local variable declaration.
    /// Dispatches to complex, struct, array, or scalar-with-braces handlers.
    pub(super) fn lower_local_init_list(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
        is_complex: bool,
        complex_elem_ctype: &Option<CType>,
        decl: &Declaration,
        declarator_name: &str,
    ) {
        // Resolve brace elision, designator chains and overrides of
        // struct/union and array-of-aggregate initializers once
        // (`init_canon`); canonical lists are lowered as written.
        let canonical = da.c_type.as_ref().and_then(|t| {
            let t = self.complete_array_ctype(t, da.alloc_size);
            self.canonical_init_items(items, &t, None)
        });
        let items = canonical.as_deref().unwrap_or(items);
        if is_complex {
            self.lower_complex_init_list(items, alloca, decl);
        } else if da.is_struct {
            self.lower_struct_init_list(items, alloca, declarator_name);
        } else if da.is_array && da.elem_size > 0 {
            self.lower_array_init_list_dispatch(
                items,
                alloca,
                da,
                complex_elem_ctype,
                decl,
                declarator_name,
            );
        } else if let Some(ref ct) = da.c_type {
            if ct.is_vector() {
                self.lower_vector_init_list(items, alloca, ct);
            } else {
                // Scalar with braces: int x = { 1 };
                self.lower_scalar_braced_init(items, alloca, da);
            }
        } else {
            // Scalar with braces: int x = { 1 };
            self.lower_scalar_braced_init(items, alloca, da);
        }
    }

    /// Complex initializer list: `_Complex double z = {real, imag}`
    fn lower_complex_init_list(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        decl: &Declaration,
    ) {
        let complex_ctype = self.type_spec_to_ctype(&decl.type_spec);
        let comp_ty = Self::complex_component_ir_type(&complex_ctype);
        // Store real part (first item)
        if let Some(item) = items.first() {
            if let Initializer::Expr(expr) = &item.init {
                let comp_ct = Self::complex_component_ctype(&complex_ctype);
                let val = self.lower_init_leaf(expr, &comp_ct);
                self.emit(Instruction::Store {
                    volatile: false,
                    val,
                    ptr: alloca,
                    ty: comp_ty,
                    seg_override: AddressSpace::Default,
                });
            }
        } else {
            let zero = Self::complex_zero(comp_ty);
            self.emit(Instruction::Store {
                volatile: false,
                val: zero,
                ptr: alloca,
                ty: comp_ty,
                seg_override: AddressSpace::Default,
            });
        }
        // Store imag part (second item) at offset
        let comp_size = Self::complex_component_size(&complex_ctype);
        let imag_ptr = self.emit_gep_offset(alloca, comp_size, IrType::I8);
        if let Some(item) = items.get(1) {
            if let Initializer::Expr(expr) = &item.init {
                let comp_ct = Self::complex_component_ctype(&complex_ctype);
                let val = self.lower_init_leaf(expr, &comp_ct);
                self.emit(Instruction::Store {
                    volatile: false,
                    val,
                    ptr: imag_ptr,
                    ty: comp_ty,
                    seg_override: AddressSpace::Default,
                });
            }
        } else {
            let zero = Self::complex_zero(comp_ty);
            self.emit(Instruction::Store {
                volatile: false,
                val: zero,
                ptr: imag_ptr,
                ty: comp_ty,
                seg_override: AddressSpace::Default,
            });
        }
    }

    /// Struct initializer list with designated initializer support.
    fn lower_struct_init_list(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        declarator_name: &str,
    ) {
        if let Some(layout) = self
            .func_mut()
            .locals
            .get(declarator_name)
            .and_then(|l| l.struct_layout.clone())
        {
            // Always zero-initialize the entire struct before writing explicit values.
            // The C standard (C11 6.7.9p21) requires that all members not explicitly
            // initialized in a brace-enclosed list are implicitly zero-initialized.
            // This handles partial array field init (e.g., struct { int arr[8]; } x = {0};)
            // where a single initializer item covers only one element of an array field.
            self.zero_init_alloca(alloca, layout.size);
            self.emit_struct_init(items, alloca, &layout, 0);
        }
    }

    /// Dispatch array initializer list handling based on array kind.
    fn lower_array_init_list_dispatch(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
        complex_elem_ctype: &Option<CType>,
        decl: &Declaration,
        declarator_name: &str,
    ) {
        // Ext-vector arrays use a shared dimension/designator planner. Zero
        // the complete C object first, then write each initialized vector at
        // its padded C stride; the pointer IR carrier is never an array stride.
        if let Some(array_type) = da
            .c_type
            .as_ref()
            .map(|ty| self.complete_array_ctype(ty, da.alloc_size))
        {
            if let Some(plan) = self.plan_vector_array_init(items, &array_type) {
                self.lower_local_vector_array_plan(plan, alloca, da.alloc_size);
                return;
            }
        }

        // Arrays of scalars of any rank, including `{ "str" }` for character
        // arrays: resolved by the array planner.
        if let Some(geo) = da
            .c_type
            .as_ref()
            .map(|t| self.complete_array_ctype(t, da.alloc_size))
            .and_then(|t| self.scalar_array_geometry(&t))
        {
            let plan = self.plan_array_init(items, &geo);
            self.store_array_plan(&plan, &geo, alloca, 0, false);
            return;
        }

        // For arrays of function pointers or pointer arrays, the struct layout
        // (from the return type or pointee type) must not trigger struct init path.
        let elem_struct_layout = if da.is_array_of_func_ptrs || da.is_array_of_pointers {
            None
        } else {
            self.func_mut()
                .locals
                .get(declarator_name)
                .and_then(|l| l.struct_layout.clone())
        };

        if let Some(cplx_ctype) = complex_elem_ctype {
            // Array of complex elements (handles both 1D and multi-dimensional)
            self.lower_array_of_complex_init(items, alloca, da, cplx_ctype);
        } else if da.is_array_of_pointers || da.is_array_of_func_ptrs {
            // Array of pointers (including pointer-to-struct): use scalar init with pointer stride.
            // Note: elem_struct_layout may be set for p->field access, but the array stride
            // is sizeof(pointer) not sizeof(struct), so we must NOT route to lower_array_of_structs_init.
            if da.array_dim_strides.len() > 1 {
                self.zero_init_alloca(alloca, da.alloc_size);
                let ptr_ct = CType::Pointer(Box::new(CType::Void), AddressSpace::Default);
                self.lower_array_init_list(items, alloca, &ptr_ct, &da.array_dim_strides);
            } else {
                self.lower_1d_array_init(items, alloca, da, decl);
            }
        } else if da.array_dim_strides.len() > 1 && elem_struct_layout.is_none() {
            // Multi-dimensional array of scalars
            self.zero_init_alloca(alloca, da.alloc_size);
            let md_elem_ct = self.init_leaf_ctype(da.c_type.as_ref(), &decl.type_spec);
            self.lower_array_init_list(items, alloca, &md_elem_ct, &da.array_dim_strides);
        } else if let Some(ref s_layout) = elem_struct_layout {
            // Array of structs
            self.lower_array_of_structs_init(items, alloca, da, s_layout);
        } else {
            // 1D array of scalars
            self.lower_1d_array_init(items, alloca, da, decl);
        }
    }

    /// Emit stores/copies for the row-major vector leaves selected by the
    /// shared initializer planner. Missing entries and all padding are already
    /// zero from the whole-object initialization.
    fn lower_local_vector_array_plan(
        &mut self,
        plan: VectorArrayInitPlan,
        alloca: Value,
        allocation_size: usize,
    ) {
        self.zero_init_alloca(alloca, allocation_size);
        let vector_size = plan.vector_type.size();
        for (index, init) in plan.elements.into_iter().enumerate() {
            let Some(init) = init else { continue };
            let offset = index.saturating_mul(vector_size);
            let vector_alloca = if offset == 0 {
                alloca
            } else {
                self.emit_gep_offset(alloca, offset, IrType::I8)
            };
            match init {
                Initializer::List(vector_items) => {
                    self.lower_vector_init_list(&vector_items, vector_alloca, &plan.vector_type);
                }
                Initializer::Expr(expr) => {
                    self.copy_vector_expr_to_alloca(&expr, vector_alloca, &plan.vector_type);
                }
            }
        }
    }

    /// Array of structs initialization with designator support.
    /// Handles arbitrary dimensionality (1D, 2D, 3D, etc.) via recursive
    /// stride peeling through `lower_struct_array_init_recursive`.
    fn lower_array_of_structs_init(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
        s_layout: &crate::common::types::StructLayout,
    ) {
        let struct_size = s_layout.size;
        if struct_size == 0 {
            return;
        }
        self.zero_init_alloca(alloca, da.alloc_size);
        let strides = da.array_dim_strides.clone();
        let s_layout_cloned = s_layout.clone();
        let mut flat_idx = 0usize;
        self.lower_struct_array_init_recursive(
            items,
            alloca,
            &strides,
            struct_size,
            &s_layout_cloned,
            &mut flat_idx,
        );
    }

    /// Recursively initialize a (possibly multi-dimensional) array of structs.
    ///
    /// `dim_strides` contains the byte strides for each remaining dimension.
    /// For `struct t a[2][2][2]` with struct_size=8, strides are [32, 16, 8].
    /// At each level, if stride > struct_size, the brace-delimited `List` items
    /// represent sub-arrays that must be recursed into. When stride == struct_size,
    /// we've reached leaf elements and each `List` is a single struct initializer.
    fn lower_struct_array_init_recursive(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        dim_strides: &[usize],
        struct_size: usize,
        s_layout: &crate::common::types::StructLayout,
        flat_idx: &mut usize,
    ) {
        let (this_stride, remaining) = if dim_strides.len() > 1 {
            (dim_strides[0], &dim_strides[1..])
        } else if dim_strides.len() == 1 {
            (dim_strides[0], &dim_strides[0..0])
        } else {
            (struct_size, &dim_strides[0..0])
        };

        // How many leaf structs fit in one element at this dimension level
        let elems_per_slot = if struct_size > 0 {
            this_stride / struct_size
        } else {
            1
        };
        // Recurse whenever there are remaining dimensions, even when this_stride
        // == struct_size (singleton inner dimensions like `a[][1]`) — the brace
        // level still corresponds to a sub-array (Regehr yarpgen
        // *_singleton_inner_dim_* tests).
        let is_subarray = !remaining.is_empty();
        let region_base_flat = *flat_idx;

        let mut item_idx = 0usize;
        while item_idx < items.len() {
            let item = &items[item_idx];

            // Split designators into leading array indices (`[i][j]...`) and any
            // trailing member/sub-array designators (`.field`, `.n.x`, `.arr[k]`).
            let leading_index_count = item
                .designators
                .iter()
                .take_while(|d| matches!(d, Designator::Index(_)))
                .count();
            if leading_index_count > 0 {
                let mut offset_elems = 0usize;
                for (k, d) in item.designators[..leading_index_count].iter().enumerate() {
                    if let Designator::Index(idx_expr) = d {
                        if let Some(idx_val) = self.eval_const_expr_for_designator(idx_expr) {
                            let stride = dim_strides.get(k).copied().unwrap_or(struct_size);
                            let per_dim = if struct_size > 0 {
                                stride / struct_size
                            } else {
                                1
                            };
                            offset_elems += idx_val * per_dim;
                        }
                    }
                }
                *flat_idx = region_base_flat + offset_elems;
            }
            let trailing_designators = &item.designators[leading_index_count..];

            let base_byte_offset = *flat_idx * struct_size;

            // `[idx].field = ...` or `[i][j].n.x = ...`: delegate trailing
            // designators to `emit_struct_init`, which handles scalars, bitfields,
            // braced lists, compound literals (`memcpy`), and nested designators.
            if !trailing_designators.is_empty() {
                let sub_item = InitializerItem {
                    designators: trailing_designators.to_vec(),
                    init: item.init.clone(),
                };
                self.emit_struct_init(
                    std::slice::from_ref(&sub_item),
                    alloca,
                    s_layout,
                    base_byte_offset,
                );
                item_idx += 1;
                continue;
            }

            let remaining_after_desig = if leading_index_count > 1 {
                &dim_strides[leading_index_count.min(dim_strides.len())..]
            } else {
                remaining
            };
            let target_is_subarray = if leading_index_count > 1 {
                !remaining_after_desig.is_empty()
            } else {
                is_subarray
            };

            match &item.init {
                Initializer::List(sub_items) => {
                    if target_is_subarray {
                        // Sub-array: recurse with next dimension level
                        let start_flat = *flat_idx;
                        let target_elems_per_slot = if leading_index_count > 1 {
                            let target_stride = dim_strides
                                .get(leading_index_count - 1)
                                .copied()
                                .unwrap_or(struct_size);
                            if struct_size > 0 {
                                target_stride / struct_size
                            } else {
                                1
                            }
                        } else {
                            elems_per_slot
                        };
                        let elem_base = self.emit_gep_offset(alloca, base_byte_offset, IrType::I8);
                        self.zero_init_alloca(elem_base, target_elems_per_slot * struct_size);
                        self.lower_struct_array_init_recursive(
                            sub_items,
                            alloca,
                            remaining_after_desig,
                            struct_size,
                            s_layout,
                            flat_idx,
                        );
                        // After recursion, advance flat_idx to the next sub-array boundary
                        // to handle partial initialization (C11 6.7.9p21: uninitialized
                        // elements are zero-initialized, already done by zero_init_alloca).
                        let boundary = start_flat + target_elems_per_slot;
                        if *flat_idx < boundary {
                            *flat_idx = boundary;
                        }
                        item_idx += 1;
                        continue; // flat_idx already advanced
                    } else {
                        // Leaf: this List is a single struct initializer. For
                        // singleton inner dimensions the parser can leave redundant
                        // single-list wrappers (e.g. `struct S a[][1] = {{{1,2}}}`);
                        // unwrap them so scalar fields aren't lost (Regehr yarpgen
                        // *_singleton_inner_dim_* tests).
                        let elem_base = self.emit_gep_offset(alloca, base_byte_offset, IrType::I8);
                        self.zero_init_alloca(elem_base, struct_size);
                        let elem_items =
                            self.normalize_leaf_struct_element_items(sub_items, s_layout);
                        self.lower_local_struct_init(elem_items, elem_base, s_layout);
                    }
                }
                Initializer::Expr(e) => {
                    if self.struct_value_size(e).is_some() {
                        let src_addr = self.get_struct_base_addr(e);
                        self.emit_memcpy_at_offset(alloca, base_byte_offset, src_addr, struct_size);
                    } else {
                        // Flat scalar init without braces (e.g., `struct t a[2] = {1,2,3,4}`)
                        let consumed = self.emit_struct_init(
                            &items[item_idx..],
                            alloca,
                            s_layout,
                            base_byte_offset,
                        );
                        item_idx += consumed.max(1);
                        *flat_idx += 1;
                        continue;
                    }
                }
            }
            item_idx += 1;
            *flat_idx += 1;
        }
    }

    /// Normalize a leaf struct-element initializer from a multi-dimensional array.
    ///
    /// For singleton inner dimensions (e.g., `struct S a[][1] = {{{1,2}}};`), the
    /// parser shape at the leaf can be `List([List([expr, expr, ...])])` where the
    /// outer list is the array-element wrapper and the inner list is the struct
    /// initializer. Passing the outer list straight to `lower_local_struct_init`
    /// makes scalar first fields consume only the first inner expression and the
    /// remaining fields are lost.
    ///
    /// Unwrap redundant singleton list layers only when the struct's first
    /// initializable field is scalar-like (not array/struct/union/vector/complex),
    /// where extra wrappers cannot represent required subobject braces for the
    /// first field. (Adopted from Regehr's yarpgen branch.)
    fn normalize_leaf_struct_element_items<'a>(
        &self,
        sub_items: &'a [InitializerItem],
        s_layout: &crate::common::types::StructLayout,
    ) -> &'a [InitializerItem] {
        let first_field_idx =
            match s_layout.resolve_init_field(None, 0, &*self.types.borrow_struct_layouts()) {
                Some(crate::common::types::InitFieldResolution::Direct(idx)) => idx,
                _ => return sub_items,
            };
        let first_ty = &s_layout.fields[first_field_idx].ty;
        let first_field_needs_nested_braces = matches!(
            first_ty,
            CType::Array(..)
                | CType::Struct(..)
                | CType::Union(..)
                | CType::Vector(..)
                | CType::ComplexFloat
                | CType::ComplexDouble
                | CType::ComplexLongDouble
        );

        if first_field_needs_nested_braces {
            return sub_items;
        }

        // For scalar-leading structs, singleton inner dimensions may introduce
        // multiple redundant list wrappers (e.g., [][1][1] with {{{{...}}}}).
        // Peel all empty-designator single-list layers so all scalar field
        // initializers remain visible to lower_local_struct_init.
        let mut normalized = sub_items;
        loop {
            if normalized.len() != 1 || !normalized[0].designators.is_empty() {
                break;
            }
            match &normalized[0].init {
                Initializer::List(inner) => {
                    normalized = inner.as_slice();
                }
                _ => break,
            }
        }
        normalized
    }

    /// Array of complex elements initialization.
    /// Handles both 1D and multi-dimensional arrays of complex types.
    fn lower_array_of_complex_init(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
        cplx_ctype: &CType,
    ) {
        self.zero_init_alloca(alloca, da.alloc_size);
        let mut flat_idx = 0usize;
        self.lower_complex_init_recursive(
            items,
            alloca,
            da,
            cplx_ctype,
            &da.array_dim_strides.clone(),
            &mut flat_idx,
        );
    }

    /// Recursive helper for multi-dimensional complex array initialization.
    /// Flattens nested brace initializers into flat element indices.
    fn lower_complex_init_recursive(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
        cplx_ctype: &CType,
        dim_strides: &[usize],
        flat_idx: &mut usize,
    ) {
        // The leaf complex element size is always the last stride
        let leaf_size = *da.array_dim_strides.last().unwrap_or(&da.elem_size);
        // How many leaf elements per sub-array at this level
        let sub_elem_count = if dim_strides.len() > 1 && leaf_size > 0 {
            dim_strides[0] / leaf_size
        } else {
            1
        };

        for item in items.iter() {
            let start_index = *flat_idx;

            if let Some(Designator::Index(idx_expr)) = item.designators.first() {
                if let Some(idx_val) = self.eval_const_expr_for_designator(idx_expr) {
                    if dim_strides.len() > 1 {
                        // Designator at outer dimension: set flat index to start of that row
                        *flat_idx = idx_val * sub_elem_count;
                    } else {
                        *flat_idx = idx_val;
                    }
                }
            }

            match &item.init {
                Initializer::List(sub_items) => {
                    if dim_strides.len() > 1 {
                        // Recurse into nested brace list for inner dimensions
                        self.lower_complex_init_recursive(
                            sub_items,
                            alloca,
                            da,
                            cplx_ctype,
                            &dim_strides[1..],
                            flat_idx,
                        );
                        // Advance to next sub-array boundary
                        let boundary = start_index + sub_elem_count;
                        if *flat_idx < boundary {
                            *flat_idx = boundary;
                        }
                    } else {
                        // At leaf dimension, unwrap single-element brace list
                        if let Some(e) = Self::unwrap_nested_init_expr(sub_items) {
                            let src = self.lower_expr_to_complex(e, cplx_ctype);
                            self.emit_memcpy_at_offset(
                                alloca,
                                *flat_idx * leaf_size,
                                src,
                                leaf_size,
                            );
                        }
                        *flat_idx += 1;
                    }
                }
                Initializer::Expr(e) => {
                    let src = self.lower_expr_to_complex(e, cplx_ctype);
                    self.emit_memcpy_at_offset(alloca, *flat_idx * leaf_size, src, leaf_size);
                    *flat_idx += 1;
                }
            }
        }
    }

    /// 1D array initialization with designator and string literal support.
    fn lower_1d_array_init(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
        decl: &Declaration,
    ) {
        let num_elems = da.alloc_size / da.elem_size.max(1);
        let has_designators = items.iter().any(|item| !item.designators.is_empty());
        if has_designators || items.len() < num_elems {
            self.zero_init_alloca(alloca, da.alloc_size);
        }

        let is_complex_elem_array = self.is_type_complex(&decl.type_spec);
        // The innermost element C type of the object, through typedefs and
        // `typeof` (`typedef _Bool B3[3]; B3 b = {...}` has `_Bool` elements
        // although its type specifier is not `_Bool`).
        let elem_ct = (!da.is_array_of_pointers && !da.is_array_of_func_ptrs)
            .then(|| self.init_leaf_ctype(da.c_type.as_ref(), &decl.type_spec));
        let is_bool_elem_array = elem_ct == Some(CType::Bool);

        let elem_store_ty = if da.is_array_of_pointers || da.is_array_of_func_ptrs {
            IrType::Ptr
        } else {
            da.elem_ir_ty
        };

        let mut current_idx = 0usize;
        for item in items.iter() {
            if let Some(Designator::Index(idx_expr)) = item.designators.first() {
                if let Some(idx_val) = self.eval_const_expr_for_designator(idx_expr) {
                    current_idx = idx_val;
                }
            }

            let init_expr = match &item.init {
                Initializer::Expr(e) => Some(e),
                Initializer::List(sub_items) => Self::unwrap_nested_init_expr(sub_items),
            };
            if let Some(e) = init_expr {
                if !da.is_array_of_pointers
                    && (da.elem_ir_ty == IrType::I8 || da.elem_ir_ty == IrType::U8)
                {
                    if let Expr::StringLiteral(s, _) = e {
                        self.emit_string_to_alloca(
                            alloca,
                            s,
                            current_idx * da.elem_size,
                            da.elem_size,
                        );
                        current_idx += 1;
                        continue;
                    }
                }
                // wchar_t array with braced wide string: wchar_t w[] = {L"hello"};
                if !da.is_array_of_pointers
                    && (da.elem_ir_ty == IrType::I32 || da.elem_ir_ty == IrType::U32)
                {
                    if let Expr::WideStringLiteral(s, _) = e {
                        let at = current_idx * da.elem_size;
                        let room = da.alloc_size.saturating_sub(at);
                        self.emit_wide_string_to_alloca(alloca, s, at, room);
                        current_idx += 1;
                        continue;
                    }
                }
                // char16_t array with braced char16 string: char16_t w[] = {u"hello"};
                if !da.is_array_of_pointers
                    && (da.elem_ir_ty == IrType::I16 || da.elem_ir_ty == IrType::U16)
                {
                    if let Expr::Char16StringLiteral(s, _) = e {
                        let at = current_idx * da.elem_size;
                        let room = da.alloc_size.saturating_sub(at);
                        self.emit_char16_string_to_alloca(alloca, s, at, room);
                        current_idx += 1;
                        continue;
                    }
                }
                if is_complex_elem_array {
                    let val = self.lower_expr(e);
                    let src = self.operand_to_value(val);
                    self.emit_memcpy_at_offset(
                        alloca,
                        current_idx * da.elem_size,
                        src,
                        da.elem_size,
                    );
                } else if is_bool_elem_array {
                    // _Bool array elements: normalize (any nonzero -> 1) per C11 6.3.1.2
                    let val = self.lower_expr(e);
                    let expr_ty = self.value_ir_type(e);
                    let val = self.emit_expr_bool_conversion(val, expr_ty, e);
                    self.emit_array_element_store(
                        alloca,
                        val,
                        current_idx * da.elem_size,
                        elem_store_ty,
                    );
                } else {
                    let val = self.lower_expr(e);
                    let expr_ty = self.value_ir_type(e);
                    let val = match &elem_ct {
                        Some(ct) => {
                            let src_ct = self.get_expr_ctype(e);
                            self.convert_assign_value(
                                val,
                                expr_ty,
                                src_ct.as_ref(),
                                ct,
                                elem_store_ty,
                            )
                        }
                        None => self.emit_implicit_cast(val, expr_ty, elem_store_ty),
                    };
                    self.emit_array_element_store(
                        alloca,
                        val,
                        current_idx * da.elem_size,
                        elem_store_ty,
                    );
                }
            }
            current_idx += 1;
        }
    }

    /// Vector init list: `v4hi a = {1, 2, 3, 4};`, lanes positional or
    /// `[i]`-designated (the canonical form of an elided or designated
    /// vector initializer). Lanes not mentioned are zero. The CType supplies
    /// the logical lane count, the padded ext-vector storage and the packed
    /// `_Bool` lane representation (`store_vector_lane`).
    pub(super) fn lower_vector_init_list(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        vector_ct: &CType,
    ) {
        let Some((elem_ct, num_elems)) = vector_ct.vector_info() else {
            return;
        };
        let elem_ir_ty = IrType::from_ctype(elem_ct);
        let total_size = self.ctype_size(vector_ct);
        // Clear the object, sized by the object, exactly when some byte is
        // not defined by a plain lane store: a short or designated list
        // leaves C-mandated zero lanes, packed `_Bool` lane stores are
        // read-modify-write, and ext-vector padding is written by no lane.
        // Uniform constant lists (`{}`, `{0}`, `{c, c, ...}`) on a SIMD
        // shape: one packed zero/broadcast store (`vector_simd.rs`).
        if self.try_lower_vector_init_list_simd(items, alloca, vector_ct) {
            return;
        }
        let designated = items.iter().any(|it| !it.designators.is_empty());
        if designated
            || items.len() < num_elems
            || crate::ir::lowering::lower::vector_temp_needs_zero_init(vector_ct, num_elems)
        {
            self.zero_init_alloca(alloca, total_size);
        }
        let mut idx = 0usize;
        for item in items {
            if let Some(Designator::Index(d)) = item.designators.first() {
                match self.eval_const_expr(d).and_then(|c| c.to_usize()) {
                    Some(i) => idx = i,
                    None => continue,
                }
            }
            let init_expr = match &item.init {
                Initializer::Expr(e) => Some(e),
                Initializer::List(sub_items) => Self::unwrap_nested_init_expr(sub_items),
            };
            if let (true, Some(e)) = (idx < num_elems, init_expr) {
                let val = self.lower_expr(e);
                let expr_ty = self.value_ir_type(e);
                let val = if matches!(elem_ct, CType::Bool) {
                    self.emit_expr_bool_conversion(val, expr_ty, e)
                } else {
                    let src_ct = self.get_expr_ctype(e);
                    self.convert_assign_value(val, expr_ty, src_ct.as_ref(), elem_ct, elem_ir_ty)
                };
                self.store_vector_lane(
                    alloca,
                    vector_ct,
                    idx,
                    val,
                    if matches!(elem_ct, CType::Bool) {
                        IrType::I32
                    } else {
                        elem_ir_ty
                    },
                    AddressSpace::Default,
                    false,
                );
            }
            idx += 1;
        }
    }

    /// Scalar with braces: `int x = { 1 };` or `int x = {{{1}}};`
    fn lower_scalar_braced_init(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        da: &DeclAnalysis,
    ) {
        if let Some(expr) = Self::unwrap_nested_init_expr(items) {
            let val = self.lower_expr(expr);
            let expr_ty = self.value_ir_type(expr);
            // For _Bool, normalize before any truncation to preserve high bits
            let val = if da.is_bool {
                self.emit_expr_bool_conversion(val, expr_ty, expr)
            } else if let Some(target_ct) = &da.c_type {
                let src_ct = self.get_expr_ctype(expr);
                self.convert_assign_value(val, expr_ty, src_ct.as_ref(), target_ct, da.var_ty)
            } else {
                self.emit_implicit_cast(val, expr_ty, da.var_ty)
            };
            self.emit(Instruction::Store {
                volatile: false,
                val,
                ptr: alloca,
                ty: da.var_ty,
                seg_override: AddressSpace::Default,
            });
        }
    }
}

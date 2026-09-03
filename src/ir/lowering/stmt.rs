use super::definitions::{DeclAnalysis, FuncSig, GlobalInfo, LocalInfo};
use super::lower::Lowerer;
use super::structs::VlaDynCursor;
use crate::common::types::{target_int_ir_type, AddressSpace, CType, IrType, StructLayout};
use crate::frontend::parser::ast::{
    BlockItem, CompoundStmt, Declaration, DerivedDeclarator, Designator, Expr, InitDeclarator,
    Initializer, InitializerItem, Stmt, StructFieldDecl, TypeSpecifier,
};
use crate::frontend::sema::type_context::extract_fptr_typedef_info;
use crate::ir::reexports::{
    GlobalInit, Instruction, IrBinOp, IrConst, IrGlobal, Operand, Terminator, Value,
};

impl Lowerer {
    pub(super) fn lower_compound_stmt(&mut self, compound: &CompoundStmt) {
        // If this block has __label__ declarations, push a local label scope
        // that maps each declared name to a unique scope-qualified name.
        let has_local_labels = !compound.local_labels.is_empty();
        if has_local_labels {
            let scope_id = self.next_local_label_scope;
            self.next_local_label_scope += 1;
            let mut scope = crate::common::fx_hash::FxHashMap::default();
            for name in &compound.local_labels {
                scope.insert(name.clone(), format!("{}$ll{}", name, scope_id));
            }
            self.local_label_scopes.push(scope);
        }

        // Check if this block contains any declarations. If not, we can skip
        // scope tracking entirely since statements don't introduce new bindings.
        let has_declarations = compound.items.iter().any(|item| {
            matches!(item, BlockItem::Declaration(_))
                || matches!(item, BlockItem::NestedFunction(_))
        });

        if has_declarations {
            // Push a scope frame to track additions/modifications in this block.
            // On scope exit, we undo changes instead of cloning entire HashMaps.
            self.push_scope();

            for item in &compound.items {
                match item {
                    BlockItem::Declaration(decl) => {
                        self.func_mut().current_span = decl.span;
                        self.collect_enum_constants_scoped(&decl.type_spec);
                        self.lower_local_decl(decl);
                    }
                    BlockItem::Statement(stmt) => self.lower_stmt(stmt),
                    BlockItem::NestedFunction(nested) => {
                        self.lower_nested_function_def(nested);
                    }
                }
            }

            self.pop_scope();
        } else {
            // No declarations: just lower statements without scope overhead.
            for item in &compound.items {
                if let BlockItem::Statement(stmt) = item {
                    self.lower_stmt(stmt);
                }
            }
        }

        // Pop the local label scope if we pushed one
        if has_local_labels {
            self.local_label_scopes.pop();
        }
    }

    pub(super) fn lower_local_decl(&mut self, decl: &Declaration) {
        let mut resolved_type_spec = None;
        let type_spec = self.resolve_local_type_spec(decl, &mut resolved_type_spec);

        self.register_struct_type(type_spec);

        if decl.is_typedef() {
            self.lower_local_typedef(decl, type_spec);
            return;
        }

        for declarator in &decl.declarators {
            if declarator.name.is_empty() {
                continue;
            }
            if decl.is_extern() && self.lower_extern_decl(decl, declarator) {
                continue;
            }
            if self.try_lower_block_func_decl(decl, declarator) {
                continue;
            }

            let mut da = self.analyze_declaration(type_spec, &declarator.derived);
            da.base_type_volatile = decl.is_volatile();
            let elem_size = da.c_type.as_ref().map_or(0, |ct| ct.size());
            if let Some(vs) = decl.resolve_vector_size(elem_size) {
                da.apply_vector_size(vs);
            }
            self.fixup_unsized_array(&mut da, type_spec, &declarator.derived, &declarator.init);

            let is_complex = !da.is_pointer && !da.is_array && self.is_type_complex(type_spec);
            let complex_elem_ctype = self.detect_complex_array_elem(type_spec, &da);

            if decl.is_static() {
                if da.is_array_of_pointers || da.is_array_of_func_ptrs {
                    da.struct_layout = None;
                    da.is_struct = false;
                }
                self.lower_local_static_decl(decl, declarator, &da, type_spec);
                continue;
            }

            let vla_size = if da.is_array {
                self.compute_vla_runtime_size(type_spec, &declarator.derived)
            } else if da.is_struct {
                // GCC extension: block-scope structs may contain VLA members
                // (`struct S { char w[y]; } a;`).  Such objects need dynamic
                // stack allocation and a runtime size for later by-value
                // assignment/copy.  Previously only array declarators used
                // DynAlloca, so VLA-struct locals got zero-byte allocas and
                // their struct assignments lowered to zero-byte memcpy
                // (gcc.c-torture/execute/20070919-1.c).
                self.compute_vla_size_from_type_spec(type_spec)
            } else {
                None
            };

            let explicit_align = self.compute_explicit_alignment(decl, type_spec);

            // Incorporate the type's natural alignment (e.g., a struct with
            // __attribute__((aligned(16))) fields inherits 16-byte alignment).
            // Without this, `struct S x;` where S requires 16-byte alignment
            // would get align=0 (default platform alignment) instead of 16,
            // causing misaligned stack slots on i686.
            let type_align = self.alignof_type(type_spec);
            let alloca_align = if type_align > explicit_align {
                type_align
            } else {
                explicit_align
            };

            let alloca = if let Some(vla_size_val) = vla_size {
                self.emit_vla_alloca(vla_size_val, alloca_align)
            } else {
                self.emit_entry_alloca(
                    if da.is_array || da.is_struct || is_complex {
                        IrType::Ptr
                    } else {
                        da.var_ty
                    },
                    da.actual_alloc_size,
                    alloca_align,
                    // The OBJECT is volatile only when the base type's
                    // qualifier applies to this declaration level: for
                    // `volatile T *p` the volatile qualifies the POINTEE,
                    // not the pointer object itself.
                    da.base_type_volatile && !da.is_pointer,
                )
            };

            self.register_local_var(
                decl,
                declarator,
                type_spec,
                &da,
                alloca,
                alloca_align,
                vla_size,
            );
            self.track_fptr_sig(declarator, type_spec);

            if let Some(ref init) = declarator.init {
                self.lower_local_var_init(
                    init,
                    decl,
                    declarator,
                    &da,
                    alloca,
                    is_complex,
                    &complex_elem_ctype,
                );
            }
        }
    }

    /// Resolve typeof/auto_type to a concrete TypeSpecifier, or return the original.
    fn resolve_local_type_spec<'a>(
        &mut self,
        decl: &'a Declaration,
        resolved: &'a mut Option<TypeSpecifier>,
    ) -> &'a TypeSpecifier {
        if !matches!(
            &decl.type_spec,
            TypeSpecifier::Typeof(_) | TypeSpecifier::TypeofType(_) | TypeSpecifier::AutoType
        ) {
            return &decl.type_spec;
        }
        let ts = if matches!(&decl.type_spec, TypeSpecifier::AutoType) {
            if let Some(first) = decl.declarators.first() {
                if let Some(Initializer::Expr(ref init_expr)) = first.init {
                    if let Some(ctype) = self.get_expr_ctype(init_expr) {
                        Self::ctype_to_type_spec(&ctype)
                    } else {
                        self.emit_warning(
                            "could not infer type for '__auto_type'; defaulting to 'int'",
                            init_expr.span(),
                        );
                        TypeSpecifier::Int
                    }
                } else {
                    self.emit_warning(
                        "'__auto_type' requires an initializer; defaulting to 'int'",
                        decl.span,
                    );
                    TypeSpecifier::Int
                }
            } else {
                TypeSpecifier::Int
            }
        } else {
            self.resolve_typeof(&decl.type_spec)
        };
        *resolved = Some(ts);
        resolved.as_ref().expect("just assigned Some above")
    }

    fn lower_local_typedef(&mut self, decl: &Declaration, type_spec: &TypeSpecifier) {
        for declarator in &decl.declarators {
            if declarator.name.is_empty() {
                continue;
            }
            if let Some(fti) = extract_fptr_typedef_info(type_spec, &declarator.derived) {
                self.types.func_ptr_typedefs.insert(declarator.name.clone());
                self.types
                    .func_ptr_typedef_info
                    .insert(declarator.name.clone(), fti);
            }
            let mut resolved_ctype = self.build_full_ctype(type_spec, &declarator.derived);
            crate::common::type_builder::apply_declaration_address_space(
                &mut resolved_ctype,
                decl.address_space,
            );
            if let Some(vs) = decl.resolve_vector_size(resolved_ctype.size()) {
                resolved_ctype = CType::Vector(Box::new(resolved_ctype), vs);
            }
            // Preserve sema's anonymous struct key for consistency with _Generic
            // type matching (see corresponding fix in lower_typedef for globals).
            if let Some(existing) = self.types.typedefs.get(&declarator.name) {
                let is_new_anon = matches!(&resolved_ctype, CType::Struct(k) | CType::Union(k) if k.starts_with("__anon_"));
                let is_existing_anon = matches!(existing, CType::Struct(k) | CType::Union(k) if k.starts_with("__anon_"));
                if is_new_anon && is_existing_anon && resolved_ctype != *existing {
                    let new_key = match &resolved_ctype {
                        CType::Struct(k) | CType::Union(k) => k.to_string(),
                        _ => unreachable!(),
                    };
                    let old_key = match existing {
                        CType::Struct(k) | CType::Union(k) => k.to_string(),
                        _ => unreachable!(),
                    };
                    let layout_copy = self
                        .types
                        .borrow_struct_layouts()
                        .get(&new_key)
                        .map(|l| l.as_ref().clone());
                    if let Some(layout) = layout_copy {
                        self.types
                            .insert_struct_layout_scoped_from_ref(&old_key, layout);
                    }
                    resolved_ctype = existing.clone();
                }
            }
            self.types
                .insert_typedef_scoped(declarator.name.clone(), resolved_ctype);

            let effective_alignment = {
                let mut align = decl.alignment;
                if let Some(ref sizeof_ts) = decl.alignment_sizeof_type {
                    let real_sizeof = self.sizeof_type(sizeof_ts);
                    align = Some(align.map_or(real_sizeof, |a| a.max(real_sizeof)));
                }
                align.or_else(|| decl.alignas_type.as_ref().map(|ts| self.alignof_type(ts)))
            };
            if let Some(align) = effective_alignment {
                self.types
                    .insert_typedef_alignment_scoped(declarator.name.clone(), align);
            }
            if self.func_state.is_some() {
                let copied_from_struct = {
                    let td = self.types.typedefs.get(&declarator.name);
                    let fs = self.func_state.as_ref();
                    match (td, fs) {
                        (Some(CType::Struct(key) | CType::Union(key)), Some(fs)) => {
                            fs.vla_typedef_sizes.get(key.as_ref()).copied()
                        }
                        _ => None,
                    }
                };
                if let Some(vla_size) = copied_from_struct {
                    self.func_mut()
                        .insert_vla_typedef_size_scoped(declarator.name.clone(), vla_size);
                } else if let Some(vla_size) =
                    self.compute_vla_runtime_size(type_spec, &declarator.derived)
                {
                    self.func_mut()
                        .insert_vla_typedef_size_scoped(declarator.name.clone(), vla_size);
                }
            }
        }
    }

    fn detect_complex_array_elem(
        &self,
        type_spec: &TypeSpecifier,
        da: &DeclAnalysis,
    ) -> Option<CType> {
        if da.is_array && !da.is_pointer {
            let ctype = self.type_spec_to_ctype(type_spec);
            match ctype {
                CType::ComplexFloat => Some(CType::ComplexFloat),
                CType::ComplexDouble => Some(CType::ComplexDouble),
                CType::ComplexLongDouble => Some(CType::ComplexLongDouble),
                _ => None,
            }
        } else {
            None
        }
    }

    fn compute_explicit_alignment(&self, decl: &Declaration, type_spec: &TypeSpecifier) -> usize {
        let mut ea = if let Some(ref alignas_ts) = decl.alignas_type {
            self.alignof_type(alignas_ts)
        } else {
            decl.alignment.unwrap_or(0)
        };
        if let Some(ref sizeof_ts) = decl.alignment_sizeof_type {
            ea = ea.max(self.sizeof_type(sizeof_ts));
        }
        if let Some(&td_align) = self.typedef_alignment_for_type_spec(type_spec) {
            ea = ea.max(td_align);
        }
        ea
    }

    /// Emit VLA dynamic alloca with stack save at function and scope level.
    pub(super) fn emit_vla_alloca(&mut self, vla_size_val: Value, explicit_align: usize) -> Value {
        let alloca = self.fresh_value();
        if !self.func().has_vla {
            self.func_mut().has_vla = true;
            let save_val = self.fresh_value();
            self.emit(Instruction::StackSave { dest: save_val });
            self.func_mut().vla_stack_save = Some(save_val);
        }
        if let Some(frame) = self.func().scope_stack.last() {
            if frame.scope_stack_save.is_none() {
                let scope_save = self.fresh_value();
                self.emit(Instruction::StackSave { dest: scope_save });
                self.func_mut()
                    .scope_stack
                    .last_mut()
                    .unwrap()
                    .scope_stack_save = Some(scope_save);
            }
        }
        let vla_align = explicit_align.max(16);
        self.emit(Instruction::DynAlloca {
            dest: alloca,
            size: Operand::Value(vla_size_val),
            align: vla_align,
        });
        alloca
    }

    fn register_local_var(
        &mut self,
        decl: &Declaration,
        declarator: &InitDeclarator,
        type_spec: &TypeSpecifier,
        da: &DeclAnalysis,
        alloca: Value,
        explicit_align: usize,
        vla_size: Option<Value>,
    ) {
        let mut local_info = LocalInfo::from_analysis(da, alloca, decl.is_const());
        local_info.var.address_space = decl.address_space;
        // `T __seg_fs *p`: the qualifier lives on the pointer's CType (the
        // space the pointer points into), not just on the variable slot.
        // Without this, `*p` resolves AddressSpace::Default through
        // get_expr_ctype and the segment prefix is silently dropped.
        if let Some(ref mut ct) = local_info.var.c_type {
            crate::common::type_builder::apply_declaration_address_space(ct, decl.address_space);
        }
        if explicit_align > 0 {
            local_info.var.explicit_alignment = Some(explicit_align);
        }
        local_info.vla_size = vla_size;
        if vla_size.is_some() {
            let strides = self.compute_vla_local_strides(type_spec, &declarator.derived);
            if !strides.is_empty() {
                local_info.vla_strides = strides;
            }
        }
        local_info.asm_register = declarator.attrs.asm_register.clone();
        local_info.asm_register_has_init =
            declarator.attrs.asm_register.is_some() && declarator.init.is_some();
        local_info.cleanup_fn = declarator.attrs.cleanup_fn.clone();
        if let Some(ref cleanup_fn_name) = declarator.attrs.cleanup_fn {
            if let Some(frame) = self.func_mut().scope_stack.last_mut() {
                frame.cleanup_vars.push((cleanup_fn_name.clone(), alloca));
            }
        }
        self.insert_local_scoped(declarator.name.clone(), local_info);
    }

    fn track_fptr_sig(&mut self, declarator: &InitDeclarator, type_spec: &TypeSpecifier) {
        for (i, d) in declarator.derived.iter().enumerate() {
            if let DerivedDeclarator::FunctionPointer(params, _) = d {
                // The derived layout is: [return_type_pointers..., syntax_marker_Pointer,
                // FunctionPointer, ...]. Pointers before the FunctionPointer include the
                // syntax marker (1) plus any return-type pointer indirections.
                // If there are return-type pointers, the return type is a pointer.
                let ptr_count_before = declarator.derived[..i]
                    .iter()
                    .filter(|d| {
                        matches!(d, DerivedDeclarator::Pointer | DerivedDeclarator::Array(_))
                    })
                    .count();
                // Subtract 1 for the syntax marker pointer
                let return_type_ptrs = ptr_count_before.saturating_sub(1);
                let ret_ty = if return_type_ptrs > 0 {
                    IrType::Ptr
                } else {
                    self.type_spec_to_ir(type_spec)
                };
                let param_tys: Vec<IrType> = params
                    .iter()
                    .map(|p| self.type_spec_to_ir(&p.type_spec))
                    .collect();
                self.func_meta
                    .ptr_sigs
                    .insert(declarator.name.clone(), FuncSig::for_ptr(ret_ty, param_tys));
                break;
            }
        }
    }

    fn lower_local_var_init(
        &mut self,
        init: &Initializer,
        decl: &Declaration,
        declarator: &InitDeclarator,
        da: &DeclAnalysis,
        alloca: Value,
        is_complex: bool,
        complex_elem_ctype: &Option<CType>,
    ) {
        match init {
            Initializer::Expr(expr) => {
                if decl.is_const() && !da.is_pointer && !da.is_array && !da.is_struct && !is_complex
                {
                    if let Some(const_val) = self.eval_const_expr(expr) {
                        if let Some(ival) = self.const_to_i64(&const_val) {
                            // Coerce the constant to the declared type before caching it
                            // in const_local_values. Otherwise `const char c = 220` caches
                            // +220 instead of -36 (char is signed on x86), so `c < 0`
                            // wrongly folds false and a following `|| (g = 9)` side effect
                            // runs (Regehr yarpgen regress_const_local_char_init_coercion).
                            // Match the sign/width of the declared IR type.
                            let coerced = match da.var_ty {
                                IrType::I8 => ival as i8 as i64,
                                IrType::U8 => ival as u8 as i64,
                                IrType::I16 => ival as i16 as i64,
                                IrType::U16 => ival as u16 as i64,
                                IrType::I32 => ival as i32 as i64,
                                IrType::U32 => ival as u32 as i64,
                                _ => ival, // I64/U64 already full-width
                            };
                            // A 128-bit initializer cannot, in general, live in the
                            // i64 const-local cache: the cache is read back as
                            // `IrConst::I64(cached)` and widened by the consumer's own
                            // context, so anything but an exact round trip folds every
                            // read of the variable to ONE HALF of the value
                            // (`const unsigned __int128 c = (u128)3<<100 | 3` cached
                            // low=3; a later `c >> 64` folded to
                            // 3i64.wrapping_shr(64 & 63 = 0) = 3 — the high half read
                            // the LOW value and the shift masked away). Cache only
                            // values that survive both widenings bit-exactly:
                            // non-negative and equal to the full 128-bit constant.
                            // Everything else stays uncached and reads take the
                            // (correct) alloca path — a missed fold, never a wrong one.
                            let cacheable = !matches!(da.var_ty, IrType::I128 | IrType::U128)
                                || (ival >= 0 && const_val.to_i128() == Some(ival as i128));
                            if cacheable {
                                self.insert_const_local_scoped(declarator.name.clone(), coerced);
                            }
                        }
                    }
                }
                self.lower_local_init_expr(expr, alloca, da, is_complex, decl);
            }
            Initializer::List(items) => {
                self.lower_local_init_list(
                    items,
                    alloca,
                    da,
                    is_complex,
                    complex_elem_ctype,
                    decl,
                    &declarator.name,
                );
            }
        }
    }

    /// Handle static local variable declarations: emit as globals with mangled names.
    /// Static locals are initialized once at program start (via .data/.bss),
    /// not at every function call.
    fn lower_local_static_decl(
        &mut self,
        decl: &Declaration,
        declarator: &InitDeclarator,
        da: &DeclAnalysis,
        type_spec: &TypeSpecifier,
    ) {
        let static_id = self.next_static_local;
        let static_name = format!("{}.{}.{}", self.func_mut().name, declarator.name, static_id);

        // Register the bare name -> mangled name mapping before processing the initializer
        // so that &x in another static's initializer can resolve to the mangled name.
        self.insert_static_local_scoped(declarator.name.clone(), static_name.clone());

        // Register the global before evaluating its initializer so that
        // self-referential initializers (e.g., static struct work w = { .entry = { &w.entry, &w.entry } })
        // can resolve &w.entry via resolve_chained_member_access -> self.globals.get().
        // This mirrors the same pattern used in lower_global_decl().
        let ginfo = GlobalInfo::from_analysis(da);
        self.globals.insert(static_name.clone(), ginfo);

        // Determine initializer (evaluated at compile time for static locals)
        // For pointer arrays and scalar pointers, use Ptr as the base type for
        // initializer coercion (matching file-scope global handling). Without this,
        // base_ty is the pointee type (e.g., I8 for char*), causing NULL pointer
        // entries in pointer arrays to be emitted as 1-byte .byte 0 instead of
        // 8-byte .quad 0, corrupting the array layout.
        let init = if let Some(ref initializer) = declarator.init {
            let init_base_ty = if da.is_pointer && !da.is_array {
                da.var_ty // scalar pointer: var_ty = Ptr
            } else if da.is_array_of_pointers || da.is_array_of_func_ptrs {
                IrType::Ptr // pointer array: each element is Ptr
            } else {
                da.base_ty
            };
            self.lower_global_init(
                initializer,
                type_spec,
                init_base_ty,
                da.is_array,
                da.elem_size,
                da.actual_alloc_size,
                &da.struct_layout,
                &da.array_dim_strides,
            )
        } else {
            GlobalInit::Zero
        };

        // Compute natural alignment from the type specifier. This picks up
        // struct/union __attribute__((aligned(N))) that is part of the type
        // definition rather than the variable declaration. Without this,
        // `static struct fxregs_state fxregs` would get da.var_ty.align() == 1
        // (IrType::I8) instead of the struct's required 16-byte alignment.
        let c_align = self.alignof_type(type_spec);
        let natural = if c_align > 0 {
            c_align.max(da.var_ty.align())
        } else {
            da.var_ty.align()
        };

        // Respect explicit __attribute__((aligned(N))) / _Alignas(N) on static locals.
        // Resolve _Alignas(type) via the lowerer for accurate typedef resolution.
        let mut explicit_align = if let Some(ref alignas_ts) = decl.alignas_type {
            Some(self.alignof_type(alignas_ts))
        } else {
            decl.alignment
        };
        // Recompute sizeof for aligned(sizeof(type)) with full layout info
        if let Some(ref sizeof_ts) = decl.alignment_sizeof_type {
            let real_sizeof = self.sizeof_type(sizeof_ts);
            explicit_align = Some(explicit_align.map_or(real_sizeof, |a| a.max(real_sizeof)));
        }
        // Also incorporate alignment from typedef
        if let Some(&td_align) = self.typedef_alignment_for_type_spec(type_spec) {
            explicit_align = Some(explicit_align.map_or(td_align, |a: usize| a.max(td_align)));
        }
        let has_explicit_align = explicit_align.is_some();
        let align = if let Some(explicit) = explicit_align {
            natural.max(explicit)
        } else {
            natural
        };

        // For struct initializers emitted as byte arrays, set element type to I8
        let global_ty = da.resolve_global_ty(&init);

        // Function-scope statics get the same const classification as
        // file-scope globals -- see the long comment in
        // `global_decl.rs::lower_global_decl`.  `decl.is_const()` describes the
        // POINTEE for pointer declarators, while `T *const p` (the pointer
        // itself read-only) is carried separately by `is_pointer_const()`.
        //
        // This is the path that matters for the case that motivated the fix,
        // because the declaration is inside a function:
        //
        //     static const char* const notErrorCode = "Unspecified error code";
        //         -- lib/zstd/common/error_private.c, ERR_getErrorString()
        //
        // Misclassifying it as non-const put it in `.data` with an absolute
        // R_X86_64_64 relocation, and the kernel's boot-decompressor link
        // rejected the result with
        //   "ld: Unexpected run-time relocations (.rela) detected!".
        let var_is_const = (decl.is_const()
            && !da.is_pointer
            && !da.is_array_of_pointers
            && !da.is_array_of_func_ptrs)
            || (decl.is_pointer_const() && da.is_pointer);

        self.emitted_global_names.insert(static_name.clone());
        self.module.globals.push(IrGlobal {
            name: static_name.clone(),
            ty: global_ty,
            size: da.actual_alloc_size,
            align,
            init,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: declarator.attrs.section.clone(),
            is_weak: false,
            visibility: None,
            has_explicit_align,
            is_const: var_is_const,
            is_used: declarator.attrs.is_used(),
            is_thread_local: decl.is_thread_local(),
        });

        // Update the pre-registered global info with explicit alignment if present
        if let Some(ea) = explicit_align {
            if let Some(ginfo) = self.globals.get_mut(&static_name) {
                ginfo.var.explicit_alignment = Some(ea);
            }
        }

        // Store type info in locals (with static_global_name set so each use site
        // emits a fresh GlobalAddr in its own basic block, avoiding unreachable-block issues).
        let mut local_info = LocalInfo::for_static(da, static_name, decl.is_const());
        if let Some(ea) = explicit_align {
            local_info.var.explicit_alignment = Some(ea);
        }
        self.insert_local_scoped(declarator.name.clone(), local_info);
        self.next_static_local += 1;

        // Track function pointer return and param types for static locals too,
        // so that calls through static function pointers apply correct argument
        // promotions (e.g. float->double for unprototyped `float (*sfp)()`).
        for (i, d) in declarator.derived.iter().enumerate() {
            if let DerivedDeclarator::FunctionPointer(params, _) = d {
                let ptr_count_before = declarator.derived[..i]
                    .iter()
                    .filter(|d| {
                        matches!(d, DerivedDeclarator::Pointer | DerivedDeclarator::Array(_))
                    })
                    .count();
                let return_type_ptrs = ptr_count_before.saturating_sub(1);
                let ret_ty = if return_type_ptrs > 0 {
                    IrType::Ptr
                } else {
                    self.type_spec_to_ir(type_spec)
                };
                let param_tys: Vec<IrType> = params
                    .iter()
                    .map(|p| self.type_spec_to_ir(&p.type_spec))
                    .collect();
                self.func_meta
                    .ptr_sigs
                    .insert(declarator.name.clone(), FuncSig::for_ptr(ret_ty, param_tys));
                break;
            }
        }
    }

    /// Lower a {real, imag} list initializer for a complex field.
    /// Stores the real and imaginary parts at dest_addr and dest_addr+comp_size.
    ///
    /// Also handles "braces around scalar" (C11 6.7.9): when _Complex is initialized
    /// with extra braces like `{{expr}}`, the inner braces are unwrapped.
    /// If the single expression is itself complex-typed, it is stored as a whole
    /// complex value rather than being treated as just the real component.
    pub(super) fn lower_complex_list_init(
        &mut self,
        sub_items: &[InitializerItem],
        dest_addr: Value,
        complex_ctype: &CType,
    ) {
        // Handle "braces around scalar" for _Complex types (C11 6.7.9):
        // If the list has a single item that is itself a List, unwrap the extra braces.
        if sub_items.len() == 1 && sub_items[0].designators.is_empty() {
            if let Initializer::List(inner_items) = &sub_items[0].init {
                return self.lower_complex_list_init(inner_items, dest_addr, complex_ctype);
            }
        }

        // If the list has a single expression that is complex-typed, store it as a
        // whole complex value (not as just the real component).
        if sub_items.len() == 1 && sub_items[0].designators.is_empty() {
            if let Initializer::Expr(e) = &sub_items[0].init {
                let expr_ctype = self.expr_ctype(e);
                if expr_ctype.is_complex() {
                    let complex_size = complex_ctype.size();
                    let src = self.lower_expr_to_complex(e, complex_ctype);
                    self.emit(Instruction::Memcpy {
                        dest: dest_addr,
                        src,
                        size: complex_size,
                    });
                    return;
                }
            }
        }

        let comp_ty = Self::complex_component_ir_type(complex_ctype);
        let comp_size = Self::complex_component_size(complex_ctype);
        // Store real part
        if let Some(first) = sub_items.first() {
            if let Initializer::Expr(e) = &first.init {
                let val = self.lower_expr(e);
                let expr_ty = self.get_expr_type(e);
                let val = self.emit_implicit_cast(val, expr_ty, comp_ty);
                self.emit(Instruction::Store {
                    volatile: false,
                    val,
                    ptr: dest_addr,
                    ty: comp_ty,
                    seg_override: AddressSpace::Default,
                });
            }
        } else {
            let zero = Self::complex_zero(comp_ty);
            self.emit(Instruction::Store {
                volatile: false,
                val: zero,
                ptr: dest_addr,
                ty: comp_ty,
                seg_override: AddressSpace::Default,
            });
        }
        // Store imag part
        let imag_ptr = self.emit_gep_offset(dest_addr, comp_size, IrType::I8);
        if let Some(item) = sub_items.get(1) {
            if let Initializer::Expr(e) = &item.init {
                let val = self.lower_expr(e);
                let expr_ty = self.get_expr_type(e);
                let val = self.emit_implicit_cast(val, expr_ty, comp_ty);
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

    /// Emit a complex expression to a memory location at the given offset.
    /// Handles integer-to-complex conversion properly by using lower_expr_to_complex
    /// and then memcpy-ing the result to the destination.
    pub(super) fn emit_complex_expr_to_offset(
        &mut self,
        expr: &Expr,
        base_alloca: Value,
        offset: usize,
        complex_ctype: &CType,
    ) {
        let complex_size = complex_ctype.size();
        let src = self.lower_expr_to_complex(expr, complex_ctype);
        let dest_addr = self.emit_gep_offset(base_alloca, offset, IrType::Ptr);
        self.emit(Instruction::Memcpy {
            dest: dest_addr,
            src,
            size: complex_size,
        });
    }

    /// Lower an expression to a complex value, converting if needed.
    /// Returns a Value (pointer to the complex {real, imag} pair).
    pub(super) fn lower_expr_to_complex(&mut self, expr: &Expr, target_ctype: &CType) -> Value {
        let expr_ctype = self.expr_ctype(expr);
        let val = self.lower_expr(expr);
        if expr_ctype.is_complex() {
            if expr_ctype != *target_ctype {
                let val_v = self.operand_to_value(val);
                let converted = self.complex_to_complex(val_v, &expr_ctype, target_ctype);
                self.operand_to_value(converted)
            } else {
                self.operand_to_value(val)
            }
        } else {
            let converted = self.real_to_complex(val, &expr_ctype, target_ctype);
            self.operand_to_value(converted)
        }
    }

    /// Initialize a local struct from an initializer list.
    /// `base` is the base address of the struct in memory.
    pub(super) fn lower_local_struct_init(
        &mut self,
        items: &[InitializerItem],
        base: Value,
        layout: &StructLayout,
    ) {
        use super::global_init_helpers as h;
        use crate::common::types::InitFieldResolution;
        let mut current_field_idx = 0usize;
        let mut item_idx = 0usize;
        while item_idx < items.len() {
            let item = &items[item_idx];
            let desig_name = h::first_field_designator(item);
            let resolution = match layout.resolve_init_field(
                desig_name,
                current_field_idx,
                &*self.types.borrow_struct_layouts(),
            ) {
                Some(r) => r,
                None => break,
            };

            // Handle anonymous member: drill into the anonymous struct/union
            let field_idx = match &resolution {
                InitFieldResolution::Direct(idx) => {
                    let f = &layout.fields[*idx];
                    // For positional init, if this is an anonymous struct/union member,
                    // drill into it and consume multiple init items for inner fields.
                    if desig_name.is_none() && f.name.is_empty() && f.bit_width.is_none() {
                        if let CType::Struct(key) | CType::Union(key) = &f.ty {
                            let sub_layout =
                                self.types.borrow_struct_layouts().get(&**key).cloned();
                            if let Some(sub_layout) = sub_layout {
                                let anon_offset = f.offset;
                                let sub_base = self.emit_gep_offset(base, anon_offset, IrType::Ptr);
                                // If the current item is a braced sub-initializer (List),
                                // it is the initializer for this anonymous member as a whole.
                                // Unwrap and pass the inner items to the sub-layout.
                                // This handles `(T){ { .field = val } }` where the inner
                                // braces wrap the anonymous union/struct member.
                                if let Initializer::List(sub_items) = &item.init {
                                    self.lower_local_struct_init(sub_items, sub_base, &sub_layout);
                                    item_idx += 1;
                                    current_field_idx = *idx + 1;
                                    continue;
                                }
                                let anon_field_count = sub_layout
                                    .fields
                                    .iter()
                                    .filter(|ff| !ff.name.is_empty() || ff.bit_width.is_none())
                                    .count();
                                let remaining = &items[item_idx..];
                                let consume_count = remaining.len().min(anon_field_count);
                                self.lower_local_struct_init(
                                    &remaining[..consume_count],
                                    sub_base,
                                    &sub_layout,
                                );
                                item_idx += consume_count;
                                current_field_idx = *idx + 1;
                                continue;
                            }
                        }
                    }
                    *idx
                }
                InitFieldResolution::AnonymousMember {
                    anon_field_idx,
                    inner_name,
                } => {
                    let extra_desigs = if item.designators.len() > 1 {
                        &item.designators[1..]
                    } else {
                        &[]
                    };
                    let anon_res = h::resolve_anonymous_member(
                        layout,
                        *anon_field_idx,
                        inner_name,
                        &item.init,
                        extra_desigs,
                        &self.types.borrow_struct_layouts(),
                    );
                    if let Some(res) = anon_res {
                        let sub_base = self.emit_gep_offset(base, res.anon_offset, IrType::Ptr);
                        self.lower_local_struct_init(&[res.sub_item], sub_base, &res.sub_layout);
                    }
                    current_field_idx = *anon_field_idx + 1;
                    item_idx += 1;
                    continue;
                }
            };

            let field = &layout.fields[field_idx];
            let field_offset = field.offset;

            // Complex fields need special handling: memcpy data instead of storing pointer
            if field.ty.is_complex() {
                let complex_ctype = field.ty.clone();
                let complex_size = complex_ctype.size();
                let dest_addr = self.emit_gep_offset(base, field_offset, IrType::Ptr);
                match &item.init {
                    Initializer::Expr(e) => {
                        let src = self.lower_expr_to_complex(e, &complex_ctype);
                        self.emit(Instruction::Memcpy {
                            dest: dest_addr,
                            src,
                            size: complex_size,
                        });
                    }
                    Initializer::List(sub_items) => {
                        self.lower_complex_list_init(sub_items, dest_addr, &complex_ctype);
                    }
                }
                current_field_idx = field_idx + 1;
                item_idx += 1;
                continue;
            }

            let field_ty = IrType::from_ctype(&field.ty);

            match &item.init {
                Initializer::Expr(e) => {
                    // For struct/union fields initialized from expressions that produce
                    // struct values (e.g., function calls returning structs via sret),
                    // we need to emit a memcpy rather than a scalar store, because
                    // lower_expr returns the sret alloca address (a pointer), not the
                    // struct data itself.
                    let is_struct_field = matches!(field.ty, CType::Struct(_) | CType::Union(_));
                    if is_struct_field && self.struct_value_size(e).is_some() {
                        let src_addr = self.get_struct_base_addr(e);
                        let field_size = self.resolve_ctype_size(&field.ty);
                        let field_addr = self.emit_gep_offset(base, field_offset, IrType::Ptr);
                        self.emit(Instruction::Memcpy {
                            dest: field_addr,
                            src: src_addr,
                            size: field_size,
                        });
                    } else if let CType::Array(ref elem_ty, Some(arr_size)) = field.ty {
                        // Char array field initialized by a string literal:
                        // copy the string bytes instead of storing the pointer.
                        let is_char_array = matches!(**elem_ty, CType::Char | CType::UChar);
                        if is_char_array {
                            if let Expr::StringLiteral(ref s, _) = e {
                                self.emit_string_to_alloca(base, s, field_offset, arr_size);
                                // Zero-fill remaining bytes if string is shorter than array
                                let str_len = s.chars().count() + 1; // +1 for null terminator
                                for i in str_len..arr_size {
                                    let val = Operand::Const(IrConst::I8(0));
                                    self.emit_store_at_offset(
                                        base,
                                        field_offset + i,
                                        val,
                                        IrType::I8,
                                    );
                                }
                            } else {
                                let val = self.lower_and_cast_init_expr(e, field_ty);
                                let field_addr = self.emit_gep_offset(base, field_offset, field_ty);
                                self.emit(Instruction::Store {
                                    volatile: false,
                                    val,
                                    ptr: field_addr,
                                    ty: field_ty,
                                    seg_override: AddressSpace::Default,
                                });
                            }
                        } else if let CType::Struct(ref key) | CType::Union(ref key) = **elem_ty {
                            // Array-of-structs field with flat expression initializer:
                            // delegate to emit_struct_init which knows how to consume
                            // the right number of flat items for each struct element.
                            let elem_size = self.resolve_ctype_size(elem_ty);
                            let sub_layout =
                                self.types.borrow_struct_layouts().get(&**key).cloned();
                            if let Some(sub_layout) = sub_layout {
                                let mut ai = 0usize;
                                let mut consumed_total = 0usize;
                                while ai < arr_size {
                                    let elem_offset = field_offset + ai * elem_size;
                                    let remaining = &items[item_idx + consumed_total..];
                                    if remaining.is_empty() {
                                        break;
                                    }
                                    let consumed = self.emit_struct_init(
                                        remaining,
                                        base,
                                        &sub_layout,
                                        elem_offset,
                                    );
                                    consumed_total += consumed.max(1);
                                    ai += 1;
                                }
                                // We consumed the first item already (counted by the outer loop),
                                // so subtract 1 from consumed_total.
                                if consumed_total > 0 {
                                    item_idx += consumed_total - 1;
                                }
                            }
                        } else {
                            // Non-char, non-struct array field with flat expression initializer:
                            // consume up to arr_size items from the init list to fill array elements
                            let elem_ir_ty = IrType::from_ctype(elem_ty);
                            let elem_size = elem_ir_ty.size().max(1);
                            let elem_is_bool = **elem_ty == CType::Bool;
                            let val = self.lower_init_expr_bool_aware(e, elem_ir_ty, elem_is_bool);
                            let field_addr = self.emit_gep_offset(base, field_offset, elem_ir_ty);
                            self.emit(Instruction::Store {
                                volatile: false,
                                val,
                                ptr: field_addr,
                                ty: elem_ir_ty,
                                seg_override: AddressSpace::Default,
                            });
                            // Consume additional items for remaining array elements
                            let mut arr_idx = 1usize;
                            while arr_idx < arr_size && item_idx + 1 < items.len() {
                                item_idx += 1;
                                let next_item = &items[item_idx];
                                // Stop if we hit a designator (it targets a different field)
                                if !next_item.designators.is_empty() {
                                    item_idx -= 1; // put it back
                                    break;
                                }
                                let expr_opt = match &next_item.init {
                                    Initializer::Expr(next_e) => Some(next_e),
                                    Initializer::List(sub_items) => {
                                        Self::unwrap_nested_init_expr(sub_items)
                                    }
                                };
                                if let Some(next_e) = expr_opt {
                                    let next_val = self.lower_init_expr_bool_aware(
                                        next_e,
                                        elem_ir_ty,
                                        elem_is_bool,
                                    );
                                    let offset = field_offset + arr_idx * elem_size;
                                    let elem_addr = self.emit_gep_offset(base, offset, elem_ir_ty);
                                    self.emit(Instruction::Store {
                                        volatile: false,
                                        val: next_val,
                                        ptr: elem_addr,
                                        ty: elem_ir_ty,
                                        seg_override: AddressSpace::Default,
                                    });
                                }
                                arr_idx += 1;
                            }
                        }
                    } else {
                        let val =
                            self.lower_init_expr_bool_aware(e, field_ty, field.ty == CType::Bool);
                        let field_addr = self.emit_gep_offset(base, field_offset, field_ty);
                        if let (Some(bit_offset), Some(bit_width)) =
                            (field.bit_offset, field.bit_width)
                        {
                            self.store_bitfield(
                                field_addr, field_ty, bit_offset, bit_width, val, false, field.sso,
                            );
                        } else {
                            let val = self.emit_sso_store_fixup(val, field_ty, field.sso);
                            self.emit(Instruction::Store {
                                volatile: false,
                                val,
                                ptr: field_addr,
                                ty: field_ty,
                                seg_override: AddressSpace::Default,
                            });
                        }
                    }
                }
                Initializer::List(sub_items) => {
                    let field_addr = self.emit_gep_offset(base, field_offset, field_ty);
                    self.lower_struct_field_init_list(sub_items, field_addr, &field.ty);
                }
            }
            current_field_idx = field_idx + 1;
            item_idx += 1;
        }
    }

    /// Initialize a struct field from a nested initializer list.
    /// Handles sub-struct fields and array fields.
    fn lower_struct_field_init_list(
        &mut self,
        items: &[InitializerItem],
        base: Value,
        field_ctype: &CType,
    ) {
        match field_ctype {
            CType::Struct(key) | CType::Union(key) => {
                let sub_layout = self.types.borrow_struct_layouts().get(&**key).cloned();
                if let Some(sub_layout) = sub_layout {
                    self.lower_local_struct_init(items, base, &sub_layout);
                }
            }
            CType::Array(ref elem_ty, arr_size_opt) => {
                // Check for char array initialized by a brace-wrapped string literal:
                // e.g., struct field `char a[10]` initialized as `{"hello"}`
                let is_char_array = matches!(**elem_ty, CType::Char | CType::UChar);
                if is_char_array && items.len() == 1 {
                    if let Initializer::Expr(Expr::StringLiteral(ref s, _)) = items[0].init {
                        let max_bytes = match arr_size_opt {
                            Some(sz) => *sz,
                            None => usize::MAX,
                        };
                        self.emit_string_to_alloca(base, s, 0, max_bytes);
                        // Zero-fill remaining bytes if string is shorter than array
                        if let Some(arr_size) = arr_size_opt {
                            let str_len = s.chars().count() + 1; // +1 for null terminator
                            for i in str_len..*arr_size {
                                let val = Operand::Const(IrConst::I8(0));
                                self.emit_store_at_offset(base, i, val, IrType::I8);
                            }
                        }
                        return;
                    }
                }
                // Array field: init elements with [idx]=val designator support
                let elem_ir_ty = IrType::from_ctype(elem_ty);
                let elem_size = self.resolve_ctype_size(elem_ty);
                let mut ai = 0usize;
                for item in items {
                    // Check for index designator: [idx]=val
                    if let Some(Designator::Index(ref idx_expr)) = item.designators.first() {
                        if let Some(idx) = self.eval_const_expr_for_designator(idx_expr) {
                            ai = idx;
                        }
                    }
                    let elem_is_bool = **elem_ty == CType::Bool;
                    match &item.init {
                        Initializer::Expr(e) => {
                            let val = self.lower_init_expr_bool_aware(e, elem_ir_ty, elem_is_bool);
                            self.emit_store_at_offset(base, ai * elem_size, val, elem_ir_ty);
                        }
                        Initializer::List(sub_items) => {
                            // Braced sub-initializer for array element (e.g., struct element)
                            let elem_addr = self.emit_gep_offset(base, ai * elem_size, elem_ir_ty);
                            self.lower_struct_field_init_list(sub_items, elem_addr, elem_ty);
                        }
                    }
                    ai += 1;
                }
            }
            _ => {
                // Scalar field with nested braces (e.g., int x = {5})
                if let Some(first) = items.first() {
                    if let Initializer::Expr(e) = &first.init {
                        let field_ir_ty = IrType::from_ctype(field_ctype);
                        let val = self.lower_init_expr_bool_aware(
                            e,
                            field_ir_ty,
                            *field_ctype == CType::Bool,
                        );
                        self.emit(Instruction::Store {
                            volatile: false,
                            val,
                            ptr: base,
                            ty: field_ir_ty,
                            seg_override: AddressSpace::Default,
                        });
                    }
                }
            }
        }
    }

    /// Unwrap nested `Initializer::List` items to find the innermost expression.
    /// Used for double-brace init like `{{expr}}` to extract the `expr`.
    pub(super) fn unwrap_nested_init_expr(items: &[InitializerItem]) -> Option<&Expr> {
        if let Some(first) = items.first() {
            match &first.init {
                Initializer::Expr(e) => Some(e),
                Initializer::List(sub_items) => Self::unwrap_nested_init_expr(sub_items),
            }
        } else {
            None
        }
    }

    /// Lower a multi-dimensional array initializer list.
    /// Handles nested braces like `{{1,2,3},{4,5,6}}` for `int a[2][3]`.
    pub(super) fn lower_array_init_list(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        base_ty: IrType,
        array_dim_strides: &[usize],
    ) {
        let mut flat_index = 0usize;
        self.lower_array_init_recursive(items, alloca, base_ty, array_dim_strides, &mut flat_index);
    }

    /// Recursive helper for multi-dimensional array initialization.
    /// Processes each initializer item, recursing for nested braces and
    /// advancing the flat_index to track the current element position.
    fn lower_array_init_recursive(
        &mut self,
        items: &[InitializerItem],
        alloca: Value,
        base_ty: IrType,
        array_dim_strides: &[usize],
        flat_index: &mut usize,
    ) {
        let elem_size = *array_dim_strides.last().unwrap_or(&1);
        let base_type_size = base_ty.size().max(1);
        let sub_elem_count = if array_dim_strides.len() > 1 && elem_size > 0 {
            array_dim_strides[0] / elem_size
        } else {
            1
        };

        for item in items {
            // Check for multi-dimensional index designators: [i][j]...
            let index_designators: Vec<usize> = item
                .designators
                .iter()
                .filter_map(|d| {
                    if let Designator::Index(ref idx_expr) = d {
                        self.eval_const_expr_for_designator(idx_expr)
                    } else {
                        None
                    }
                })
                .collect();

            if !index_designators.is_empty() {
                // Compute flat scalar index from multi-dimensional indices
                let mut target_flat = 0usize;
                for (i, &idx) in index_designators.iter().enumerate() {
                    let elems_per_entry = if i < array_dim_strides.len() && base_type_size > 0 {
                        array_dim_strides[i] / base_type_size
                    } else {
                        1
                    };
                    target_flat += idx * elems_per_entry;
                }

                match &item.init {
                    Initializer::List(sub_items) => {
                        // Set flat_index to designated position and recurse
                        *flat_index = target_flat;
                        let remaining_dims = array_dim_strides
                            .len()
                            .saturating_sub(index_designators.len());
                        let sub_strides =
                            &array_dim_strides[array_dim_strides.len() - remaining_dims..];
                        if remaining_dims > 0 {
                            self.lower_array_init_recursive(
                                sub_items,
                                alloca,
                                base_ty,
                                sub_strides,
                                flat_index,
                            );
                        } else {
                            for sub_item in sub_items {
                                if let Initializer::Expr(e) = &sub_item.init {
                                    let val = self.lower_and_cast_init_expr(e, base_ty);
                                    self.emit_array_element_store(
                                        alloca,
                                        val,
                                        *flat_index * elem_size,
                                        base_ty,
                                    );
                                    *flat_index += 1;
                                }
                            }
                        }
                    }
                    Initializer::Expr(e) => {
                        if base_ty == IrType::I8 || base_ty == IrType::U8 {
                            if let Expr::StringLiteral(s, _) = e {
                                self.emit_string_to_alloca(
                                    alloca,
                                    s,
                                    target_flat * elem_size,
                                    sub_elem_count * elem_size,
                                );
                                *flat_index = target_flat + sub_elem_count;
                                continue;
                            }
                        }
                        let val = self.lower_and_cast_init_expr(e, base_ty);
                        self.emit_array_element_store(
                            alloca,
                            val,
                            target_flat * elem_size,
                            base_ty,
                        );
                        *flat_index = target_flat + 1;
                    }
                }
                continue;
            }

            let start_index = *flat_index;
            match &item.init {
                Initializer::List(sub_items) => {
                    if array_dim_strides.len() > 1 {
                        self.lower_array_init_recursive(
                            sub_items,
                            alloca,
                            base_ty,
                            &array_dim_strides[1..],
                            flat_index,
                        );
                    } else {
                        // Bottom level: treat as flat elements
                        for sub_item in sub_items {
                            if let Initializer::Expr(e) = &sub_item.init {
                                let val = self.lower_and_cast_init_expr(e, base_ty);
                                self.emit_array_element_store(
                                    alloca,
                                    val,
                                    *flat_index * elem_size,
                                    base_ty,
                                );
                                *flat_index += 1;
                            }
                        }
                    }
                    // Advance to next sub-array boundary after a braced sub-list
                    // This handles partial initialization: {{1,2},{3}} for int a[2][3]
                    // After {1,2} we advance to index 3 (next row), after {3} to index 6
                    if array_dim_strides.len() > 1 {
                        let boundary = start_index + sub_elem_count;
                        if *flat_index < boundary {
                            *flat_index = boundary;
                        }
                    }
                }
                Initializer::Expr(e) => {
                    // String literal fills a sub-array in char arrays
                    if base_ty == IrType::I8 || base_ty == IrType::U8 {
                        if let Expr::StringLiteral(s, _) = e {
                            let max_str_bytes = sub_elem_count * elem_size;
                            self.emit_string_to_alloca(
                                alloca,
                                s,
                                *flat_index * elem_size,
                                max_str_bytes,
                            );
                            let string_stride = if array_dim_strides.len() >= 2 {
                                array_dim_strides[array_dim_strides.len() - 2]
                            } else {
                                sub_elem_count
                            };
                            *flat_index += string_stride;
                            continue;
                        }
                    }
                    // Bare scalar: fills one base element without sub-array padding
                    let val = self.lower_and_cast_init_expr(e, base_ty);
                    self.emit_array_element_store(alloca, val, *flat_index * elem_size, base_ty);
                    *flat_index += 1;
                }
            }
        }
    }

    /// Lower an expression and cast it to the target type (e.g., int to float).
    pub(super) fn lower_and_cast_init_expr(&mut self, expr: &Expr, target_ty: IrType) -> Operand {
        let val = self.lower_expr(expr);
        let expr_ty = self.get_expr_type(expr);
        self.emit_implicit_cast(val, expr_ty, target_ty)
    }

    /// Lower an init expression with bool-aware handling.
    /// For bool targets, normalizes to 0/1. For other types, casts to target_ty.
    pub(super) fn lower_init_expr_bool_aware(
        &mut self,
        expr: &Expr,
        target_ty: IrType,
        is_bool: bool,
    ) -> Operand {
        if is_bool {
            let val = self.lower_expr(expr);
            let expr_ty = self.get_expr_type(expr);
            self.emit_bool_normalize_typed(val, expr_ty)
        } else {
            self.lower_and_cast_init_expr(expr, target_ty)
        }
    }

    /// Main statement lowering dispatcher. Delegates to per-statement-type methods.
    pub(super) fn lower_stmt(&mut self, stmt: &Stmt) {
        // Fresh FP expression root per statement (OP-36): every FP arith
        // result lowered inside this statement is tagged with it, so
        // -ffp-contract=on fuses mul+add only within one source expression.
        self.func_mut().fp_expr_root = self.func_mut().fp_expr_root.wrapping_add(1);
        // Set the current span for debug info tracking. All instructions emitted
        // while lowering this statement will inherit this source location.
        if let Some(span) = stmt.span() {
            self.func_mut().current_span = span;
        }
        match stmt {
            Stmt::Return(expr, _span) => {
                // GNU C: `return void_expr;` in a void function (accepted by
                // sema for GCC compatibility — glibc `return helper();`
                // shims). The expression is evaluated for side effects only;
                // the Return itself must carry no operand, otherwise it
                // references the "result" of a void call that no instruction
                // defines (malloc.c __libc_free ICE: Return(Some(v59)) with
                // v59 undefined).
                let func_returns_void = self
                    .func_state
                    .as_ref()
                    .map(|fs| fs.return_type == IrType::Void)
                    .unwrap_or(false);
                let op = if func_returns_void {
                    if let Some(e) = expr.as_ref() {
                        self.lower_expr_discard(e);
                    }
                    None
                } else {
                    expr.as_ref().map(|e| self.lower_return_expr(e))
                };
                // Emit cleanup calls for all active scopes before returning
                let all_cleanups = self.collect_all_scope_cleanup_vars();
                self.emit_cleanup_calls(&all_cleanups);
                self.terminate(Terminator::Return(op));
                let label = self.fresh_label();
                self.start_block(label);
            }
            Stmt::Expr(Some(expr)) => {
                // For expression statements, use the expression's span for better precision
                self.func_mut().current_span = expr.span();
                // The expression's result is discarded: post-inc/dec can skip
                // their volatile old-value spill.
                self.lower_expr_discard(expr);
            }
            Stmt::Expr(None) => {}
            Stmt::Compound(compound) => self.lower_compound_stmt(compound),
            Stmt::If(cond, then_stmt, else_stmt, _span) => {
                self.lower_if_stmt(cond, then_stmt, else_stmt.as_deref())
            }
            Stmt::While(cond, body, _span) => self.lower_while_stmt(cond, body),
            Stmt::For(init, cond, inc, body, _span) => self.lower_for_stmt(init, cond, inc, body),
            Stmt::DoWhile(body, cond, _span) => self.lower_do_while_stmt(body, cond),
            Stmt::Break(_span) => self.lower_break_stmt(),
            Stmt::Continue(_span) => self.lower_continue_stmt(),
            Stmt::Switch(expr, body, _span) => self.lower_switch_stmt(expr, body),
            Stmt::Case(expr, stmt, _span) => self.lower_case_stmt(expr, stmt),
            Stmt::CaseRange(low_expr, high_expr, stmt, _span) => {
                self.lower_case_range_stmt(low_expr, high_expr, stmt)
            }
            Stmt::Default(stmt, _span) => self.lower_default_stmt(stmt),
            Stmt::Goto(label, _span) => self.lower_goto_stmt(label),
            Stmt::GotoIndirect(expr, _span) => self.lower_goto_indirect_stmt(expr),
            Stmt::Label(name, stmt, _span) => self.lower_label_stmt(name, stmt),
            Stmt::Declaration(decl) => {
                self.func_mut().current_span = decl.span;
                self.lower_local_decl(decl);
            }
            Stmt::InlineAsm {
                template,
                outputs,
                inputs,
                clobbers,
                goto_labels,
            } => {
                self.lower_inline_asm_stmt(template, outputs, inputs, clobbers, goto_labels);
            }
        }
    }

    /// Compute the runtime sizeof for a VLA local variable.
    /// Returns Some(Value) if any array dimension is a non-constant expression.
    /// The Value holds the total byte size (product of all dimensions * element_size).
    /// Returns None if all dimensions are compile-time constants.
    pub(super) fn compute_vla_runtime_size(
        &mut self,
        type_spec: &TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> Option<Value> {
        // Collect array dimensions from derived declarators
        let array_dims: Vec<&Option<Box<Expr>>> = derived
            .iter()
            .filter_map(|d| {
                if let DerivedDeclarator::Array(size) = d {
                    Some(size)
                } else {
                    None
                }
            })
            .collect();

        if array_dims.is_empty() {
            // Check if the type_spec itself is an Array (typedef'd VLA)
            return self.compute_vla_size_from_type_spec(type_spec);
        }

        // Check if any dimension is non-constant
        let mut has_vla = false;
        for expr in array_dims.iter().copied().flatten() {
            if self.expr_as_array_size(expr).is_none() {
                has_vla = true;
                break;
            }
        }

        // Runtime element size: GCC's VLA-in-struct extension makes arrays
        // of such structs (or of VLA typedefs) dynamically sized even when
        // every dimension is a compile-time constant. For `struct S s[2]`
        // with `struct S { int i[n]; ... }`, the object needs
        // 2 * sizeof(S) bytes of dynamic stack storage — a static alloca
        // sized from the FAM-based static sizeof would only reserve a
        // fraction, and the k * sizeof(S) element addressing would then
        // write past the object (gcc.c-torture/execute align-nest.c).
        let elem_runtime = self.compute_vla_size_from_type_spec(type_spec);

        if !has_vla {
            // All dimensions are compile-time constants: dynamic iff the
            // element type is.
            if let Some(ev) = elem_runtime {
                let mut count: usize = 1;
                for expr in array_dims.iter().copied().flatten() {
                    if let Some(c) = self.expr_as_array_size(expr) {
                        count = count.saturating_mul(c as usize);
                    }
                }
                let ptr_int_ty = target_int_ir_type();
                return Some(if count > 1 {
                    self.emit_binop_val(
                        IrBinOp::Mul,
                        Operand::Value(ev),
                        Operand::Const(IrConst::ptr_int(count as i64)),
                        ptr_int_ty,
                    )
                } else {
                    ev
                });
            }
            return None;
        }

        // Compute element size, accounting for pointer/function-pointer derivations.
        // For `int *ap[n]`, derived = [Pointer, Array(n)] and type_spec = int.
        // The element type is actually `int*` (8 bytes), not `int` (4 bytes).
        // We detect this by checking if any Pointer/FunctionPointer derivation
        // appears before (or among) the array dimensions.
        let base_elem_size = self.vla_base_element_size(type_spec, derived);

        // Build runtime product: dim0 * dim1 * ... * elem_size
        let mut result: Option<Value> = None;
        // With a runtime element size, the constant dimension product folds
        // separately and the element factor is multiplied in at the end.
        let mut const_product: usize = if elem_runtime.is_some() {
            1
        } else {
            base_elem_size
        };

        for expr in array_dims.iter().copied().flatten() {
            if let Some(const_val) = self.expr_as_array_size(expr) {
                // Constant dimension - accumulate
                const_product *= const_val as usize;
            } else {
                // Runtime dimension - emit multiplication
                let dim_val = self.lower_expr(expr);
                let dim_value = self.operand_to_value(dim_val);

                let ptr_int_ty = target_int_ir_type();
                result = if let Some(prev) = result {
                    let mul = self.emit_binop_val(
                        IrBinOp::Mul,
                        Operand::Value(prev),
                        Operand::Value(dim_value),
                        ptr_int_ty,
                    );
                    Some(mul)
                } else {
                    // First runtime dim: multiply by accumulated constants
                    if const_product > 1 {
                        let mul = self.emit_binop_val(
                            IrBinOp::Mul,
                            Operand::Value(dim_value),
                            Operand::Const(IrConst::ptr_int(const_product as i64)),
                            ptr_int_ty,
                        );
                        const_product = 1;
                        Some(mul)
                    } else {
                        Some(dim_value)
                    }
                };
            }
        }

        // Multiply in the runtime element factor (VLA-member struct or VLA
        // typedef element): total = dims_product * elem_size.
        if let Some(ev) = elem_runtime {
            let ptr_int_ty = target_int_ir_type();
            let dims_val = match result {
                Some(r) if const_product > 1 => Some(self.emit_binop_val(
                    IrBinOp::Mul,
                    Operand::Value(r),
                    Operand::Const(IrConst::ptr_int(const_product as i64)),
                    ptr_int_ty,
                )),
                r => r,
            };
            return match dims_val {
                Some(d) => Some(self.emit_binop_val(
                    IrBinOp::Mul,
                    Operand::Value(d),
                    Operand::Value(ev),
                    ptr_int_ty,
                )),
                None => {
                    // No runtime dims reached the loop: const_product holds
                    // the full constant count.
                    if const_product > 1 {
                        Some(self.emit_binop_val(
                            IrBinOp::Mul,
                            Operand::Value(ev),
                            Operand::Const(IrConst::ptr_int(const_product as i64)),
                            ptr_int_ty,
                        ))
                    } else {
                        Some(ev)
                    }
                }
            };
        }

        // If we have remaining constant factors, multiply them in
        if let Some(prev) = result {
            if const_product > 1 {
                let ptr_int_ty = target_int_ir_type();
                let mul = self.emit_binop_val(
                    IrBinOp::Mul,
                    Operand::Value(prev),
                    Operand::Const(IrConst::ptr_int(const_product as i64)),
                    ptr_int_ty,
                );
                return Some(mul);
            }
            return Some(prev);
        }

        None
    }

    /// Compute VLA strides for a local VLA variable declaration.
    ///
    /// For `double a[n][m]`, we need strides:
    ///   stride[0] = m * sizeof(double)   (stride for first dimension = row stride)
    ///   stride[1] = sizeof(double)       (stride for second dimension = element stride)
    ///
    /// The strides array has one entry per array dimension. Each entry is
    /// `Some(Value)` if the stride requires a runtime computation, or `None`
    /// if it's a compile-time constant (handled by the fallback path).
    ///
    /// We process dimensions from innermost to outermost, accumulating the
    /// product of inner dimensions * base element size.
    pub(super) fn compute_vla_local_strides(
        &mut self,
        type_spec: &TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> Vec<Option<Value>> {
        // Collect array dimensions from derived declarators
        let array_dims: Vec<&Option<Box<Expr>>> = derived
            .iter()
            .filter_map(|d| {
                if let DerivedDeclarator::Array(size) = d {
                    Some(size)
                } else {
                    None
                }
            })
            .collect();

        if array_dims.len() < 2 {
            // For 1D VLAs, no stride info needed (the element size is known at compile time)
            return vec![];
        }

        // Check if any dimension is a VLA
        let mut has_vla = false;
        for expr in array_dims.iter().copied().flatten() {
            if self.expr_as_array_size(expr).is_none() {
                has_vla = true;
                break;
            }
        }
        if !has_vla {
            return vec![];
        }

        // Account for pointer derivations in element size (same as compute_vla_runtime_size)
        let base_elem_size = self.vla_base_element_size(type_spec, derived);
        let num_dims = array_dims.len();
        let num_strides = num_dims + 1; // +1 for base element size level
        let mut vla_strides: Vec<Option<Value>> = vec![None; num_strides];

        // Process dimensions from innermost (last) to outermost (first).
        // Each stride[i] = product of all inner dimension sizes * base_elem_size.
        // stride[num_dims-1] is always base_elem_size (the innermost element stride).
        // stride[i] = array_dims[i+1] * stride[i+1] for i < num_dims-1.
        let mut current_stride: Option<Value> = None;
        let mut current_const_stride: usize = base_elem_size;

        for i in (0..num_dims).rev() {
            if i == num_dims - 1 {
                // Innermost dimension: stride is base_elem_size (compile-time constant)
                // No need to set vla_strides[i] since the fallback handles constants.
                // But we need to track it for computing outer strides.
                continue;
            }

            // The stride for dimension i = dimension_size[i+1] * stride[i+1]
            // We need to compute this from the (i+1)th dimension.
            let inner_dim = &array_dims[i + 1];
            if let Some(expr) = inner_dim {
                if let Some(const_val) = self.expr_as_array_size(expr) {
                    // Inner dimension is a compile-time constant
                    current_const_stride *= const_val as usize;
                    if current_stride.is_some() {
                        // Previous stride was runtime, multiply by constant
                        let ptr_int_ty = target_int_ir_type();
                        let stride_val = self.emit_binop_val(
                            IrBinOp::Mul,
                            Operand::Value(current_stride.unwrap()),
                            Operand::Const(IrConst::ptr_int(const_val)),
                            ptr_int_ty,
                        );
                        current_stride = Some(stride_val);
                        vla_strides[i] = Some(stride_val);
                    }
                    // else: purely const, fallback handles it
                } else {
                    // Inner dimension is a runtime VLA dimension
                    let dim_val = self.lower_expr(expr);
                    let dim_value = self.operand_to_value(dim_val);

                    let stride_val = if let Some(prev) = current_stride {
                        let ptr_int_ty = target_int_ir_type();
                        self.emit_binop_val(
                            IrBinOp::Mul,
                            Operand::Value(dim_value),
                            Operand::Value(prev),
                            ptr_int_ty,
                        )
                    } else {
                        // First runtime dimension: multiply by accumulated const
                        if current_const_stride > 1 {
                            let ptr_int_ty = crate::common::types::target_int_ir_type();
                            self.emit_binop_val(
                                IrBinOp::Mul,
                                Operand::Value(dim_value),
                                Operand::Const(IrConst::ptr_int(current_const_stride as i64)),
                                ptr_int_ty,
                            )
                        } else {
                            dim_value
                        }
                    };
                    current_stride = Some(stride_val);
                    current_const_stride = 0;
                    vla_strides[i] = Some(stride_val);
                }
            }
        }

        vla_strides
    }

    /// Compute the base element size for a VLA declaration, accounting for
    /// pointer/function-pointer derivations in the declarator.
    ///
    /// For `int *ap[n]`, derived = [Pointer, Array(n)] and type_spec = int.
    /// The element type is `int*` (pointer, 8 bytes), not `int` (4 bytes).
    /// We detect this by checking whether any Pointer or FunctionPointer
    /// derivation appears in the derived list alongside Array entries.
    /// If the outermost (last) derivation is an Array and there's a Pointer
    /// before it, the array elements are pointers (8 bytes on 64-bit).
    fn vla_base_element_size(
        &self,
        type_spec: &TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> usize {
        let has_pointer = derived
            .iter()
            .any(|d| matches!(d, DerivedDeclarator::Pointer));
        let has_func_ptr = derived.iter().any(|d| {
            matches!(
                d,
                DerivedDeclarator::FunctionPointer(_, _) | DerivedDeclarator::Function(_, _)
            )
        });
        let has_array = derived
            .iter()
            .any(|d| matches!(d, DerivedDeclarator::Array(_)));
        let last_is_array = matches!(derived.last(), Some(DerivedDeclarator::Array(_)));

        if has_array && (has_pointer || has_func_ptr) && last_is_array {
            // Array of pointers (e.g., int *ap[n], void (*fns[n])(int))
            // Each element is a pointer.
            crate::common::types::target_ptr_size()
        } else {
            // Plain array of the base type (e.g., int a[n], double b[n][m])
            self.sizeof_type(type_spec)
        }
    }

    /// Compute VLA size from a typedef'd array type (e.g., typedef char buf[n])
    /// or from a struct/union type whose fields include a VLA.
    pub(super) fn compute_vla_size_from_type_spec(
        &mut self,
        type_spec: &TypeSpecifier,
    ) -> Option<Value> {
        if let TypeSpecifier::TypedefName(name) = type_spec {
            if let Some(&vla_size) = self
                .func_state
                .as_ref()
                .and_then(|fs| fs.vla_typedef_sizes.get(name))
            {
                return Some(vla_size);
            }
        }
        if let TypeSpecifier::Struct(Some(tag), ..) = type_spec {
            let key = format!("struct.{}", tag);
            if let Some(&vla_size) = self
                .func_state
                .as_ref()
                .and_then(|fs| fs.vla_typedef_sizes.get(&key))
            {
                return Some(vla_size);
            }
        }
        if let TypeSpecifier::Union(Some(tag), ..) = type_spec {
            let key = format!("union.{}", tag);
            if let Some(&vla_size) = self
                .func_state
                .as_ref()
                .and_then(|fs| fs.vla_typedef_sizes.get(&key))
            {
                return Some(vla_size);
            }
        }
        if let TypeSpecifier::Typeof(expr) = type_spec {
            if let Expr::Identifier(name, _) = &**expr {
                if let Some(vla_size) = self
                    .func_state
                    .as_ref()
                    .and_then(|fs| fs.locals.get(name))
                    .and_then(|info| info.vla_size)
                {
                    return Some(vla_size);
                }
            }
        }
        let resolved = self.resolve_type_spec(type_spec).clone();
        match &resolved {
            TypeSpecifier::Array(elem, Some(size_expr)) => {
                if self.expr_as_array_size(size_expr).is_some() {
                    return None; // Constant size
                }
                // Clone what we need before mutable borrow
                let size_expr_clone = size_expr.clone();
                let elem_size = self.sizeof_type(elem);
                // Runtime size expression
                let dim_val = self.lower_expr(&size_expr_clone);
                let dim_value = self.operand_to_value(dim_val);
                if elem_size > 1 {
                    let ptr_int_ty = crate::common::types::target_int_ir_type();
                    let mul = self.emit_binop_val(
                        IrBinOp::Mul,
                        Operand::Value(dim_value),
                        Operand::Const(IrConst::ptr_int(elem_size as i64)),
                        ptr_int_ty,
                    );
                    Some(mul)
                } else {
                    Some(dim_value)
                }
            }
            TypeSpecifier::Struct(
                _,
                Some(fields),
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => self.compute_vla_struct_sizeof(
                fields,
                false,
                *is_packed,
                *struct_aligned,
                *pragma_pack,
                reverse_sso.is_some_and(|v| v),
            ),
            TypeSpecifier::Union(
                _,
                Some(fields),
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => self.compute_vla_struct_sizeof(
                fields,
                true,
                *is_packed,
                *struct_aligned,
                *pragma_pack,
                reverse_sso.is_some_and(|v| v),
            ),
            TypeSpecifier::TypedefName(name) => {
                if let Some(ct) = self.types.typedefs.get(name).cloned() {
                    match ct {
                        CType::Struct(key) | CType::Union(key) => self
                            .func_state
                            .as_ref()
                            .and_then(|fs| fs.vla_typedef_sizes.get(key.as_ref()).copied()),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Runtime sizeof of a struct/union that contains at least one VLA field.
    ///
    /// GCC extension: a local struct may have a VLA member, and `sizeof` of
    /// that type is evaluated at the point of the type definition (the VLA
    /// bound is captured, later stores to the bound expression are ignored).
    /// Covers gcc.c-torture 20040423-1 (`typedef struct { int c[i+2]; } c`)
    /// and 20041218-2 (`struct s { char b[n]; } packed; n++; sizeof(struct s)`).
    fn compute_vla_struct_sizeof(
        &mut self,
        fields: &[StructFieldDecl],
        is_union: bool,
        is_packed: bool,
        struct_aligned: Option<usize>,
        pragma_pack: Option<usize>,
        reverse_sso: bool,
    ) -> Option<Value> {
        let ptr_int_ty = target_int_ir_type();

        // Locate the first VLA member (pure probe — the walk below produces
        // the size Values, so probing must not emit).
        let vla_idx = fields
            .iter()
            .position(|f| f.bit_width.is_none() && self.field_vla_runtime_size_probe(f));
        let Some(vla_idx) = vla_idx else { return None };

        // Union size is the max of members, not the sum. The VLA member is
        // typically the largest; returning it is correct for a single VLA
        // field (the only form the torture suite uses). Bitfield members
        // never grow a union past its VLA member here.
        if is_union {
            let f = &fields[vla_idx];
            return self.field_vla_runtime_size(f);
        }

        // Seed the cursor with the VLA member's STATIC offset: everything
        // before it is fixed-size and its exact offset (including any
        // pre-VLA bitfield units) is already in the static layout.
        let max_field_align = if is_packed { Some(1) } else { pragma_pack };
        let layout =
            self.compute_struct_union_layout_packed(fields, false, max_field_align, reverse_sso);
        let vla_member_static_offset = fields[..vla_idx]
            .iter()
            .filter_map(|f| {
                let name = f.name.as_deref()?;
                layout
                    .fields
                    .iter()
                    .find(|fl| fl.name == name)
                    .map(|fl| fl.offset)
            })
            .next()
            .unwrap_or(0);

        let mut cursor = VlaDynCursor::new(vla_member_static_offset, is_packed);
        let mut max_align: usize = 1;
        let mut last_bitfield_unit = 0usize;

        for f in &fields[vla_idx..] {
            let bit_width = f
                .bit_width
                .as_ref()
                .and_then(|bw| self.eval_const_expr(bw).and_then(|c| c.to_u32()));
            if let Some(bw) = bit_width {
                let ct = self.struct_field_ctype(f);
                if is_packed {
                    // A packed zero-width bitfield still forces the next
                    // boundary of its declared type (GCC packed `T :0`
                    // semantics); nonzero widths pack bit-granularly.
                    if bw == 0 {
                        let a = self.vla_field_effective_align(f, &ct, is_packed, pragma_pack);
                        self.vla_cursor_close_run_for_field(&mut cursor);
                        self.vla_cursor_align(&mut cursor, a);
                        if a > max_align {
                            max_align = a;
                        }
                    } else {
                        self.vla_cursor_bitfield_packed(&mut cursor, bw);
                        last_bitfield_unit = 1; // byte-rounded on close
                    }
                } else {
                    let unit_size = self.ctype_size(&ct);
                    let unit_align = self.vla_field_effective_align(f, &ct, is_packed, pragma_pack);
                    if unit_size == 0 {
                        continue;
                    }
                    if unit_align > max_align {
                        max_align = unit_align;
                    }
                    if bw == 0 {
                        self.vla_cursor_close_run_for_field(&mut cursor);
                        self.vla_cursor_align(&mut cursor, unit_align);
                    } else if unit_size <= cursor.guarantee {
                        self.vla_cursor_bitfield_unit(&mut cursor, bw, unit_size);
                        last_bitfield_unit = unit_size;
                    } else {
                        // Straddle undecidable: fresh aligned unit.
                        self.vla_cursor_close_run_for_field(&mut cursor);
                        self.vla_cursor_align(&mut cursor, unit_align);
                        self.vla_cursor_add_fixed(&mut cursor, unit_size);
                        last_bitfield_unit = 0;
                    }
                }
                continue;
            }
            let ct = self.struct_field_ctype(f);
            let align = self.vla_field_effective_align(f, &ct, is_packed, pragma_pack);
            if align > max_align {
                max_align = align;
            }
            self.vla_cursor_close_run_for_field(&mut cursor);
            self.vla_cursor_align(&mut cursor, align);
            last_bitfield_unit = 0;

            if let Some(vla_sz) = self.field_vla_runtime_size(f) {
                self.vla_cursor_add_vla(&mut cursor, vla_sz);
                continue;
            }
            let sz = self.ctype_size(&ct);
            self.vla_cursor_add_fixed(&mut cursor, sz);
        }
        // A trailing bitfield run extends the size to the end of its last
        // storage unit (packed: to the next byte).
        if cursor.run_open {
            let close_unit = if is_packed || last_bitfield_unit == 0 {
                0
            } else {
                last_bitfield_unit
            };
            self.vla_cursor_close_run_for_size(&mut cursor, close_unit);
        }

        let total = self.vla_cursor_total(&cursor);

        // Struct-level __attribute__((aligned(N))) raises the minimum
        // alignment (and rounds the size) exactly like the static path in
        // resolve_struct_or_union does. A packed struct without it rounds to
        // alignment 1 (no rounding).
        if struct_aligned.is_some_and(|a| a > max_align) {
            max_align = struct_aligned.unwrap();
        }
        if max_align > 1 {
            let mask = (max_align - 1) as i64;
            let added = self.emit_binop_val(
                IrBinOp::Add,
                total,
                Operand::Const(IrConst::ptr_int(mask)),
                ptr_int_ty,
            );
            let aligned = self.emit_binop_val(
                IrBinOp::And,
                Operand::Value(added),
                Operand::Const(IrConst::ptr_int(!mask)),
                ptr_int_ty,
            );
            Some(aligned)
        } else {
            match total {
                Operand::Value(v) => Some(v),
                // The walk only runs after the probe found a runtime-VLA
                // member, so the total always carries a runtime component.
                _ => None,
            }
        }
    }

    /// Runtime byte size of a struct field if it is (or contains) a VLA.
    pub(super) fn field_vla_runtime_size(&mut self, field: &StructFieldDecl) -> Option<Value> {
        if !field.derived.is_empty() {
            if let Some(v) = self.compute_vla_runtime_size(&field.type_spec, &field.derived) {
                return Some(v);
            }
        }
        self.compute_vla_size_from_type_spec(&field.type_spec)
    }
}

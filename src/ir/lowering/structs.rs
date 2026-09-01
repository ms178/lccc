use super::lower::Lowerer;
use crate::common::types::{AddressSpace, CType, IrType, RcLayout, SsoMode, StructLayout};
use crate::frontend::parser::ast::{
    BinOp, Declaration, DerivedDeclarator, Expr, StructFieldDecl, TypeSpecifier, UnaryOp,
};
use crate::ir::reexports::{Instruction, IrConst, Operand, Value};
use std::rc::Rc;

/// Dynamic byte/bit cursor for laying out the part of a struct that follows
/// a VLA member (GCC's VLA-in-struct extension).
///
/// The cursor value is `post + runtime` bytes, where `post` is a compile-time
/// byte total and `runtime` is an IR value (the sum of VLA member sizes, plus
/// any post-VLA runtime-aligned deltas folded in by [`Lowerer::vla_cursor_align`]).
/// An open bitfield run tracks `run_bits` of bit-granular placement on top.
///
/// Invariant: at every closed-run boundary, `post + runtime ≡ 0 (mod guarantee)`
/// at runtime. `guarantee` starts at the VLA member's element alignment (its
/// static offset is aligned to it and its runtime size is a multiple of it)
/// and is reduced by `gcd` on fixed deltas or raised by explicit alignment.
/// Every divisor of `guarantee` therefore divides the cursor — which is what
/// makes the SysV bitfield straddle test decidable for
/// `sizeof(T) <= guarantee`.
pub(super) struct VlaDynCursor {
    /// Compile-time byte prefix (VLA member's static offset + fixed deltas).
    pub(super) post: usize,
    /// Runtime byte component, if any VLA member has been seen.
    pub(super) runtime: Option<Value>,
    /// An open bitfield run is placing bits.
    pub(super) run_open: bool,
    /// Bits placed since the run opened (measured from a cursor that was
    /// ≡ 0 (mod guarantee) — every divisor of guarantee divides the run base).
    pub(super) run_bits: u32,
    /// Packed struct: bitfields pack at bit granularity, no unit constraint.
    pub(super) packed: bool,
    /// cursor ≡ 0 (mod guarantee) at every closed-run boundary.
    pub(super) guarantee: usize,
}

impl VlaDynCursor {
    pub(super) fn new(seed_offset: usize, packed: bool) -> Self {
        Self {
            post: seed_offset,
            runtime: None,
            run_open: false,
            run_bits: 0,
            packed,
            guarantee: 1,
        }
    }
}

fn gcd_usize(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd_usize(b, a % b)
    }
}

impl Lowerer {
    /// Allocation size for a temporary that stores packed small-struct data.
    /// For odd sizes (3/5/6/7), the carrier register type is wider (I32/I64),
    /// so reserve enough bytes to hold the full store without overrunning.
    pub(super) fn packed_spill_alloc_size(size: usize) -> usize {
        let size = if size == 0 { 8 } else { size };
        size.max(Self::packed_store_type(size).size())
    }

    /// Spill packed small-struct data from a scalar/register value into a temporary
    /// alloca and return its address.
    pub(super) fn spill_packed_data_to_alloca(
        &mut self,
        val: Operand,
        size: usize,
        align: usize,
    ) -> Value {
        let alloc_size = Self::packed_spill_alloc_size(size);
        let store_ty = Self::packed_store_type(size);
        let alloca = self.fresh_value();
        self.emit(Instruction::Alloca {
            dest: alloca,
            size: alloc_size,
            ty: store_ty,
            align,
            volatile: false,
            semantic_volatile: false,
        });
        self.emit(Instruction::Store {
            val,
            ptr: alloca,
            ty: store_ty,
            seg_override: AddressSpace::Default,
            volatile: false,
        });
        alloca
    }

    /// Store packed small-struct data to a destination pointer without clobbering
    /// adjacent bytes for odd non-power-of-two sizes (e.g. a 5-byte packed struct
    /// must not overwrite the 3 bytes that follow it when stored as an I64).
    pub(super) fn store_packed_data_exact(
        &mut self,
        dest: Value,
        val: Operand,
        size: usize,
    ) {
        if size == 0 {
            return;
        }
        match size {
            1 | 2 | 4 | 8 => {
                self.emit(Instruction::Store {
                    val,
                    ptr: dest,
                    ty: Self::packed_store_type(size),
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
            }
            _ => {
                let tmp = self.spill_packed_data_to_alloca(val, size, 0);
                self.emit(Instruction::Memcpy {
                    dest,
                    src: tmp,
                    size,
                });
            }
        }
    }

    /// Load up to 8 bytes of struct data into an I64 carrier value.
    /// Odd sizes are loaded via temp+memcpy to avoid out-of-bounds reads.
    pub(super) fn load_packed_struct_i64(&mut self, src: Value, size: usize) -> Value {
        let size = if size == 0 { 8 } else { size };
        debug_assert!(
            size <= 8,
            "load_packed_struct_i64 expects size <= 8, got {}",
            size
        );

        match size {
            1 => {
                let raw = self.fresh_value();
                self.emit(Instruction::Load {
                    dest: raw,
                    ptr: src,
                    ty: IrType::U8,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                self.emit_cast_val(Operand::Value(raw), IrType::U8, IrType::I64)
            }
            2 => {
                let raw = self.fresh_value();
                self.emit(Instruction::Load {
                    dest: raw,
                    ptr: src,
                    ty: IrType::U16,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                self.emit_cast_val(Operand::Value(raw), IrType::U16, IrType::I64)
            }
            4 => {
                let raw = self.fresh_value();
                self.emit(Instruction::Load {
                    dest: raw,
                    ptr: src,
                    ty: IrType::U32,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                self.emit_cast_val(Operand::Value(raw), IrType::U32, IrType::I64)
            }
            8 => {
                let packed = self.fresh_value();
                self.emit(Instruction::Load {
                    dest: packed,
                    ptr: src,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                packed
            }
            _ => {
                let tmp = self.fresh_value();
                self.emit(Instruction::Alloca {
                    dest: tmp,
                    size: 8,
                    ty: IrType::I64,
                    align: 0,
                    volatile: false,
                    semantic_volatile: false,
                });
                self.emit(Instruction::Store {
                    val: Operand::Const(IrConst::I64(0)),
                    ptr: tmp,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                self.emit(Instruction::Memcpy {
                    dest: tmp,
                    src,
                    size,
                });
                let packed = self.fresh_value();
                self.emit(Instruction::Load {
                    dest: packed,
                    ptr: tmp,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                });
                packed
            }
        }
    }

    /// Register a struct/union type definition from a TypeSpecifier, computing and
    /// caching its layout in self.types.struct_layouts. Also recursively registers any
    /// nested struct/union types defined in the fields.
    pub(super) fn register_struct_type(&mut self, ts: &TypeSpecifier) {
        match ts {
            TypeSpecifier::Struct(
                tag,
                Some(fields),
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => {
                // Recursively register nested struct/union types in fields
                self.register_nested_struct_types(fields);
                let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                let mut layout = self.compute_struct_union_layout_packed(
                    fields,
                    false,
                    max_field_align,
                    reverse_sso.is_some_and(|v| v),
                );
                // Apply struct-level __attribute__((aligned(N))): sets minimum alignment
                if let Some(a) = struct_aligned {
                    if *a > layout.align {
                        layout.align = *a;
                        let mask = layout.align - 1;
                        layout.size = (layout.size + mask) & !mask;
                    }
                }
                let key = self.struct_layout_key(tag, false);
                // Record runtime sizeof for structs with VLA members so
                // sizeof(struct s) / sizeof(typedef_of_s) is not a compile-time
                // constant. Bound expressions are evaluated here, at the type
                // definition, matching C99/GCC (later stores to `n` are ignored).
                // (Runs before insert_struct_layout_scoped, which takes the
                // layout by value and the dynamic-offset walk reads it.)
                if self.func_state.is_some() {
                    if let Some(vla_size) = self.compute_vla_size_from_type_spec(ts) {
                        self.func_mut()
                            .insert_vla_typedef_size_scoped(key.clone(), vla_size);
                    }
                    self.compute_vla_field_strides_for_decl(&key, fields);
                    self.compute_vla_field_offsets_for_decl(
                        &key,
                        fields,
                        *is_packed,
                        *pragma_pack,
                        &layout,
                    );
                }
                self.insert_struct_layout_scoped(key.clone(), layout);
                // Also invalidate the ctype_cache for this tag so sizeof picks
                // up the new definition
                if tag.is_some() {
                    self.invalidate_ctype_cache_scoped(&key);
                }
            }
            TypeSpecifier::Union(
                tag,
                Some(fields),
                is_packed,
                pragma_pack,
                struct_aligned,
                reverse_sso,
            ) => {
                // Recursively register nested struct/union types in fields
                self.register_nested_struct_types(fields);
                let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                let mut layout = self.compute_struct_union_layout_packed(
                    fields,
                    true,
                    max_field_align,
                    reverse_sso.is_some_and(|v| v),
                );
                // Apply struct-level __attribute__((aligned(N))): sets minimum alignment
                if let Some(a) = struct_aligned {
                    if *a > layout.align {
                        layout.align = *a;
                        let mask = layout.align - 1;
                        layout.size = (layout.size + mask) & !mask;
                    }
                }
                let key = self.struct_layout_key(tag, true);
                self.insert_struct_layout_scoped(key.clone(), layout);
                if tag.is_some() {
                    self.invalidate_ctype_cache_scoped(&key);
                }
                if self.func_state.is_some() {
                    if let Some(vla_size) = self.compute_vla_size_from_type_spec(ts) {
                        self.func_mut()
                            .insert_vla_typedef_size_scoped(key, vla_size);
                    }
                }
            }
            _ => {}
        }
    }

    /// Re-compute struct/union layouts that contain vector typedef fields.
    /// Sema computed these layouts before vector_size was applied to typedefs,
    /// so their field sizes may be wrong (e.g., float4 treated as float instead
    /// of Vector(float, 16)). This updates the EXISTING layout entries by key
    /// rather than creating new ones, since CType::Union/Struct references
    /// use the key assigned by sema.
    pub(super) fn recompute_vector_struct_layouts(
        &mut self,
        tu: &crate::frontend::parser::ast::TranslationUnit,
    ) {
        use crate::frontend::parser::ast::ExternalDecl;
        let mut type_specs_to_recompute: Vec<&TypeSpecifier> = Vec::new();
        for decl in &tu.decls {
            match decl {
                ExternalDecl::Declaration(decl) => {
                    Self::collect_struct_union_type_specs(
                        &decl.type_spec,
                        &mut type_specs_to_recompute,
                    );
                }
                ExternalDecl::FunctionDef(func) => {
                    Self::collect_struct_union_type_specs(
                        &func.return_type,
                        &mut type_specs_to_recompute,
                    );
                    for p in &func.params {
                        Self::collect_struct_union_type_specs(
                            &p.type_spec,
                            &mut type_specs_to_recompute,
                        );
                    }
                }
                ExternalDecl::TopLevelAsm(_) => {}
            }
        }
        for ts in type_specs_to_recompute {
            self.recompute_layout_if_vector_fields(ts);
        }
    }

    /// Collect all struct/union TypeSpecifiers with inline field definitions from a type.
    fn collect_struct_union_type_specs<'a>(
        ts: &'a TypeSpecifier,
        out: &mut Vec<&'a TypeSpecifier>,
    ) {
        match ts {
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => {
                out.push(ts);
                for f in fields {
                    Self::collect_struct_union_type_specs(&f.type_spec, out);
                }
            }
            _ => {}
        }
    }

    /// Re-compute a struct/union layout if any of its fields use vector typedefs.
    /// Updates the existing layout entry in the map using the key from sema.
    fn recompute_layout_if_vector_fields(&mut self, ts: &TypeSpecifier) {
        let (tag, fields, is_union, is_packed, pragma_pack, struct_aligned, reverse_sso) =
            match ts {
                TypeSpecifier::Struct(
                    tag,
                    Some(fields),
                    is_packed,
                    pragma_pack,
                    struct_aligned,
                    reverse_sso,
                ) => (
                    tag,
                    fields,
                    false,
                    *is_packed,
                    *pragma_pack,
                    *struct_aligned,
                    reverse_sso.is_some_and(|v| v),
                ),
                TypeSpecifier::Union(
                    tag,
                    Some(fields),
                    is_packed,
                    pragma_pack,
                    struct_aligned,
                    reverse_sso,
                ) => (
                    tag,
                    fields,
                    true,
                    *is_packed,
                    *pragma_pack,
                    *struct_aligned,
                    reverse_sso.is_some_and(|v| v),
                ),
                _ => return,
            };
        // Check if any field uses a vector typedef
        let has_vector_field = fields.iter().any(|f| {
            let ctype = self.struct_field_ctype(f);
            ctype.is_vector()
        });
        if !has_vector_field {
            return;
        }

        // Find the existing layout key. For tagged types, use the tag-based key.
        // For anonymous types (e.g., typedef union { ... } name), find the key
        // that sema assigned by searching typedefs for a matching CType.
        let existing_key = if let Some(name) = tag {
            let prefix = if is_union { "union." } else { "struct." };
            Some(format!("{}{}", prefix, name))
        } else {
            let layouts = self.types.borrow_struct_layouts();
            let mut found_key = None;
            for (_, ctype) in self.types.typedefs.iter() {
                match ctype {
                    CType::Struct(key) | CType::Union(key) => {
                        if let Some(layout) = layouts.get(&**key) {
                            if layout.is_union == is_union && layout.fields.len() == fields.len() {
                                found_key = Some(key.to_string());
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            drop(layouts);
            found_key
        };

        if let Some(key) = existing_key {
            let max_field_align = if is_packed { Some(1) } else { pragma_pack };
            let mut layout =
                self.compute_struct_union_layout_packed(
                    fields,
                    is_union,
                    max_field_align,
                    reverse_sso,
                );
            if let Some(a) = struct_aligned {
                if a > layout.align {
                    layout.align = a;
                    let mask = layout.align - 1;
                    layout.size = (layout.size + mask) & !mask;
                }
            }
            self.types.insert_struct_layout_from_ref(&key, layout);
        }
    }

    /// Insert a struct layout into the cache, tracking the change in the current
    /// scope frame so it can be undone on scope exit.
    fn insert_struct_layout_scoped(&mut self, key: String, layout: StructLayout) {
        self.types.insert_struct_layout_scoped(key, layout);
    }

    /// Invalidate a ctype_cache entry for a struct/union tag, tracking the change
    /// in the current scope frame so it can be restored on scope exit.
    fn invalidate_ctype_cache_scoped(&mut self, key: &str) {
        self.types.invalidate_ctype_cache_scoped(key);
    }

    /// Recursively register any struct/union types defined inline in field declarations.
    /// This handles cases like:
    ///   struct Outer { struct Inner { int x; } field; };
    /// where `struct Inner` needs to be registered so it can be referenced later.
    fn register_nested_struct_types(&mut self, fields: &[StructFieldDecl]) {
        for field in fields {
            self.register_nested_in_type_spec(&field.type_spec);
        }
    }

    /// Walk a TypeSpecifier and register any struct/union definitions found within it.
    fn register_nested_in_type_spec(&mut self, ts: &TypeSpecifier) {
        match ts {
            TypeSpecifier::Struct(_tag, Some(_fields), ..) => {
                // This is a struct definition inside a field (named or anonymous) - register it
                self.register_struct_type(ts);
            }
            TypeSpecifier::Union(_tag, Some(_fields), ..) => {
                // This is a union definition inside a field (named or anonymous) - register it
                self.register_struct_type(ts);
            }
            TypeSpecifier::Pointer(inner, _) => {
                // Walk through pointer types to find nested struct defs
                self.register_nested_in_type_spec(inner);
            }
            TypeSpecifier::Array(inner, _) => {
                self.register_nested_in_type_spec(inner);
            }
            _ => {}
        }
    }

    /// Compute a layout key for a struct/union.
    fn struct_layout_key(&mut self, tag: &Option<String>, is_union: bool) -> String {
        let prefix = if is_union { "union." } else { "struct." };
        if let Some(name) = tag {
            format!("{}{}", prefix, name)
        } else {
            let id = self.next_anon_struct;
            self.next_anon_struct += 1;
            format!("{}__anon_{}", prefix, id)
        }
    }

    /// Get the StructLayout key for a union TypeSpecifier.
    /// Returns the layout map key if the type is a union (directly or via typedef).
    pub(super) fn union_layout_key(&self, ts: &TypeSpecifier) -> Option<String> {
        match ts {
            TypeSpecifier::Union(tag, ..) => {
                let prefix = "union.";
                tag.as_ref().map(|name| format!("{}{}", prefix, name))
            }
            TypeSpecifier::TypedefName(name) => {
                if let Some(CType::Union(key)) = self.types.typedefs.get(name) {
                    return Some(key.to_string());
                }
                None
            }
            _ => None,
        }
    }

    /// Mark a transparent_union attribute on the union's StructLayout.
    ///
    /// Resolves the union layout key either from the type specifier directly
    /// or by searching through declarator names for typedef aliases.
    /// This is needed in two places: the pre-pass in `lower()` and `lower_global_decl`.
    pub(super) fn mark_transparent_union(&mut self, decl: &Declaration) {
        let mut found_key = self.union_layout_key(&decl.type_spec);
        if found_key.is_none() {
            for declarator in &decl.declarators {
                if !declarator.name.is_empty() {
                    if let Some(CType::Union(key)) = self.types.typedefs.get(&declarator.name) {
                        found_key = Some(key.to_string());
                        break;
                    }
                }
            }
        }
        if let Some(key) = found_key {
            if let Some(layout) = self.types.borrow_struct_layouts_mut().get_mut(&key) {
                Rc::make_mut(layout).is_transparent_union = true;
            }
        }
    }

    /// Get the cached struct layout for a TypeSpecifier, if it's a struct/union type.
    /// Prefers cached layout from struct_layouts when a tag name is available.
    /// Returns Rc<StructLayout> for cheap cloning.
    pub(super) fn get_struct_layout_for_type(&self, ts: &TypeSpecifier) -> Option<RcLayout> {
        // For TypedefName, resolve through CType
        if let TypeSpecifier::TypedefName(name) = ts {
            if let Some(ctype) = self.types.typedefs.get(name) {
                return self.struct_layout_from_ctype(ctype);
            }
            return None;
        }
        // For typeof(expr), resolve through the expression's CType.
        // This handles patterns like typeof(*ptr) where ptr is a struct pointer,
        // which appear in offsetof patterns: &((typeof(*ptr) *)0)->member
        if let TypeSpecifier::Typeof(expr) = ts {
            if let Some(ctype) = self.get_expr_ctype(expr) {
                return self.struct_layout_from_ctype(&ctype);
            }
            return None;
        }
        let ts = self.resolve_type_spec(ts);
        match ts {
            TypeSpecifier::Struct(
                tag,
                Some(fields),
                is_packed,
                pragma_pack,
                _,
                reverse_sso,
            ) => {
                if let Some(tag) = tag {
                    let layouts = self.types.borrow_struct_layouts();
                    if let Some(layout) = layouts
                        .get(&format!("struct.{}", tag))
                        .or_else(|| layouts.get(tag.as_str()))
                    {
                        return Some(layout.clone());
                    }
                }
                let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                Some(Rc::new(self.compute_struct_union_layout_packed(
                    fields,
                    false,
                    max_field_align,
                    reverse_sso.is_some_and(|v| v),
                )))
            }
            TypeSpecifier::Struct(Some(tag), None, ..) => {
                let layouts = self.types.borrow_struct_layouts();
                layouts
                    .get(&format!("struct.{}", tag))
                    .cloned()
                    .or_else(|| {
                        // Anonymous structs from typeof/ctype_to_type_spec use the
                        // raw CType key (e.g., "__anon_struct_N") as the tag.
                        layouts.get(tag.as_str()).cloned()
                    })
            }
            TypeSpecifier::Union(
                tag,
                Some(fields),
                is_packed,
                pragma_pack,
                _,
                reverse_sso,
            ) => {
                if let Some(tag) = tag {
                    let layouts = self.types.borrow_struct_layouts();
                    if let Some(layout) = layouts
                        .get(&format!("union.{}", tag))
                        .or_else(|| layouts.get(tag.as_str()))
                    {
                        return Some(layout.clone());
                    }
                }
                let max_field_align = if *is_packed { Some(1) } else { *pragma_pack };
                Some(Rc::new(self.compute_struct_union_layout_packed(
                    fields,
                    true,
                    max_field_align,
                    reverse_sso.is_some_and(|v| v),
                )))
            }
            TypeSpecifier::Union(Some(tag), None, ..) => {
                let layouts = self.types.borrow_struct_layouts();
                layouts.get(&format!("union.{}", tag)).cloned().or_else(|| {
                    // Anonymous unions from typeof/ctype_to_type_spec use the
                    // raw CType key (e.g., "__anon_struct_N") as the tag.
                    layouts.get(tag.as_str()).cloned()
                })
            }
            // For typedef'd array types like `typedef S arr_t[4]`, peel the
            // Array wrapper(s) to find the inner struct/union element type.
            TypeSpecifier::Array(inner, _) => self.get_struct_layout_for_type(inner),
            _ => None,
        }
    }

    /// Get the base address of a struct variable (for member access).
    /// For struct locals, the alloca IS the struct base.
    /// For struct globals, we emit GlobalAddr.
    pub(super) fn get_struct_base_addr(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::ConvertVector(_, _, _) => {
                // Lowering returns the 16-byte result alloca pointer.
                let op = self.lower_expr(expr);
                return self.operand_to_value(op);
            }
            Expr::Identifier(name, _) => {
                // Static local variables: resolve via mangled global name
                if let Some(mangled) = self.func_mut().static_local_names.get(name).cloned() {
                    let addr = self.fresh_value();
                    self.emit(Instruction::GlobalAddr {
                        dest: addr,
                        name: mangled,
                    });
                    return addr;
                }
                if let Some(info) = self.func_mut().locals.get(name).cloned() {
                    // Static locals: emit fresh GlobalAddr
                    if let Some(ref global_name) = info.static_global_name {
                        let addr = self.fresh_value();
                        self.emit(Instruction::GlobalAddr {
                            dest: addr,
                            name: global_name.clone(),
                        });
                        return addr;
                    }
                    if info.is_struct {
                        // The alloca is the struct base address
                        return info.alloca;
                    }
                    // Vector types also store data inline at their alloca address
                    if info.c_type.as_ref().is_some_and(|ct| ct.is_vector()) {
                        return info.alloca;
                    }
                    // It's a pointer to struct: load the pointer
                    let loaded = self.fresh_value();
                    self.emit(Instruction::Load {
                        volatile: false,
                        dest: loaded,
                        ptr: info.alloca,
                        ty: IrType::Ptr,
                        seg_override: AddressSpace::Default,
                    });
                    return loaded;
                }
                if self.globals.contains_key(name) {
                    // Apply __asm__("label") redirect (glibc hidden_proto data:
                    // `extern struct opaque_auth _null_auth __asm__("__GI__null_auth")
                    // __attribute__((visibility("hidden")))` — member accesses
                    // like `_null_auth.oa_flavor` must reference __GI__null_auth,
                    // otherwise the object has an unversioned plain reference
                    // that no member defines).
                    let resolved_name = self.resolve_ref_name(name);
                    let addr = self.fresh_value();
                    self.emit(Instruction::GlobalAddr {
                        dest: addr,
                        name: resolved_name,
                    });
                    return addr;
                }
                // Unknown - try to evaluate as expression
                let val = self.lower_expr(expr);
                match val {
                    Operand::Value(v) => v,
                    Operand::Const(_) => {
                        let tmp = self.fresh_value();
                        self.emit(Instruction::Copy {
                            dest: tmp,
                            src: val,
                        });
                        tmp
                    }
                }
            }
            Expr::Deref(inner, _) => {
                // (*ptr).field - evaluate ptr to get base address
                let val = self.lower_expr(inner);
                match val {
                    Operand::Value(v) => v,
                    Operand::Const(_) => {
                        let tmp = self.fresh_value();
                        self.emit(Instruction::Copy {
                            dest: tmp,
                            src: val,
                        });
                        tmp
                    }
                }
            }
            Expr::ArraySubscript(base, index, _) => {
                // array[i].field
                self.compute_array_element_addr(base, index)
            }
            Expr::MemberAccess(inner_base, inner_field, _) => {
                // Nested member access: s.inner.field
                // A field that follows a VLA member has a runtime offset
                // (recorded by compute_vla_field_offsets_for_decl); it must
                // be preferred over the static layout offset, which treats
                // the VLA member as a zero-size flexible array
                // (gcc.c-torture/execute align-nest.c: s[k].mi resolved at
                // static offset 1 instead of the runtime 4n+5/9).
                let inner_offset = match self.get_vla_field_offset(inner_base, inner_field, false) {
                    Some(vo) => vo,
                    None => {
                        let (off, _) = self.resolve_member_access(inner_base, inner_field);
                        let addr = self.get_struct_base_addr(inner_base);
                        let inner_addr = self.fresh_value();
                        self.emit(Instruction::GetElementPtr {
                            dest: inner_addr,
                            base: addr,
                            offset: Operand::Const(IrConst::ptr_int(off as i64)),
                            ty: IrType::Ptr,
                        });
                        return inner_addr;
                    }
                };
                let inner_base_addr = self.get_struct_base_addr(inner_base);
                let inner_addr = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: inner_addr,
                    base: inner_base_addr,
                    offset: Operand::Value(inner_offset),
                    ty: IrType::Ptr,
                });
                inner_addr
            }
            Expr::PointerMemberAccess(inner_base, inner_field, _) => {
                // Nested access via pointer: p->inner.field
                // Load the pointer, compute offset of the inner field, return address
                let ptr_val = self.lower_expr(inner_base);
                let base_addr = match ptr_val {
                    Operand::Value(v) => v,
                    Operand::Const(_) => {
                        let tmp = self.fresh_value();
                        self.emit(Instruction::Copy {
                            dest: tmp,
                            src: ptr_val,
                        });
                        tmp
                    }
                };
                let inner_offset = match self.get_vla_field_offset(inner_base, inner_field, true) {
                    Some(vo) => vo,
                    None => {
                        let (off, _) =
                            self.resolve_pointer_member_access(inner_base, inner_field);
                        let inner_addr = self.fresh_value();
                        self.emit(Instruction::GetElementPtr {
                            dest: inner_addr,
                            base: base_addr,
                            offset: Operand::Const(IrConst::ptr_int(off as i64)),
                            ty: IrType::Ptr,
                        });
                        return inner_addr;
                    }
                };
                let inner_addr = self.fresh_value();
                self.emit(Instruction::GetElementPtr {
                    dest: inner_addr,
                    base: base_addr,
                    offset: Operand::Value(inner_offset),
                    ty: IrType::Ptr,
                });
                inner_addr
            }
            Expr::FunctionCall(func_expr, _, _) => {
                // Function returning a struct: call the function, then store
                // the return value to a temporary alloca so we have an address.
                let struct_size = self.struct_value_size(expr).unwrap_or(8);

                // Check if this is an sret call (struct > 16 bytes with hidden pointer)
                // or a two-register return (9-16 bytes) - both return an alloca address.
                // On i686 (32-bit), ALL struct returns use sret regardless of size.
                let is_32bit = crate::common::types::target_is_32bit();
                let returns_address = if is_32bit {
                    // i686: all struct returns use hidden pointer (sret)
                    true
                } else if let Expr::Identifier(name, _) = func_expr.as_ref() {
                    // Detect function pointer variables: identifiers that are
                    // local/global variables rather than known function names
                    let is_fptr_var = (self.func_mut().locals.contains_key(name)
                        && !self.known_functions.contains(name))
                        || (!self.func_mut().locals.contains_key(name)
                            && self.globals.contains_key(name)
                            && !self.known_functions.contains(name));
                    if is_fptr_var {
                        // Indirect call through variable: use struct size to determine ABI
                        struct_size > 8
                    } else {
                        match self.func_meta.sigs.get(name.as_str()) {
                            Some(s) => s.sret_size.is_some() || s.two_reg_ret_size.is_some(),
                            None => {
                                // No seeded signature (e.g. an SSE/AVX builtin
                                // whose system-header declaration is ISA-gated
                                // out of the build). Classify by the return
                                // type: a struct/vector return > 8 bytes comes
                                // back by address (sret or an intrinsic result
                                // alloca); <= 8 bytes is packed register data.
                                // Without this, _mm256_clmulepi64_epi128 was
                                // misclassified as a small packed return: its
                                // result ADDRESS was spilled as 8 bytes and
                                // the 32-byte init memcpy read through it
                                // (clmul256 regression).
                                let ret_size = self
                                    .types
                                    .func_return_ctypes
                                    .get(name.as_str())
                                    .filter(|c| c.is_struct_or_union() || c.is_vector())
                                    .map(|c| self.ctype_size(c))
                                    .unwrap_or(struct_size);
                                ret_size > 8
                            }
                        }
                    }
                } else {
                    // Indirect call through expression: determine from return type
                    struct_size > 8
                };

                if returns_address {
                    // For sret and two-register calls, lower_expr returns the alloca
                    // address directly (the struct data is already there)
                    let val = self.lower_expr(expr);
                    match val {
                        Operand::Value(v) => v,
                        Operand::Const(_) => {
                            let tmp = self.fresh_value();
                            self.emit(Instruction::Copy {
                                dest: tmp,
                                src: val,
                            });
                            tmp
                        }
                    }
                } else {
                    // Small struct (<= 8 bytes): return value in rax IS
                    // the packed struct data, not an address.
                    let val = self.lower_expr(expr);
                    let alloca = self.fresh_value();
                    let alloc_size = if struct_size > 0 { struct_size } else { 8 };
                    let store_ty = Self::packed_store_type(alloc_size);
                    self.emit(Instruction::Alloca {
                        dest: alloca,
                        size: alloc_size,
                        ty: store_ty,
                        align: 0,
                        volatile: false,
                        semantic_volatile: false,
                    });
                    self.emit(Instruction::Store {
                        volatile: false,
                        val,
                        ptr: alloca,
                        ty: store_ty,
                        seg_override: AddressSpace::Default,
                    });
                    alloca
                }
            }
            // Ternary and assignment on struct values produce rvalues (temporaries).
            // We must copy the struct data to a fresh temporary so that subsequent
            // member writes don't modify the original objects.
            Expr::Conditional(_, _, _, _)
            | Expr::GnuConditional(_, _, _)
            | Expr::Assign(_, _, _) => {
                if let Some(struct_size) = self.struct_value_size(expr) {
                    if self.expr_produces_packed_struct_data(expr) {
                        // Small struct packed in a register: spill to alloca
                        let val = self.lower_expr(expr);
                        let alloca = self.fresh_value();
                        let alloc_size = if struct_size > 0 { struct_size } else { 8 };
                        let store_ty = Self::packed_store_type(alloc_size);
                        self.emit(Instruction::Alloca {
                            dest: alloca,
                            size: alloc_size,
                            ty: store_ty,
                            align: 0,
                            volatile: false,
                            semantic_volatile: false,
                        });
                        self.emit(Instruction::Store {
                            volatile: false,
                            val,
                            ptr: alloca,
                            ty: store_ty,
                            seg_override: AddressSpace::Default,
                        });
                        alloca
                    } else {
                        // Struct returned by address: copy to a fresh temporary
                        let src_val = self.lower_expr(expr);
                        let src_addr = self.operand_to_value(src_val);
                        let tmp_alloca = self.fresh_value();
                        self.emit(Instruction::Alloca {
                            dest: tmp_alloca,
                            size: struct_size,
                            ty: IrType::Ptr,
                            align: 0,
                            volatile: false,
                            semantic_volatile: false,
                        });
                        self.emit(Instruction::Memcpy {
                            dest: tmp_alloca,
                            src: src_addr,
                            size: struct_size,
                        });
                        tmp_alloca
                    }
                } else {
                    // Not a struct type - evaluate normally
                    let val = self.lower_expr(expr);
                    match val {
                        Operand::Value(v) => v,
                        Operand::Const(_) => {
                            let tmp = self.fresh_value();
                            self.emit(Instruction::Copy {
                                dest: tmp,
                                src: val,
                            });
                            tmp
                        }
                    }
                }
            }
            _ => {
                // For expressions that might produce packed struct data (e.g. ternary
                // with struct-returning function calls), detect and spill to an alloca.
                if self.expr_produces_packed_struct_data(expr) {
                    let struct_size = self.struct_value_size(expr).unwrap_or(8);
                    let val = self.lower_expr(expr);
                    let alloca = self.fresh_value();
                    let alloc_size = if struct_size > 0 { struct_size } else { 8 };
                    let store_ty = Self::packed_store_type(alloc_size);
                    self.emit(Instruction::Alloca {
                        dest: alloca,
                        size: alloc_size,
                        ty: store_ty,
                        align: 0,
                        volatile: false,
                        semantic_volatile: false,
                    });
                    self.emit(Instruction::Store {
                        volatile: false,
                        val,
                        ptr: alloca,
                        ty: store_ty,
                        seg_override: AddressSpace::Default,
                    });
                    alloca
                } else {
                    let val = self.lower_expr(expr);
                    match val {
                        Operand::Value(v) => v,
                        Operand::Const(_) => {
                            let tmp = self.fresh_value();
                            self.emit(Instruction::Copy {
                                dest: tmp,
                                src: val,
                            });
                            tmp
                        }
                    }
                }
            }
        }
    }

    /// Check if an expression produces packed struct data (non-address value)
    /// rather than a pointer to struct data. This happens for small structs
    /// (<= 8 bytes) returned from non-sret, non-two-reg function calls, either
    /// directly or through ternary/comma expressions.
    pub(super) fn expr_produces_packed_struct_data(&self, expr: &Expr) -> bool {
        match expr {
            Expr::FunctionCall(func_expr, _, _) => {
                // On i686 (32-bit), ALL struct returns use sret, never packed data
                if crate::common::types::target_is_32bit() {
                    return false;
                }
                let struct_size = self.struct_value_size(expr).unwrap_or(8);
                if struct_size > 8 {
                    // 9+ byte structs: sret or two-reg return, both produce addresses
                    return false;
                }
                // Small struct (<= 8 bytes): produces packed data unless somehow sret
                if let Expr::Identifier(name, _) = func_expr.as_ref() {
                    self.func_meta
                        .sigs
                        .get(name.as_str())
                        .is_none_or(|s| s.sret_size.is_none() && s.two_reg_ret_size.is_none())
                } else {
                    true
                }
            }
            Expr::Conditional(_, then_expr, else_expr, _) => {
                // If either branch produces packed struct data, the ternary does too
                self.expr_produces_packed_struct_data(then_expr)
                    || self.expr_produces_packed_struct_data(else_expr)
            }
            Expr::GnuConditional(cond, else_expr, _) => {
                self.expr_produces_packed_struct_data(cond)
                    || self.expr_produces_packed_struct_data(else_expr)
            }
            Expr::Comma(_, last, _) => self.expr_produces_packed_struct_data(last),
            Expr::StmtExpr(compound, _) => {
                // Statement expression: check if the last expression statement
                // produces packed struct data (e.g., ({ pfn_pte(...); }))
                if let Some(crate::frontend::parser::ast::BlockItem::Statement(
                    crate::frontend::parser::ast::Stmt::Expr(Some(inner_expr)),
                )) = compound.items.last()
                {
                    return self.expr_produces_packed_struct_data(inner_expr);
                }
                false
            }
            _ => false,
        }
    }

    /// Look up the struct layout for a member access expression.
    /// For direct access (s.field), gets the layout of the base expression.
    /// For pointer access (p->field), gets the layout that the pointer points to.
    fn get_member_layout(&self, base_expr: &Expr, is_pointer_access: bool) -> Option<RcLayout> {
        if is_pointer_access {
            self.get_pointed_struct_layout(base_expr)
        } else {
            self.get_layout_for_expr(base_expr)
        }
    }

    /// Resolve member access: returns (byte_offset, ir_type_of_field).
    /// Works for both direct (s.field) and pointer (p->field) access.
    pub(super) fn resolve_member_access(
        &self,
        base_expr: &Expr,
        field_name: &str,
    ) -> (usize, IrType) {
        self.resolve_member_access_impl(base_expr, field_name, false)
    }

    /// Look up the reverse-SSO handling mode of a member access.
    /// Returns SsoMode::None when the record is native-order or the member is
    /// not found (the overwhelmingly common case, at zero layout cost).
    pub(super) fn sso_mode_of_member(
        &self,
        base_expr: &Expr,
        is_pointer_access: bool,
        field_name: &str,
    ) -> SsoMode {
        self.sso_storage_of_member(base_expr, is_pointer_access, field_name).0
    }

    /// Look up the reverse-SSO mode AND the unit-wide access storage type
    /// (for standard bitfields sharing a multi-byte unit under reverse SSO).
    pub(super) fn sso_storage_of_member(
        &self,
        base_expr: &Expr,
        is_pointer_access: bool,
        field_name: &str,
    ) -> (SsoMode, Option<CType>) {
        if let Some(layout) = self.get_member_layout(base_expr, is_pointer_access) {
            if !layout.reverse_sso {
                return (SsoMode::None, None);
            }
            for f in &layout.fields {
                if f.name == field_name {
                    return (f.sso, f.sso_storage_ty.clone());
                }
            }
        }
        (SsoMode::None, None)
    }

    /// Resolve pointer member access (p->field): returns (byte_offset, ir_type_of_field).
    pub(super) fn resolve_pointer_member_access(
        &self,
        base_expr: &Expr,
        field_name: &str,
    ) -> (usize, IrType) {
        self.resolve_member_access_impl(base_expr, field_name, true)
    }

    fn resolve_member_access_impl(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer: bool,
    ) -> (usize, IrType) {
        if let Some(layout) = self.get_member_layout(base_expr, is_pointer) {
            if let Some((offset, ctype)) =
                layout.field_offset(field_name, &*self.types.borrow_struct_layouts())
            {
                return (offset, IrType::from_ctype(&ctype));
            }
        }
        (0, IrType::I32)
    }

    /// Resolve member access with full bitfield info.
    /// Returns (byte_offset, ir_type, Option<(bit_offset, bit_width)>).
    pub(super) fn resolve_member_access_full(
        &self,
        base_expr: &Expr,
        field_name: &str,
    ) -> (usize, IrType, Option<(u32, u32)>) {
        self.resolve_member_access_full_impl(base_expr, field_name, false)
    }

    /// Resolve pointer member access with full bitfield info.
    pub(super) fn resolve_pointer_member_access_full(
        &self,
        base_expr: &Expr,
        field_name: &str,
    ) -> (usize, IrType, Option<(u32, u32)>) {
        self.resolve_member_access_full_impl(base_expr, field_name, true)
    }

    fn resolve_member_access_full_impl(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer: bool,
    ) -> (usize, IrType, Option<(u32, u32)>) {
        if let Some(layout) = self.get_member_layout(base_expr, is_pointer) {
            if let Some(fl) = layout.field_layout(field_name) {
                let ir_ty = IrType::from_ctype(&fl.ty);
                let bf = match (fl.bit_offset, fl.bit_width) {
                    (Some(bo), Some(bw)) => Some((bo, bw)),
                    _ => None,
                };
                return (fl.offset, ir_ty, bf);
            }
            // Fallback: search anonymous struct/union members recursively,
            // preserving bitfield metadata (bit_offset, bit_width).
            if let Some((offset, ctype, bit_offset, bit_width)) =
                layout.field_offset_with_bitfield(field_name, &*self.types.borrow_struct_layouts())
            {
                let bf = match (bit_offset, bit_width) {
                    (Some(bo), Some(bw)) => Some((bo, bw)),
                    _ => None,
                };
                return (offset, IrType::from_ctype(&ctype), bf);
            }
        }
        (0, IrType::I32, None)
    }

    /// Try to determine the struct layout that an expression (a pointer) points to.
    fn get_pointed_struct_layout(&self, expr: &Expr) -> Option<RcLayout> {
        match expr {
            Expr::Identifier(name, _) => {
                // Check static locals first (only in function context)
                if let Some(ref fs) = self.func_state {
                    if let Some(mangled) = fs.static_local_names.get(name) {
                        if let Some(ginfo) = self.globals.get(mangled) {
                            if ginfo.struct_layout.is_some() {
                                return ginfo.struct_layout.clone();
                            }
                        }
                    }
                }
                // Check if this variable has struct layout info (pointer to struct).
                // Prefer live lookup from struct_layouts over the VarInfo snapshot,
                // because the snapshot may be stale when a struct tag is redefined
                // in the current scope (the VarInfo was captured at declaration time
                // before the full struct definition was processed).
                if let Some(vi) = self.lookup_var_info(name) {
                    // First try live lookup via c_type -> struct_layouts map
                    if let Some(ref ct) = vi.c_type {
                        if let Some(layout) = self.resolve_struct_from_pointer_ctype(ct) {
                            return Some(layout);
                        }
                    }
                    // Fall back to the snapshot captured at declaration time
                    if let Some(ref layout) = vi.struct_layout {
                        return Some(layout.clone());
                    }
                }
                // Fallback: resolve struct layout from the variable's CType
                // (handles forward-declared struct pointers where layout was None at decl time,
                // and array typedef parameters like `typedef struct S arr[1]`)
                match self.get_expr_ctype(expr).as_ref() {
                    Some(CType::Pointer(pointee, _)) | Some(CType::Array(pointee, _)) => {
                        return self.struct_layout_from_ctype(pointee);
                    }
                    _ => {}
                }
                None
            }
            Expr::MemberAccess(base, field, _) | Expr::PointerMemberAccess(base, field, _) => {
                let is_ptr = matches!(expr, Expr::PointerMemberAccess(..));
                self.resolve_field_struct_layout(base, field, is_ptr, true)
            }
            Expr::UnaryOp(UnaryOp::PreInc | UnaryOp::PreDec, inner, _) => {
                self.get_pointed_struct_layout(inner)
            }
            Expr::PostfixOp(_, inner, _) => self.get_pointed_struct_layout(inner),
            Expr::BinaryOp(op, lhs, rhs, _) if matches!(op, BinOp::Add | BinOp::Sub) => {
                // Pointer arithmetic: (p + i) or (p - i) preserves the pointed-to struct type
                if let Some(layout) = self.get_pointed_struct_layout(lhs) {
                    return Some(layout);
                }
                // Try rhs for commutative case: (i + p)
                if matches!(op, BinOp::Add) {
                    if let Some(layout) = self.get_pointed_struct_layout(rhs) {
                        return Some(layout);
                    }
                }
                None
            }
            Expr::Cast(type_spec, inner, _) => {
                // Cast to struct pointer type
                if let Some(layout) = self.get_struct_layout_for_pointer_type(type_spec) {
                    return Some(layout);
                }
                // Try inner expression
                self.get_pointed_struct_layout(inner)
            }
            Expr::AddressOf(inner, _) => {
                // &expr - result is a pointer to expr's type
                self.get_layout_for_expr(inner)
            }
            Expr::Deref(inner, _) => {
                // *pp where pp is a pointer to pointer to struct
                // or *pa where pa is a pointer to array of struct (array decays to struct pointer)
                if let Some(CType::Pointer(inner_ct, _)) = self.get_expr_ctype(inner).as_ref() {
                    if let CType::Pointer(pointee, _) = inner_ct.as_ref() {
                        return self.struct_layout_from_ctype(pointee);
                    }
                    // Pointer to array of struct: *pa dereferences to the array,
                    // which decays to a pointer to the element struct type.
                    // This handles patterns like cpumask_var_t (typedef struct cpumask[1])
                    // where &var is pointer-to-array and *&var decays to struct pointer.
                    if let CType::Array(elem, _) = inner_ct.as_ref() {
                        return self.struct_layout_from_ctype(elem);
                    }
                }
                // Fallback: propagate through inner
                self.get_pointed_struct_layout(inner)
            }
            Expr::ArraySubscript(base, _, _) => {
                // pp[i] where pp is an array of struct pointers
                if let Some(CType::Array(elem, _) | CType::Pointer(elem, _)) =
                    self.get_expr_ctype(base).as_ref()
                {
                    if let CType::Pointer(pointee, _) = elem.as_ref() {
                        return self.struct_layout_from_ctype(pointee);
                    }
                }
                // Fallback: try base directly
                self.get_pointed_struct_layout(base)
            }
            Expr::FunctionCall(func, _, _) => {
                self.resolve_func_call_struct_layout(func, expr, true)
            }
            Expr::Conditional(_, then_expr, else_expr, _) => {
                // Try both branches: one may be a typed struct pointer and
                // the other may be (void*)0 (e.g., ql_last macro pattern).
                self.get_pointed_struct_layout(then_expr)
                    .or_else(|| self.get_pointed_struct_layout(else_expr))
            }
            Expr::GnuConditional(cond, else_expr, _) => self
                .get_pointed_struct_layout(cond)
                .or_else(|| self.get_pointed_struct_layout(else_expr)),
            Expr::Comma(_, last, _) => self.get_pointed_struct_layout(last),
            Expr::StmtExpr(..) => {
                // Statement expression: use CType resolution to find the
                // pointed-to struct type, since inner variables may not be
                // in scope yet during layout lookup.
                if let Some(CType::Pointer(pointee, _)) = self.get_expr_ctype(expr).as_ref() {
                    return self.struct_layout_from_ctype(pointee);
                }
                None
            }
            Expr::Assign(lhs, rhs, _) | Expr::CompoundAssign(_, lhs, rhs, _) => {
                // For assignment, the result has the LHS type; try LHS first, then RHS
                if let Some(layout) = self.get_pointed_struct_layout(lhs) {
                    return Some(layout);
                }
                self.get_pointed_struct_layout(rhs)
            }
            _ => {
                // Generic fallback: try CType-based resolution
                if let Some(ctype) = self.get_expr_ctype(expr) {
                    if let CType::Pointer(pointee, _) = &ctype {
                        return self.struct_layout_from_ctype(pointee);
                    }
                    // Array of struct decays to struct pointer (e.g., after deref of
                    // pointer-to-array-of-struct, as in typedef struct S arr_t[1] patterns).
                    if let CType::Array(elem, _) = &ctype {
                        return self.struct_layout_from_ctype(elem);
                    }
                }
                None
            }
        }
    }

    /// Get struct layout from a CType (struct or union).
    /// Prefers cached layout from struct_layouts when available.
    pub(super) fn struct_layout_from_ctype(&self, ctype: &CType) -> Option<RcLayout> {
        match ctype {
            CType::Struct(key) | CType::Union(key) => self
                .types
                .borrow_struct_layouts()
                .get(key.as_ref())
                .cloned(),
            _ => None,
        }
    }

    /// Get struct layout from a type specifier that should be a pointer to struct
    fn get_struct_layout_for_pointer_type(&self, type_spec: &TypeSpecifier) -> Option<RcLayout> {
        let ctype = self.type_spec_to_ctype(type_spec);
        if let CType::Pointer(pointee, _) = &ctype {
            return self.struct_layout_from_ctype(pointee);
        }
        None
    }

    /// Try to get the struct layout for an expression (value, not pointer).
    fn get_layout_for_expr(&self, expr: &Expr) -> Option<RcLayout> {
        match expr {
            Expr::Identifier(name, _) => {
                // Check static locals first (resolved via mangled name)
                // Use func_state directly to avoid panic when called from global initializer context
                if let Some(ref fs) = self.func_state {
                    if let Some(mangled) = fs.static_local_names.get(name) {
                        if let Some(ginfo) = self.globals.get(mangled) {
                            return ginfo.struct_layout.clone();
                        }
                    }
                }
                if let Some(vi) = self.lookup_var_info(name) {
                    return vi.struct_layout.clone();
                }
                None
            }
            Expr::MemberAccess(base, field, _) | Expr::PointerMemberAccess(base, field, _) => {
                let is_ptr = matches!(expr, Expr::PointerMemberAccess(..));
                self.resolve_field_struct_layout(base, field, is_ptr, false)
            }
            Expr::ArraySubscript(base, _, _) => {
                // array[i] where array is of struct type
                // First try getting the layout directly from base
                if let Some(layout) = self.get_layout_for_expr(base) {
                    return Some(layout);
                }
                // If base is a member access yielding an array of structs,
                // use CType to resolve the element type
                if let Some(base_ctype) = self.get_expr_ctype(base) {
                    let inner = match &base_ctype {
                        CType::Array(elem, _) => Some(elem.as_ref()),
                        CType::Pointer(pointee, _) => Some(pointee.as_ref()),
                        _ => None,
                    };
                    if let Some(inner_ct) = inner {
                        if let Some(layout) = self.struct_layout_from_ctype(inner_ct) {
                            return Some(layout);
                        }
                    }
                }
                // Also try: base is a pointer to struct/union (for ptr[i] patterns)
                self.get_pointed_struct_layout(base)
            }
            Expr::CompoundLiteral(type_spec, _, _) => {
                // Compound literal: (struct tag){...} - get layout from the type specifier
                self.get_struct_layout_for_type(type_spec)
            }
            Expr::Deref(inner, _) => {
                // (*ptr) where ptr points to a struct
                self.get_pointed_struct_layout(inner)
            }
            Expr::Cast(type_spec, inner, _) => {
                // Cast to struct type, or cast wrapping a compound literal
                self.get_struct_layout_for_type(type_spec)
                    .or_else(|| self.get_layout_for_expr(inner))
            }
            Expr::FunctionCall(func, _, _) => {
                self.resolve_func_call_struct_layout(func, expr, false)
            }
            Expr::Assign(lhs, _, _) | Expr::CompoundAssign(_, lhs, _, _) => {
                self.get_layout_for_expr(lhs)
            }
            Expr::Conditional(_, then_expr, _, _) => self.get_layout_for_expr(then_expr),
            Expr::GnuConditional(cond, _, _) => self.get_layout_for_expr(cond),
            Expr::Comma(_, last, _) => self.get_layout_for_expr(last),
            Expr::StmtExpr(..) => {
                // Statement expression: use CType resolution (which handles
                // inner scopes via sema) to find the struct type, since the
                // inner variables may not be in scope yet during layout lookup.
                if let Some(ctype) = self.get_expr_ctype(expr) {
                    return self.struct_layout_from_ctype(&ctype);
                }
                None
            }
            Expr::VaArg(_, type_spec, _) => self.get_struct_layout_for_type(type_spec),
            _ => None,
        }
    }

    /// Shared helper: given a field access (base.field or base->field), resolve the
    /// struct layout of the field's type. Used by both get_pointed_struct_layout and
    /// get_layout_for_expr for their MemberAccess/PointerMemberAccess arms.
    ///
    /// `is_pointer_base`: whether the base is accessed via pointer (->)
    /// `want_pointer_deref`: whether the caller wants the field treated as a pointer
    ///   (true for get_pointed_struct_layout: field is a pointer to struct, resolve its pointee)
    ///   (false for get_layout_for_expr: field is a struct, resolve its own layout)
    fn resolve_field_struct_layout(
        &self,
        base: &Expr,
        field: &str,
        is_pointer_base: bool,
        want_pointer_deref: bool,
    ) -> Option<RcLayout> {
        let base_layout = self.get_member_layout(base, is_pointer_base)?;
        let (_offset, ctype) =
            base_layout.field_offset(field, &*self.types.borrow_struct_layouts())?;
        if want_pointer_deref {
            // Caller wants what the field points to (get_pointed_struct_layout path)
            if let Some(layout) = self.resolve_struct_from_pointer_ctype(&ctype) {
                return Some(layout);
            }
        } else {
            // Caller wants the field's own layout (get_layout_for_expr path)
            if let Some(layout) = self.struct_layout_from_ctype(&ctype) {
                return Some(layout);
            }
        }
        // Both paths: if the field is an array of structs, resolve the element type
        if let CType::Array(ref elem, _) = ctype {
            return self.struct_layout_from_ctype(elem);
        }
        None
    }

    /// Shared helper: resolve struct layout from a function call expression.
    /// For get_pointed_struct_layout: the return type is a pointer to struct, resolve its pointee.
    /// For get_layout_for_expr: the return type is a struct, resolve its layout.
    fn resolve_func_call_struct_layout(
        &self,
        func: &Expr,
        call_expr: &Expr,
        want_pointer_deref: bool,
    ) -> Option<RcLayout> {
        // Try direct function name first
        if let Expr::Identifier(name, _) = func {
            if let Some(ctype) = self
                .func_meta
                .sigs
                .get(name.as_str())
                .and_then(|s| s.return_ctype.as_ref())
            {
                if want_pointer_deref {
                    if let CType::Pointer(pointee, _) = ctype {
                        return self.struct_layout_from_ctype(pointee);
                    }
                } else if let Some(layout) = self.struct_layout_from_ctype(ctype) {
                    return Some(layout);
                }
            }
        }
        // For indirect calls through function pointers, resolve from CType
        if let Some(ctype) = self.get_expr_ctype(call_expr) {
            if want_pointer_deref {
                if let CType::Pointer(pointee, _) = &ctype {
                    return self.struct_layout_from_ctype(pointee);
                }
            } else {
                return self.struct_layout_from_ctype(&ctype);
            }
        }
        None
    }

    /// Resolve member access to get the CType of the field (not just the IrType).
    /// This is needed to check if a field is an array type (for array decay behavior).
    /// Handles both direct (s.field) and pointer (p->field) access.
    pub(super) fn resolve_member_field_ctype_impl(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer: bool,
    ) -> Option<CType> {
        if let Some(layout) = self.get_member_layout(base_expr, is_pointer) {
            if let Some((_offset, ctype)) =
                layout.field_offset(field_name, &*self.types.borrow_struct_layouts())
            {
                return Some(ctype.clone());
            }
        }
        None
    }

    /// Resolve member access and return full info including CType.
    /// Returns (byte_offset, ir_type, bitfield_info, field_ctype).
    pub(super) fn resolve_member_access_with_ctype(
        &self,
        base_expr: &Expr,
        field_name: &str,
    ) -> (usize, IrType, Option<(u32, u32)>, Option<CType>) {
        self.resolve_member_access_with_ctype_impl(base_expr, field_name, false)
    }

    /// Resolve pointer member access and return full info including CType.
    /// Returns (byte_offset, ir_type, bitfield_info, field_ctype).
    pub(super) fn resolve_pointer_member_access_with_ctype(
        &self,
        base_expr: &Expr,
        field_name: &str,
    ) -> (usize, IrType, Option<(u32, u32)>, Option<CType>) {
        self.resolve_member_access_with_ctype_impl(base_expr, field_name, true)
    }

    fn resolve_member_access_with_ctype_impl(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer: bool,
    ) -> (usize, IrType, Option<(u32, u32)>, Option<CType>) {
        if let Some(layout) = self.get_member_layout(base_expr, is_pointer) {
            if let Some(fl) = layout.field_layout(field_name) {
                let ir_ty = IrType::from_ctype(&fl.ty);
                let bf = match (fl.bit_offset, fl.bit_width) {
                    (Some(bo), Some(bw)) => Some((bo, bw)),
                    _ => None,
                };
                return (fl.offset, ir_ty, bf, Some(fl.ty.clone()));
            }
            // Search anonymous struct/union members recursively,
            // preserving bitfield metadata (bit_offset, bit_width).
            if let Some((offset, ctype, bit_offset, bit_width)) =
                layout.field_offset_with_bitfield(field_name, &*self.types.borrow_struct_layouts())
            {
                let bf = match (bit_offset, bit_width) {
                    (Some(bo), Some(bw)) => Some((bo, bw)),
                    _ => None,
                };
                return (offset, IrType::from_ctype(&ctype), bf, Some(ctype.clone()));
            }
        }
        (0, IrType::I32, None, None)
    }

    pub(super) fn get_vla_field_offset(
        &self,
        base_expr: &Expr,
        field_name: &str,
        is_pointer: bool,
    ) -> Option<Value> {
        let fs = self.func_state.as_ref()?;
        let ctype = self.get_expr_ctype(base_expr)?;
        let mut struct_key = None;
        if is_pointer {
            match &ctype {
                CType::Pointer(inner, _) | CType::Array(inner, _) => match &**inner {
                    CType::Struct(k) | CType::Union(k) => struct_key = Some(k.to_string()),
                    _ => {}
                },
                _ => {}
            }
        } else {
            match &ctype {
                CType::Struct(k) | CType::Union(k) => struct_key = Some(k.to_string()),
                _ => {}
            }
        }
        let k = struct_key?;
        let field_key = format!("{k}::{field_name}");
        fs.vla_field_offsets.get(&field_key).copied()
    }

    /// Given a CType that should be a Pointer to a struct, resolve the struct layout.
    /// Handles self-referential structs by looking up the cache when fields are empty.
    /// Also handles array types (e.g., `typedef struct S my_arr[1]` parameters decay
    /// to pointers, so the element type is the pointed-to struct).
    fn resolve_struct_from_pointer_ctype(&self, ctype: &CType) -> Option<RcLayout> {
        match ctype {
            CType::Pointer(inner, _) | CType::Array(inner, _) => {
                self.struct_layout_from_ctype(inner)
            }
            _ => None,
        }
    }

    pub(super) fn compute_vla_field_strides_for_decl(
        &mut self,
        struct_key: &str,
        fields: &[StructFieldDecl],
    ) {
        use crate::frontend::parser::ast::DerivedDeclarator;
        use crate::ir::ops::IrBinOp;
        let ptr_int_ty = crate::common::types::target_int_ir_type();
        for f in fields {
            let Some(ref field_name) = f.name else { continue };
            let mut dims: Vec<Option<Expr>> = Vec::new();
            let mut curr_ts = &f.type_spec;
            while let TypeSpecifier::Array(inner, dim) = curr_ts {
                dims.push(dim.as_deref().cloned());
                curr_ts = inner.as_ref();
            }
            for d in &f.derived {
                if let DerivedDeclarator::Array(dim) = d {
                    dims.push(dim.as_deref().cloned());
                }
            }
            let mut inner_strides: Vec<Value> = Vec::new();
            let mut elem_size_val: Option<Value> = None;
            if let TypeSpecifier::TypedefName(tname) = curr_ts {
                if let Some(fs) = self.func_state.as_ref() {
                    if let Some(&sz) = fs.vla_typedef_sizes.get(tname) {
                        elem_size_val = Some(sz);
                    }
                    if let Some(istrides) = fs.vla_field_strides.get(tname) {
                        inner_strides = istrides.clone();
                    }
                }
            }
            let elem_size = self.sizeof_type(curr_ts);
            let mut dim_ops: Vec<Option<Operand>> = Vec::new();
            let mut has_vla = elem_size_val.is_some() || !inner_strides.is_empty();
            for dim in &dims {
                if let Some(dim_node) = dim {
                    if let Some(c) = self.expr_as_array_size(dim_node) {
                        dim_ops.push(Some(Operand::Const(IrConst::ptr_int(c as i64))));
                    } else {
                        let op = self.lower_expr(dim_node);
                        dim_ops.push(Some(op));
                        has_vla = true;
                    }
                } else {
                    dim_ops.push(None);
                }
            }
            if !has_vla {
                continue;
            }
            let n = dim_ops.len();
            let mut strides: Vec<Value> = Vec::with_capacity(n + inner_strides.len());
            for dim_idx in 0..n {
                let mut stride_op = if let Some(esz) = elem_size_val {
                    Operand::Value(esz)
                } else {
                    Operand::Const(IrConst::ptr_int(elem_size as i64))
                };
                for dv in dim_ops.iter().skip(dim_idx + 1) {
                    if let Some(d) = dv {
                        let extended = self.emit_implicit_cast(
                            d.clone(),
                            IrType::I32,
                            ptr_int_ty,
                        );
                        let mul = self.emit_binop_val(
                            IrBinOp::Mul,
                            stride_op,
                            extended,
                            ptr_int_ty,
                        );
                        stride_op = Operand::Value(mul);
                    }
                }
                let stride_val = self.operand_to_value(stride_op);
                strides.push(stride_val);
            }
            if !inner_strides.is_empty() {
                strides.extend(inner_strides);
            } else if elem_size_val.is_some() {
                let cty = self.type_spec_to_ctype(curr_ts);
                let elem_sz = match &cty {
                    CType::Array(inner, _) | CType::Pointer(inner, _) => inner.size(),
                    _ => 4,
                };
                let c_op = Operand::Const(IrConst::ptr_int(elem_sz as i64));
                let c_val = self.operand_to_value(c_op);
                strides.push(c_val);
            }
            let field_key = format!("{struct_key}::{field_name}");
            self.func_mut()
                .insert_vla_field_strides_scoped(field_key, strides);
        }
    }

    pub(super) fn compute_vla_field_offsets_for_decl(
        &mut self,
        struct_key: &str,
        fields: &[StructFieldDecl],
        is_packed: bool,
        pragma_pack: Option<usize>,
        layout: &crate::common::types::StructLayout,
    ) {
        use crate::common::types::StructFieldLayout;

        // Locate the first VLA member. Without one there are no dynamic
        // offsets to record.
        let mut vla_idx = None;
        for (idx, f) in fields.iter().enumerate() {
            if f.bit_width.is_some() {
                continue;
            }
            if self.field_vla_runtime_size_probe(f) {
                vla_idx = Some(idx);
                break;
            }
        }
        let Some(vla_idx) = vla_idx else { return };

        // Seed the dynamic cursor with the VLA member's STATIC offset: every
        // field before it is fixed-size, so the static layout already knows
        // its exact offset — including any bitfields that precede the VLA
        // member. (The previous implementation recomputed the pre-VLA prefix
        // by hand and skipped bitfields, so `struct { unsigned b:3; int
        // i[n]; ... }` started the dynamic walk at 0 instead of 4.)
        let vla_member_static_offset = fields[..vla_idx]
            .iter()
            .filter_map(|f| {
                let name = f.name.as_deref()?;
                layout
                    .fields
                    .iter()
                    .find(|fl: &&StructFieldLayout| fl.name == name)
                    .map(|fl| fl.offset)
            })
            .next()
            .unwrap_or(0);

        let mut cursor = VlaDynCursor::new(vla_member_static_offset, is_packed);

        for f in &fields[vla_idx..] {
            let bit_width = f
                .bit_width
                .as_ref()
                .and_then(|bw| self.eval_const_expr(bw).and_then(|c| c.to_u32()));
            if let Some(bw) = bit_width {
                // Bitfield: advance the bit cursor. Offsets of bitfields are
                // not recorded (member access to a bitfield that follows a
                // VLA member keeps the static path — no test exercises it).
                let ct = self.struct_field_ctype(f);
                if is_packed {
                    self.vla_cursor_bitfield_packed(&mut cursor, bw);
                } else {
                    let unit_size = self.ctype_size(&ct);
                    let unit_align = self.ctype_align(&ct).max(1);
                    if unit_size == 0 {
                        continue;
                    }
                    if bw == 0 {
                        // Zero-width: next field at the type's unit boundary.
                        self.vla_cursor_close_run_for_field(&mut cursor);
                        self.vla_cursor_align(&mut cursor, unit_align);
                    } else if unit_size <= cursor.guarantee {
                        // Cursor ≡ 0 (mod unit_size) is statically known, so
                        // the SysV straddle test is decidable.
                        self.vla_cursor_bitfield_unit(&mut cursor, bw, unit_size);
                    } else {
                        // Straddle undecidable at compile time: fall back to
                        // one fresh aligned unit per bitfield (self-consistent,
                        // overlap-free).
                        self.vla_cursor_close_run_for_field(&mut cursor);
                        self.vla_cursor_align(&mut cursor, unit_align);
                        self.vla_cursor_add_fixed(&mut cursor, unit_size);
                    }
                }
                continue;
            }
            let ct = self.struct_field_ctype(f);
            let align = self.vla_field_effective_align(f, &ct, is_packed, pragma_pack);
            // Close any open bitfield run and align the cursor to the field.
            self.vla_cursor_close_run_for_field(&mut cursor);
            self.vla_cursor_align(&mut cursor, align);

            // Named field following the VLA member: record its dynamic
            // offset so member access uses it instead of the static layout
            // offset (which treats the VLA member as a zero-size flexible
            // array). This includes subsequent VLA members themselves
            // (pr82210.c: `struct S { T a[size]; int b[size]; }` — b's
            // runtime offset is align_up(a_bytes, alignof(int))).
            if let Some(ref name) = f.name {
                if let Some(off_val) = self.vla_cursor_offset_value(&cursor) {
                    let field_key = format!("{struct_key}::{name}");
                    self.func_mut()
                        .insert_vla_field_offset_scoped(field_key, off_val);
                }
            }
            if let Some(vla_sz) = self.field_vla_runtime_size(f) {
                self.vla_cursor_add_vla(&mut cursor, vla_sz);
                continue;
            }
            let sz = self.ctype_size(&ct);
            self.vla_cursor_add_fixed(&mut cursor, sz);
        }
    }

    /// True when `field` is (or contains) a VLA whose byte size is only
    /// known at runtime. Pure probe: emits no IR.
    pub(super) fn field_vla_runtime_size_probe(&self, field: &StructFieldDecl) -> bool {
        if !field.derived.is_empty()
            && self.probe_vla_runtime_size(&field.type_spec, &field.derived)
        {
            return true;
        }
        self.probe_vla_size_from_type_spec(&field.type_spec)
    }

    fn probe_vla_runtime_size(
        &self,
        type_spec: &TypeSpecifier,
        derived: &[DerivedDeclarator],
    ) -> bool {
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
            return self.probe_vla_size_from_type_spec(type_spec);
        }
        // Non-constant dimension → VLA.
        for expr in array_dims.iter().copied().flatten() {
            if self.expr_as_array_size(expr).is_none() {
                return true;
            }
        }
        // All dimensions constant, but the element type may still be a
        // VLA-member struct (or a VLA typedef).
        self.probe_vla_size_from_type_spec(type_spec)
    }

    fn probe_vla_size_from_type_spec(&self, type_spec: &TypeSpecifier) -> bool {
        match type_spec {
            TypeSpecifier::TypedefName(name) => {
                if let Some(fs) = self.func_state.as_ref() {
                    if fs.vla_typedef_sizes.contains_key(name) {
                        return true;
                    }
                }
                // typedef of a struct with VLA members: the struct's size was
                // recorded under its key when the type was defined.
                if let Some(ct) = self.types.typedefs.get(name) {
                    match ct {
                        CType::Struct(key) | CType::Union(key) => {
                            if let Some(fs) = self.func_state.as_ref() {
                                if fs.vla_typedef_sizes.contains_key(key.as_ref()) {
                                    return true;
                                }
                            }
                        }
                        CType::Array(_, None) => return true,
                        _ => {}
                    }
                }
                false
            }
            TypeSpecifier::Struct(Some(tag), ..) | TypeSpecifier::Union(Some(tag), ..) => {
                let key = if matches!(type_spec, TypeSpecifier::Union(..)) {
                    format!("union.{}", tag)
                } else {
                    format!("struct.{}", tag)
                };
                self.func_state
                    .as_ref()
                    .is_some_and(|fs| fs.vla_typedef_sizes.contains_key(&key))
            }
            TypeSpecifier::Struct(_, Some(fields), ..) | TypeSpecifier::Union(_, Some(fields), ..) => {
                fields.iter().any(|f| {
                    if f.bit_width.is_some() {
                        return false;
                    }
                    if !f.derived.is_empty()
                        && self.probe_vla_runtime_size(&f.type_spec, &f.derived)
                    {
                        return true;
                    }
                    self.probe_vla_size_from_type_spec(&f.type_spec)
                })
            }
            TypeSpecifier::Typeof(expr) => {
                // Mirror compute_vla_size_from_type_spec: typeof(vla_local)
                // is dynamic iff the local is a VLA with a runtime size.
                if let Expr::Identifier(name, _) = &**expr {
                    if let Some(info) = self.func_state.as_ref().and_then(|fs| fs.locals.get(name))
                    {
                        return info.vla_size.is_some();
                    }
                }
                false
            }
            TypeSpecifier::Array(_, Some(size_expr)) => self.expr_as_array_size(size_expr).is_none(),
            TypeSpecifier::Array(_, None) => true,
            _ => false,
        }
    }

    /// Effective alignment of a struct field for the dynamic VLA cursor,
    /// mirroring `StructLayoutBuilder::compute_field_alignment`: a per-field
    /// `packed` forces 1; an explicit `aligned(N)` attribute raises above the
    /// natural alignment and overrides struct-level packing
    /// (pr82210.c: `__attribute__((aligned(16))) struct T { short c; }
    /// a[size];` gives the member 16-byte alignment — the struct's size
    /// rounds to 16 even though the member's runtime span does not); a
    /// struct-level `packed` or `#pragma pack` caps the natural alignment.
    pub(super) fn vla_field_effective_align(
        &self,
        f: &StructFieldDecl,
        ct: &CType,
        is_packed: bool,
        pragma_pack: Option<usize>,
    ) -> usize {
        let natural = self.ctype_align(ct).max(1);
        if f.is_packed {
            return 1;
        }
        if let Some(explicit) = f.alignment {
            return natural.max(explicit);
        }
        let cap = if is_packed { Some(1) } else { pragma_pack };
        match cap {
            Some(c) => natural.min(c.max(1)),
            None => natural,
        }
    }

    /// Packed bitfield placement: bits accumulate contiguously with no
    /// storage-unit constraint (GCC packed-bitfield semantics).
    pub(super) fn vla_cursor_bitfield_packed(&mut self, cursor: &mut VlaDynCursor, bw: u32) {
        if bw == 0 {
            // Zero-width packed: round up to the next byte boundary. (The
            // type-alignment forcing of a packed `T :0` is applied by the
            // caller, which knows the type.)
            self.vla_cursor_close_run_for_field(cursor);
            return;
        }
        if !cursor.run_open {
            cursor.run_open = true;
            cursor.run_bits = 0;
        }
        cursor.run_bits += bw;
    }

    /// Unpacked bitfield placement with a decidable straddle rule: the field
    /// must fit inside one `unit_size`-byte storage unit; a straddling field
    /// starts a fresh unit. Requires cursor ≡ 0 (mod unit_size) at run start
    /// (guaranteed by the caller checking `unit_size <= cursor.guarantee`).
    pub(super) fn vla_cursor_bitfield_unit(&mut self, cursor: &mut VlaDynCursor, bw: u32, unit_size: usize) {
        if !cursor.run_open {
            cursor.run_open = true;
            cursor.run_bits = 0;
        }
        let unit_bits = (unit_size * 8) as u32;
        let unit_rel = cursor.run_bits % unit_bits;
        if unit_rel + bw > unit_bits {
            // Straddle: start the next unit.
            cursor.run_bits += unit_bits - unit_rel;
        }
        cursor.run_bits += bw;
    }

    /// Close an open bitfield run so a byte-addressed field can follow.
    /// Packed: next byte after the last bit. Unpacked: same rounding (the
    /// caller then aligns to the field's alignment).
    pub(super) fn vla_cursor_close_run_for_field(&mut self, cursor: &mut VlaDynCursor) {
        if !cursor.run_open {
            return;
        }
        let bytes = ((cursor.run_bits + 7) / 8) as usize;
        cursor.post += bytes;
        cursor.guarantee = gcd_usize(cursor.guarantee, bytes.max(1));
        cursor.run_open = false;
        cursor.run_bits = 0;
    }

    /// Close an open bitfield run at the END of the struct: the size extends
    /// to the end of the last storage unit (GCC: `struct {unsigned b:1;}` is
    /// 4 bytes, not 1). `last_unit` is the last bitfield's storage size;
    /// packed structs round to a byte instead.
    pub(super) fn vla_cursor_close_run_for_size(&mut self, cursor: &mut VlaDynCursor, last_unit: usize) {
        if !cursor.run_open {
            return;
        }
        if cursor.packed {
            let bytes = ((cursor.run_bits + 7) / 8) as usize;
            cursor.post += bytes;
            cursor.guarantee = gcd_usize(cursor.guarantee, bytes.max(1));
        } else {
            let unit = last_unit.max(1);
            let units = (cursor.run_bits as usize).div_ceil(unit * 8);
            cursor.post += units * unit;
            cursor.guarantee = gcd_usize(cursor.guarantee, unit);
        }
        cursor.run_open = false;
        cursor.run_bits = 0;
    }

    /// Align the byte cursor up to `align` (no-op when already guaranteed).
    pub(super) fn vla_cursor_align(&mut self, cursor: &mut VlaDynCursor, align: usize) {
        if align <= 1 {
            return;
        }
        if align <= cursor.guarantee {
            return; // cursor is already ≡ 0 (mod align)
        }
        let mask = (align - 1) as i64;
        match cursor.runtime.take() {
            Some(rt) => {
                // Fold the static prefix into the runtime value, align the
                // sum, and restart the static tail at zero.
                let v = if cursor.post > 0 {
                    self.emit_binop_val(
                        crate::ir::ops::IrBinOp::Add,
                        Operand::Value(rt),
                        Operand::Const(IrConst::ptr_int(cursor.post as i64)),
                        crate::common::types::target_int_ir_type(),
                    )
                } else {
                    rt
                };
                let ty = crate::common::types::target_int_ir_type();
                let added = self.emit_binop_val(
                    crate::ir::ops::IrBinOp::Add,
                    Operand::Value(v),
                    Operand::Const(IrConst::ptr_int(mask)),
                    ty,
                );
                let aligned = self.emit_binop_val(
                    crate::ir::ops::IrBinOp::And,
                    Operand::Value(added),
                    Operand::Const(IrConst::ptr_int(!mask)),
                    ty,
                );
                cursor.runtime = Some(aligned);
                cursor.post = 0;
            }
            None => {
                cursor.post = (cursor.post + align - 1) & !(align - 1);
            }
        }
        cursor.guarantee = align;
    }

    /// Add a runtime-sized contribution (a VLA member's byte size).
    ///
    /// The cursor's alignment guarantee drops to 1: a VLA member's runtime
    /// size carries no statically decidable divisibility in general — its
    /// element may itself be dynamically sized (arrays of VLA-member
    /// structs), and a member alignment attribute (e.g.
    /// `__attribute__((aligned(16))) struct T a[size];`) raises the member's
    /// OFFSET alignment without constraining its runtime SIZE at all
    /// (gcc.c-torture/execute pr82210.c: sizeof(struct T) = 2, so a[15]
    /// spans 30 bytes — not a multiple of the member's 16-byte alignment).
    /// The worst case costs one runtime align-up for the first post-VLA
    /// field; soundness comes first.
    pub(super) fn vla_cursor_add_vla(&mut self, cursor: &mut VlaDynCursor, sz: Value) {
        match cursor.runtime.take() {
            Some(rt) => {
                let v = if cursor.post > 0 {
                    self.emit_binop_val(
                        crate::ir::ops::IrBinOp::Add,
                        Operand::Value(rt),
                        Operand::Const(IrConst::ptr_int(cursor.post as i64)),
                        crate::common::types::target_int_ir_type(),
                    )
                } else {
                    rt
                };
                let added = self.emit_binop_val(
                    crate::ir::ops::IrBinOp::Add,
                    Operand::Value(v),
                    Operand::Value(sz),
                    crate::common::types::target_int_ir_type(),
                );
                cursor.runtime = Some(added);
                cursor.post = 0;
            }
            None => {
                cursor.runtime = Some(sz);
            }
        }
        cursor.guarantee = 1;
    }

    /// Add a fixed-size contribution (a non-bitfield field's byte size).
    pub(super) fn vla_cursor_add_fixed(&mut self, cursor: &mut VlaDynCursor, d: usize) {
        if d == 0 {
            return;
        }
        cursor.post += d;
        cursor.guarantee = gcd_usize(cursor.guarantee.max(1), d);
    }

    /// Materialize the current byte cursor as an IR value, if it has any
    /// runtime component. Pure in the `None` case (compile-time-only cursor);
    /// the `Some` case emits one Add when a static tail is pending.
    pub(super) fn vla_cursor_offset_value(&mut self, cursor: &VlaDynCursor) -> Option<Value> {
        let rt = cursor.runtime?;
        let ty = crate::common::types::target_int_ir_type();
        Some(if cursor.post > 0 {
            self.emit_binop_val(
                crate::ir::ops::IrBinOp::Add,
                Operand::Value(rt),
                Operand::Const(IrConst::ptr_int(cursor.post as i64)),
                ty,
            )
        } else {
            rt
        })
    }

    /// Total byte size accumulated by the cursor, as an IR value. Unlike
    /// [`Lowerer::vla_cursor_offset_value`] this also materializes a pure
    /// constant cursor (used for the sizeof walk, where the trailing
    /// alignment rounding needs an operand either way).
    pub(super) fn vla_cursor_total(&mut self, cursor: &VlaDynCursor) -> Operand {
        let ty = crate::common::types::target_int_ir_type();
        match cursor.runtime {
            Some(rt) => {
                if cursor.post > 0 {
                    Operand::Value(self.emit_binop_val(
                        crate::ir::ops::IrBinOp::Add,
                        Operand::Value(rt),
                        Operand::Const(IrConst::ptr_int(cursor.post as i64)),
                        ty,
                    ))
                } else {
                    Operand::Value(rt)
                }
            }
            None => Operand::Const(IrConst::ptr_int(cursor.post as i64)),
        }
    }
}

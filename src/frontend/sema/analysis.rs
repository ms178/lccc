//! Semantic analysis pass.
//!
//! Walks the AST to:
//! - Build a scoped symbol table of declarations
//! - Track function signatures for call validation
//! - Resolve typedef names and typeof(expr) via ExprTypeChecker
//! - Collect information needed by the IR lowering phase
//! - Map __builtin_* identifiers to their libc equivalents
//!
//! Expression CType inference is available via `type_checker::ExprTypeChecker`,
//! which uses SymbolTable + TypeContext + FunctionInfo to infer types without
//! depending on lowering state. This enables typeof(expr) resolution and
//! will eventually support type annotations on AST nodes.
//!
//! This pass does NOT reject programs with type errors (yet); it collects
//! information for the lowerer. Full type checking is TODO.

mod constexpr;

use super::const_eval::{ConstMap, SemaConstEval};
use super::type_context::{EnumConstant, FunctionTypedefInfo, TypeContext};
use super::void_declarators::{VoidRole, void_declarator_error};
use crate::common::error::DiagnosticEngine;
use crate::common::source::Span;
use crate::common::symbol_table::{FoldedValue, Symbol, SymbolTable};
use crate::common::type_builder;
use crate::common::type_builder::TypeConvertContext;
use crate::common::types::{
    AddressSpace, CType, ExtVectorLayout, ExtVectorSelectorError, FunctionType, RcLayout,
    StructFieldLayout, StructLayout,
};
use crate::frontend::parser::ast::{
    BinOp, BlockItem, CompoundStmt, Declaration, DerivedDeclarator, Designator, EnumVariant, Expr,
    ExprId, ExternalDecl, ForInit, FunctionDef, GenericAssociation, InitDeclarator, Initializer,
    InitializerItem, ParamDecl, PostfixOp, SizeofArg, StaticAssert, Stmt, StructFieldDecl,
    TranslationUnit, TypeSpecifier, UnaryOp,
};
use crate::frontend::sema::builtins;
use crate::frontend::sema::intrinsic_arity;
use crate::frontend::sema::operand_constraints::{self, Operand};
use crate::ir::reexports::IrConst;

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;

/// Outcome of a case segment in a switch statement for -Wreturn-type analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwitchSegmentOutcome {
    /// Segment returns or otherwise diverges (goto, continue to outer loop, infinite loop)
    Returns,
    /// Segment breaks out of the switch
    Breaks,
    /// Segment falls through (to next case or end of switch body)
    FallsThrough,
}

#[derive(Clone, Copy)]
enum FormatLength {
    Default,
    Hh,
    H,
    L,
    Ll,
    J,
    Z,
    T,
    BigL,
}

#[derive(Clone)]
enum FormatExpectation {
    Exact(CType),
    IntegerEither,
    CharacterPointer,
    AnyPointer,
    PointerTo(CType),
}

fn gcc_format_type_name(ty: &CType) -> String {
    match ty {
        CType::Bool => "_Bool".to_string(),
        CType::SChar => "signed char".to_string(),
        CType::UChar => "unsigned char".to_string(),
        CType::Short => "short int".to_string(),
        CType::UShort => "short unsigned int".to_string(),
        CType::Int => "int".to_string(),
        CType::UInt => "unsigned int".to_string(),
        CType::Long => "long int".to_string(),
        CType::ULong => "long unsigned int".to_string(),
        CType::LongLong => "long long int".to_string(),
        CType::ULongLong => "long long unsigned int".to_string(),
        CType::Int128 => "__int128".to_string(),
        CType::UInt128 => "unsigned __int128".to_string(),
        CType::Pointer(pointee, _) => format!("{} *", gcc_format_type_name(pointee)),
        _ => ty.to_string(),
    }
}

impl FormatExpectation {
    fn description(&self) -> String {
        match self {
            Self::Exact(ty) => gcc_format_type_name(ty),
            Self::IntegerEither => "int or unsigned int".to_string(),
            Self::CharacterPointer => "char *".to_string(),
            Self::AnyPointer => "void *".to_string(),
            Self::PointerTo(ty) => format!("{} *", gcc_format_type_name(ty)),
        }
    }

    fn accepts(&self, actual: &CType) -> bool {
        let promoted = match actual {
            CType::Float | CType::Float16 | CType::BFloat16 => CType::Double,
            other => other.decayed_value_ctype().integer_promoted(),
        };
        match self {
            Self::Exact(expected) => &promoted == expected,
            Self::IntegerEither => matches!(promoted, CType::Int | CType::UInt),
            Self::CharacterPointer => matches!(
                promoted,
                CType::Pointer(ref pointee, _) if matches!(pointee.as_ref(), CType::Char)
            ),
            Self::AnyPointer => matches!(promoted, CType::Pointer(..) | CType::NullPtr),
            Self::PointerTo(pointee) => matches!(
                promoted,
                CType::Pointer(ref actual_pointee, _)
                    if actual_pointee.as_ref() == pointee
            ),
        }
    }
}

/// Map from AST expression node identity to its inferred CType.
///
/// Keyed by [`ExprId`], a type-safe wrapper around each `Expr` node's identity.
/// The AST is allocated once during parsing and is not moved or reallocated
/// before the lowerer consumes it, so node identities are stable throughout
/// the compilation pipeline.
///
/// This is the core data structure for Step 3 of the typed-AST plan: sema
/// annotates every expression it can type-check, and the lowerer consults
/// these annotations as a fallback before doing its own (more expensive)
/// type inference.
pub type ExprTypeMap = FxHashMap<ExprId, CType>;

/// Information about a function collected during semantic analysis.
#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub return_type: CType,
    pub params: Vec<(CType, Option<String>)>,
    pub variadic: bool,
    /// Visible call-site prototype, distinct from a K&R definition's body
    /// parameters. Keeping body types does not make an identifier-list
    /// definition a prototype (C11 6.7.6.3p14).
    pub prototype: Option<FunctionType>,
    pub is_defined: bool,
    /// Whether the function is declared with __attribute__((noreturn)) or _Noreturn
    pub is_noreturn: bool,
    /// `const` bits of the RETURN type's pointer levels (bit `L-1` is pointer
    /// level `L`, level 1 closest to the base type). `CType` erases
    /// qualifiers, so `*g() = 0` for `int *const *g(void)` is only
    /// diagnosable if the record keeps the middle level's `const`.
    pub return_pointer_const_levels: u32,
}

impl FunctionInfo {
    /// Callable type, without leaking K&R body parameter declarations into
    /// function-pointer inference. Lowering still reads `params` for the ABI.
    pub(super) fn call_type(&self) -> FunctionType {
        self.prototype.clone().unwrap_or_else(|| FunctionType {
            return_type: self.return_type.clone(),
            params: Vec::new(),
            variadic: self.variadic,
            has_prototype: false,
        })
    }
}

/// Results of semantic analysis, used by the lowering phase.
#[derive(Debug)]
pub struct SemaResult {
    /// Function signatures discovered during analysis.
    pub functions: FxHashMap<String, FunctionInfo>,
    /// Type context populated by sema: typedefs, enum constants, struct layouts,
    /// function typedefs, function pointer typedefs.
    pub type_context: TypeContext,
    /// Expression type annotations: maps `ExprId` keys to their inferred
    /// CTypes. Populated during sema's analyze_expr walk using
    /// ExprTypeChecker. The lowerer consults this as a fallback in
    /// get_expr_ctype() after its own lowering-state-based inference fails.
    pub expr_types: ExprTypeMap,
    /// The entity a call resolves to, keyed by the call expression, recorded
    /// while the declaring scope was live.  Only shadowed bindings appear:
    /// a call to an unshadowed name reads its record by basename, and keeping
    /// the map sparse is what makes the extra work invisible on the hot path.
    pub call_entities: FxHashMap<ExprId, String>,
    /// Pre-computed constant expression values: maps `ExprId` keys to their
    /// compile-time IrConst values. Populated during sema's walk
    /// using SemaConstEval (handles float literals, cast chains, sizeof,
    /// binary ops with proper signedness semantics). The lowerer consults
    /// this as an O(1) fast path before its own eval_const_expr.
    pub const_values: ConstMap,
    /// Identifier uses that denote an enumeration constant, with its value.
    /// Sema resolves C's ordinary-identifier namespace with the scopes live
    /// (`SymbolTable::lookup_ordinary`), so this verdict is exact where the
    /// lowerer's own lookup is not: an enumerator declared in an expression's
    /// type-name (`sizeof(enum { E = 30 }) + E`) or in an inner block that
    /// shadows an outer object of the same name.
    pub enumerator_refs: FxHashMap<ExprId, i64>,
}

impl Default for SemaResult {
    fn default() -> Self {
        Self {
            functions: FxHashMap::default(),
            type_context: TypeContext::new(),
            expr_types: ExprTypeMap::default(),
            call_entities: FxHashMap::default(),
            const_values: ConstMap::default(),
            enumerator_refs: FxHashMap::default(),
        }
    }
}

/// Semantic analyzer that builds a scoped symbol table and collects type info.
pub struct SemanticAnalyzer {
    /// The scoped symbol table for name resolution.
    symbol_table: SymbolTable,
    /// Accumulated results for the lowerer.
    result: SemaResult,
    // Anonymous struct/union keys are generated via TypeContext::next_anon_struct_id()
    // to ensure a single shared counter across both declaration processing and
    // expression type inference, avoiding key collisions between the two paths.
    /// Structured diagnostic engine for error/warning reporting.
    /// All sema errors and warnings are emitted with source spans through this
    /// engine, which handles rendering, filtering (-Wall/-Werror), and counting.
    /// Uses RefCell for interior mutability so that &self methods (e.g.,
    /// eval_const_expr_as_usize from TypeConvertContext) can emit diagnostics.
    diagnostics: RefCell<DiagnosticEngine>,
    /// Set of struct/union keys that have been defined (with a body, even if
    /// empty). Used to distinguish `struct X {}` (defined, complete) from
    /// `struct X;` (forward declaration, incomplete) for the incomplete type
    /// check. Uses RefCell for interior mutability since resolve_struct_or_union
    /// takes &self.
    defined_structs: RefCell<FxHashSet<String>>,
    /// Return type of the function body currently being checked.
    current_return_type: Option<CType>,
    /// Stack of enclosing function definition names for nested functions.
    /// Entry i is the name of the function containing the body currently
    /// being analyzed; the LAST entry is the innermost (possibly nested)
    /// function, which is also the prefix of the mangled `parent.nested` key that
    /// `analyze_nested_function_def` registers in `result.functions`: it is how a
    /// block-scope declaration finds the nested entity it belongs to instead of
    /// writing through the shared basename.
    function_nesting_stack: Vec<String>,
    /// Whether ordinary hosted-library builtin folds are disabled.
    no_builtins: bool,
    builtin_policy: crate::common::pass_policy::BuiltinFlags,
    /// The dialect predates C99 (`-std=c90`/`gnu89`/`iso9899:199409`): GCC
    /// then still treats an overflowing signed left shift as a constant.
    pre_c99: bool,
    pending_loop_labels: Vec<String>,
    active_break_targets: Vec<Vec<String>>,
    active_continue_targets: Vec<Vec<String>>,
}

/// Where an implicit conversion occurs, for GCC 16.2's verbatim
/// `-Wint-conversion` phrasing of each context (all four forms
/// measured against gcc-16.2.0).
#[derive(Clone, Copy)]
enum IConvContext<'a> {
    Initialization,
    Assignment,
    Returning,
    Argument { index: usize, callee: &'a str },
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        let mut analyzer = Self {
            symbol_table: SymbolTable::new(),
            result: SemaResult::default(),
            diagnostics: RefCell::new(DiagnosticEngine::new()),
            defined_structs: RefCell::new(FxHashSet::default()),
            current_return_type: None,
            function_nesting_stack: Vec::new(),
            no_builtins: false,
            builtin_policy: crate::common::pass_policy::BuiltinFlags::default(),
            pre_c99: false,
            pending_loop_labels: Vec::new(),
            active_break_targets: Vec::new(),
            active_continue_targets: Vec::new(),
        };
        // Pre-populate with common implicit declarations
        analyzer.declare_implicit_functions();
        analyzer
    }

    /// Analyze a translation unit. Builds symbol table and collects semantic info.
    /// Returns Ok(()) on success, or an error count on failure.
    /// All errors are emitted through the DiagnosticEngine with source spans.
    pub fn analyze(&mut self, tu: &TranslationUnit) -> Result<(), usize> {
        for decl in &tu.decls {
            match decl {
                ExternalDecl::FunctionDef(func) => {
                    self.analyze_function_def(func);
                }
                ExternalDecl::Declaration(decl) => {
                    self.analyze_declaration(decl, /* is_global */ true);
                }
                ExternalDecl::TopLevelAsm(_) => {
                    // Top-level asm is passed through verbatim; no semantic analysis needed
                }
                ExternalDecl::StaticAssert(assertion) => self.check_static_assert(assertion),
            }
        }
        // File-scope internal-linkage objects participate in -Wunused-variable
        // too, and their uses can occur in declarations processed later.
        self.warn_unused_scope();
        // Validate `__attribute__((ifunc("resolver")))` / `[[gnu::ifunc("resolver")]]`
        // resolver targets across the translation unit (GCC 16.2 parity).
        let has_toplevel_asm = tu
            .decls
            .iter()
            .any(|d| matches!(d, ExternalDecl::TopLevelAsm(_)));
        if !has_toplevel_asm {
            let mut defined_resolvers = crate::common::fx_hash::FxHashSet::default();
            for d in &tu.decls {
                match d {
                    ExternalDecl::FunctionDef(f) => {
                        defined_resolvers.insert(f.name.as_str());
                    }
                    ExternalDecl::Declaration(decl) => {
                        for init_decl in &decl.declarators {
                            if (init_decl.attrs.alias_target.is_some()
                                || init_decl.attrs.ifunc_target.is_some())
                                && !init_decl.name.is_empty()
                            {
                                defined_resolvers.insert(init_decl.name.as_str());
                            }
                            if let Some(ref reg) = init_decl.attrs.asm_register
                                && (init_decl.attrs.alias_target.is_some()
                                    || init_decl.attrs.ifunc_target.is_some())
                            {
                                defined_resolvers.insert(reg.as_str());
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Also register `__asm__("label")` names attached to defined functions.
            for d in &tu.decls {
                if let ExternalDecl::Declaration(decl) = d {
                    for init_decl in &decl.declarators {
                        if let Some(ref reg) = init_decl.attrs.asm_register
                            && defined_resolvers.contains(init_decl.name.as_str())
                        {
                            defined_resolvers.insert(reg.as_str());
                        }
                    }
                }
            }
            for d in &tu.decls {
                if let ExternalDecl::Declaration(decl) = d {
                    if decl.is_typedef() {
                        continue;
                    }
                    for init_decl in &decl.declarators {
                        if let Some(ref resolver) = init_decl.attrs.ifunc_target
                            && !init_decl.name.is_empty()
                            && self.result.functions.contains_key(&init_decl.name)
                            && !defined_resolvers.contains(resolver.as_str())
                        {
                            self.diagnostics.borrow_mut().error(
                                format!(
                                    "'{}' aliased to undefined symbol '{}'",
                                    init_decl.name, resolver
                                ),
                                init_decl.span,
                            );
                        }
                    }
                }
            }
        }

        let diag = self.diagnostics.borrow();
        if diag.has_errors() {
            Err(diag.error_count())
        } else {
            Ok(())
        }
    }

    /// Get the analysis results (consumed after analysis).
    pub fn into_result(self) -> SemaResult {
        // Both sema's declaration processing and expression type inference now use
        // TypeContext::next_anon_struct_id(), so no counter synchronization is needed.
        self.result
    }

    /// Set a pre-configured diagnostic engine on the analyzer.
    pub fn set_diagnostics(&mut self, engine: DiagnosticEngine) {
        self.diagnostics = RefCell::new(engine);
    }

    /// Disable ordinary hosted-library builtin folds (`-fno-builtin`).
    /// Compiler-prefixed `__builtin_*` forms remain available.
    pub fn set_no_builtins(&mut self, no_builtins: bool) {
        self.no_builtins = no_builtins;
    }

    /// Install the CLI's builtin-withdrawal policy (blanket and per-name
    /// state).  Evaluators consult this instead of an ambient mode.
    pub fn set_builtin_policy(&mut self, policy: crate::common::pass_policy::BuiltinFlags) {
        self.builtin_policy = policy;
    }

    /// Select a pre-C99 dialect (see [`Self::pre_c99`]).
    pub fn set_pre_c99_dialect(&mut self, pre_c99: bool) {
        self.pre_c99 = pre_c99;
    }

    /// Take the diagnostic engine back from the analyzer.
    pub fn take_diagnostics(&mut self) -> DiagnosticEngine {
        self.diagnostics.replace(DiagnosticEngine::new())
    }

    // === Function analysis ===

    fn analyze_function_def(&mut self, func: &FunctionDef) {
        self.function_nesting_stack.push(func.name.clone());
        self.analyze_function_def_inner(func);
        self.function_nesting_stack.pop();
    }

    /// Analyze a GNU C nested function definition (block scope).
    ///
    /// The nested function's symbol is declared in the CURRENT block scope
    /// (so calls resolve with correct shadowing), its body is analyzed with
    /// its own return-type context, and its signature is registered in the
    /// function table under the plain name (first definition wins, matching
    /// the file-scope rule). A separate record keyed by the mangled
    /// `parent.nested` name is added for the lowerer.
    fn analyze_nested_function_def(&mut self, func: &FunctionDef) {
        let parent = self
            .function_nesting_stack
            .last()
            .cloned()
            .unwrap_or_default();
        let mangled = format!("{}.{}", parent, func.name);

        let return_type = self.type_spec_to_ctype(&func.return_type);
        self.collect_enum_constants_from_type_spec(&func.return_type, func.span);

        let params: Vec<(CType, Option<String>)> = func
            .params
            .iter()
            .map(|p| (self.param_decl_ctype(p), p.name.clone()))
            .collect();

        let prototype = if func.has_prototype {
            Some(FunctionType {
                return_type: return_type.clone(),
                params: params.clone(),
                variadic: func.variadic,
                has_prototype: true,
            })
        } else {
            self.symbol_table
                .lookup_current(&func.name)
                .and_then(|symbol| {
                    if let CType::Function(ft) = &symbol.ty {
                        ft.has_prototype.then(|| ft.as_ref().clone())
                    } else {
                        None
                    }
                })
        };
        let mut func_info = FunctionInfo {
            return_type: return_type.clone(),
            params: params.clone(),
            variadic: func.variadic,
            prototype,
            is_defined: true,
            is_noreturn: func.attrs.is_noreturn(),
            return_pointer_const_levels: self
                .combine_return_const_levels(&func.return_type, func.return_pointer_const_levels),
        };
        // A block-scope prototype may precede the definition and carry facts the
        // definition does not repeat; `noreturn` is one of them. The file-scope
        // path merges with `|=`, so the nested path merges too -- otherwise the
        // same redeclaration sequence means different things depending on whether
        // the function is nested.
        if let Some(prior) = self.result.functions.get(&mangled) {
            func_info.is_noreturn |= prior.is_noreturn;
            if func_info.prototype.is_none() {
                func_info.prototype = prior.prototype.clone();
            }
        }
        let func_ctype = CType::Function(Box::new(func_info.call_type()));
        self.result.functions.insert(mangled, func_info);

        // Declare the symbol in the enclosing block scope so identifier
        // resolution finds it with proper lexical scoping.
        self.symbol_table.declare(Symbol {
            name: func.name.clone(),
            ty: func_ctype,
            explicit_alignment: None,
            type_align: None,
            address_space: AddressSpace::Default,
        });

        // Analyze the body in its own scope, with its own return type.
        self.symbol_table.push_scope();
        self.result.type_context.push_scope();
        for param in &func.params {
            if let Some(name) = &param.name {
                let ty = self.param_decl_ctype(param);
                let const_base = self.decl_const_base(&param.type_spec, param.is_const);
                self.symbol_table.declare(Symbol {
                    name: name.clone(),
                    ty,
                    explicit_alignment: None,
                    type_align: None,
                    address_space: AddressSpace::Default,
                });
                self.symbol_table.mark_automatic(name);
                if const_base {
                    self.symbol_table.mark_const_base(name);
                }
                // Top-level const for C11 6.5.16p1: a parameter with any
                // star is read-only only when the `const` sits on the star
                // closest to the name (`int *const p`); without a star the
                // declaration's own `const` decides (`const int x`,
                // `const P q`).
                let top_const = if param.decayed_array {
                    false
                } else if param.pointer_depth > 0 {
                    param.pointer_const
                } else {
                    param.is_const || self.base_typedef_is_top_const(&param.type_spec)
                };
                if top_const {
                    self.symbol_table.mark_top_const(name);
                }
                // Middle levels of a multi-level pointer keep their `const`
                // only here: `CType` has no qualifier slots, so `int *const
                // *p` would otherwise look identical to `int **p` and `*p = 0`
                // would be accepted (GCC rejects it, C11 6.5.16p1).
                {
                    // Levels inherited from a typedef the parameter is built on
                    // come first: they are closer to the base type.
                    let mut pointer_quals = self.base_typedef_pointer_quals(&param.type_spec);
                    pointer_quals.extend(param.pointer_quals.iter().copied());
                    if !pointer_quals.is_empty() {
                        self.symbol_table.mark_pointer_quals(name, pointer_quals);
                    }
                }
            }
        }
        let previous_return_type = self.current_return_type.replace(return_type.clone());
        self.function_nesting_stack.push(func.name.clone());
        self.analyze_compound_stmt(&func.body);
        self.function_nesting_stack.pop();
        self.current_return_type = previous_return_type;
        self.result.type_context.pop_scope();
        self.symbol_table.pop_scope();
    }

    fn analyze_function_def_inner(&mut self, func: &FunctionDef) {
        let return_type = self.type_spec_to_ctype(&func.return_type);
        if func.is_kr {
            for p in &func.params {
                self.check_void_param(p, VoidRole::KrParameter, func.span);
            }
        } else {
            self.check_void_params(&func.params, func.variadic, func.span);
        }

        // Register enum constants from the return type (e.g., anonymous enums
        // used as return types: `static enum { A, B } func(void) { ... }`).
        // Must happen at file scope before pushing the function body scope.
        self.collect_enum_constants_from_type_spec(&func.return_type, func.span);

        let params: Vec<(CType, Option<String>)> = func
            .params
            .iter()
            .map(|p| (self.param_decl_ctype(p), p.name.clone()))
            .collect();

        // Preserve noreturn from a prior declaration (e.g., the prototype may have
        // __attribute__((noreturn)) even if the definition doesn't repeat it).
        let prior_noreturn = self
            .result
            .functions
            .get(&func.name)
            .is_some_and(|fi| fi.is_noreturn);
        let prototype = if func.has_prototype {
            Some(FunctionType {
                return_type: return_type.clone(),
                params: params.clone(),
                variadic: func.variadic,
                has_prototype: true,
            })
        } else {
            self.result
                .functions
                .get(&func.name)
                .and_then(|info| info.prototype.clone())
        };
        let func_info = FunctionInfo {
            return_type: return_type.clone(),
            params: params.clone(),
            variadic: func.variadic,
            prototype,
            is_defined: true,
            is_noreturn: func.attrs.is_noreturn() || prior_noreturn,
            return_pointer_const_levels: self
                .combine_return_const_levels(&func.return_type, func.return_pointer_const_levels),
        };
        let func_ctype = CType::Function(Box::new(func_info.call_type()));

        // Register body metadata separately from the visible callable type.
        self.result.functions.insert(func.name.clone(), func_info);

        // Register in symbol table
        self.symbol_table.declare(Symbol {
            name: func.name.clone(),
            ty: func_ctype,
            explicit_alignment: None,
            type_align: None,
            address_space: AddressSpace::Default,
        });

        // Push scope for function body (both symbol table and type context,
        // so local struct/union definitions don't overwrite global layouts)
        self.symbol_table.push_scope();
        self.result.type_context.push_scope();

        // Declare parameters in function scope. Function-pointer parameters
        // (`int (*cb)(void*)`) store the function type in `fptr_params`, not
        // in `type_spec`; using only type_spec typed them as `int *` and
        // rejected assigning them to matching fields (sqlite3 amalgamation).
        for param in &func.params {
            if let Some(name) = &param.name {
                let ty = self.param_decl_ctype(param);
                let const_base = self.decl_const_base(&param.type_spec, param.is_const);
                self.symbol_table.declare(Symbol {
                    name: name.clone(),
                    ty,
                    explicit_alignment: None,
                    type_align: None,
                    address_space: AddressSpace::Default,
                });
                self.symbol_table.mark_automatic(name);
                if const_base {
                    self.symbol_table.mark_const_base(name);
                }
                // Top-level const for C11 6.5.16p1: a parameter with any
                // star is read-only only when the `const` sits on the star
                // closest to the name (`int *const p`); without a star the
                // declaration's own `const` decides (`const int x`,
                // `const P q`).
                let top_const = if param.decayed_array {
                    false
                } else if param.pointer_depth > 0 {
                    param.pointer_const
                } else {
                    param.is_const || self.base_typedef_is_top_const(&param.type_spec)
                };
                if top_const {
                    self.symbol_table.mark_top_const(name);
                }
                // Middle levels of a multi-level pointer keep their `const`
                // only here: `CType` has no qualifier slots, so `int *const
                // *p` would otherwise look identical to `int **p` and `*p = 0`
                // would be accepted (GCC rejects it, C11 6.5.16p1).
                {
                    // Levels inherited from a typedef the parameter is built on
                    // come first: they are closer to the base type.
                    let mut pointer_quals = self.base_typedef_pointer_quals(&param.type_spec);
                    pointer_quals.extend(param.pointer_quals.iter().copied());
                    if !pointer_quals.is_empty() {
                        self.symbol_table.mark_pointer_quals(name, pointer_quals);
                    }
                }
            }
        }

        // C11 6.4.2.2p1: each function body begins as if with `static const
        // char __func__[] = "function-name";` (GCC's two spellings alike), so
        // `sizeof __func__` is the name's length plus one and the identifier
        // is an array, an address constant.
        for builtin in ["__func__", "__FUNCTION__", "__PRETTY_FUNCTION__"] {
            self.symbol_table.declare(Symbol {
                name: builtin.to_string(),
                ty: CType::Array(Box::new(CType::plain_char()), Some(func.name.len() + 1)),
                explicit_alignment: None,
                type_align: None,
                address_space: AddressSpace::Default,
            });
        }
        // Analyze function body with explicit return-type context.
        let previous_return_type = self.current_return_type.replace(return_type.clone());
        self.analyze_compound_stmt(&func.body);
        self.current_return_type = previous_return_type;

        // -Wreturn-type: warn if a non-void function can fall through without returning.
        // Skip the check for:
        //   - void functions (no return value expected)
        //   - noreturn functions (never return by contract)
        //   - naked functions (body is pure inline asm; return is handled by asm)
        //   - main() (C99 5.1.2.2.3: reaching } is equivalent to return 0)
        if !matches!(return_type, CType::Void)
            && !func.attrs.is_noreturn()
            && !func.attrs.is_naked()
            && func.name != "main"
            && self.compound_can_fall_through(&func.body)
        {
            self.diagnostics.borrow_mut().warning_with_kind(
                "control reaches end of non-void function",
                func.span,
                crate::common::error::WarningKind::ReturnType,
            );
        }

        // Pop function scope
        self.result.type_context.pop_scope();
        self.symbol_table.pop_scope();
    }

    // === Declaration analysis ===

    /// Resolve a declaration-level vector attribute while preserving its
    /// distinct layout contract: `vector_size` specifies bytes, whereas
    /// `ext_vector_type` specifies logical lanes and receives Clang padding.
    fn resolve_decl_vector_ctype(&self, decl: &Declaration, element: CType) -> Option<CType> {
        if let Some(bytes) = decl.vector_size {
            return Some(CType::Vector(Box::new(element), bytes));
        }
        if let Some(lanes) = decl.ext_vector_nelem {
            return match ExtVectorLayout::new(&element, lanes) {
                Some(layout) => Some(CType::ExtVector(Box::new(element), layout)),
                None => {
                    self.diagnostics.borrow_mut().error(
                        "invalid element type or size for ext_vector_type",
                        decl.span,
                    );
                    None
                }
            };
        }
        decl.vector_size_expr
            .as_deref()
            .and_then(|e| self.eval_const_expr(e))
            .and_then(|v| usize::try_from(v).ok())
            .map(|bytes| CType::Vector(Box::new(element), bytes))
    }

    /// Diagnose a file-scope array whose size is not an integer constant
    /// expression (C11 6.7.6.2p1).
    ///
    /// A non-constant size used to turn the declaration into an *incomplete*
    /// array with no diagnostic, silently changing the object's size (and any
    /// later `sizeof`); GCC, Clang and ICX all report "variably modified 'a' at
    /// file scope" (measured — as does GCC for `__builtin_cpu_supports`, which
    /// is a runtime builtin and therefore never a constant expression).
    fn check_global_array_size(&self, decl: &Declaration, declarator: &InitDeclarator) {
        let mut dims: Vec<&Expr> = Vec::new();
        let mut spec = &decl.type_spec;
        while let TypeSpecifier::Array(elem, size) = spec {
            if let Some(size) = size.as_deref() {
                dims.push(size);
            }
            spec = elem;
        }
        for derived in &declarator.derived {
            if let DerivedDeclarator::Array(Some(size)) = derived {
                dims.push(size.as_ref());
            }
        }
        for dim in dims {
            if self.eval_const_expr(dim).is_none() {
                self.diagnostics.borrow_mut().error(
                    format!("variably modified '{}' at file scope", declarator.name),
                    dim.span(),
                );
            }
        }
    }

    // === `void` in declarators (C11 6.7p7, 6.7.2.1p3, 6.7.6.2p1, 6.7.6.3p10,
    // 6.5.2.5p1); see `void_declarators` ===
    //
    // Checked at the AST sites sema visits exactly once -- a declaration, a
    // function definition's parameters, a type name in an expression -- since
    // the diagnostics engine does not deduplicate and the type resolvers
    // (`convert_struct_fields`, `resolve_struct_or_union`) run repeatedly.

    fn report_void_declarator(
        &self,
        ts: &TypeSpecifier,
        derived: &[DerivedDeclarator],
        name: Option<&str>,
        role: VoidRole,
        span: Span,
    ) {
        let typedefs = &self.result.type_context.typedefs;
        let lookup = |n: &str| typedefs.get(n).cloned();
        if let Some(msg) = void_declarator_error(ts, derived, name, role, &lookup) {
            self.diagnostics.borrow_mut().error(msg, span);
        }
    }

    fn check_void_declaration(&self, decl: &Declaration, is_global: bool) {
        self.check_void_members(&decl.type_spec, decl.span);
        for d in &decl.declarators {
            let role = if decl.is_typedef()
                || decl.is_extern()
                || DerivedDeclarator::declares_function(&d.derived)
            {
                VoidRole::Other
            } else if is_global && !decl.is_static() {
                VoidRole::External {
                    initialized: d.init.is_some(),
                }
            } else {
                VoidRole::Object
            };
            let name = (!d.name.is_empty()).then_some(d.name.as_str());
            self.report_void_declarator(&decl.type_spec, &d.derived, name, role, d.span);
            self.check_void_derived_params(&d.derived, d.span);
        }
    }

    /// The parameter lists inside a declarator's function derivations.
    fn check_void_derived_params(&self, derived: &[DerivedDeclarator], span: Span) {
        for d in derived {
            if let DerivedDeclarator::Function(params, variadic, _)
            | DerivedDeclarator::FunctionPointer(params, variadic, _) = d
            {
                self.check_void_params(params, *variadic, span);
            }
        }
    }

    /// A prototype's parameter list.  `(void)` reaches here as an empty list;
    /// a lone unnamed `V` (`typedef void V;`) means the same and is accepted.
    fn check_void_params(&self, params: &[ParamDecl], variadic: bool, span: Span) {
        for p in params {
            let role = VoidRole::Parameter {
                sole: params.len() == 1,
                variadic,
                qualified: p.is_const || p.is_volatile,
            };
            self.check_void_param(p, role, span);
        }
    }

    fn check_void_param(&self, p: &ParamDecl, role: VoidRole, span: Span) {
        // The constraints apply to the type as declared, before `T a[2]` was
        // adjusted to `T *a` (which is all `type_spec` still says).
        let unadjusted;
        let ts = match &p.type_spec {
            TypeSpecifier::Pointer(elem, _) if p.decayed_array => {
                unadjusted = TypeSpecifier::Array(elem.clone(), None);
                &unadjusted
            }
            ts => ts,
        };
        self.report_void_declarator(ts, &[], p.name.as_deref(), role, span);
        if let Some(inner) = &p.fptr_params {
            self.check_void_params(inner, p.fptr_variadic, span);
        }
    }

    /// The members of every struct/union defined in the type specifier,
    /// reported at the declaration (members carry no span of their own).
    fn check_void_members(&self, ts: &TypeSpecifier, span: Span) {
        match ts {
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => {
                let is_union = matches!(ts, TypeSpecifier::Union(..));
                self.check_counted_by_members(fields, is_union, span);
                for f in fields {
                    if let Some(name) = f.name.as_deref() {
                        self.report_void_declarator(
                            &f.type_spec,
                            &f.derived,
                            Some(name),
                            VoidRole::Member,
                            span,
                        );
                    }
                    self.check_void_derived_params(&f.derived, span);
                    self.check_void_members(&f.type_spec, span);
                }
            }
            TypeSpecifier::Pointer(inner, _)
            | TypeSpecifier::Array(inner, _)
            | TypeSpecifier::TypeofType(inner) => self.check_void_members(inner, span),
            _ => {}
        }
    }

    /// A type name (cast, `sizeof`, `_Alignof`, `va_arg`; compound literals
    /// use `VoidRole::CompoundLiteral`).
    fn check_void_type_name(&self, ts: &TypeSpecifier, span: Span) {
        self.check_void_members(ts, span);
        self.report_void_declarator(ts, &[], None, VoidRole::Other, span);
    }

    fn analyze_declaration(&mut self, decl: &Declaration, is_global: bool) {
        // Register enum constants from this declaration's type specifier.
        // This recursively walks into struct/union fields to find inline
        // enum definitions (e.g., `struct { enum { A, B } mode; }`).
        // C23 constexpr: checked before the specifier registers its tag.
        if decl.is_constexpr() {
            self.check_constexpr_declaration(decl);
        }
        self.collect_enum_constants_from_type_spec(&decl.type_spec, decl.span);

        // C11 6.7.6.2p1/6.7.6.2p4: every array dimension of a file-scope object
        // (and of a typedef'd array type) must be an integer constant
        // expression.  Checked here — before the type is refined below — so a
        // non-constant dimension is reported instead of silently degrading the
        // type to an incomplete array.
        if is_global {
            for declarator in &decl.declarators {
                self.check_global_array_size(decl, declarator);
            }
        }
        self.check_void_declaration(decl, is_global);

        // Handle __auto_type / C23 `auto` (6.7.10): requires a single plain-identifier
        // declarator with an expression initializer.
        let base_type = if matches!(&decl.type_spec, TypeSpecifier::AutoType) {
            if decl.declarators.len() > 1 {
                self.diagnostics.borrow_mut().error(
                    "'auto' can only be specified for single declarations".to_string(),
                    decl.span,
                );
            }
            if let Some(first) = decl.declarators.first() {
                if !first.derived.is_empty() {
                    self.diagnostics.borrow_mut().error(
                        "'auto' requires a plain identifier, possibly with attributes, as declarator"
                            .to_string(),
                        first.span,
                    );
                }
                match first.init {
                    Some(Initializer::Expr(ref init_expr)) => {
                        let checker = self.expr_type_checker();
                        checker
                            .infer_expr_ctype(init_expr)
                            .map_or(CType::Int, |ct| ct.decayed_value_ctype())
                    }
                    Some(Initializer::List(_)) => {
                        self.diagnostics.borrow_mut().error(
                            "expected expression before '{' token".to_string(),
                            first.span,
                        );
                        CType::Int
                    }
                    None => {
                        self.diagnostics.borrow_mut().error(
                            "'auto' requires an initialized data declaration".to_string(),
                            first.span,
                        );
                        CType::Int
                    }
                }
            } else {
                self.diagnostics.borrow_mut().error(
                    "'auto' requires an initialized data declaration".to_string(),
                    decl.span,
                );
                CType::Int
            }
        } else {
            self.type_spec_to_ctype(&decl.type_spec)
        };
        if decl.is_transparent_union() {
            self.mark_transparent_union(&base_type, decl.span);
        }

        // Handle typedef declarations: populate TypeContext with typedef info
        if decl.is_typedef() {
            for declarator in &decl.declarators {
                if declarator.name.is_empty() {
                    continue;
                }
                // Check for function typedef (e.g., typedef int func_t(int, int);),
                // including function types RETURNING function pointers
                // (`typedef void (*fn_t(int))(void);`), which must not be
                // mistaken for function-pointer typedefs merely because the
                // return type contains a FunctionPointer declarator.
                let has_func_derived = DerivedDeclarator::declares_function(&declarator.derived);
                if has_func_derived {
                    // Function typedef like `typedef int func_t(int x);` or
                    // `typedef int (*fn_t(int))(char);`. The typedef's own
                    // parameter list is the LAST derived.
                    if let Some(DerivedDeclarator::Function(params, variadic, has_prototype)) =
                        declarator.derived.last()
                    {
                        let return_type = super::type_context::build_prefix_return_type_spec(
                            &decl.type_spec,
                            &declarator.derived,
                            declarator.derived.len() - 1,
                        );
                        self.result.type_context.function_typedefs.insert(
                            declarator.name.clone(),
                            FunctionTypedefInfo {
                                return_type,
                                params: params.clone(),
                                variadic: *variadic,
                                has_prototype: *has_prototype,
                            },
                        );
                    }
                } else if let Some(fti) = super::type_context::extract_fptr_typedef_info(
                    &decl.type_spec,
                    &declarator.derived,
                ) {
                    // Function pointer typedef (e.g., `typedef void *(*lua_Alloc)(void *, ...)`)
                    self.result
                        .type_context
                        .func_ptr_typedefs
                        .insert(declarator.name.clone());
                    self.result
                        .type_context
                        .func_ptr_typedef_info
                        .insert(declarator.name.clone(), fti);
                }

                // Store resolved CType for the typedef.
                // Use build_full_ctype_with_base to reuse the already-resolved base
                // type, avoiding re-resolution of anonymous struct type specs.
                let mut resolved_ctype = if declarator.derived.is_empty() {
                    base_type.clone()
                } else {
                    type_builder::build_full_ctype_with_base(
                        self,
                        base_type.clone(),
                        &declarator.derived,
                    )
                };
                // Same declaration-level address space rule as variables
                // (see analyze_declaration); lowering applies it to typedefs
                // too (stmt.rs lower_local_typedef).
                type_builder::apply_declaration_address_space_for_object(
                    &mut resolved_ctype,
                    decl.address_space,
                    &declarator.derived,
                );
                // Mirror lowering's vector-attribute handling: without the
                // vector wrap, sema records `typedef float xmm_t
                // __attribute__((vector_size(16)))` as a plain 4-byte float,
                // and every struct layout computed through sema's
                // TypeConvertContext (cached in the SHARED type_context
                // layout table) is wrong. glibc's La_x86_64_regs then put
                // lr_vector at offset 96 instead of 192 in the gen-as-const
                // pipeline, and elf/dl-trampoline.S failed its
                // "LR_VECTOR_OFFSET must be multiple of VEC_SIZE" #error.
                let resolved_ctype = if resolved_ctype.is_vector() {
                    resolved_ctype
                } else {
                    self.resolve_decl_vector_ctype(decl, resolved_ctype.clone())
                        .unwrap_or(resolved_ctype)
                };
                // Pointee qualifier of a pointer typedef, from the declaration's
                // base-type `const`/`volatile`, or inherited from a typedef
                // name it is built on (`typedef CNPT CNPT2;`).
                let pointee_qualified = (decl.is_const() || decl.is_volatile())
                    && matches!(resolved_ctype, CType::Pointer(..))
                    || (declarator.derived.is_empty()
                        && matches!(&decl.type_spec, TypeSpecifier::TypedefName(base)
                            if self.result.type_context.qualified_pointee_typedefs.contains(base)));
                self.result
                    .type_context
                    .set_typedef_pointee_qualified(&declarator.name, pointee_qualified);
                let const_base = self.decl_const_base(&decl.type_spec, decl.is_const());
                self.result
                    .type_context
                    .set_typedef_const_base(&declarator.name, const_base);
                // An alias must keep the per-level qualifiers of the type it
                // names: `typedef int *const cp;` used as `cp *p` declares a
                // pointer to a const pointer, and `*p = 0` is a constraint
                // violation (C11 6.5.16p1). Levels inherited from a typedef the
                // alias is built on come first (they are closer to the base).
                let typedef_quals = {
                    let mut quals = self.base_typedef_pointer_quals(&decl.type_spec);
                    quals.extend(type_builder::declarator_pointer_quals(&declarator.derived));
                    quals
                };
                self.result
                    .type_context
                    .set_typedef_pointer_quals(&declarator.name, typedef_quals);
                // `typedef int *const cp;` declares a read-only pointer
                // object; every `cp x` is read-only like a written-out
                // `int *const x`, and `x = 0` is a constraint violation.
                self.result.type_context.set_typedef_top_const(
                    &declarator.name,
                    matches!(resolved_ctype, CType::Pointer(..)) && declarator.attrs.pointer_const,
                );
                self.result
                    .type_context
                    .typedefs
                    .insert(declarator.name.clone(), resolved_ctype);

                // Preserve alignment override from __attribute__((aligned(N))) on the typedef.
                // E.g. typedef struct S aligned_S __attribute__((aligned(32)));
                // Also handle _Alignas(type) on typedefs by computing alignment from the type.
                //
                // When alignment_sizeof_type is set, the parser saw aligned(sizeof(type))
                // but may have computed sizeof incorrectly for struct/union types.  Recompute
                // sizeof here with full struct layout information and use that as alignment.
                let effective_alignment = {
                    let mut align = decl.alignment;
                    if let Some(ref sizeof_ts) = decl.alignment_sizeof_type {
                        let ctype = self.type_spec_to_ctype(sizeof_ts);
                        let real_sizeof =
                            ctype.size_ctx(&*self.result.type_context.borrow_struct_layouts());
                        align = Some(align.map_or(real_sizeof, |a| a.max(real_sizeof)));
                    }
                    align.or_else(|| {
                        decl.alignas_type.as_ref().map(|ts| {
                            // _Alignas(type) means align to alignof(type)
                            let ctype = self.type_spec_to_ctype(ts);
                            ctype.align_ctx(&*self.result.type_context.borrow_struct_layouts())
                        })
                    })
                };
                if let Some(align) = effective_alignment {
                    self.result
                        .type_context
                        .typedef_alignments
                        .insert(declarator.name.clone(), align);
                }
            }
            return; // typedefs don't declare variables
        }

        // Declaration-level __attribute__((vector_size(N))) /
        // __attribute__((ext_vector_type(N))): apply the Vector wrap here for
        // VARIABLE declarations.  Builtin element types already carry the
        // wrap inside their resolved TypeSpecifier (the parser attaches the
        // attribute to the specifier itself); when the element type is a
        // typedef name, however, the attribute is parsed after the typedef
        // name and lands in decl.vector_size, and without this wrap the
        // object collapses to its element type: partial initializers and
        // element stores overrun the (too small) object and element indexing
        // dereferences the first element's value as a base address
        // (gcc.c-torture pr123625: SEGV + wrong checksum on both backends at
        // every opt level).  Mirrors the typedef path above; the matches!
        // guard keeps already-wrapped specifiers from being wrapped twice.
        let base_type = if base_type.is_vector() {
            base_type
        } else {
            self.resolve_decl_vector_ctype(decl, base_type.clone())
                .unwrap_or(base_type)
        };

        for init_decl in &decl.declarators {
            let mut full_type = if init_decl.derived.is_empty() {
                base_type.clone()
            } else {
                // Use build_full_ctype_with_base to reuse the already-resolved base
                // type. This is critical for anonymous structs: re-resolving the type
                // spec would call next_anon_struct_id() again and produce a different
                // key for each declarator, breaking pointer-array compatibility checks.
                type_builder::build_full_ctype_with_base(
                    self,
                    base_type.clone(),
                    &init_decl.derived,
                )
            };
            // The declaration-level named address space (`struct c __seg_gs *p`,
            // `struct c __seg_gs *f(void)`) qualifies the pointer it sits on.
            // Lowering applies the same rule (stmt.rs / global_decl.rs); sema must
            // agree or every expression naming this object is typed generic and
            // a correct call is rejected as an address-space mismatch.
            // The space belongs to the OBJECT's storage only when no pointer in
            // the declarator absorbed it; a pointer declarator makes it a
            // qualifier on the pointee instead (see the function's contract).
            //
            // The procedural rule (`apply_declaration_address_space_for_object`):
            // a pointer from the TYPE SPECIFIER — a typedef or `typeof` name,
            // the kernel's `DEFINE_PER_CPU(type, name)` spelling — never
            // absorbs, so `typeof(obj)` stays plain and `&obj` carries the
            // segment exactly as gcc models it (drivers/base/cacheinfo.c).
            type_builder::apply_declaration_address_space_for_object(
                &mut full_type,
                self.declaration_address_space(decl.address_space, &decl.type_spec),
                &init_decl.derived,
            );
            // Storage space: the shared complement of the call above (single
            // predicate, so sema and lowering can never disagree).
            let object_address_space = type_builder::object_storage_address_space_with_object(
                self.declaration_address_space(decl.address_space, &decl.type_spec),
                &init_decl.derived,
                init_decl.attrs.object_space,
            );

            // Resolve incomplete array sizes from initializers (e.g., int arr[] = {1,2,3})
            // This must happen before storing the symbol so sizeof(arr) works in
            // subsequent global initializer const-evaluation.
            if let CType::Array(ref elem, None) = full_type {
                if let Some(ref init) = init_decl.init {
                    if let Some(count) = self.count_initializer_elements(init, elem) {
                        full_type = CType::Array(elem.clone(), Some(count));
                    }
                }
            }

            if init_decl.attrs.ifunc_target.is_some() {
                if !matches!(full_type, CType::Function(_)) {
                    self.diagnostics
                        .borrow_mut()
                        .warning("'ifunc' attribute ignored", init_decl.span);
                } else if init_decl.attrs.is_weak() {
                    self.diagnostics.borrow_mut().error(
                        format!("weak '{}' cannot be defined 'ifunc'", init_decl.name),
                        init_decl.span,
                    );
                }
            }

            // Check if this is a function declaration (prototype)
            if let CType::Function(ref ft) = full_type {
                let is_noreturn = init_decl.attrs.is_noreturn();
                let prototype = ft.has_prototype.then(|| ft.as_ref().clone());
                // A declaration inside a function body describes that body's view of
                // the name, not the translation unit's. This matters because a GNU
                // nested function is a *distinct entity* that merely shares a
                // basename: rewriting the file-scope record's `prototype` from a
                // nested `auto double f(double);` then retypes the *outer* `f`,
                // because `lower.rs` prefers `fi.prototype` over `fi.params` for any
                // non-K&R definition -- an `int f(int)` emitted with a double
                // argument ABI, on the strength of a declaration that was never about
                // it. The flag that says which case this is was already threaded in
                // (`is_global`) and ignored, which is how the hole survived the
                // scope-aware call lookup: the read side learned about scopes, the
                // write side did not.
                //
                // Type equality cannot be the predicate -- it errs in both
                // directions. `auto int f(int)` beside a file-scope `int f(int)` has
                // an *identical* type, so inequality calls it the same entity and lets
                // the nested declaration retype the outer one (the F01 hole, in the
                // very path taken when no mangled record exists yet); conversely
                // `int f(const char *)` beside `int f(char *)` is one entity per
                // C11 6.7.6.3p15, so inequality wrongly splits it and drops the
                // prototype merge. Storage class is the fact that separates the two:
                // C11 6.7.1p5 says every block-scope function declaration *without*
                // `auto` denotes the externally-visible entity, and `auto` is exactly
                // the spelling of a nested function of its own.
                let distinct_entity_here = !is_global && init_decl.attrs.is_nested_fn();
                if distinct_entity_here {
                    // The nested entity gets its own record *whether or not* this
                    // declaration carries a parameter list.  `void n(void)` is a
                    // prototype with no parameters, so gating the record on
                    // `prototype.is_some()` lost exactly the entities whose whole
                    // purpose is to say "this one does not return" -- the scoped read
                    // side then found no record, fell back to the outer basename, and
                    // the declaration was ignored where it mattered.
                    let has_prototype = prototype.is_some();
                    if let Some(parent) = self.function_nesting_stack.last() {
                        let mangled = format!("{parent}.{}", init_decl.name);
                        match self.result.functions.get_mut(&mangled) {
                            Some(existing) => {
                                existing.is_noreturn |= is_noreturn;
                                if has_prototype {
                                    existing.prototype = prototype;
                                }
                            }
                            // No definition yet, so no record: create one.  A
                            // prototype-only nested declaration cannot be called (GCC
                            // rejects "nested function declared but never defined"), but
                            // the record is what tells analysis which entity a scoped
                            // query is about, and that is a fact about the declaration.
                            None => {
                                self.result.functions.insert(
                                    mangled,
                                    FunctionInfo {
                                        return_type: ft.return_type.clone(),
                                        params: ft.params.clone(),
                                        variadic: ft.variadic,
                                        prototype,
                                        is_defined: false,
                                        is_noreturn,
                                        return_pointer_const_levels: self
                                            .combine_return_const_levels(
                                                &decl.type_spec,
                                                declarator_return_const_levels(
                                                    &init_decl.derived,
                                                    &ft.return_type,
                                                ),
                                            ),
                                    },
                                );
                            }
                        }
                    }
                    if !has_prototype {
                        if let Some(info) = self.result.functions.get(&init_decl.name) {
                            // A block-scope declaration *without* a prototype absorbs the
                            // visible prototype's type (6.2.1p4) -- that much does read
                            // the shared record; it never writes it.
                            let merged = info.call_type();
                            full_type = CType::Function(Box::new(merged));
                        }
                    }
                    self.symbol_table.declare(Symbol {
                        name: init_decl.name.clone(),
                        ty: full_type,
                        explicit_alignment: None,
                        type_align: None,
                        // The same object-storage predicate the ordinary path below
                        // uses (one rule for both, so a `__seggs` block-scope
                        // declaration cannot mean one thing to sema and another to
                        // lowering).  A function declarator resolves to `Default`
                        // through it, which is the point: the value is *derived*, not
                        // assumed, so this line cannot drift from the path it
                        // short-circuits.
                        address_space: object_address_space,
                    });
                    return;
                }
                // A later prototype may be the one that spells the return
                // type's level qualifiers; keep what either declared. Computed
                // before the table entry is borrowed.
                let return_levels = self.combine_return_const_levels(
                    &decl.type_spec,
                    declarator_return_const_levels(&init_decl.derived, &ft.return_type),
                );
                if let Some(existing) = self.result.functions.get_mut(&init_decl.name) {
                    existing.is_noreturn |= is_noreturn;
                    existing.return_pointer_const_levels |= return_levels;
                    if prototype.is_some() {
                        existing.prototype = prototype;
                        // A later prototype refines an implicit declaration;
                        // it must not overwrite a definition's body types.
                        if !existing.is_defined {
                            existing.return_type = ft.return_type.clone();
                            existing.params = ft.params.clone();
                            existing.variadic = ft.variadic;
                        }
                    }
                } else {
                    self.result.functions.insert(
                        init_decl.name.clone(),
                        FunctionInfo {
                            return_type: ft.return_type.clone(),
                            params: ft.params.clone(),
                            variadic: ft.variadic,
                            prototype,
                            is_defined: false,
                            is_noreturn,
                            return_pointer_const_levels: self.combine_return_const_levels(
                                &decl.type_spec,
                                declarator_return_const_levels(&init_decl.derived, &ft.return_type),
                            ),
                        },
                    );
                }
                // An unspecified-parameter redeclaration must not erase an
                // already visible prototype from the scoped callable type.
                // `call_type()` is read back out of the table rather than kept as
                // `ft`, because the update above may have merged a prior prototype.
                // The index cannot fail: the `else` branch just inserted the record
                // when it was absent.
                let merged = self.result.functions[&init_decl.name].call_type();
                full_type = CType::Function(Box::new(merged));
            }

            // Check for incomplete struct/union types in variable declarations.
            // A variable with an incomplete type (forward-declared struct/union with no
            // definition) cannot be allocated. This is only an error for definitions,
            // not for extern declarations (which don't allocate storage).
            // Also catches arrays of incomplete element types.
            // GCC error: "storage size of 'x' isn't known"
            if !decl.is_extern() && !matches!(full_type, CType::Function(_)) {
                // Extract the innermost element type (unwrap arrays)
                let mut elem_type = &full_type;
                while let CType::Array(inner, _) = elem_type {
                    elem_type = inner;
                }
                let is_incomplete = match elem_type {
                    CType::Struct(key) | CType::Union(key) => {
                        // Keep the RefCell borrow in a named, completed scope.
                        // Besides documenting the lock boundary, this retains
                        // identical behavior across Rust 2021/2024 temporary
                        // drop-order rules before diagnostics are mutated.
                        let is_defined = self.defined_structs.borrow().contains(&**key);
                        !is_defined
                    }
                    _ => false,
                };
                if is_incomplete {
                    self.diagnostics.borrow_mut().error(
                        format!("storage size of '{}' isn't known", init_decl.name),
                        init_decl.span,
                    );
                }
            }

            // Resolve explicit alignment from _Alignas or __attribute__((aligned(N))).
            // For _Alignas(type), resolve the type alignment via sema's type resolution.
            // For _Alignas(N) or __attribute__((aligned(N))), use the parsed numeric value.
            let explicit_alignment = if let Some(ref alignas_ts) = decl.alignas_type {
                let ct = self.type_spec_to_ctype(alignas_ts);
                // Evaluate with a short, explicit layout borrow.  Rust 2024
                // drops it before later diagnostic/symbol-table mutations.
                let a = {
                    let layouts = self.result.type_context.borrow_struct_layouts();
                    ct.align_ctx(&*layouts)
                };
                if a > 0 { Some(a) } else { None }
            } else {
                decl.alignment
            };

            // Composite type (C11 6.2.7p4): a redeclaration of an object with
            // linkage as an array of unknown size takes the size from the
            // prior visible declaration.  gcc.c-torture/compile 20001018-1:
            // `extern char i[10]; { extern char i[]; sizeof (i) == 10 }`
            // was rejected ("size of array is negative") because the inner
            // declaration shadowed the complete type.  Only `extern`
            // redeclarations and file-scope ones (tentative `int a[];`)
            // qualify, and only against a declaration with linkage: an
            // `extern` in a block whose visible `i` is a local refers to a
            // different object.
            let links = decl.is_extern() || self.symbol_table.at_file_scope();
            if links {
                if let CType::Array(ref elem, None) = full_type {
                    let prior = self
                        .symbol_table
                        .lookup_linked(&init_decl.name)
                        .map(|s| s.ty.clone());
                    if let Some(CType::Array(prior_elem, Some(n))) = prior {
                        if prior_elem == *elem {
                            full_type = CType::Array(prior_elem, Some(n));
                        }
                    }
                }
            }

            // An automatic object has no segment to live in: GCC 14.2 rejects a
            // block-scope `__seg_gs` object unless it is static or extern.
            if object_address_space != AddressSpace::Default && !links && !decl.is_static() {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "'{}' specified for auto variable '{}'",
                        object_address_space, init_decl.name
                    ),
                    init_decl.span,
                );
            }
            let is_object = !matches!(full_type, CType::Function(_)) && !decl.is_typedef();
            let const_base = is_object && self.decl_const_base(&decl.type_spec, decl.is_const());
            self.symbol_table.declare(Symbol {
                name: init_decl.name.clone(),
                ty: full_type,
                explicit_alignment,
                type_align: self.resolve_typedef_alignment(&decl.type_spec),
                address_space: object_address_space,
            });
            if const_base {
                self.symbol_table.mark_const_base(&init_decl.name);
            }
            // Per-level pointer qualifiers (`int *const *p`): the middle levels
            // are erased from `CType`, so the read-only check reads them from
            // the symbol table. Recorded in CType creation order.
            if is_object {
                let mut pointer_quals = self.base_typedef_pointer_quals(&decl.type_spec);
                pointer_quals.extend(type_builder::declarator_pointer_quals(&init_decl.derived));
                if !pointer_quals.is_empty() {
                    self.symbol_table
                        .mark_pointer_quals(&init_decl.name, pointer_quals);
                }
            }
            // Top-level const for assignment/`++`/`--` (C11 6.5.16p1): with
            // any pointer star the `const` closest to the name decides
            // (`int *const p` read-only, `const int *q` writable, `q`'s
            // pointee constness already recorded above as `const_base`);
            // without a star the declaration's own `const` does (`const int
            // x`, `const int a[]`, `const P q` for `typedef int *P`).
            if is_object {
                let has_star = init_decl
                    .derived
                    .iter()
                    .any(|d| matches!(d, DerivedDeclarator::Pointer(..)));
                let top_const = if has_star {
                    init_decl.attrs.pointer_const
                } else {
                    // Without a star the object's own qualifier comes from the
                    // declaration's `const`, or from the alias it is declared
                    // with (`typedef int *const cp; cp x;`).
                    decl.is_const() || self.base_typedef_is_top_const(&decl.type_spec)
                };
                if top_const {
                    self.symbol_table.mark_top_const(&init_decl.name);
                }
            }
            if links && !self.symbol_table.at_file_scope() {
                self.symbol_table.mark_linked(&init_decl.name);
            } else if is_object && !decl.is_static() && !self.symbol_table.at_file_scope() {
                self.symbol_table.mark_automatic(&init_decl.name);
            }
            if is_object
                && ((!links && !self.symbol_table.at_file_scope())
                    || (is_global && decl.is_static()))
            {
                self.symbol_table.mark_local_object(
                    &init_decl.name,
                    init_decl.span,
                    init_decl.attrs.is_maybe_unused() || init_decl.attrs.is_used(),
                );
            }

            // Analyze array size expressions in derived declarators
            // (catches undeclared identifiers in e.g. `int arr[UNDECLARED];`)
            let mut has_vla_bound = false;
            let is_direct_array_object = !decl.is_typedef()
                && !init_decl.derived.iter().any(|d| {
                    matches!(
                        d,
                        DerivedDeclarator::Pointer(..)
                            | DerivedDeclarator::Function(..)
                            | DerivedDeclarator::FunctionPointer(..)
                    )
                });
            for derived in &init_decl.derived {
                if let DerivedDeclarator::Array(Some(size_expr)) = derived {
                    self.analyze_expr(size_expr);
                    if is_direct_array_object && self.eval_const_expr_value(size_expr).is_none() {
                        has_vla_bound = true;
                    }
                }
            }
            if has_vla_bound {
                if let Some(init) = &init_decl.init {
                    if !matches!(init, Initializer::List(items) if items.is_empty()) {
                        self.diagnostics.borrow_mut().error(
                            "variable-sized object may not be initialized except with an empty initializer"
                                .to_string(),
                            init_decl.span,
                        );
                    }
                }
            }

            // An array with AUTOMATIC storage duration cannot be initialized
            // from an array-typed expression.  An array is neither an
            // assignable value (6.3.2.1p3) nor an acceptable array initializer
            // (6.7.9p13), so the only expression that can stand here is a
            // compound literal -- and GCC rejects exactly that shape for
            // automatic storage, while a static-storage object is initialized
            // by copying the literal's bytes, which GCC accepts.  A string
            // literal stays legal (6.7.9p14) and must not be caught here: it
            // is an ARRAY-typed expression too.
            //
            // `static` and `links` (file scope or `extern`) are the whole set
            // of static-duration spellings that can reach this point, and the
            // narrowness is deliberate rather than an oversight:
            // `_Thread_local` needs no separate arm: C11 6.7.1p3 does not allow
            // a block-scope `_Thread_local` declaration without `static` or
            // `extern`, and GCC diagnoses such a statement as
            // "function-scope 'a' implicitly auto and declared '_Thread_local'"
            // in addition to this very array-expression error (gcc 14.2,
            // -std=gnu11, verified). An arm for it would therefore only teach
            // lccc to accept a statement GCC refuses; `static _Thread_local` is
            // already covered by `is_static()` and `extern _Thread_local` by
            // `links`. lccc used to accept the automatic form as well and then
            // leave the array uninitialised, so `nm_t l[2] = (nm_t[2]){ ... }`
            // in a function silently read garbage.
            let static_duration = decl.is_static() || links;
            if let (Some(Initializer::Expr(expr)), false) = (&init_decl.init, static_duration) {
                let array_target = self
                    .symbol_table
                    .lookup(&init_decl.name)
                    .is_some_and(|s| matches!(s.ty, CType::Array(..)));
                // Conjunction order is the cost model: `array_target` is a hash
                // lookup on a name we just inserted, while `infer_expr_ctype`
                // walks the initializer (its memo is cold here — the sema walk
                // runs later in this same function). Short-circuit on the
                // structural test so the walk happens only for array targets.
                let array_source = array_target && self.initializer_is_array_valued(expr);
                if array_source {
                    // GCC's wording depends on the expression: a compound
                    // literal is the array-expression case it names; any other
                    // array-typed expression (`int b[3] = a;`) is a plain
                    // invalid initializer.
                    let message = if matches!(expr, Expr::CompoundLiteral(..)) {
                        "array initialized from non-constant array expression"
                    } else {
                        "invalid initializer"
                    };
                    self.diagnostics
                        .borrow_mut()
                        .error(message.to_string(), expr.span());
                }
            }

            // Analyze initializer expressions
            if let Some(init) = &init_decl.init {
                self.analyze_initializer(init);
                self.check_object_initializer(decl, init_decl, init);
                if decl.is_constexpr() {
                    self.check_constexpr_initializer(decl, init_decl, init);
                }
                if let Initializer::List(_) = init {
                    let var_ty = self
                        .symbol_table
                        .lookup(&init_decl.name)
                        .map(|s| s.ty.clone());
                    if let Some(var_ty) = var_ty {
                        self.check_initializer_element_spaces(&var_ty, init);
                    }
                }
                // Check for invalid pointer <-> float conversions in scalar initializers
                if let Initializer::Expr(init_expr) = init {
                    let var_ty = self
                        .symbol_table
                        .lookup(&init_decl.name)
                        .map(|s| s.ty.clone());
                    if let Some(var_ty) = var_ty {
                        let checker = self.expr_type_checker();
                        if self.check_assignment_operand(
                            init_expr,
                            &var_ty,
                            init_expr.span(),
                            IConvContext::Initialization,
                        ) {
                            // Diagnosed; the checks below would only repeat it.
                        } else if let Some(init_ty) = checker.infer_expr_ctype(init_expr) {
                            self.check_pointer_float_conversion(
                                &init_ty,
                                &var_ty,
                                init_expr.span(),
                            );
                            let init_ty = self.conversion_source_ctype(init_expr, init_ty);
                            let null_constant = self.is_void_null_pointer_constant(init_expr);
                            self.check_address_space_conversion(
                                &init_ty,
                                &var_ty,
                                init_expr.span(),
                                null_constant,
                            );
                            // Scalar <-> GNU vector in scalar initializers
                            // follows exactly the implicit-assignment rule:
                            // oracles reject it at any size ("initializing
                            // 'd1' with an expression of incompatible
                            // type"), with the sole documented exception of
                            // the ext-vector splat. Accepting it also
                            // silently miscompiled: the scalar rvalue's
                            // VALUE was used as the SOURCE ADDRESS of the
                            // whole-vector copy, faulting at runtime.
                            let scalar_side = init_ty.is_integer()
                                || init_ty.is_floating()
                                || init_ty.is_soft_float()
                                || init_ty.is_complex();
                            let mismatched = (matches!(var_ty, CType::Vector(..)) && scalar_side)
                                || (matches!(init_ty, CType::Vector(..) | CType::ExtVector(..))
                                    && (var_ty.is_integer()
                                        || var_ty.is_floating()
                                        || var_ty.is_soft_float()
                                        || var_ty.is_complex()));
                            if mismatched {
                                self.diagnostics.borrow_mut().error(
                                    format!(
                                        "incompatible types (have '{}' but expected '{}')",
                                        init_ty, var_ty
                                    ),
                                    init_expr.span(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Designator validity for one braced initializer (C11 6.7.9p2-p6).
    ///
    /// A `.name` designator must name a member of the record it applies to,
    /// searching the members of anonymous struct and union members. A `[i]` or
    /// `[lo ... hi]` designator must be a non-negative index within the bounds
    /// of an array of known length. Designator chains are followed through
    /// braced lists. A braced positional element of an array is checked against
    /// the element type when the list has no designators (its index is then its
    /// position). Positional members of records are not tracked, so designators
    /// inside them are not checked. That is the one gap in this pass.
    fn check_designators(&self, ct: &CType, init: &Initializer, span: Span) {
        let Initializer::List(items) = init else {
            return;
        };
        let any_designator = items.iter().any(|item| !item.designators.is_empty());
        for item in items {
            // The type this item initializes: a designator chain selects it from
            // the list's type; a positional item is an element only of an array
            // when the list has no designators (its index is then its position).
            let target: Option<CType> = if item.designators.is_empty() {
                match ct {
                    CType::Array(elem, _) if !any_designator => Some((**elem).clone()),
                    _ => None,
                }
            } else {
                let mut cur = Some(ct.clone());
                for designator in &item.designators {
                    cur = match cur {
                        Some(c) => self.designator_step(&c, designator, span),
                        None => None,
                    };
                }
                cur
            };
            if let (Some(sub), Initializer::List(_)) = (target, &item.init) {
                self.check_designators(&sub, &item.init, span);
            }
        }
    }

    /// The type of the member `name` of a record, searching the members of
    /// anonymous struct and union members (C11 6.7.2.2p13).
    fn find_named_member(&self, layout: &StructLayout, name: &str) -> Option<CType> {
        for field in &layout.fields {
            if field.name == name {
                return Some(field.ty.clone());
            }
            if field.name.is_empty() {
                if let CType::Struct(key) | CType::Union(key) = &field.ty {
                    let inner = self
                        .result
                        .type_context
                        .borrow_struct_layouts()
                        .get(&**key)
                        .cloned();
                    if let Some(ty) = inner.and_then(|l| self.find_named_member(&l, name)) {
                        return Some(ty);
                    }
                }
            }
        }
        None
    }

    /// The type a single designator selects inside `cur`, or `None` after a
    /// diagnostic (or when the type is not known well enough to decide).
    fn designator_step(&self, cur: &CType, designator: &Designator, span: Span) -> Option<CType> {
        match (designator, cur) {
            (Designator::Field(name), CType::Struct(key) | CType::Union(key)) => {
                // A record whose layout is not available is skipped, not reported.
                let layout = self
                    .result
                    .type_context
                    .borrow_struct_layouts()
                    .get(&**key)?
                    .clone();
                match self.find_named_member(&layout, name) {
                    Some(ty) => Some(ty),
                    None => {
                        self.diagnostics.borrow_mut().error(
                            format!("unknown field '{name}' specified in initializer"),
                            span,
                        );
                        None
                    }
                }
            }
            // `void`/function objects are diagnosed by the declarator checks.
            (_, CType::Void | CType::Function(_)) => None,
            // GCC 16.2: a field designator into ANY non-record (array,
            // scalar, vector) and an index designator into ANY non-array.
            (Designator::Field(_), _) => {
                self.diagnostics.borrow_mut().error(
                    "field name not in record or union initializer".to_string(),
                    span,
                );
                None
            }
            (Designator::Index(index) | Designator::Range(index, _), CType::Array(elem, len)) => {
                self.check_designator_bound(index, *len, span);
                if let Designator::Range(_, hi) = designator {
                    self.check_designator_bound(hi, *len, span);
                }
                Some((**elem).clone())
            }
            (Designator::Index(_) | Designator::Range(..), _) => {
                self.diagnostics
                    .borrow_mut()
                    .error("array index in non-array initializer".to_string(), span);
                None
            }
        }
    }

    /// One array designator index: non-negative, and below a known length.
    /// A zero length (GNU `T a[0]`) and an unknown one (`T a[]`) are not bounded.
    fn check_designator_bound(&self, index: &Expr, len: Option<usize>, span: Span) {
        let Some(value) = self.eval_const_expr(index) else {
            return;
        };
        let too_big = len.is_some_and(|n| n > 0 && value >= n as i64);
        if value < 0 || too_big {
            self.diagnostics.borrow_mut().error(
                "array index in initializer exceeds array bounds".to_string(),
                span,
            );
        }
    }

    /// The constant-initializer rules for one object (C11 6.7.9p4).
    ///
    /// A block-scope `static` object needs a constant initializer, so every
    /// leaf must be an arithmetic or address constant (`static_init`). A const
    /// object whose initializer is constant has its value folded, which later
    /// initializers may use (GCC accepts `const int c = 5; static int v = c;`).
    fn check_object_initializer(
        &mut self,
        decl: &Declaration,
        init_decl: &InitDeclarator,
        init: &Initializer,
    ) {
        let (non_constant, folds) = {
            let type_checker = self.expr_type_checker();
            // An object with static storage (file scope, or a block-scope
            // `static`) needs a constant initializer (C11 6.7.9p4), and its
            // initializer is folded the way GCC folds one; an automatic
            // object's is not.
            let static_storage = decl.is_static() || self.symbol_table.at_file_scope();
            let eval = |e: &Expr| {
                if static_storage {
                    self.eval_initializer_value(e)
                } else {
                    self.eval_const_expr_value(e)
                }
            };
            let ctype = |e: &Expr| type_checker.infer_expr_ctype(e);
            let select = |controlling: &Expr, associations: &[GenericAssociation]| {
                type_checker.generic_selection_index(controlling, associations)
            };
            let checker = super::static_init::StaticInitChecker::new(
                &self.symbol_table,
                &eval,
                &ctype,
                &select,
            );
            let non_constant = if static_storage {
                // The object's own type decides what an array-typed compound
                // literal means here: its braced list, or a decayed address.
                let object_ty =
                    type_builder::build_full_ctype(self, &decl.type_spec, &init_decl.derived);
                checker.non_constant_leaves_for(init, &object_ty)
            } else {
                Vec::new()
            };
            let folds =
                declares_const_object(decl, init_decl) && checker.is_constant_initializer(init);
            (non_constant, folds)
        };
        for span in non_constant {
            self.diagnostics
                .borrow_mut()
                .error("initializer element is not constant".to_string(), span);
        }
        if folds {
            let value = self.folded_value(decl, init_decl, init);
            self.symbol_table.mark_folded(&init_decl.name, value);
        }
        if matches!(init, Initializer::List(_)) {
            let ct = type_builder::build_full_ctype(self, &decl.type_spec, &init_decl.derived);
            self.check_designators(&ct, init, init_decl.span);
        }
    }

    /// The value of a const object with a constant scalar initializer, after
    /// conversion to the object's type (`const unsigned char c = 300;` is 44,
    /// `const float f = 0.1;` is the nearest float; see [`fold_to_type`]).
    /// `None` for initializers that are not a single constant scalar.
    fn folded_value(
        &self,
        decl: &Declaration,
        init_decl: &InitDeclarator,
        init: &Initializer,
    ) -> Option<FoldedValue> {
        let expr = match init {
            Initializer::Expr(e) => e,
            // `const int c = {5};` is the same scalar initializer.
            Initializer::List(items) => match items.as_slice() {
                [item] if item.designators.is_empty() => match &item.init {
                    Initializer::Expr(e) => e,
                    Initializer::List(_) => return None,
                },
                _ => return None,
            },
        };
        let ct = type_builder::build_full_ctype(self, &decl.type_spec, &init_decl.derived);
        let evaluator = SemaConstEval {
            types: &self.result.type_context,
            symbols: &self.symbol_table,
            functions: &self.result.functions,
            const_values: Some(&self.result.const_values),
            expr_types: Some(&self.result.expr_types),
            builtin_policy: self.builtin_policy.clone(),
            folding_initializer: false,
            pre_c99: self.pre_c99,
            named_constants_only: false,
        };
        let value = evaluator.eval_const_expr(expr)?;
        let source = self.expr_type_checker().infer_expr_ctype(expr);
        fold_to_type(value, source.as_ref(), &ct)
    }

    /// Count the number of array elements from an initializer.
    /// Used to resolve incomplete array types (e.g., `int arr[] = {1,2,3}`)
    /// so that sizeof(arr) works correctly in subsequent const evaluation.
    ///
    /// Uses the same index-tracking algorithm as lowering's
    /// `compute_init_list_array_size`. Handles initializer lists with
    /// designators, string literals for char arrays, and brace-wrapped
    /// string literals.
    fn count_initializer_elements(&self, init: &Initializer, elem_ty: &CType) -> Option<usize> {
        match init {
            Initializer::List(items) => {
                // Check for brace-wrapped string literal: char c[] = {"hello"}
                let is_char_elem = matches!(
                    elem_ty,
                    CType::Char | CType::SChar | CType::UChar | CType::PlainUChar
                );
                let is_int_elem = matches!(elem_ty, CType::Int | CType::UInt);
                if items.len() == 1 && items[0].designators.is_empty() {
                    if is_char_elem {
                        if let Initializer::Expr(Expr::StringLiteral(s, _)) = &items[0].init {
                            return Some(s.chars().count() + 1);
                        }
                    }
                    if is_int_elem {
                        if let Initializer::Expr(Expr::WideStringLiteral(s, _)) = &items[0].init {
                            // For wchar_t arrays, each Unicode codepoint is one element
                            return Some(s.chars().count() + 1);
                        }
                    }
                    // char16_t array: unsigned short c[] = {u"hello"}
                    if matches!(elem_ty, CType::UShort | CType::Short) {
                        if let Initializer::Expr(Expr::Char16StringLiteral(s, _)) = &items[0].init {
                            // One element per UTF-16 code unit (C11 6.4.5p6).
                            return Some(s.encode_utf16().count() + 1);
                        }
                    }
                }

                // C11 6.7.9: flat initializers for struct arrays fill fields
                // sequentially, so array members consume multiple scalar items.
                // E.g. struct { int arr[4]; int b, e, k; } needs 7 items per element.
                let flat_scalars_per_elem = self.flat_scalar_count_for_type(elem_ty);

                // General case: track current index through the initializer list.
                let mut max_idx = 0usize;
                let mut current_idx = 0usize;
                let mut fields_consumed = 0usize;
                let has_any_designator = items.iter().any(|item| !item.designators.is_empty());

                for item in items {
                    // If this item has an array index designator, jump to that index
                    if let Some(designator) = item.designators.first() {
                        match designator {
                            Designator::Index(idx_expr) => {
                                if let Some(idx) = self.eval_designator_index(idx_expr) {
                                    current_idx = idx;
                                    fields_consumed = 0;
                                }
                            }
                            Designator::Range(_lo, hi) => {
                                // GNU range designator [lo ... hi]: size determined by hi
                                if let Some(idx) = self.eval_designator_index(hi) {
                                    current_idx = idx;
                                    fields_consumed = 0;
                                }
                            }
                            Designator::Field(_) => {
                                // Field designator on struct element - don't advance
                            }
                        }
                    }
                    if current_idx >= max_idx {
                        max_idx = current_idx + 1;
                    }
                    // Only advance if this item doesn't have a field designator
                    // (field designators target fields within the same struct element)
                    let has_field_designator = item
                        .designators
                        .iter()
                        .any(|d| matches!(d, Designator::Field(_)));
                    if !has_field_designator {
                        if flat_scalars_per_elem > 1 && !has_any_designator {
                            // Struct with array fields: group flat scalars per struct.
                            // Designators disable grouping (conservative per-item count).
                            let is_braced = matches!(item.init, Initializer::List(_));
                            if is_braced {
                                // Braced sub-list counts as one complete element
                                current_idx += 1;
                                fields_consumed = 0;
                            } else {
                                fields_consumed += 1;
                                if fields_consumed >= flat_scalars_per_elem {
                                    current_idx += 1;
                                    fields_consumed = 0;
                                }
                            }
                        } else {
                            current_idx += 1;
                        }
                    }
                }

                // Count partial struct element at the end
                if flat_scalars_per_elem > 1 && fields_consumed > 0 && current_idx >= max_idx {
                    max_idx = current_idx + 1;
                }

                if has_any_designator {
                    Some(max_idx)
                } else {
                    Some(max_idx.max(items.len().div_ceil(flat_scalars_per_elem.max(1))))
                }
            }
            Initializer::Expr(expr) => {
                // String literal initializer for char arrays: char s[] = "hello"
                match (elem_ty, expr) {
                    (
                        CType::Char | CType::SChar | CType::UChar | CType::PlainUChar,
                        Expr::StringLiteral(s, _),
                    ) => {
                        Some(s.chars().count() + 1) // +1 for null terminator
                    }
                    (CType::Int | CType::UInt, Expr::WideStringLiteral(s, _)) => {
                        // For wchar_t/char32_t arrays, each Unicode codepoint is one element
                        Some(s.chars().count() + 1)
                    }
                    (CType::UShort | CType::Short, Expr::Char16StringLiteral(s, _)) => {
                        // char16_t arrays hold UTF-16 code units (C11 6.4.5p6):
                        // a code point above U+FFFF takes a surrogate pair.
                        Some(s.encode_utf16().count() + 1)
                    }
                    _ => None,
                }
            }
        }
    }

    /// Compute the flat scalar count for a type - how many scalar initializer
    /// items are needed to fully initialize one value of this type.
    /// Scalar types = 1, arrays = element_count * per_element, structs = sum of fields.
    fn flat_scalar_count_for_type(&self, ty: &CType) -> usize {
        match ty {
            CType::Array(elem_ty, Some(size)) => *size * self.flat_scalar_count_for_type(elem_ty),
            CType::Array(_, None) => 0, // unsized arrays contribute 0 scalars
            CType::Struct(key) | CType::Union(key) => {
                let layouts = self.result.type_context.borrow_struct_layouts();
                if let Some(layout) = layouts.get(&**key) {
                    if layout.fields.is_empty() {
                        return 0;
                    }
                    if layout.is_union {
                        layout
                            .fields
                            .iter()
                            .map(|f| self.flat_scalar_count_for_type(&f.ty))
                            .max()
                            .unwrap_or(1)
                    } else {
                        layout
                            .fields
                            .iter()
                            .map(|f| self.flat_scalar_count_for_type(&f.ty))
                            .sum()
                    }
                } else {
                    1
                }
            }
            _ => 1,
        }
    }

    /// Try to evaluate a designator index expression as a usize.
    /// Handles integer and char literals, and falls back to the full
    /// constant expression evaluator for enum values, sizeof, etc.
    fn eval_designator_index(&self, expr: &Expr) -> Option<usize> {
        match expr {
            Expr::IntLiteral(n, _) | Expr::LongLiteral(n, _) | Expr::LongLongLiteral(n, _) => {
                Some(*n as usize)
            }
            Expr::UIntLiteral(n, _) | Expr::ULongLiteral(n, _) | Expr::ULongLongLiteral(n, _) => {
                Some(*n as usize)
            }
            Expr::CharLiteral(n, _) => Some(*n as usize),
            _ => {
                let val = self.eval_const_expr(expr)?;
                if val >= 0 { Some(val as usize) } else { None }
            }
        }
    }

    /// Evaluate, register and type the enumerators of one enum definition and
    /// return the completed enum (tag `name`) for tag registration. With a
    /// C23 fixed underlying type, every value -- explicit or implicit -- must
    /// be representable in it (6.7.2.2p5; GCC 16.2 wording); without one the
    /// 64-bit count wraps as GCC's extension allows.
    fn process_enum_variants(
        &mut self,
        name: &Option<String>,
        variants: &[EnumVariant],
        is_packed: bool,
        fixed: Option<CType>,
    ) -> crate::common::types::EnumType {
        // Inside the body an enumerator has its provisional type -- with a
        // fixed type, the enumerated type, already complete -- and the enum's
        // completion below retypes all of them together.
        let fixed_enum = fixed.as_ref().map(|f| {
            CType::Enum(crate::common::types::EnumType {
                name: name.clone(),
                variants: Vec::new(),
                is_packed,
                fixed: Some(Box::new(f.clone())),
            })
        });
        let mut next: i128 = 0;
        let mut values = Vec::with_capacity(variants.len());
        for variant in variants {
            if let Some(val_expr) = &variant.value {
                // Walk the enum value expression through analyze_expr first to
                // populate the expr_types cache. Without this, infer_expr_ctype
                // called from eval_const_expr would have no cached sub-expression
                // types and could exhibit O(2^N) blowup on deep expression chains
                // (e.g., `enum { NUM = +1+1+1+...+1 }`).
                self.analyze_expr(val_expr);
                // C11 6.7.2.2p2: an enumerator's value must be an integer
                // constant expression.  Keeping the previous value on failure
                // silently gave `enum { A = f() }` the value 0 and shifted
                // every following enumerator; GCC, Clang and ICX all reject
                // that program with this exact wording (measured).
                match self.eval_const_expr(val_expr) {
                    Some(val) => {
                        // Read in its own type: `0xffffffffffffffff` is not -1.
                        let source = self.expr_type_checker().infer_expr_ctype(val_expr);
                        next = source.as_ref().map_or(i128::from(val), |s| {
                            integer_value_in_type(i128::from(val), s)
                        });
                        if fixed
                            .as_ref()
                            .is_some_and(|f| !integer_representable(next, f))
                        {
                            self.diagnostics.borrow_mut().error(
                                "enumerator value outside the range of underlying type".to_string(),
                                val_expr.span(),
                            );
                        }
                    }
                    None => self.diagnostics.borrow_mut().error(
                        format!(
                            "enumerator value for '{}' is not an integer constant",
                            variant.name
                        ),
                        val_expr.span(),
                    ),
                }
            } else if fixed
                .as_ref()
                .is_some_and(|f| !integer_representable(next, f))
            {
                self.diagnostics
                    .borrow_mut()
                    .error("overflow in enumeration values".to_string(), variant.span);
            }
            // The i64 carrier: an unsigned 64-bit value keeps its bits.
            let value = next as i64;
            let constant = match &fixed_enum {
                Some(ty) => EnumConstant {
                    value,
                    ty: ty.clone(),
                },
                None => EnumConstant::provisional(value),
            };
            // Use insert_enum_scoped so that enum constants defined inside
            // function bodies are properly removed when the scope exits.
            // At global scope the scope stack is empty, so insert_enum_scoped
            // behaves identically to a direct insert.
            self.symbol_table.declare(Symbol {
                name: variant.name.clone(),
                ty: constant.ty.clone(),
                explicit_alignment: None,
                type_align: None,
                address_space: AddressSpace::Default,
            });
            self.symbol_table.mark_enumerator(&variant.name);
            self.result
                .type_context
                .insert_enum_scoped(variant.name.clone(), constant);
            values.push((variant.name.clone(), value));
            next = if fixed.is_some() {
                next + 1
            } else {
                i128::from(value.wrapping_add(1))
            };
        }
        let enum_type = crate::common::types::EnumType {
            name: name.clone(),
            variants: values,
            is_packed,
            fixed: fixed.map(Box::new),
        };
        let ty = self
            .result
            .type_context
            .complete_enum_constants(&enum_type, true);
        for (variant, _) in &enum_type.variants {
            self.symbol_table.declare(Symbol {
                name: variant.clone(),
                ty: ty.clone(),
                explicit_alignment: None,
                type_align: None,
                address_space: AddressSpace::Default,
            });
            self.symbol_table.mark_enumerator(variant);
        }
        enum_type
    }

    /// The fixed underlying type of an enum specifier that defines
    /// (`defines`) or forward-declares (`enum E : T;`) its tag, after the C23
    /// 6.7.2.2 checks against the tag's declaration in this scope (an inner
    /// scope's `enum E` is a new type). GCC 16.2 wording.
    fn enum_fixed_type(
        &mut self,
        tag: Option<&str>,
        underlying: Option<&TypeSpecifier>,
        defines: bool,
        span: Span,
    ) -> Option<CType> {
        let mut errors = Vec::new();
        let fixed = underlying.and_then(|u| {
            let ty = crate::common::types::EnumType::fixed_underlying(self.type_spec_to_ctype(u));
            if ty.is_none() {
                errors.push("invalid 'enum' underlying type");
            }
            ty
        });
        let previous = tag
            .and_then(|t| self.result.type_context.enum_type_in_current_scope(t))
            .map(|et| et.fixed.as_deref().cloned());
        match (previous, &fixed) {
            (Some(Some(p)), Some(f)) if p != *f => {
                errors.push("'enum' underlying type incompatible with previous declaration");
            }
            (Some(Some(_)), None) if defines && underlying.is_none() => {
                errors.push("'enum' declared with but defined without fixed underlying type");
            }
            (Some(None), Some(_)) => {
                errors.push("'enum' declared both with and without fixed underlying type");
            }
            _ => {}
        }
        let mut diagnostics = self.diagnostics.borrow_mut();
        for message in errors {
            diagnostics.error(message.to_string(), span);
        }
        fixed
    }

    /// Recursively collect enum constants from a type specifier, walking into
    /// struct/union field types. This ensures that inline enum definitions
    /// within structs (e.g., `struct { enum { A, B } mode; }`) have their
    /// constants registered in the enclosing scope, matching GCC/Clang behavior.
    fn collect_enum_constants_from_type_spec(&mut self, ts: &TypeSpecifier, span: Span) {
        match ts {
            TypeSpecifier::Enum(name, Some(variants), is_packed, underlying) => {
                let fixed =
                    self.enum_fixed_type(name.as_deref(), underlying.as_deref(), true, span);
                let enum_type = self.process_enum_variants(name, variants, *is_packed, fixed);
                // Register EVERY completed named body -- packed or not -- for
                // all subsequent body-less references: a reference resolves
                // through `TypeContext::enum_types`, so it gets the same values
                // and fixed type, and therefore the same compatible integer type
                // (C11 6.7.2.2p4), as the definition (a reference to a
                // non-packed enum once carried no values and reported `unsigned
                // int` for `enum md_submodule_id`, whose first enumerator is -1,
                // rejecting the kernel's `kstrtoint(buf, 10, &submodule->id)` in
                // drivers/md/md.c). The tag map is scoped so a nested
                // same-spelling enum does not leak past its C tag scope.
                if let Some(tag) = name {
                    self.result
                        .type_context
                        .insert_enum_type_scoped(tag.clone(), enum_type);
                }
            }
            // C23 `enum E : T;` declares the tag, complete with its fixed type.
            TypeSpecifier::Enum(Some(tag), None, is_packed, Some(underlying)) => {
                let fixed = self.enum_fixed_type(Some(tag), Some(underlying), false, span);
                if fixed.is_some()
                    && self
                        .result
                        .type_context
                        .enum_type_in_current_scope(tag)
                        .is_none()
                {
                    self.result.type_context.insert_enum_type_scoped(
                        tag.clone(),
                        crate::common::types::EnumType {
                            name: Some(tag.clone()),
                            variants: Vec::new(),
                            is_packed: *is_packed,
                            fixed: fixed.map(Box::new),
                        },
                    );
                }
            }
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => {
                for field in fields {
                    self.collect_enum_constants_from_type_spec(&field.type_spec, span);
                }
            }
            TypeSpecifier::Array(inner, _) | TypeSpecifier::Pointer(inner, _) => {
                self.collect_enum_constants_from_type_spec(inner, span);
            }
            _ => {}
        }
    }

    /// A tagged type defined inside an expression's type-name (`sizeof(enum
    /// { E = 30 })`, a cast, a compound literal, `va_arg`, `_Alignof`,
    /// `__builtin_types_compatible_p`, a `_Generic` association) declares its
    /// enumerators in the enclosing scope from that point on (C11 6.2.1p7),
    /// exactly as the same specifier does in a declaration (GCC 16.2).
    fn declare_type_name_tags(&mut self, expr: &Expr) {
        let span = expr.span();
        expr.for_each_type_name(&mut |ts| self.collect_enum_constants_from_type_spec(ts, span));
    }

    // === Statement analysis ===

    fn warn_unused_scope(&mut self) {
        for (name, span) in self.symbol_table.unused_local_objects() {
            self.diagnostics.borrow_mut().warning_with_kind(
                format!("unused variable '{name}'"),
                span,
                crate::common::error::WarningKind::UnusedVariable,
            );
        }
    }

    fn analyze_compound_stmt(&mut self, compound: &CompoundStmt) {
        self.symbol_table.push_scope();
        self.result.type_context.push_scope();
        for item in &compound.items {
            match item {
                BlockItem::Declaration(decl) => {
                    self.analyze_declaration(decl, /* is_global */ false);
                }
                BlockItem::Statement(stmt) => {
                    self.analyze_stmt(stmt);
                }
                BlockItem::NestedFunction(nested) => {
                    self.analyze_nested_function_def(nested);
                }
                BlockItem::StaticAssert(assertion) => self.check_static_assert(assertion),
            }
        }
        self.warn_unused_scope();
        self.result.type_context.pop_scope();
        self.symbol_table.pop_scope();
    }

    /// Decide a `_Static_assert` the parser left open because its condition
    /// needs types (`sizeof`, `_Generic`, enumerator types). Evaluated at its
    /// lexical position, so the enumerators, typedefs and tags in scope there
    /// are the ones it sees. A condition that does not fold is rejected as
    /// not constant, so an evaluator gap rejects a valid program: every
    /// constant form the oracles fold must fold here (`offsetof` of an
    /// anonymous record included -- `tests/regression/c23_attrs.c`).
    /// C11 6.7.10: the condition is an integer constant expression; GCC
    /// 16.2 (followed here) asks only that it have integer type and fold,
    /// with enumerators and C23 `constexpr` objects the only named values
    /// (`2.0 > 1` and `constexpr double d = 2; ... d == 2` pass; strict form
    /// is a `-pedantic` matter there), and words the three failures apart.
    fn check_static_assert(&mut self, assertion: &StaticAssert) {
        let cond = &assertion.cond;
        self.analyze_expr(cond);
        let integer = self
            .expr_type_checker()
            .infer_expr_ctype(cond)
            .is_none_or(|ty| ty.is_integer());
        let (message, span) = if !integer {
            (
                "expression in static assertion is not an integer".to_string(),
                cond.span(),
            )
        } else {
            let evaluator = SemaConstEval {
                const_values: None,
                named_constants_only: true,
                ..self.sema_const_eval()
            };
            match evaluator.eval_const_expr(cond).map(|v| v.is_nonzero()) {
                Some(true) => return,
                None => (
                    "expression in static assertion is not constant".to_string(),
                    cond.span(),
                ),
                Some(false) => (
                    match &assertion.message {
                        Some(m) => format!("static assertion failed: {m}"),
                        None => "static assertion failed".to_string(),
                    },
                    assertion.span,
                ),
            }
        };
        self.diagnostics.borrow_mut().error(message, span);
    }

    /// A selection or iteration statement is a block, and so is each of its
    /// substatements (C11 6.8.4p3, 6.8.5p5): an identifier or tag declared
    /// in one -- an enumerator or tag of a type-name -- ends with it. `for`
    /// opens its scope itself (its clause may declare objects). Mirrored by
    /// lowering's `lower_stmt`.
    fn analyze_stmt(&mut self, stmt: &Stmt) {
        if matches!(
            stmt,
            Stmt::If(..) | Stmt::While(..) | Stmt::DoWhile(..) | Stmt::Switch(..)
        ) {
            self.symbol_table.push_scope();
            self.result.type_context.push_scope();
            self.analyze_stmt_in_scope(stmt);
            self.warn_unused_scope();
            self.result.type_context.pop_scope();
            self.symbol_table.pop_scope();
        } else {
            self.analyze_stmt_in_scope(stmt);
        }
    }

    /// Analyze a substatement of a selection or iteration statement in a
    /// scope of its own (a compound statement opens its own).
    fn analyze_substatement(&mut self, stmt: &Stmt) {
        if matches!(stmt, Stmt::Compound(_)) {
            self.analyze_stmt(stmt);
        } else {
            self.symbol_table.push_scope();
            self.result.type_context.push_scope();
            self.analyze_stmt(stmt);
            self.warn_unused_scope();
            self.result.type_context.pop_scope();
            self.symbol_table.pop_scope();
        }
    }

    fn analyze_stmt_in_scope(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Expr(Some(expr)) => {
                self.analyze_expr(expr);
            }
            Stmt::Expr(None) => {}
            Stmt::Return(Some(expr), span) => {
                self.analyze_expr(expr);
                if let Some(expected) = self.current_return_type.clone() {
                    if matches!(expected, CType::Void) {
                        // GCC compatibility: `return void_expr;` in a void
                        // function is accepted silently in GNU mode (glibc
                        // uses `return __set_errno(...)`/`return helper();`
                        // shims throughout malloc.c, qsort.c, libio).
                        // A non-void value uses the dialect-dependent
                        // return-mismatch class, just like a bare return
                        // in a non-void function.
                        let checker = self.expr_type_checker();
                        let expr_is_void =
                            matches!(checker.infer_expr_ctype(expr), Some(CType::Void) | None);
                        if !expr_is_void {
                            self.diagnostics.borrow_mut().warning_with_kind(
                                "'return' with a value, in function returning void",
                                expr.span(),
                                crate::common::error::WarningKind::ReturnMismatch,
                            );
                        }
                    } else {
                        let checker = self.expr_type_checker();
                        if self.check_assignment_operand(
                            expr,
                            &expected,
                            *span,
                            IConvContext::Returning,
                        ) {
                            // Diagnosed; the checks below would only repeat it.
                        } else if let Some(actual) = checker.infer_expr_ctype(expr) {
                            let actual = self.conversion_source_ctype(expr, actual);
                            let null_constant = self.is_void_null_pointer_constant(expr);
                            self.check_assignment_compatibility(
                                &actual,
                                &expected,
                                *span,
                                matches!(expr, Expr::Cast(..)),
                                null_constant,
                            );
                        }
                    }
                }
            }
            Stmt::Return(None, span) => {
                // The bare-return subcase has dialect-dependent severity
                // and pre-C99 enablement, distinct from returning a value
                // — gcc.c-torture/execute/920415-1.c relies on this in gnu89
                // code. The lowered function returns an undefined value.
                if self
                    .current_return_type
                    .as_ref()
                    .is_some_and(|t| !matches!(t, CType::Void))
                {
                    self.diagnostics.borrow_mut().missing_return_value(*span);
                }
            }
            Stmt::If(cond, then_br, else_br, _) => {
                self.analyze_expr(cond);
                self.check_scalar_condition(cond);
                self.analyze_substatement(then_br);
                if let Some(else_stmt) = else_br {
                    self.analyze_substatement(else_stmt);
                }
            }
            Stmt::While(cond, body, _) => {
                let names = std::mem::take(&mut self.pending_loop_labels);
                self.active_break_targets.push(names.clone());
                self.active_continue_targets.push(names);
                self.analyze_expr(cond);
                self.check_scalar_condition(cond);
                self.analyze_substatement(body);
                self.active_continue_targets.pop();
                self.active_break_targets.pop();
            }
            Stmt::DoWhile(body, cond, _) => {
                let names = std::mem::take(&mut self.pending_loop_labels);
                self.active_break_targets.push(names.clone());
                self.active_continue_targets.push(names);
                self.analyze_substatement(body);
                self.analyze_expr(cond);
                self.check_scalar_condition(cond);
                self.active_continue_targets.pop();
                self.active_break_targets.pop();
            }
            Stmt::For(init, cond, inc, body, _) => {
                let names = std::mem::take(&mut self.pending_loop_labels);
                self.active_break_targets.push(names.clone());
                self.active_continue_targets.push(names);
                self.symbol_table.push_scope();
                self.result.type_context.push_scope();
                if let Some(init) = init {
                    match init.as_ref() {
                        ForInit::Declaration(decl) => {
                            self.analyze_declaration(decl, false);
                        }
                        ForInit::Expr(expr) => {
                            self.analyze_expr(expr);
                        }
                    }
                }
                if let Some(cond) = cond {
                    self.analyze_expr(cond);
                    self.check_scalar_condition(cond);
                }
                if let Some(inc) = inc {
                    self.analyze_expr(inc);
                }
                self.analyze_substatement(body);
                self.warn_unused_scope();
                self.result.type_context.pop_scope();
                self.symbol_table.pop_scope();
                self.active_continue_targets.pop();
                self.active_break_targets.pop();
            }
            Stmt::Compound(compound) => {
                if !compound.is_synthetic_init_scope {
                    self.pending_loop_labels.clear();
                }
                self.analyze_compound_stmt(compound);
            }
            Stmt::Switch(expr, body, _) => {
                let names = std::mem::take(&mut self.pending_loop_labels);
                self.active_break_targets.push(names);
                self.analyze_expr(expr);
                // C11 6.8.4.2: The controlling expression of a switch statement
                // shall have integer type.
                if let Some(ctype) = self.result.expr_types.get(&expr.id()) {
                    if !ctype.is_integer() {
                        let diag = crate::common::error::Diagnostic::error(
                            "switch quantity is not an integer",
                        )
                        .with_span(expr.span())
                        .with_note(
                            crate::common::error::Diagnostic::note(format!(
                                "expression has type '{}'",
                                ctype
                            ))
                            .with_span(expr.span()),
                        );
                        self.diagnostics.borrow_mut().emit(&diag);
                    }
                }
                self.analyze_substatement(body);
                self.active_break_targets.pop();
            }
            Stmt::Case(expr, body, _) => {
                self.analyze_expr(expr);
                self.analyze_stmt(body);
            }
            Stmt::CaseRange(low, high, body, _) => {
                self.analyze_expr(low);
                self.analyze_expr(high);
                self.analyze_stmt(body);
            }
            Stmt::Default(body, _) => {
                self.analyze_stmt(body);
            }
            Stmt::Label(name, body, _) => {
                if body.labels_loop_or_switch() {
                    self.pending_loop_labels.push(name.clone());
                } else {
                    self.pending_loop_labels.clear();
                }
                self.analyze_stmt(body);
                self.pending_loop_labels.clear();
            }
            Stmt::Declaration(decl) => {
                self.analyze_declaration(decl, /* is_global */ false);
            }
            Stmt::Break(target, span) => {
                if let Some(lbl) = target {
                    let ok = self
                        .active_break_targets
                        .iter()
                        .any(|names| names.iter().any(|n| n == lbl));
                    if !ok {
                        self.diagnostics.borrow_mut().error(
                            format!(
                                "break statement label '{}' does not name an enclosing loop or switch",
                                lbl
                            ),
                            *span,
                        );
                    }
                }
            }
            Stmt::Continue(target, span) => {
                if let Some(lbl) = target {
                    let ok = self
                        .active_continue_targets
                        .iter()
                        .any(|names| names.iter().any(|n| n == lbl));
                    if !ok {
                        self.diagnostics.borrow_mut().error(
                            format!(
                                "continue statement label '{}' does not name an enclosing loop",
                                lbl
                            ),
                            *span,
                        );
                    }
                }
            }
            Stmt::Goto(_, _) => {}
            Stmt::GotoIndirect(expr, _) => {
                self.analyze_expr(expr);
            }
            Stmt::InlineAsm {
                outputs, inputs, ..
            } => {
                for out in outputs {
                    self.analyze_expr(&out.expr);
                }
                for inp in inputs {
                    self.analyze_expr(&inp.expr);
                }
            }
        }
    }

    // === Return-type analysis (-Wreturn-type) ===

    /// Check whether a compound statement (function body) can fall through
    /// without executing a return statement. Used by `-Wreturn-type` to detect
    /// non-void functions that may not return a value.
    ///
    /// Returns `true` if control can reach the end of the compound statement,
    /// `false` if all paths through the block are guaranteed to return/diverge.
    fn compound_can_fall_through(&self, compound: &CompoundStmt) -> bool {
        if compound.items.is_empty() {
            return true;
        }
        // Check each item - if any item doesn't fall through, the rest is unreachable
        for item in &compound.items {
            let falls_through = match item {
                BlockItem::Statement(stmt) => self.stmt_can_fall_through(stmt),
                BlockItem::Declaration(_)
                | BlockItem::NestedFunction(_)
                | BlockItem::StaticAssert(_) => true,
            };
            if !falls_through {
                return false;
            }
        }
        true
    }

    /// Check whether a statement can fall through (i.e., control can reach the
    /// point immediately after this statement without a return/goto/diverge).
    fn stmt_can_fall_through(&self, stmt: &Stmt) -> bool {
        match stmt {
            // return always diverges
            Stmt::Return(_, _) => false,

            // goto always diverges (transfers control elsewhere)
            Stmt::Goto(_, _) | Stmt::GotoIndirect(_, _) => false,

            // break/continue diverge from the current construct
            // (conservative for our purposes: they exit the current statement)
            Stmt::Break(..) | Stmt::Continue(..) => false,

            // if/else: falls through only if either branch can fall through
            // if without else: can fall through unless the condition is constant true
            // and the then-branch diverges (e.g., `if(1) noreturn_call()`)
            Stmt::If(cond, then_br, else_br, _) => {
                match else_br {
                    Some(else_stmt) => {
                        // Both branches must not fall through for the if/else to not fall through
                        self.stmt_can_fall_through(then_br) || self.stmt_can_fall_through(else_stmt)
                    }
                    None => {
                        // if without else: can fall through unless condition is constant true
                        // and the body diverges (handles `if(1) abort()` and BUILD_BUG patterns)
                        !self.is_constant_true_expr(cond) || self.stmt_can_fall_through(then_br)
                    }
                }
            }

            // while/do-while/for: conservatively can fall through, unless the
            // condition is a compile-time constant true (infinite loop), or the
            // body itself diverges (contains __builtin_unreachable(), etc.)
            Stmt::While(cond, _body, _) => {
                if self.is_constant_true_expr(cond) {
                    false // infinite loop
                } else {
                    true // might not execute body at all, so can fall through
                }
            }
            Stmt::DoWhile(body, cond, _) => {
                // Can fall through only if not an infinite loop AND body can fall through
                !self.is_constant_true_expr(cond) && self.stmt_can_fall_through(body)
            }
            Stmt::For(_init, cond, _inc, _body, _) => {
                // for(;;) with no condition is an infinite loop
                match cond {
                    None => false, // for(;;) is infinite
                    Some(c) => !self.is_constant_true_expr(c),
                }
            }

            // switch: can fall through unless it has a default case and every
            // case segment ends with a return/goto/diverge (not break/fallthrough).
            Stmt::Switch(_, body, _) => self.switch_can_fall_through(body),

            // compound: delegate to compound analysis
            Stmt::Compound(compound) => self.compound_can_fall_through(compound),

            // labels: the relevant question is whether the labeled statement falls through
            Stmt::Label(_, inner, _) => self.stmt_can_fall_through(inner),
            Stmt::Case(_, inner, _) => self.stmt_can_fall_through(inner),
            Stmt::CaseRange(_, _, inner, _) => self.stmt_can_fall_through(inner),
            Stmt::Default(inner, _) => self.stmt_can_fall_through(inner),

            // expression statements: fall through unless they call a noreturn function
            Stmt::Expr(Some(expr)) => !self.is_noreturn_call(expr),
            Stmt::Expr(None) => true,
            Stmt::Declaration(_) => true,
            Stmt::InlineAsm { .. } => true,
        }
    }

    /// Check whether a switch statement can fall through to the next statement.
    ///
    /// A switch cannot fall through if:
    /// 1. It has a `default` label (so all values are covered), AND
    /// 2. No case segment contains a `break` that exits the switch, AND
    /// 3. The last segment in the compound body doesn't fall through.
    ///
    /// Segments that fall through to the next case (without break) are allowed,
    /// since control just continues to the next case label.
    fn switch_can_fall_through(&self, body: &Stmt) -> bool {
        let compound = match body {
            Stmt::Compound(c) => c,
            // Non-compound switch body: conservatively say it can fall through
            _ => return true,
        };

        if compound.items.is_empty() {
            return true;
        }

        // Must have a default label to cover all values
        if !self.switch_has_default(&compound.items) {
            return true;
        }

        // Collect case segment boundaries
        let mut segment_starts: Vec<usize> = Vec::new();
        for (i, item) in compound.items.iter().enumerate() {
            if let BlockItem::Statement(stmt) = item {
                if self.is_case_label(stmt) {
                    segment_starts.push(i);
                }
            }
        }

        if segment_starts.is_empty() {
            return true;
        }

        // Check each segment:
        // - If ANY segment ends with a break, the switch falls through
        // - If the LAST segment falls through, the switch falls through
        for (seg_idx, &start) in segment_starts.iter().enumerate() {
            let end = if seg_idx + 1 < segment_starts.len() {
                segment_starts[seg_idx + 1]
            } else {
                compound.items.len()
            };

            let segment = &compound.items[start..end];
            let outcome = self.segment_outcome(segment);
            match outcome {
                SwitchSegmentOutcome::Returns => {
                    // This segment returns/diverges - doesn't cause fallthrough
                }
                SwitchSegmentOutcome::Breaks => {
                    // This segment breaks out of the switch - switch falls through
                    return true;
                }
                SwitchSegmentOutcome::FallsThrough => {
                    // Falls through to next segment. Only a problem if this is the last segment.
                    if seg_idx + 1 >= segment_starts.len() {
                        return true;
                    }
                    // Otherwise, fallthrough to next case is fine (C semantics)
                }
            }
        }

        false
    }

    /// Determine the outcome of a case segment: does it return, break, or fall through?
    ///
    /// Walks items sequentially: if an earlier item diverges (returns/breaks),
    /// subsequent items are dead code and don't affect the outcome. This handles
    /// patterns like `return NULL; break;` where the break is unreachable.
    fn segment_outcome(&self, segment: &[BlockItem]) -> SwitchSegmentOutcome {
        if segment.is_empty() {
            return SwitchSegmentOutcome::FallsThrough;
        }

        for item in segment {
            let outcome = match item {
                BlockItem::Declaration(_)
                | BlockItem::NestedFunction(_)
                | BlockItem::StaticAssert(_) => continue,
                BlockItem::Statement(stmt) => {
                    let inner = self.unwrap_case_label(stmt);
                    self.stmt_switch_outcome(inner)
                }
            };
            // If this item returns or breaks, subsequent items are dead code
            if outcome != SwitchSegmentOutcome::FallsThrough {
                return outcome;
            }
        }
        SwitchSegmentOutcome::FallsThrough
    }

    /// Determine what a statement does in a switch context: return, break, or fall through.
    fn stmt_switch_outcome(&self, stmt: &Stmt) -> SwitchSegmentOutcome {
        match stmt {
            Stmt::Return(_, _) | Stmt::Goto(_, _) | Stmt::GotoIndirect(_, _) => {
                SwitchSegmentOutcome::Returns
            }
            Stmt::Break(..) => SwitchSegmentOutcome::Breaks,
            Stmt::Continue(..) => SwitchSegmentOutcome::Returns,

            Stmt::Compound(compound) => {
                // Walk items sequentially: if an item diverges (returns/breaks),
                // subsequent items are dead code. This handles `return x; break;`.
                for item in &compound.items {
                    let outcome = match item {
                        BlockItem::Declaration(_)
                        | BlockItem::NestedFunction(_)
                        | BlockItem::StaticAssert(_) => continue,
                        BlockItem::Statement(s) => self.stmt_switch_outcome(s),
                    };
                    if outcome != SwitchSegmentOutcome::FallsThrough {
                        return outcome;
                    }
                }
                SwitchSegmentOutcome::FallsThrough
            }

            Stmt::If(_, then_br, Some(else_stmt), _) => {
                let then_out = self.stmt_switch_outcome(then_br);
                let else_out = self.stmt_switch_outcome(else_stmt);
                if then_out == SwitchSegmentOutcome::Returns
                    && else_out == SwitchSegmentOutcome::Returns
                {
                    SwitchSegmentOutcome::Returns
                } else if then_out == SwitchSegmentOutcome::Breaks
                    || else_out == SwitchSegmentOutcome::Breaks
                {
                    SwitchSegmentOutcome::Breaks
                } else {
                    SwitchSegmentOutcome::FallsThrough
                }
            }
            Stmt::If(..) => SwitchSegmentOutcome::FallsThrough,

            Stmt::While(cond, _, _) => {
                if self.is_constant_true_expr(cond) {
                    SwitchSegmentOutcome::Returns
                } else {
                    SwitchSegmentOutcome::FallsThrough
                }
            }
            Stmt::DoWhile(body, cond, _) => {
                // Returns if: infinite loop OR body diverges
                if self.is_constant_true_expr(cond) || !self.stmt_can_fall_through(body) {
                    SwitchSegmentOutcome::Returns
                } else {
                    SwitchSegmentOutcome::FallsThrough
                }
            }
            Stmt::For(_, cond, _, _, _) => match cond {
                None => SwitchSegmentOutcome::Returns,
                Some(c) => {
                    if self.is_constant_true_expr(c) {
                        SwitchSegmentOutcome::Returns
                    } else {
                        SwitchSegmentOutcome::FallsThrough
                    }
                }
            },

            Stmt::Label(_, inner, _)
            | Stmt::Case(_, inner, _)
            | Stmt::CaseRange(_, _, inner, _)
            | Stmt::Default(inner, _) => self.stmt_switch_outcome(inner),

            Stmt::Switch(_, inner_body, _) => {
                if self.switch_can_fall_through(inner_body) {
                    SwitchSegmentOutcome::FallsThrough
                } else {
                    SwitchSegmentOutcome::Returns
                }
            }

            Stmt::Expr(Some(expr)) => {
                if self.is_noreturn_call(expr) {
                    SwitchSegmentOutcome::Returns
                } else {
                    SwitchSegmentOutcome::FallsThrough
                }
            }

            _ => SwitchSegmentOutcome::FallsThrough,
        }
    }

    /// Unwrap case/default label wrappers to get to the inner statement.
    fn unwrap_case_label<'b>(&self, stmt: &'b Stmt) -> &'b Stmt {
        match stmt {
            Stmt::Case(_, inner, _) | Stmt::CaseRange(_, _, inner, _) | Stmt::Default(inner, _) => {
                self.unwrap_case_label(inner)
            }
            other => other,
        }
    }

    /// Check if a statement is a case/default label.
    fn is_case_label(&self, stmt: &Stmt) -> bool {
        matches!(
            stmt,
            Stmt::Case(_, _, _) | Stmt::CaseRange(_, _, _, _) | Stmt::Default(_, _)
        )
    }

    /// Check if any item in a switch body contains a `default` label.
    fn switch_has_default(&self, items: &[BlockItem]) -> bool {
        for item in items {
            if let BlockItem::Statement(stmt) = item {
                if self.stmt_contains_default(stmt) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a statement is or contains a `default` label (handles nested case labels).
    fn stmt_contains_default(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Default(_, _) => true,
            Stmt::Case(_, inner, _) | Stmt::CaseRange(_, _, inner, _) => {
                self.stmt_contains_default(inner)
            }
            _ => false,
        }
    }

    /// Check if an expression diverges (never falls through to subsequent code).
    /// This includes calls to noreturn functions (`abort()`, `exit()`,
    /// `__builtin_unreachable()`, or any function declared with
    /// `__attribute__((noreturn))`), and GCC statement expressions whose
    /// compound body cannot fall through (e.g. `({ return val; })`).
    /// The record a call to `name` resolves to *here*, honouring a block-scope
    /// declaration that shadows the file-scope entity of the same basename.
    ///
    /// Nested entities live under `parent.name`, and `parent` is by construction
    /// the function being analyzed, so the scoped key is exact rather than a
    /// guess. The binding must be a function declared below file scope: a local
    /// of pointer-to-function type is also a block-scope binding, and must not
    /// redirect the query. A block-scope `extern` declaration is *not* a distinct
    /// entity (C11 6.7.1p5) and keeps reading the file-scope record, which is
    /// what the fallback does.
    /// The mangled record of a nested entity that `name` resolves to here, if
    /// the innermost binding of `name` is a function declared below file scope.
    fn scoped_call_entity(&self, name: &str) -> Option<String> {
        let shadowed_by_nested_binding = matches!(
            self.symbol_table.lookup(name).map(|sym| &sym.ty),
            Some(CType::Function(_))
        ) && self.symbol_table.lookup_depth(name).unwrap_or(0) > 0;
        if !shadowed_by_nested_binding {
            return None;
        }
        let parent = self.function_nesting_stack.last()?;
        let key = format!("{parent}.{name}");
        self.result.functions.contains_key(&key).then_some(key)
    }

    fn visible_function_info(&self, name: &str) -> Option<&FunctionInfo> {
        let shadowed_by_nested_binding = matches!(
            self.symbol_table.lookup(name).map(|sym| &sym.ty),
            Some(CType::Function(_))
        ) && self.symbol_table.lookup_depth(name).unwrap_or(0) > 0;
        if shadowed_by_nested_binding {
            if let Some(parent) = self.function_nesting_stack.last() {
                if let Some(info) = self.result.functions.get(&format!("{parent}.{name}")) {
                    return Some(info);
                }
            }
        }
        self.result.functions.get(name)
    }

    fn is_noreturn_call(&self, expr: &Expr) -> bool {
        match expr {
            Expr::FunctionCall(callee, _, _) => {
                if let Expr::Identifier(name, _) = callee.as_ref() {
                    // Check built-in noreturn functions
                    if matches!(
                        name.as_str(),
                        "__builtin_unreachable"
                            | "__builtin_return"
                            | "__builtin_longjmp"
                            | "__builtin_trap"
                            | "__builtin_abort"
                            | "abort"
                            | "exit"
                            | "_exit"
                            | "_Exit"
                            | "quick_exit"
                            | "__assert_fail"
                            | "__assert_rtn"
                            | "longjmp"
                            | "siglongjmp"
                            | "__longjmp_chk"
                    ) {
                        return true;
                    }
                    // Check user-declared noreturn functions. Through the
                    // *visible binding*, not by bare name: a nested `auto`
                    // function that returns, shadowing a file-scope `noreturn`
                    // function of the same basename, was classified noreturn
                    // here -- and the fallthrough and switch-outcome analyses
                    // then treated the code after the call as dead (and silenced
                    // the missing-return diagnostic). The converse hid a real
                    // noreturn nested call.
                    match self.result.call_entities.get(&ExprId::of(expr)) {
                        Some(key) => self
                            .result
                            .functions
                            .get(key)
                            .map_or(false, |func_info| func_info.is_noreturn),
                        None => self
                            .visible_function_info(name)
                            .map_or(false, |func_info| func_info.is_noreturn),
                    }
                } else {
                    false
                }
            }
            // Handle comma expressions: (void)0, __builtin_unreachable()
            Expr::Comma(_, rhs, _) => self.is_noreturn_call(rhs),
            // Handle cast expressions: (void)noreturn_call()
            Expr::Cast(_, inner, _, _) => self.is_noreturn_call(inner),
            // Handle GCC statement expressions: ({ ...; return val; })
            // A return inside a statement expression returns from the enclosing
            // function, so if the compound body cannot fall through, the
            // expression itself diverges.
            Expr::StmtExpr(compound, _) => !self.compound_can_fall_through(compound),
            _ => false,
        }
    }

    /// Check if an expression is a compile-time constant that evaluates to true (non-zero).
    /// Used to detect infinite loops like `while(1)` and `for(;1;)`, and for
    /// `if(1)` patterns like `if(!(0)) noreturn_call()` in BUILD_BUG macros.
    fn is_constant_true_expr(&self, expr: &Expr) -> bool {
        self.try_eval_constant_bool(expr).unwrap_or_default()
    }

    /// Try to evaluate an expression as a compile-time boolean (true=non-zero, false=zero).
    /// Returns None if the expression is not a compile-time constant.
    fn try_eval_constant_bool(&self, expr: &Expr) -> Option<bool> {
        match expr {
            Expr::IntLiteral(val, _) => Some(*val != 0),
            Expr::UIntLiteral(val, _) => Some(*val != 0),
            Expr::LongLiteral(val, _) => Some(*val != 0),
            Expr::ULongLiteral(val, _) => Some(*val != 0),
            Expr::LongLongLiteral(val, _) => Some(*val != 0),
            Expr::ULongLongLiteral(val, _) => Some(*val != 0),
            // !expr: invert the constant
            Expr::UnaryOp(crate::frontend::parser::ast::UnaryOp::LogicalNot, inner, _) => {
                self.try_eval_constant_bool(inner).map(|v| !v)
            }
            // Cast to another type preserves truthiness
            Expr::Cast(_, inner, _, _) => self.try_eval_constant_bool(inner),
            // Enum constants (e.g., kernel's `true` defined as `enum { false = 0, true = 1 }`)
            Expr::Identifier(name, _) => self
                .result
                .type_context
                .enum_constants
                .get(name)
                .map(|constant| constant.value != 0),
            _ => None,
        }
    }

    // === Expression analysis ===

    fn analyze_expr(&mut self, expr: &Expr) {
        self.declare_type_name_tags(expr);
        match expr {
            Expr::Identifier(name, span) => {
                self.symbol_table.mark_used(name);
                if name == "__func__" && self.symbol_table.lookup(name).is_none() {
                    self.diagnostics.borrow_mut().warning(
                        "'__func__' is not defined outside of function scope".to_string(),
                        *span,
                    );
                }
                // Check if it's a known symbol
                if self.symbol_table.lookup(name).is_none()
                    && !self.result.type_context.enum_constants.contains_key(name)
                    && !builtins::is_builtin(name)
                    && !self.result.functions.contains_key(name)
                    && !crate::frontend::parser::ast::is_function_name_builtin(name)
                {
                    self.diagnostics
                        .borrow_mut()
                        .error(format!("'{}' undeclared", name), *span);
                }
            }
            Expr::FunctionCall(callee, args, _) => self.analyze_call(expr, callee, args),
            Expr::BinaryOp(op, lhs, rhs, span) => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
                // Check pointer subtraction type compatibility (C11 6.5.6p3):
                // both operands must point to compatible types.
                if *op == BinOp::Sub {
                    self.check_pointer_subtraction_compat(lhs, rhs, *span);
                }
                self.check_pointer_comparison_spaces(op, lhs, rhs, *span);
                self.warn_sign_compare(*op, lhs, rhs, *span);
                self.check_binary_operands(*op, lhs, rhs, *span);
            }
            Expr::UnaryOp(op, operand, span) => {
                self.analyze_expr(operand);
                if let Some(ty) = self.constraint_operand(operand) {
                    let message = match op {
                        UnaryOp::PreInc => operand_constraints::increment(true, &ty),
                        UnaryOp::PreDec => operand_constraints::increment(false, &ty),
                        _ => operand_constraints::unary(*op, &ty),
                    };
                    self.constraint_error(message, *span);
                }
                if matches!(op, UnaryOp::PreInc | UnaryOp::PreDec) {
                    if !self.check_ext_vector_assignable(operand, *span)
                        && !self.check_ext_vector_step(operand, *op == UnaryOp::PreInc, *span)
                    {
                        self.check_lvalue_operand(operand, *span);
                        let verb = if *op == UnaryOp::PreInc {
                            "increment"
                        } else {
                            "decrement"
                        };
                        self.check_readonly_mutation(operand, *span, verb);
                    }
                }
            }
            Expr::PostfixOp(op, operand, span) => {
                self.analyze_expr(operand);
                if let Some(ty) = self.constraint_operand(operand) {
                    let message = operand_constraints::increment(*op == PostfixOp::PostInc, &ty);
                    self.constraint_error(message, *span);
                }
                if !self.check_ext_vector_assignable(operand, *span)
                    && !self.check_ext_vector_step(operand, *op == PostfixOp::PostInc, *span)
                {
                    self.check_lvalue_operand(operand, *span);
                    let verb = if *op == PostfixOp::PostInc {
                        "increment"
                    } else {
                        "decrement"
                    };
                    self.check_readonly_mutation(operand, *span, verb);
                }
            }
            Expr::Assign(lhs, rhs, span) => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
                if !self.check_ext_vector_assignable(lhs, *span) {
                    self.check_lvalue_operand(lhs, *span);
                }
                self.check_readonly_mutation(lhs, *span, "assignment");
                // Check for invalid pointer <-> float conversions in assignment
                let checker = self.expr_type_checker();
                if let (Some(lhs_ty), Some(rhs_ty)) =
                    (checker.infer_expr_ctype(lhs), checker.infer_expr_ctype(rhs))
                {
                    // An operand-constraint error makes the checks below
                    // redundant (they would only repeat it).
                    if !self.check_assignment_operand(rhs, &lhs_ty, *span, IConvContext::Assignment)
                    {
                        let rhs_ty = self.conversion_source_ctype(rhs, rhs_ty);
                        let null_constant = self.is_void_null_pointer_constant(rhs);
                        self.check_assignment_compatibility(
                            &rhs_ty,
                            &lhs_ty,
                            *span,
                            matches!(&**rhs, Expr::Cast(..)),
                            null_constant,
                        );
                    }
                }
            }
            Expr::CompoundAssign(op, lhs, rhs, span) => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
                // `E1 op= E2` has the operand constraints of `E1 op E2`.
                self.check_binary_operands(*op, lhs, rhs, *span);
                if !self.check_ext_vector_assignable(lhs, *span) {
                    self.check_lvalue_operand(lhs, *span);
                }
                // GCC phrases `op=` on a const operand as an assignment.
                self.check_readonly_mutation(lhs, *span, "assignment");
            }
            Expr::Conditional(cond, then_expr, else_expr, span) => {
                self.analyze_expr(cond);
                self.analyze_expr(then_expr);
                self.analyze_expr(else_expr);
                self.check_conditional_pointer_spaces(then_expr, else_expr, *span);
                self.check_scalar_condition(cond);
                self.check_conditional_arms(then_expr, else_expr, *span);
            }
            Expr::GnuConditional(cond, else_expr, span) => {
                self.analyze_expr(cond);
                self.analyze_expr(else_expr);
                // `a ?: b` is `a ? a : b` with `a` evaluated once.
                self.check_scalar_condition(cond);
                self.check_conditional_arms(cond, else_expr, *span);
            }
            Expr::ArraySubscript(arr, idx, span) => {
                self.analyze_expr(arr);
                self.analyze_expr(idx);
                self.check_subscript_operands(arr, idx, *span);
            }
            Expr::MemberAccess(obj, field_name, span) => {
                self.analyze_expr(obj);
                self.check_member_exists(obj, field_name, *span, false);
            }
            Expr::PointerMemberAccess(obj, field_name, span) => {
                self.analyze_expr(obj);
                self.check_member_exists(obj, field_name, *span, true);
            }
            Expr::Cast(ts, inner, span, _) => {
                self.check_void_type_name(ts, *span);
                self.analyze_expr(inner);
                self.check_explicit_cast_address_space(expr, inner, *span);
                self.check_cast_operand(ts, inner, *span);
                // Scalar <-> vector casts with mismatched sizes are ill-formed
                // (GCC: "cannot convert a value of type 'int' to vector type
                // '__vector(1) double' which has different size"). Equal-size
                // casts are the documented raw reinterpretation. An unresolved
                // operand type (`inner` depends on a not-yet-inferable decl)
                // skips the check rather than risking a false positive.
                let checker = self.expr_type_checker();
                if let Some(inner_ty) = checker.infer_expr_ctype(inner) {
                    let target_ty = self.type_spec_to_ctype(ts);
                    if Self::is_vector_scalar_size_mismatch(&inner_ty, &target_ty) {
                        // GCC's two messages: scalar -> vector is "cannot
                        // convert a value of type 'int' to vector type
                        // '__vector(1) double' which has different size";
                        // vector -> scalar is "cannot convert a vector of
                        // type 'd1' to type 'int' which has different size".
                        let message = if inner_ty.is_vector() {
                            format!(
                                "cannot convert a vector of type '{}' to type '{}' which has different size",
                                inner_ty, target_ty
                            )
                        } else {
                            format!(
                                "cannot convert a value of type '{}' to vector type '{}' which has different size",
                                inner_ty, target_ty
                            )
                        };
                        self.diagnostics.borrow_mut().error(message, *span);
                    }
                }
            }
            Expr::Sizeof(arg, span) => {
                // sizeof is unevaluated context, but identifiers must still be declared
                match arg.as_ref() {
                    SizeofArg::Expr(inner) => {
                        self.analyze_expr(inner);
                    }
                    SizeofArg::Type(ts) => {
                        self.check_void_type_name(ts, *span);
                        // Check for sizeof on incomplete struct/union types.
                        // sizeof(struct X) where struct X has no definition is an error.
                        // (Pointers to incomplete types are fine.)
                        self.check_sizeof_incomplete_type(ts, *span);
                    }
                }
            }
            Expr::Countof(arg, span) => match arg.as_ref() {
                SizeofArg::Expr(inner) => {
                    self.analyze_expr(inner);
                    let is_string_lit = matches!(
                        inner,
                        Expr::StringLiteral(..)
                            | Expr::WideStringLiteral(..)
                            | Expr::Char16StringLiteral(..)
                    ) || matches!(
                        inner,
                        Expr::Identifier(name, _)
                            if crate::frontend::parser::ast::is_function_name_builtin(name)
                                && self.symbol_table.lookup(name).is_none()
                    );
                    if !is_string_lit {
                        if matches!(
                            inner,
                            Expr::Comma(..) | Expr::Conditional(..) | Expr::GnuConditional(..)
                        ) {
                            self.diagnostics.borrow_mut().error(
                                "invalid application of '_Countof' to non-array type".to_string(),
                                *span,
                            );
                        } else {
                            let ct = self.expr_type_checker().infer_expr_ctype(inner);
                            match ct {
                                Some(CType::Array(_, Some(_))) => {}
                                Some(CType::Array(_, None)) => {
                                    let is_complete_or_vla = match inner {
                                        Expr::CompoundLiteral(_, init, _, _) => {
                                            matches!(
                                                &**init,
                                                Initializer::List(_) | Initializer::Expr(_)
                                            )
                                        }
                                        Expr::Identifier(name, _) => {
                                            self.symbol_table.is_automatic_object(name)
                                        }
                                        _ => true,
                                    };
                                    if !is_complete_or_vla {
                                        self.diagnostics.borrow_mut().error(
                                                "invalid application of '_Countof' to incomplete array type"
                                                    .to_string(),
                                                *span,
                                            );
                                    }
                                }
                                Some(_) => {
                                    self.diagnostics.borrow_mut().error(
                                        "invalid application of '_Countof' to non-array type"
                                            .to_string(),
                                        *span,
                                    );
                                }
                                None => {}
                            }
                        }
                    }
                }
                SizeofArg::Type(ts) => {
                    self.check_void_type_name(ts, *span);
                    if let TypeSpecifier::Array(_, Some(bound)) = ts {
                        self.analyze_expr(bound);
                    }
                    let ct = self.type_spec_to_ctype(ts);
                    match ct {
                        CType::Array(_, Some(_)) => {}
                        CType::Array(_, None) => {
                            if matches!(ts, TypeSpecifier::Array(_, None)) {
                                self.diagnostics.borrow_mut().error(
                                    "invalid application of '_Countof' to incomplete array type"
                                        .to_string(),
                                    *span,
                                );
                            }
                        }
                        _ => {
                            self.diagnostics.borrow_mut().error(
                                "invalid application of '_Countof' to non-array type".to_string(),
                                *span,
                            );
                        }
                    }
                }
            },
            Expr::Alignof(ts, span) | Expr::GnuAlignof(ts, span) => {
                self.check_void_type_name(ts, *span);
            }
            Expr::AlignofExpr(inner, _) | Expr::GnuAlignofExpr(inner, _) => {
                self.analyze_expr(inner);
            }
            Expr::AddressOf(inner, span) => {
                self.analyze_expr(inner);
                // Clang checks the value category first: `&f()[0]` is the
                // address of an rvalue, `&v[0]` of a vector element.
                if !self.check_address_of_rvalue(inner, *span) {
                    self.check_address_of_vector_element(inner, *span);
                    self.check_address_of_register_compound_literal(inner, *span);
                }
            }
            Expr::Deref(inner, _) => {
                self.analyze_expr(inner);
            }
            Expr::Comma(lhs, rhs, _) => {
                self.analyze_expr(lhs);
                self.analyze_expr(rhs);
            }
            Expr::CompoundLiteral(ts, init, span, storage) => {
                self.analyze_compound_literal_expr(expr, ts, init, *span, storage);
            }
            Expr::StmtExpr(compound, _) => {
                self.analyze_compound_stmt(compound);
            }
            Expr::VaArg(ap_expr, ts, span) => {
                self.check_void_type_name(ts, *span);
                self.analyze_expr(ap_expr);
            }
            Expr::ConvertVector(src_expr, _, _) => {
                self.analyze_expr(src_expr);
            }
            Expr::BitCast(dst_ts, src_expr, span) => {
                self.analyze_bit_cast_expr(dst_ts, src_expr, *span);
            }
            Expr::GenericSelection(controlling, associations, span) => {
                self.analyze_expr(controlling);
                for assoc in associations {
                    self.analyze_expr(&assoc.expr);
                }
                self.check_generic_selection(controlling, associations, *span);
            }
            // Literals don't need analysis
            Expr::IntLiteral(_, _)
            | Expr::UIntLiteral(_, _)
            | Expr::LongLiteral(_, _)
            | Expr::ULongLiteral(_, _)
            | Expr::LongLongLiteral(_, _)
            | Expr::ULongLongLiteral(_, _)
            | Expr::FloatLiteral(_, _)
            | Expr::FloatLiteralF32(_, _)
            | Expr::FloatLiteralLongDouble(_, _, _)
            | Expr::FloatLiteralF128(_, _, _)
            | Expr::FloatLiteralDecimal(_, _, _)
            | Expr::FloatLiteralHalf(_, _, _)
            | Expr::ImaginaryLiteral(_, _)
            | Expr::ImaginaryLiteralF32(_, _)
            | Expr::ImaginaryLiteralLongDouble(_, _, _)
            | Expr::StringLiteral(_, _)
            | Expr::WideStringLiteral(_, _)
            | Expr::Char16StringLiteral(_, _)
            | Expr::CharLiteral(_, _) => {}
            // Label address (&&label) - just a compile-time address
            Expr::LabelAddr(_, _) => {}
            // Never evaluated; its tags are declared by
            // `declare_type_name_tags` like any type name.
            Expr::TypeOperand(..) => {}
            // __builtin_types_compatible_p(type1, type2) - compile-time constant
            Expr::BuiltinTypesCompatibleP(_, _, _, _, _) => {}
        }

        // After analyzing sub-expressions, infer this expression's CType using
        // the ExprTypeChecker and store it in the annotation map. The lowerer
        // will consult this map as a fast O(1) fallback before doing its own
        // (more expensive) type inference that requires lowering state.
        self.annotate_expr_type(expr);

        // Also try to evaluate this expression as a compile-time constant.
        // The lowerer will consult the const_values map as an O(1) fast path
        // before doing its own eval_const_expr.
        self.annotate_const_value(expr);
    }

    /// A call: builtin argument rules, implicit declarations, noreturn
    /// bookkeeping and the argument checks against the callee's prototype.
    fn analyze_call(&mut self, expr: &Expr, callee: &Expr, args: &[Expr]) {
        // Check for builtin calls
        if let Expr::Identifier(name, callee_span) = callee {
            // GCC 16.2's wording and position (the builtin's name); Clang
            // 23.1.3 agrees on every form but `1 << 31`-style signed shift
            // overflow, `(_Bool)1e300` and `INT_MIN / -1` (see `ice.rs`).
            // Lowering used to take the first arm of any condition it could
            // not fold, so `__builtin_choose_expr(__builtin_constant_p(x),
            // 100, x)` returned 100 where GCC returns x.
            if name == "__builtin_choose_expr"
                && args.len() == 3
                && self.symbol_table.lookup(name).is_none()
                && !self.is_integer_constant_expr(&args[0])
            {
                self.diagnostics.borrow_mut().error(
                    "first argument to '__builtin_choose_expr' not a constant".to_string(),
                    *callee_span,
                );
            }
            if builtins::is_builtin(name) {
                self.check_builtin_vector_arguments(name, args);
                // A builtin under its own name carries no prototype, so
                // nothing below could check the call and lowering used
                // to pass whatever the user wrote straight through to
                // the backend's operand indexing -- `_mm_add_ps()` was
                // a compiler ICE (AUD-ICE-1) where GCC says "too few
                // arguments to function '_mm_add_ps'".  The arity comes
                // from the bundled headers
                // (scripts/check_intrinsic_arity.py keeps that table in
                // step with include/*.h); a builtin whose headers do not
                // pin the count down stays unconstrained.
                //
                // Only undeclared names: once a header is included the
                // name has a prototype, the ordinary argument check
                // below already reports the mismatch, and reporting it
                // twice would be noise.
                if self.symbol_table.lookup(name).is_none()
                    && !self.result.functions.contains_key(name)
                {
                    if let Some(expected) = intrinsic_arity::builtin_intrinsic_arity(name) {
                        if args.len() != expected {
                            self.diagnostics.borrow_mut().error(
                                format!(
                                    "{} arguments to function '{}' (expected {}, have {})",
                                    if args.len() < expected {
                                        "too few"
                                    } else {
                                        "too many"
                                    },
                                    name,
                                    expected,
                                    args.len()
                                ),
                                *callee_span,
                            );
                        }
                    }
                    self.check_c2y_builtin_call(name, args, *callee_span);
                }
            } else if !self.result.functions.contains_key(name)
                && self.symbol_table.lookup(name).is_none()
            {
                // Implicit function declaration (C89 style) - register it
                self.diagnostics.borrow_mut().warning_with_kind(
                    format!("implicit declaration of function '{}'", name),
                    *callee_span,
                    crate::common::error::WarningKind::ImplicitFunctionDeclaration,
                );
                let func_info = FunctionInfo {
                    return_type: CType::Int, // implicit return int
                    params: Vec::new(),
                    variadic: true, // unknown params
                    prototype: None,
                    is_defined: false,
                    is_noreturn: false,
                    return_pointer_const_levels: 0,
                };
                self.result.functions.insert(name.clone(), func_info);
            }
        }
        // C11 6.2.1p4 settles a call's binding at the call, so that is
        // where the answer is captured.  `is_noreturn_call` is asked in two
        // places with different scoping context -- per statement during the
        // walk, and for a whole body when -Wreturn-type asks whether
        // control can fall out, by which time the declaring block's scope
        // has been unwound -- and a mutable scope stack can only answer the
        // first.  Measured on `extern void die(void) __attribute__((
        // noreturn)); int g(void) { auto void die(void) { return; } die(); }`
        // the whole-body query read the file-scope record and stayed silent
        // where GCC warns.
        if let Expr::Identifier(name, _) = callee {
            if let Some(key) = self.scoped_call_entity(name) {
                self.result.call_entities.insert(ExprId::of(expr), key);
            }
        }
        self.analyze_expr(callee);
        for arg in args {
            self.analyze_expr(arg);
        }
        if let Expr::Identifier(name, _) = callee {
            self.check_printf_format(name, args, expr.span());
        }
        // Resolve the callable through lexical scope, not through
        // the TU-wide body-metadata map. Local function pointers and
        // GNU nested functions both shadow a file-scope function.
        // K&R body declarations intentionally contribute no prototype;
        // only a separately visible declaration can supply one.
        let checker = self.expr_type_checker();
        let signature = match checker.infer_expr_ctype(callee) {
            Some(CType::Function(ft)) => Some(*ft),
            Some(CType::Pointer(inner, _)) => match *inner {
                CType::Function(ft) => Some(*ft),
                _ => None,
            },
            _ => None,
        };
        // An empty prototype (`f(void)` in every dialect, or `f()` in C23)
        // still constrains calls to zero arguments. An unprototyped function
        // retains the legacy default-argument-promotion behavior.
        if let Some(ft) = signature.filter(|ft| ft.has_prototype) {
            let name = match callee {
                Expr::Identifier(name, _) => name.as_str(),
                _ => "function pointer",
            };
            self.check_call_arguments(name, args, &ft.params, ft.variadic, expr.span());
        }
    }

    /// Check printf-family calls against a literal format string, matching
    /// GCC's ordinary `-Wformat` argument checks. Dynamic formats are checked
    /// only for the narrower `-Wformat-security` case: no variadic format
    /// arguments were supplied. General nonliteral-format checking and custom
    /// `format` attributes remain separate follow-up work.
    fn check_printf_format(&self, name: &str, args: &[Expr], call_span: Span) {
        let format_index = match name {
            "printf" | "__builtin_printf" => 0,
            "fprintf" | "__builtin_fprintf" | "dprintf" | "__builtin_dprintf" | "sprintf"
            | "__builtin_sprintf" | "asprintf" => 1,
            "snprintf" | "__builtin_snprintf" => 2,
            "__printf_chk" | "__builtin___printf_chk" => 1,
            "__fprintf_chk" | "__builtin___fprintf_chk" => 2,
            "__sprintf_chk" | "__builtin___sprintf_chk" => 3,
            "__snprintf_chk" | "__builtin___snprintf_chk" => 4,
            _ => return,
        };
        let Some(format_expr) = args.get(format_index) else {
            return;
        };
        let (format, format_span) = match format_expr {
            Expr::StringLiteral(format, format_span) => (format, format_span),
            _ => {
                let security_enabled = {
                    let diagnostics = self.diagnostics.borrow();
                    diagnostics.warning_enabled(crate::common::error::WarningKind::Format)
                        && diagnostics
                            .warning_enabled(crate::common::error::WarningKind::FormatSecurity)
                };
                if args.len() == format_index + 1 && security_enabled {
                    self.diagnostics.borrow_mut().warning_with_kind(
                        "format not a string literal and no format arguments",
                        call_span,
                        crate::common::error::WarningKind::FormatSecurity,
                    );
                }
                return;
            }
        };

        fn positional_arg_index(
            format_index: usize,
            sequential: &mut usize,
            position: Option<usize>,
        ) -> Option<usize> {
            if let Some(position) = position {
                (position > 0).then_some(format_index + position)
            } else {
                let index = *sequential;
                *sequential += 1;
                Some(index)
            }
        }

        fn parse_position(bytes: &[u8], cursor: &mut usize) -> Option<usize> {
            let start = *cursor;
            let mut value = 0usize;
            while let Some(digit @ b'0'..=b'9') = bytes.get(*cursor).copied() {
                value = value
                    .saturating_mul(10)
                    .saturating_add((digit - b'0') as usize);
                *cursor += 1;
            }
            if *cursor > start && bytes.get(*cursor) == Some(&b'$') {
                *cursor += 1;
                Some(value)
            } else {
                *cursor = start;
                None
            }
        }

        let ptr_width = crate::common::types::target_ptr_size();
        let signed_word = if ptr_width == 4 {
            CType::Int
        } else {
            CType::Long
        };
        let unsigned_word = if ptr_width == 4 {
            CType::UInt
        } else {
            CType::ULong
        };
        let signed_max = if ptr_width == 4 {
            CType::LongLong
        } else {
            CType::Long
        };
        let unsigned_max = if ptr_width == 4 {
            CType::ULongLong
        } else {
            CType::ULong
        };

        let mut cursor = 0usize;
        let mut next_argument = format_index + 1;
        let bytes = format.as_bytes();
        while cursor < bytes.len() {
            if bytes[cursor] != b'%' {
                cursor += 1;
                continue;
            }
            let directive_start = cursor;
            cursor += 1;
            if cursor >= bytes.len() {
                break;
            }
            if bytes[cursor] == b'%' {
                cursor += 1;
                continue;
            }

            // POSIX positional argument: `%2$d`. A non-positional number is
            // width, so rewind unless it is immediately followed by `$`.
            let value_position = parse_position(bytes, &mut cursor);
            while cursor < bytes.len() && b"#0- +'I".contains(&bytes[cursor]) {
                cursor += 1;
            }

            // Field width. A star consumes an int argument, optionally in
            // positional form (`%*3$d`).
            if cursor < bytes.len() && bytes[cursor] == b'*' {
                cursor += 1;
                let width_position = parse_position(bytes, &mut cursor);
                let arg_index =
                    positional_arg_index(format_index, &mut next_argument, width_position);
                self.check_format_argument(
                    args,
                    format_index,
                    arg_index,
                    FormatExpectation::Exact(CType::Int),
                    "*",
                    *format_span,
                );
            } else {
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
            }

            // Precision: a star also consumes an int argument.
            if cursor < bytes.len() && bytes[cursor] == b'.' {
                cursor += 1;
                if cursor < bytes.len() && bytes[cursor] == b'*' {
                    cursor += 1;
                    let precision_position = parse_position(bytes, &mut cursor);
                    let arg_index =
                        positional_arg_index(format_index, &mut next_argument, precision_position);
                    self.check_format_argument(
                        args,
                        format_index,
                        arg_index,
                        FormatExpectation::Exact(CType::Int),
                        "*",
                        *format_span,
                    );
                } else {
                    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                        cursor += 1;
                    }
                }
            }

            let length = match bytes.get(cursor).copied() {
                Some(b'h') if bytes.get(cursor + 1) == Some(&b'h') => {
                    cursor += 2;
                    FormatLength::Hh
                }
                Some(b'h') => {
                    cursor += 1;
                    FormatLength::H
                }
                Some(b'l') if bytes.get(cursor + 1) == Some(&b'l') => {
                    cursor += 2;
                    FormatLength::Ll
                }
                Some(b'l') => {
                    cursor += 1;
                    FormatLength::L
                }
                Some(b'j') => {
                    cursor += 1;
                    FormatLength::J
                }
                Some(b'z') | Some(b'Z') => {
                    cursor += 1;
                    FormatLength::Z
                }
                Some(b't') => {
                    cursor += 1;
                    FormatLength::T
                }
                Some(b'L') => {
                    cursor += 1;
                    FormatLength::BigL
                }
                Some(b'q') => {
                    cursor += 1;
                    FormatLength::Ll
                }
                _ => FormatLength::Default,
            };
            let Some(&conversion) = bytes.get(cursor) else {
                break;
            };
            cursor += 1;
            let specifier = String::from_utf8_lossy(&bytes[directive_start..cursor]).into_owned();

            let integer_expectation = |signed: bool| match length {
                FormatLength::Default | FormatLength::Hh | FormatLength::H => {
                    if matches!(length, FormatLength::Hh | FormatLength::H) {
                        FormatExpectation::IntegerEither
                    } else if signed {
                        FormatExpectation::Exact(CType::Int)
                    } else {
                        FormatExpectation::Exact(CType::UInt)
                    }
                }
                FormatLength::L => {
                    FormatExpectation::Exact(if signed { CType::Long } else { CType::ULong })
                }
                FormatLength::Ll => FormatExpectation::Exact(if signed {
                    CType::LongLong
                } else {
                    CType::ULongLong
                }),
                FormatLength::J => FormatExpectation::Exact(if signed {
                    signed_max.clone()
                } else {
                    unsigned_max.clone()
                }),
                FormatLength::Z | FormatLength::T => FormatExpectation::Exact(if signed {
                    signed_word.clone()
                } else {
                    unsigned_word.clone()
                }),
                FormatLength::BigL => {
                    FormatExpectation::Exact(if signed { CType::Int } else { CType::UInt })
                }
            };
            let expectation = match conversion {
                b'd' | b'i' => Some(integer_expectation(true)),
                b'o' | b'u' | b'x' | b'X' | b'b' | b'B' => Some(integer_expectation(false)),
                b'D' => Some(FormatExpectation::Exact(CType::Long)),
                b'O' | b'U' => Some(FormatExpectation::Exact(CType::ULong)),
                b'c' => Some(FormatExpectation::Exact(
                    if matches!(length, FormatLength::L) {
                        CType::UInt
                    } else {
                        CType::Int
                    },
                )),
                b's' if matches!(length, FormatLength::L) => {
                    Some(FormatExpectation::PointerTo(CType::Int))
                }
                b's' => Some(FormatExpectation::CharacterPointer),
                b'p' => Some(FormatExpectation::AnyPointer),
                b'n' => {
                    let pointee = match length {
                        FormatLength::Hh => CType::SChar,
                        FormatLength::H => CType::Short,
                        FormatLength::Default => CType::Int,
                        FormatLength::L => CType::Long,
                        FormatLength::Ll => CType::LongLong,
                        FormatLength::J => signed_max.clone(),
                        FormatLength::Z | FormatLength::T => signed_word.clone(),
                        FormatLength::BigL => CType::Int,
                    };
                    Some(FormatExpectation::PointerTo(pointee))
                }
                b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => Some(
                    FormatExpectation::Exact(if matches!(length, FormatLength::BigL) {
                        CType::LongDouble
                    } else {
                        CType::Double
                    }),
                ),
                // GNU `%m` consumes no argument. An unknown conversion is
                // left to the compiler's ordinary format-string diagnostics.
                b'm' | b'%' => None,
                _ => None,
            };

            if let Some(expectation) = expectation {
                let arg_index =
                    positional_arg_index(format_index, &mut next_argument, value_position);
                self.check_format_argument(
                    args,
                    format_index,
                    arg_index,
                    expectation,
                    &specifier,
                    *format_span,
                );
            }
        }
    }

    fn check_format_argument(
        &self,
        args: &[Expr],
        format_index: usize,
        arg_index: Option<usize>,
        expected: FormatExpectation,
        specifier: &str,
        format_span: Span,
    ) {
        let Some(arg_index) = arg_index else {
            return;
        };
        let Some(arg) = args.get(arg_index) else {
            self.diagnostics.borrow_mut().warning_with_kind(
                format!(
                    "format '{}' expects a matching '{}' argument",
                    specifier,
                    expected.description()
                ),
                format_span,
                crate::common::error::WarningKind::Format,
            );
            return;
        };
        let Some(actual) = self.expr_type_checker().infer_expr_ctype(arg) else {
            return;
        };
        if !expected.accepts(&actual) {
            let format_arg = args.get(format_index).map_or(format_span, Expr::span);
            self.diagnostics.borrow_mut().warning_with_kind(
                format!(
                    "format '{}' expects argument of type '{}', but argument {} has type '{}'",
                    specifier,
                    expected.description(),
                    arg_index + 1,
                    gcc_format_type_name(&actual.decayed_value_ctype())
                ),
                format_arg,
                crate::common::error::WarningKind::Format,
            );
        }
    }

    /// Keep native vector arguments out of scalar/address positions and require
    /// each vector source's own width (not the destination width for widening).
    /// This prevents malformed raw calls reaching pointer-backed SIMD emission.
    fn check_builtin_vector_arguments(&self, name: &str, args: &[Expr]) {
        let Some(widths) = crate::common::x86_builtin_types::argument_vector_widths(name) else {
            return;
        };
        for (index, (arg, expected)) in args.iter().zip(widths).enumerate() {
            let inferred;
            let actual = if let Some(ty) = self.result.expr_types.get(&arg.id()) {
                ty
            } else {
                inferred = self.expr_type_checker().infer_expr_ctype(arg);
                let Some(ref ty) = inferred else { continue };
                ty
            };
            let width = if actual.is_vector() { actual.size() } else { 0 };
            if width != *expected {
                let required = if *expected == 0 {
                    "scalar or address operand".to_string()
                } else {
                    format!("{}-bit vector", expected * 8)
                };
                self.diagnostics.borrow_mut().error(
                    format!(
                        "argument {} to '{}' requires a {}; have '{}'",
                        index + 1,
                        name,
                        required,
                        actual
                    ),
                    arg.span(),
                );
            }
        }
    }

    /// Infer the CType of an expression via ExprTypeChecker and record it
    /// in the expr_types annotation map, keyed by the expression's `ExprId`.
    fn annotate_expr_type(&mut self, expr: &Expr) {
        let checker = self.expr_type_checker();
        if let Some(ctype) = checker.infer_expr_ctype(expr) {
            self.result.expr_types.insert(expr.id(), ctype);
        }
    }

    /// Try to evaluate an expression as a compile-time constant using
    /// SemaConstEval and record the result in the const_values map.
    /// The lowerer will consult this as an O(1) fast path.
    fn annotate_const_value(&mut self, expr: &Expr) {
        // A folded const object is recorded in the const map; an enumerator
        // in `enumerator_refs`, which the lowerer consults before its own
        // scopes (both only when that declaration is the innermost one).
        if let Expr::Identifier(name, _) = expr {
            if let Some((_, true)) = self.symbol_table.lookup_ordinary(name) {
                if let Some(constant) = self.result.type_context.enum_constants.get(name) {
                    self.result
                        .enumerator_refs
                        .insert(expr.id(), constant.value);
                }
                return;
            }
            let folded = self
                .symbol_table
                .folded_value(name)
                .map(FoldedValue::to_ir_const);
            if let Some(val) = folded {
                self.result.const_values.insert(expr.id(), val);
            }
            return;
        }
        let evaluator = SemaConstEval {
            types: &self.result.type_context,
            symbols: &self.symbol_table,
            functions: &self.result.functions,
            const_values: Some(&self.result.const_values),
            expr_types: Some(&self.result.expr_types),
            builtin_policy: self.builtin_policy.clone(),
            folding_initializer: false,
            pre_c99: self.pre_c99,
            named_constants_only: false,
        };
        if let Some(val) = evaluator.eval_const_expr(expr) {
            self.result.const_values.insert(expr.id(), val);
        }
    }

    // === Operand constraints (see `operand_constraints`) ===

    /// The type of `expr` as the operand constraints judge it: the decayed
    /// value type, `None` when unknown. A `void` type counts only where it is
    /// exact -- a cast to `void`, or a call to a declared, non-builtin
    /// function -- because builtin signatures are not all modelled and a
    /// wrong `void` would reject a valid program.
    fn constraint_operand(&self, expr: &Expr) -> Option<CType> {
        // GCC types `c ? v() : 1` as `void` (its one-void-arm extension), so
        // using the value is the void-value error.
        if let Expr::Conditional(_, then_expr, else_expr, _) = expr {
            if self.is_trusted_void(then_expr) || self.is_trusted_void(else_expr) {
                return Some(CType::Void);
            }
        }
        // A builtin's prototype is not its type: lowering intercepts the call
        // and decides what it yields. The SIMD headers' `__lccc_simd*` and
        // `__builtin_ia32_*` vector builtins are declared to return a vector
        // struct but yield a pointer to the result slot, which
        // `__CCC_M128I_FROM_BUILTIN(e)` = `*(__m128i *)(e)` dereferences.
        if let Expr::FunctionCall(callee, ..) = expr {
            if matches!(callee.as_ref(), Expr::Identifier(name, _) if builtins::is_builtin(name)) {
                return None;
            }
        }
        let ty = self
            .expr_type_checker()
            .infer_expr_ctype(expr)?
            .decayed_value_ctype();
        if matches!(ty, CType::Void) && !self.is_exact_void(expr) {
            return None;
        }
        Some(ty)
    }

    /// `expr` has type `void` and that inference can be trusted (see
    /// `is_exact_void`, which alone says nothing about the type).
    fn is_trusted_void(&self, expr: &Expr) -> bool {
        matches!(
            self.expr_type_checker().infer_expr_ctype(expr),
            Some(CType::Void)
        ) && self.is_exact_void(expr)
    }

    /// Whether an inferred `void` type of `expr` can be trusted: a `(void)`
    /// cast, or a call to a declared non-builtin function. Callers must have
    /// inferred `void` first.
    fn is_exact_void(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Cast(TypeSpecifier::Void, ..) => true,
            Expr::Comma(_, rhs, _) => self.is_exact_void(rhs),
            Expr::FunctionCall(callee, _, _) => matches!(
                callee.as_ref(),
                Expr::Identifier(name, _)
                    if self.result.functions.contains_key(name) && !builtins::is_builtin(name)
            ),
            _ => false,
        }
    }

    /// A `_Generic` without `default` must match an association (C23
    /// 6.5.1.1p2). Only an operand type inference gets exactly is judged:
    /// vector types and anonymous record keys are not exact (builtin headers
    /// give one anonymous type several `__anon_struct_N` keys).
    fn check_generic_selection(
        &self,
        controlling: &Expr,
        associations: &[GenericAssociation],
        span: Span,
    ) {
        if associations.iter().any(|a| a.type_spec.is_none()) {
            return;
        }
        let checker = self.expr_type_checker();
        let Some(ty) = checker.infer_expr_ctype(controlling) else {
            return;
        };
        let ty = match controlling {
            Expr::TypeOperand(..) => ty,
            _ => ty.decayed_value_ctype(),
        };
        fn exact(ty: &CType) -> bool {
            match ty {
                CType::Vector(..) => false,
                CType::Struct(key) | CType::Union(key) => !key.starts_with("__anon_"),
                CType::Pointer(inner, _) | CType::Array(inner, _) => exact(inner),
                CType::Function(f) => {
                    exact(&f.return_type) && f.params.iter().all(|(p, _)| exact(p))
                }
                _ => true,
            }
        }
        if exact(&ty)
            && checker
                .generic_selection_index(controlling, associations)
                .is_none()
        {
            self.diagnostics.borrow_mut().error(
                format!(
                    "'_Generic' selector of type '{ty}' is not compatible with any association"
                ),
                span,
            );
        }
    }

    /// Reports `message` at `span`; whether there was one.
    fn constraint_error(&self, message: Option<String>, span: Span) -> bool {
        let Some(message) = message else {
            return false;
        };
        self.diagnostics.borrow_mut().error(message, span);
        true
    }

    /// A null pointer constant, evaluated only when `needed` (it runs the
    /// integer-constant-expression checker).
    fn null_constant_if(&self, needed: bool, expr: &Expr) -> bool {
        needed && self.expr_type_checker().is_null_pointer_constant(expr)
    }

    /// Controlling expressions and the first operand of `?:` (C23 6.5.15p2,
    /// 6.8.4.1p1, 6.8.5p2) must have scalar type.
    fn check_scalar_condition(&self, cond: &Expr) {
        if let Some(ty) = self.constraint_operand(cond) {
            self.constraint_error(operand_constraints::scalar_required(&ty), cond.span());
        }
    }

    fn check_binary_operands(&self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) {
        let (Some(lt), Some(rt)) = (self.constraint_operand(lhs), self.constraint_operand(rhs))
        else {
            return;
        };
        if !operand_constraints::involved(&lt) && !operand_constraints::involved(&rt) {
            return;
        }
        let nullptr = matches!(lt, CType::NullPtr) || matches!(rt, CType::NullPtr);
        let lhs_op = Operand {
            ty: &lt,
            null_constant: self.null_constant_if(nullptr, lhs),
        };
        let rhs_op = Operand {
            ty: &rt,
            null_constant: self.null_constant_if(nullptr, rhs),
        };
        self.constraint_error(operand_constraints::binary(op, lhs_op, rhs_op), span);
    }

    fn check_conditional_arms(&self, then_expr: &Expr, else_expr: &Expr, span: Span) {
        if let (Some(t), Some(e)) = (
            self.constraint_operand(then_expr),
            self.constraint_operand(else_expr),
        ) {
            self.constraint_error(operand_constraints::conditional_arms(&t, &e), span);
        }
    }

    /// Simple assignment and the conversions "as if by assignment"
    /// (initialization, argument passing, `return`; C23 6.5.16.1p1) for a
    /// record, `void` or `nullptr_t` operand. Whether it reported an error.
    fn check_assignment_operand(
        &self,
        from: &Expr,
        to: &CType,
        span: Span,
        ctx: IConvContext<'_>,
    ) -> bool {
        let Some(from_ty) = self.constraint_operand(from) else {
            return false;
        };
        // GCC 16.2: an integer that is not a null-pointer constant used
        // where a pointer is expected (initialization, assignment, argument
        // passing, `return`) is `-Wint-conversion`, a default error from
        // C99 on and a warning in gnu90 (measured). Explicit casts take a
        // different path and never reach this implicit conversion.
        let integer_to_pointer = from_ty.is_integer()
            && matches!(to, CType::Pointer(..))
            && !self.expr_type_checker().is_null_pointer_constant(from);
        let pointer_to_integer = to.is_integer()
            && !matches!(to, CType::Bool)
            && matches!(
                from_ty,
                CType::Pointer(..) | CType::Array(..) | CType::Function(..)
            );
        if integer_to_pointer || pointer_to_integer {
            let context = match ctx {
                IConvContext::Initialization => {
                    format!("initialization of '{to}' from '{from_ty}'")
                }
                IConvContext::Assignment => {
                    format!("assignment to '{to}' from '{from_ty}'")
                }
                IConvContext::Returning => {
                    format!("returning '{from_ty}' from a function with return type '{to}'")
                }
                IConvContext::Argument { index, callee } => {
                    format!("passing argument {} of '{}'", index + 1, callee)
                }
            };
            // GCC's measured column policy: `return` points at the
            // returned value, while initialization, assignment, and
            // argument passing keep the surrounding conversion
            // expression's span (which matches gcc's column in all
            // three probe contexts).
            let ic_span = match ctx {
                IConvContext::Returning => from.span(),
                _ => span,
            };
            self.diagnostics.borrow_mut().warning_with_kind(
                format!(
                    "{context} makes {} without a cast",
                    if integer_to_pointer {
                        "pointer from integer"
                    } else {
                        "integer from pointer"
                    }
                ),
                ic_span,
                crate::common::error::WarningKind::IntConversion,
            );
            return true;
        }
        // Pointer-shaped source against a pointer target with
        // incompatible pointees: C11 6.5.16.1 constraint, GCC
        // `-Wincompatible-pointer-types` (a default error from C99 on and a
        // warning in gnu90; measured). The verdict comes from the very
        // function the argument path uses, so void pointers, qualified
        // pointees, arrays, function decay, and address spaces stay
        // accepted exactly where GCC 16.2 accepts them. Reporting here
        // also pre-emits the per-context checks below, replacing their
        // plain error (which `-Wno-incompatible-pointer-types` could not
        // disable) with the kind-aware, dialect-severity diagnostic.
        // Two shapes belong to other owners and must fall through to them:
        // a null-pointer constant spelled through `void` (`(void *)0` into
        // an address-space pointer is accepted, measured with the
        // `__seg_gs` conversion oracles), and a cross-address-space pair
        // whose diagnostic is the address-space one, not the pointer one.
        // The space gap may sit at ANY pointer level (`int **` against
        // `__seg_gs int **` differs below the top), so the comparison
        // mirrors `pointer_argument_compat`'s per-level space walk.
        fn space_gap(a: &CType, b: &CType) -> bool {
            match (a, b) {
                (CType::Pointer(af, asp), CType::Pointer(bt, bsp)) => {
                    asp != bsp || space_gap(af, bt)
                }
                (CType::Pointer(af, _), CType::Array(bt, _)) => space_gap(af, bt),
                (CType::Array(af, _), CType::Pointer(bt, bsp)) => {
                    let _ = bsp;
                    space_gap(af, bt)
                }
                _ => false,
            }
        }
        let space_ok = !space_gap(&from_ty, to);
        if matches!(to, CType::Pointer(..))
            && matches!(
                from_ty,
                CType::Pointer(..) | CType::Array(..) | CType::Function(..)
            )
            && space_ok
            && !self.is_void_null_pointer_constant(from)
            && pointer_argument_compat(&from_ty, to) == PtrCompat::Incompatible
        {
            // All four phrasings measured against gcc-16.2.0.
            let message = match ctx {
                IConvContext::Initialization => {
                    format!("initialization of '{to}' from incompatible pointer type '{from_ty}'")
                }
                IConvContext::Assignment => {
                    format!("assignment to '{to}' from incompatible pointer type '{from_ty}'")
                }
                IConvContext::Returning => format!(
                    "returning '{from_ty}' from a function with incompatible return type '{to}'"
                ),
                IConvContext::Argument { index, callee } => format!(
                    "passing argument {} of '{}' from incompatible pointer type",
                    index + 1,
                    callee
                ),
            };
            // Same measured column policy as the integer case: `return`
            // points at the returned value.
            let ip_span = match ctx {
                IConvContext::Returning => from.span(),
                _ => span,
            };
            self.diagnostics.borrow_mut().warning_with_kind(
                message,
                ip_span,
                crate::common::error::WarningKind::IncompatiblePointerTypes,
            );
            return true;
        }
        if !operand_constraints::involved(&from_ty) && !operand_constraints::involved(to) {
            return false;
        }
        let null_constant = self.null_constant_if(matches!(to, CType::NullPtr), from);
        let operand = Operand {
            ty: &from_ty,
            null_constant,
        };
        self.constraint_error(operand_constraints::assignment(operand, to), span)
    }

    fn check_cast_operand(&self, ts: &TypeSpecifier, inner: &Expr, span: Span) {
        let Some(from_ty) = self.constraint_operand(inner) else {
            return;
        };
        let to = self.type_spec_to_ctype(ts);
        if !operand_constraints::involved(&from_ty) && !operand_constraints::involved(&to) {
            return;
        }
        let null_constant = self.null_constant_if(matches!(to, CType::NullPtr), inner);
        let operand = Operand {
            ty: &from_ty,
            null_constant,
        };
        let union_has_member = || match &to {
            CType::Union(key) => self
                .result
                .type_context
                .borrow_struct_layouts()
                .get(&**key)
                .is_some_and(|layout| layout.fields.iter().any(|f| f.ty == from_ty)),
            _ => false,
        };
        self.constraint_error(
            operand_constraints::cast(operand, &to, union_has_member),
            span,
        );
    }

    /// Check that a struct/union member access refers to an existing field.
    /// Emits an error if the base type is a known struct/union with a complete layout
    /// and the field name is not found.
    /// C11 6.5.2.1p1: one subscript operand has pointer (array, or GNU
    /// vector) type and the other integer type.  Without this check a struct
    /// operand (including lccc's struct-based `__m256`) was lowered as a
    /// pointer load and its value converted to an integer index -- a silent
    /// miscompile of code GCC rejects.
    ///
    /// The expression type inference is not vector-exact (a vector
    /// comparison or a cast to an attribute-spelled vector type infers as its
    /// scalar element), so an operand's type is trusted only where it cannot
    /// come from such an expression: declared objects, members and literals,
    /// or any expression inferred as a struct/union.  Other operands are left
    /// to lowering, as before.
    fn check_subscript_operands(&self, base: &Expr, index: &Expr, span: Span) {
        let checker = self.expr_type_checker();
        let trusted = |expr: &Expr| {
            let ty = checker.infer_expr_ctype(expr)?;
            let declared = matches!(
                expr,
                Expr::Identifier(..)
                    | Expr::MemberAccess(..)
                    | Expr::PointerMemberAccess(..)
                    | Expr::IntLiteral(..)
                    | Expr::UIntLiteral(..)
                    | Expr::LongLiteral(..)
                    | Expr::ULongLiteral(..)
                    | Expr::LongLongLiteral(..)
                    | Expr::ULongLongLiteral(..)
                    | Expr::CharLiteral(..)
                    | Expr::FloatLiteral(..)
                    | Expr::FloatLiteralF32(..)
                    | Expr::FloatLiteralLongDouble(..)
                    | Expr::StringLiteral(..)
            );
            // A vector's type is exact wherever it comes from (a call
            // result, a cast, a selection): `1[f()]` is checked too.
            (declared
                || ty.is_vector()
                || matches!(ty, CType::Struct(_) | CType::Union(_) | CType::NullPtr))
            .then_some(ty)
        };
        let (Some(base_ty), Some(index_ty)) = (trusted(base), trusted(index)) else {
            return;
        };
        // Function designators decay to pointers; GCC's diagnostic for
        // subscripting them is a separate (pointer-to-function) rule.
        let addressable = |ty: &CType| {
            ty.is_vector()
                || matches!(
                    ty,
                    CType::Pointer(..) | CType::Array(..) | CType::Function(_)
                )
        };
        let message = match (addressable(&base_ty), addressable(&index_ty)) {
            (true, _) if index_ty.is_integer() => return,
            // `0[v]`: subscript commutativity is pointer + integer (C11
            // 6.5.2.1p1); a vector is not a pointer, so GCC 16.2 and Clang
            // 23.1.3 both reject the reversed form for vectors.
            (false, true) if index_ty.is_vector() => {
                "subscripted value is neither array nor pointer nor vector"
            }
            (false, true) if base_ty.is_integer() => return,
            (true, _) | (false, true) => "array subscript is not an integer",
            (false, false) => "subscripted value is neither array nor pointer nor vector",
        };
        self.diagnostics.borrow_mut().error(message, span);
    }

    /// The type of the object a member access names the member of: the
    /// type of `base_expr` for `.`, its pointee for `->`.
    /// An expression type checker over this analyzer's current state.
    fn expr_checker(&self) -> super::type_checker::ExprTypeChecker<'_> {
        self.expr_type_checker()
    }

    fn member_base_ctype(&self, base_expr: &Expr, is_pointer: bool) -> Option<CType> {
        let base_ctype = self.expr_checker().infer_expr_ctype(base_expr)?;
        if !is_pointer {
            return Some(base_ctype);
        }
        match base_ctype {
            CType::Pointer(inner, _) | CType::Array(inner, _) => Some(*inner),
            _ => None,
        }
    }

    /// The ext vector and selector of `expr` when it is an ext-vector
    /// element selection (`v.x`, `p->zy`, `v.wzyx.xy`).
    fn ext_vector_selection_of<'e>(&self, expr: &'e Expr) -> Option<(&'e Expr, &'e str, CType)> {
        let (base, field, is_pointer) = match expr {
            Expr::MemberAccess(b, f, _) => (&**b, f.as_str(), false),
            Expr::PointerMemberAccess(b, f, _) => (&**b, f.as_str(), true),
            _ => return None,
        };
        let vector = self.member_base_ctype(base, is_pointer)?;
        matches!(vector, CType::ExtVector(..)).then_some((base, field, vector))
    }

    /// Clang: an ext-vector element selection that repeats a lane is not
    /// assignable, at any level of a nested selection (`v.xx.y`, and
    /// `v.xxyy.yz` although the composed lanes 0 and 1 differ). A
    /// selection through `->` ends the chain: its base is a pointer.
    fn ext_vector_selection_has_duplicate_lanes(&self, expr: &Expr) -> bool {
        let mut cur = expr;
        while let Some((base, field, vector)) = self.ext_vector_selection_of(cur) {
            if let Some(lanes) = vector.ext_vector_swizzle(field) {
                if lanes
                    .iter()
                    .enumerate()
                    .any(|(i, l)| lanes[..i].contains(l))
                {
                    return true;
                }
            }
            if matches!(cur, Expr::PointerMemberAccess(..)) {
                break;
            }
            cur = base;
        }
        false
    }

    /// The operand of `=`, a compound assignment, `++` or `--`: report an
    /// ext-vector selection with a repeated lane at the operator, with
    /// Clang 23.1.3's message.
    fn check_ext_vector_assignable(&self, target: &Expr, op_span: Span) -> bool {
        let duplicate = self.ext_vector_selection_has_duplicate_lanes(target);
        if duplicate {
            self.diagnostics.borrow_mut().error(
                "vector is not assignable (contains duplicate components)",
                op_span,
            );
        }
        duplicate
    }

    /// `++`/`--` on an ext vector, whole or a multi-lane selection: Clang
    /// 23.1.3 has no vector increment in C ("cannot increment value of type
    /// ..."), at the operator. One lane is a scalar and steps normally. A
    /// GNU `vector_size` operand keeps GCC's element-wise `v++`.
    fn check_ext_vector_step(&self, operand: &Expr, increment: bool, op_span: Span) -> bool {
        let Some(ct @ CType::ExtVector(..)) = self.expr_checker().infer_expr_ctype(operand) else {
            return false;
        };
        let Some((elem, lanes)) = ct.vector_info() else {
            return false;
        };
        let verb = if increment { "increment" } else { "decrement" };
        let plural = if lanes == 1 { "" } else { "s" };
        self.diagnostics.borrow_mut().error(
            format!(
                "cannot {verb} value of type '{ct}' (vector of {lanes} '{elem}' value{plural})"
            ),
            op_span,
        );
        true
    }

    /// Whether `expr` is an rvalue in C (C11 6.3.2.1p1, 6.5.2.3p3) for the
    /// lvalue constraints of `=`, compound assignment, `++`/`--` and unary
    /// `&`. Deliberately a DENY-list: only forms that GCC 16.2 and Clang
    /// 23.1.3 both treat as rvalues answer `true`; anything not modelled
    /// (identifiers, `*p`, `p->m`, compound and string literals, `_Generic`,
    /// ...) answers `false`, so an omission here can never reject a valid
    /// program.
    ///
    /// `.member`, a vector lane `v[i]` and an ext-vector selection `v.xy`
    /// inherit their operand's category, as do `__real__`/`__imag__` and the
    /// chosen arm of `__builtin_choose_expr`. A statement expression takes
    /// its tail expression's category (GCC, which defines the extension) --
    /// except under an ext-vector element: ext vectors are Clang's
    /// extension, and Clang treats `({ v; })` as an rvalue.
    fn is_known_rvalue(&self, expr: &Expr, under_ext_vector: bool) -> bool {
        let is_ext_vector = |e: &Expr| {
            self.member_base_ctype(e, false)
                .is_some_and(|ct| matches!(ct, CType::ExtVector(..)))
        };
        match expr {
            Expr::IntLiteral(..)
            | Expr::UIntLiteral(..)
            | Expr::LongLiteral(..)
            | Expr::ULongLiteral(..)
            | Expr::LongLongLiteral(..)
            | Expr::ULongLongLiteral(..)
            | Expr::FloatLiteral(..)
            | Expr::FloatLiteralF32(..)
            | Expr::FloatLiteralLongDouble(..)
            | Expr::FloatLiteralF128(..)
            | Expr::FloatLiteralDecimal(..)
            | Expr::FloatLiteralHalf(..)
            | Expr::ImaginaryLiteral(..)
            | Expr::ImaginaryLiteralF32(..)
            | Expr::ImaginaryLiteralLongDouble(..)
            | Expr::CharLiteral(..)
            | Expr::BinaryOp(..)
            | Expr::PostfixOp(..)
            | Expr::Assign(..)
            | Expr::CompoundAssign(..)
            | Expr::Conditional(..)
            | Expr::GnuConditional(..)
            | Expr::Cast(..)
            | Expr::Sizeof(..)
            | Expr::Countof(..)
            | Expr::VaArg(..)
            | Expr::ConvertVector(..)
            | Expr::BitCast(..)
            | Expr::Alignof(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignof(..)
            | Expr::GnuAlignofExpr(..)
            | Expr::Comma(..)
            | Expr::AddressOf(..)
            | Expr::LabelAddr(..)
            | Expr::BuiltinTypesCompatibleP(..) => true,
            Expr::UnaryOp(UnaryOp::RealPart | UnaryOp::ImagPart, inner, _) => {
                self.is_known_rvalue(inner, under_ext_vector)
            }
            Expr::UnaryOp(..) => true,
            Expr::FunctionCall(callee, args, _) => {
                let choose = args.len() >= 3
                    && matches!(&**callee, Expr::Identifier(n, _) if n == "__builtin_choose_expr");
                if !choose {
                    return true;
                }
                // The chosen arm's category (the condition is an integer
                // constant expression); unevaluable, an rvalue only when
                // both arms are.
                match self.sema_const_eval().eval_required_constant(&args[0]) {
                    Some(c) => {
                        let arm = if c.is_nonzero() { &args[1] } else { &args[2] };
                        self.is_known_rvalue(arm, under_ext_vector)
                    }
                    None => {
                        self.is_known_rvalue(&args[1], under_ext_vector)
                            && self.is_known_rvalue(&args[2], under_ext_vector)
                    }
                }
            }
            Expr::MemberAccess(obj, _, _) => {
                let selection = self.ext_vector_selection_of(expr).is_some();
                self.is_known_rvalue(obj, under_ext_vector || selection)
            }
            Expr::ArraySubscript(a, b, _) => {
                let vector_operand = |e: &Expr| {
                    self.expr_checker()
                        .infer_expr_ctype(e)
                        .is_some_and(|ct| ct.is_vector())
                };
                let lane_of = if vector_operand(a) {
                    Some(&**a)
                } else if vector_operand(b) {
                    Some(&**b)
                } else {
                    None // array/pointer subscript: always an lvalue
                };
                lane_of
                    .is_some_and(|v| self.is_known_rvalue(v, under_ext_vector || is_ext_vector(v)))
            }
            Expr::StmtExpr(compound, _) => {
                if under_ext_vector {
                    return true;
                }
                match compound.items.last() {
                    Some(BlockItem::Statement(Stmt::Expr(Some(tail)))) => {
                        self.is_known_rvalue(tail, false)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// The operand of `=`, a compound assignment, `++` or `--` must be a
    /// modifiable lvalue (C11 6.5.16p2, 6.5.2.4p1, 6.5.3.1p1). Clang
    /// 23.1.3's spellings, in its precedence (duplicate lanes and the
    /// ext-vector step are reported by their own checks first): a cast says
    /// "assignment to cast is illegal, lvalue casts are not supported",
    /// every other rvalue "expression is not assignable". Without this an
    /// assignment to `f()`, `x + 1`, `(a, b)` or `sf().m` compiled to
    /// nothing.
    fn check_lvalue_operand(&self, target: &Expr, op_span: Span) {
        if !self.is_known_rvalue(target, false) {
            self.check_const_compound_literal_mutation(target, op_span);
            return;
        }
        let message = if matches!(target, Expr::Cast(..)) {
            "assignment to cast is illegal, lvalue casts are not supported"
        } else {
            "expression is not assignable"
        };
        self.diagnostics.borrow_mut().error(message, op_span);
    }

    /// `&` needs an lvalue (C11 6.5.3.2p1): Clang 23.1.3's "cannot take the
    /// address of an rvalue of type 'T'". Returns whether it reported.
    fn check_address_of_rvalue(&self, operand: &Expr, op_span: Span) -> bool {
        if !self.is_known_rvalue(operand, false) {
            return false;
        }
        let message = match self.expr_checker().infer_expr_ctype(operand) {
            Some(ct) => match ct.vector_info() {
                Some((elem, lanes)) if matches!(ct, CType::ExtVector(..)) => {
                    let plural = if lanes == 1 { "" } else { "s" };
                    format!(
                        "cannot take the address of an rvalue of type '{ct}' (vector of {lanes} '{elem}' value{plural})"
                    )
                }
                _ => format!("cannot take the address of an rvalue of type '{ct}'"),
            },
            None => "cannot take the address of an rvalue".to_string(),
        };
        self.diagnostics.borrow_mut().error(message, op_span);
        true
    }

    /// `&v.x`, `&p->zy`, `&v[i]` on a Clang ext vector: Clang 23.1.3 says
    /// "address of vector element requested" (an element has no address of
    /// its own: `_Bool` lanes are bits, `long double` lanes are 10 bytes
    /// apart). GNU `vector_size` subscripts keep GCC's rule, which allows
    /// `&v[i]`.
    fn check_address_of_vector_element(&self, operand: &Expr, op_span: Span) {
        let is_ext_vector = |e: &Expr| {
            self.member_base_ctype(e, false)
                .is_some_and(|ct| matches!(ct, CType::ExtVector(..)))
        };
        let element = match operand {
            Expr::MemberAccess(..) | Expr::PointerMemberAccess(..) => {
                self.ext_vector_selection_of(operand).is_some()
            }
            Expr::ArraySubscript(a, b, _) => is_ext_vector(a) || is_ext_vector(b),
            _ => false,
        };
        if element {
            self.diagnostics
                .borrow_mut()
                .error("address of vector element requested", op_span);
        }
    }

    fn check_member_exists(
        &self,
        base_expr: &Expr,
        field_name: &str,
        span: Span,
        is_pointer: bool,
    ) {
        let base_ctype = match self.member_base_ctype(base_expr, is_pointer) {
            Some(ct) => ct,
            None => return, // Can't determine base type; skip check
        };

        let key = match &base_ctype {
            CType::Struct(key) | CType::Union(key) => key.clone(),
            CType::ExtVector(..) => {
                // Clang ext-vector element selection (`v.x`, `v.xyzw`, `v.s0`,
                // `v.lo`): validate the spelling here so a typo or an
                // out-of-range component is an error, not offset-0 garbage.
                // Clang 23.1.3 carets an illegal component one byte past the
                // operator's first byte plus the component's byte index
                // (`p  ->  zyq` and `v.xyq` both point at the `q` that way),
                // and an out-of-range access at the operator.
                if let Some(Err(err)) = base_ctype.ext_vector_selection(field_name) {
                    let at = match &err {
                        ExtVectorSelectorError::IllegalName(range) => {
                            let offset = u32::try_from(range.start).unwrap_or(u32::MAX);
                            let len = u32::try_from(range.len().max(1)).unwrap_or(1);
                            let start = span.start.saturating_add(1).saturating_add(offset);
                            Span {
                                start,
                                end: start.saturating_add(len),
                                file_id: span.file_id,
                            }
                        }
                        ExtVectorSelectorError::ExceedsType => span,
                    };
                    self.diagnostics
                        .borrow_mut()
                        .error(err.message(field_name, &base_ctype), at);
                }
                return;
            }
            // A definite scalar base (arithmetic, pointer, a GNU vector):
            // `x.y` on a float, `p->z` on an `int *`, `w.hi.y` on a two-lane
            // ext vector whose `.hi` is already the scalar lane. Used to be
            // accepted and lowered as field offset 0 (a miscompile with no
            // diagnostic). Anything the checker cannot name (`None`,
            // typeof, incomplete) still skips.
            CType::Int
            | CType::UInt
            | CType::Char
            | CType::SChar
            | CType::UChar
            | CType::PlainUChar
            | CType::Short
            | CType::UShort
            | CType::Long
            | CType::ULong
            | CType::LongLong
            | CType::ULongLong
            | CType::Bool
            | CType::Float
            | CType::Double
            | CType::LongDouble
            | CType::Pointer(..)
            | CType::Vector(..) => {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "request for member '{}' in something not a structure or union (base type '{}')",
                        field_name, base_ctype
                    ),
                    span,
                );
                return;
            }
            _ => return, // Not a struct/union; skip check
        };

        // If the struct tag has been redefined in an inner scope, the current layout
        // in the map may not match the layout the variable was actually declared with.
        // Skip the check to avoid false positives (codegen uses layout snapshots).
        if self
            .result
            .type_context
            .is_struct_key_shadowed(key.as_ref())
        {
            return;
        }

        let layouts = self.result.type_context.borrow_struct_layouts();
        let layout = match layouts.get(key.as_ref()) {
            Some(l) => l,
            None => {
                // No layout at all - struct was never declared or defined.
                // This happens with implicit struct references in casts like
                // ((struct foo *)0)->bar where struct foo is never defined.
                // C11 6.5.2.3: member access requires the struct to be complete.
                if !self.defined_structs.borrow().contains(key.as_ref())
                    && !self
                        .result
                        .type_context
                        .borrow_struct_layouts()
                        .get(key.as_ref())
                        .is_some_and(|layout| layout.is_complete)
                {
                    let type_name = format!("{}", base_ctype);
                    self.diagnostics.borrow_mut().error(
                        format!("invalid use of undefined type '{}'", type_name),
                        span,
                    );
                }
                return;
            }
        };

        // Check if the field exists (including anonymous struct/union members)
        if layout.field_offset(field_name, &*layouts).is_none() {
            let type_name = format!("{}", base_ctype);
            self.diagnostics.borrow_mut().error(
                format!("'{}' has no member named '{}'", type_name, field_name),
                span,
            );
        }
    }

    /// Check that pointer subtraction operands have compatible pointee types.
    /// C11 6.5.6p3: both operands must point to compatible types.
    /// Emits an error if both operands are pointers to different struct/union types.
    fn check_pointer_subtraction_compat(&self, lhs: &Expr, rhs: &Expr, span: Span) {
        let checker = self.expr_type_checker();
        let lct = match checker.infer_expr_ctype(lhs) {
            Some(ct) => ct,
            None => return,
        };
        let rct = match checker.infer_expr_ctype(rhs) {
            Some(ct) => ct,
            None => return,
        };
        // Extract pointee types; only check when both sides are pointers
        let (l_inner, r_inner) = match (&lct, &rct) {
            (CType::Pointer(li, _), CType::Pointer(ri, _)) => (li.as_ref(), ri.as_ref()),
            (CType::Array(li, _), CType::Pointer(ri, _)) => (li.as_ref(), ri.as_ref()),
            (CType::Pointer(li, _), CType::Array(ri, _)) => (li.as_ref(), ri.as_ref()),
            _ => return, // Not both pointers; not a ptr-ptr subtraction
        };
        // void* is compatible with any pointer for subtraction purposes
        if matches!(l_inner, CType::Void) || matches!(r_inner, CType::Void) {
            return;
        }
        // Check if pointee types are compatible
        if !Self::pointee_types_compatible(l_inner, r_inner) {
            self.diagnostics.borrow_mut().error(
                format!(
                    "invalid operands to binary - (have '{}' and '{}')",
                    lct, rct
                ),
                span,
            );
        }
    }

    fn check_call_arguments(
        &self,
        name: &str,
        args: &[Expr],
        params: &[(CType, Option<String>)],
        variadic: bool,
        span: Span,
    ) {
        let expected = params.len();
        if args.len() < expected {
            self.diagnostics.borrow_mut().error(
                format!(
                    "too few arguments to function '{}' (expected {}, have {})",
                    name,
                    expected,
                    args.len()
                ),
                span,
            );
        } else if !variadic && args.len() > expected {
            self.diagnostics.borrow_mut().error(
                format!(
                    "too many arguments to function '{}' (expected {}, have {})",
                    name,
                    expected,
                    args.len()
                ),
                span,
            );
        }
        let checker = self.expr_type_checker();
        // Arguments past the prototype only need a value (C23 6.5.2.2p7).
        for arg in args.iter().skip(params.len()) {
            if let Some(ty) = self.constraint_operand(arg) {
                self.constraint_error(operand_constraints::value_required(&ty), arg.span());
            }
        }
        for (index, (arg, (param_ty, _))) in args.iter().zip(params).enumerate() {
            // A transparent union parameter takes any of its member types
            // (GCC `transparent_union`); `check_record_argument_compatibility`
            // judges it.
            let transparent = match param_ty {
                CType::Union(key) => self
                    .result
                    .type_context
                    .borrow_struct_layouts()
                    .get(&**key)
                    .is_some_and(|layout| layout.is_transparent_union),
                _ => false,
            };
            if transparent {
                // GCC's member rule replaces the assignment constraints; an
                // argument whose type is unknown gets no verdict, as below.
                let accepted = checker.infer_expr_ctype(arg).is_none_or(|arg_ty| {
                    let arg_ty = self.conversion_source_ctype(arg, arg_ty);
                    self.transparent_union_accepts(arg, &arg_ty, param_ty)
                });
                if !accepted {
                    self.diagnostics.borrow_mut().error(
                        format!("incompatible type for argument {} of '{}'", index + 1, name),
                        arg.span(),
                    );
                }
                continue;
            }
            if self.check_assignment_operand(
                arg,
                param_ty,
                arg.span(),
                IConvContext::Argument {
                    index,
                    callee: name,
                },
            ) {
                continue;
            }
            if let Some(arg_ty) = checker.infer_expr_ctype(arg) {
                let arg_ty = self.conversion_source_ctype(arg, arg_ty);
                self.check_pointer_float_conversion(&arg_ty, param_ty, arg.span());
                self.check_record_argument_compatibility(
                    &arg_ty,
                    param_ty,
                    name,
                    index + 1,
                    arg.span(),
                );
                if !self.is_void_null_pointer_constant(arg) {
                    self.check_pointer_argument_compatibility(
                        &arg_ty,
                        param_ty,
                        name,
                        index + 1,
                        arg.span(),
                    );
                }
            }
        }
    }

    /// A by-value record argument must have a compatible record type.
    ///
    /// C 6.5.2.2p1 makes passing an incompatible struct or union a constraint
    /// violation, but this call path only ever checked arity and pointer/float
    /// mixing -- so a `struct S { int c; }` value passed where
    /// `struct S { char c; }` was expected compiled silently and truncated at
    /// runtime (`f(s)` returned 44 for `s.c == 300`, in
    /// tests/bugs/nested_tag_struct_identity.c) while GCC 16.2 rejects it.
    ///
    /// Anonymous records are exempt exactly as in `check_assignment_
    /// compatibility`, and so is a `transparent_union` parameter, which accepts
    /// its members by ABI design.
    /// Report an incompatible pointer argument. Kept as its own diagnostic
    /// rather than folded into the record one so a reader -- and the gate's
    /// reason checks -- can tell "wrong record" from "wrong pointee" apart.
    fn check_pointer_argument_compatibility(
        &self,
        from: &CType,
        to: &CType,
        name: &str,
        index: usize,
        span: Span,
    ) {
        if pointer_argument_compat(from, to) != PtrCompat::Incompatible {
            return;
        }
        let (from_text, to_text) = (from.to_string(), to.to_string());
        // Pointers to two distinct same-tag records print identically, exactly
        // as the records themselves do, so say what actually distinguishes them.
        let message = if from_text == to_text {
            format!(
                "incompatible pointer type for argument {} to '{}': '{}' here points at a different type from the '{}' of the parameter, despite the shared spelling",
                index, name, from_text, to_text
            )
        } else {
            format!(
                "incompatible pointer type for argument {} to '{}': have '{}' but expected '{}'",
                index, name, from_text, to_text
            )
        };
        self.diagnostics.borrow_mut().error(message, span);
    }

    /// GCC `transparent_union`, on the union or on a typedef of it: mark the
    /// sema layout the call check reads, as lowering's own
    /// `mark_transparent_union` marks its layout. GCC drops the attribute
    /// with this warning unless the union has its first member's machine
    /// mode -- measured on GCC 16.2: an empty union, a later member larger
    /// than the first, or a binary/decimal floating, complex or vector first
    /// member (never the union's integer mode) leaves an ordinary union.
    fn mark_transparent_union(&self, ty: &CType, span: Span) {
        let CType::Union(key) = ty else {
            return;
        };
        let valid = {
            let layouts = self.result.type_context.borrow_struct_layouts();
            let Some(layout) = layouts.get(&**key) else {
                return;
            };
            layout.fields.first().is_some_and(|first| {
                let fp_mode = first.ty.is_floating()
                    || first.ty.is_complex()
                    || first.ty.is_decimal()
                    || first.ty.is_half_float()
                    || matches!(first.ty, CType::Float128)
                    || first.ty.is_vector();
                !fp_mode && first.ty.size_ctx(&*layouts) == layout.size
            })
        };
        if !valid {
            self.diagnostics
                .borrow_mut()
                .warning("union cannot be made transparent".to_string(), span);
            return;
        }
        if let Some(layout) = self
            .result
            .type_context
            .borrow_struct_layouts_mut()
            .get_mut(&**key)
        {
            Rc::make_mut(layout).is_transparent_union = true;
        }
    }

    /// Whether `arg` may be passed for a `transparent_union` parameter: it is
    /// the union itself, or [`transparent_member_accepts`] one of its members.
    fn transparent_union_accepts(&self, arg: &Expr, arg_ty: &CType, union_ty: &CType) -> bool {
        if arg_ty == union_ty {
            return true;
        }
        let CType::Union(key) = union_ty else {
            return false;
        };
        let null_constant = self.expr_type_checker().is_null_pointer_constant(arg);
        self.result
            .type_context
            .borrow_struct_layouts()
            .get(&**key)
            .is_some_and(|layout| {
                layout
                    .fields
                    .iter()
                    .any(|m| transparent_member_accepts(&m.ty, arg_ty, null_constant))
            })
    }

    fn check_record_argument_compatibility(
        &self,
        from: &CType,
        to: &CType,
        name: &str,
        // 1-based argument position, so the diagnostic can say which argument
        // is wrong the way GCC does ("argument 2 of 'f'") instead of leaving
        // the reader to count them off the call.
        index: usize,
        span: Span,
    ) {
        let (a, b) = match (from, to) {
            (CType::Struct(a), CType::Struct(b)) | (CType::Union(a), CType::Union(b)) => (a, b),
            _ => return,
        };
        if a == b || a.starts_with("__anon_") || b.starts_with("__anon_") {
            // Anonymous records are exempt from this comparison, and the
            // exemption is load-bearing rather than lazy. lccc does NOT
            // guarantee one key per anonymous record type: source-level
            // spellings are stable (verified on nine: typedef by value, by
            // pointer, as a return type, as a member, in an array, as an array
            // element, behind a pointer member, in `_Generic`, and a declarator
            // pair), but a builtin's parameter type is constructed by a
            // different path than the header typedef it matches, and the two
            // get distinct `__anon_struct_N` keys for what is one type.
            //
            // Removing this exemption was tried and measured: it produced 70
            // semantic errors in `include/emmintrin.h` alone and failed 35 of
            // the regression corpus, all of them SIMD. Tightening it is only
            // safe once anonymous record keys are canonicalised across
            // construction paths -- tracked as ANON-KEY-1 in backlog.md. The
            // cost of leaving it is one accepted program GCC rejects (an
            // anonymous record passed where a named one was declared), which
            // is a missed diagnostic; the cost of removing it is rejecting
            // every SSE intrinsic, which is a broken compiler.
            return;
        }
        if matches!(to, CType::Union(_))
            && self
                .result
                .type_context
                .borrow_struct_layouts()
                .get(&**b)
                .is_some_and(|layout| layout.is_transparent_union)
        {
            return;
        }
        // Two records with the same tag but different definitions print
        // identically, so say what actually distinguishes them rather than
        // emitting "have 'struct S' but expected 'struct S'".
        let (from_text, to_text) = (from.to_string(), to.to_string());
        let message = if from_text == to_text {
            format!(
                "incompatible type for argument {} to '{}': '{}' here is a different type from the '{}' of the parameter, despite the shared tag",
                index, name, from_text, to_text
            )
        } else {
            format!(
                "incompatible type for argument {} to '{}': have '{}' but expected '{}'",
                index, name, from_text, to_text
            )
        };
        self.diagnostics.borrow_mut().error(message, span);
    }

    /// Implicit conversion between pointers into different named address
    /// spaces is a constraint violation. GCC 14.2 rejects `struct c __seg_gs
    /// *p = generic_ptr;` and the reverse, and `long __seg_gs *g = &n;`, each
    /// with "from pointer to non-enclosed address space"; lccc accepted all of
    /// them and then loaded or stored through the wrong segment.
    ///
    /// The check looks at the type the right-hand side has AFTER its cast, so
    /// `gp = (int *)q` is judged on `int *` (rejected, as GCC does) and
    /// `gp = (int __seg_gs *)q` on `int __seg_gs *` (accepted). The only
    /// exemption is `null_constant`: a `void *` null pointer constant such as
    /// `(void *)0` or `NULL`, which GCC accepts for any named pointer. `(int *)0`
    /// is NOT exempt, matching GCC 14.2.
    fn check_address_space_conversion(
        &self,
        from_ty: &CType,
        to_ty: &CType,
        span: Span,
        null_constant: bool,
    ) {
        if null_constant {
            return;
        }
        if let Some(layer) = first_space_mismatch(from_ty, to_ty) {
            self.diagnostics.borrow_mut().error(
                format!(
                    "incompatible address space in pointer conversion: cannot convert pointer to '{}' memory to pointer to '{}' memory at pointer level {} ('{}' to '{}')",
                    layer.from_space, layer.to_space, layer.depth, from_ty, to_ty
                ),
                span,
            );
        }
    }

    /// Is `expr` a null pointer constant of type `void *` (`(void *)0`,
    /// `((void *)0)`, or `NULL` spelled that way)? GCC accepts exactly this
    /// shape where a named-address-space pointer is expected. A bare integer
    /// `0` never reaches the address-space check, since it is not a pointer.
    fn is_void_null_pointer_constant(&self, expr: &Expr) -> bool {
        self.expr_type_checker().is_void_pointer_null_constant(expr)
    }

    /// Brace-list and compound-literal elements convert implicitly into their
    /// element objects, so each pointer element gets the scalar initializer's
    /// named-address-space check. GCC 14 rejects `gp_t t[1] = { q };`,
    /// `struct { gp_t p; } s = { q };` and `((gp_t[]){ q })[0]` with generic
    /// `q` ("from pointer to non-enclosed address space"); lccc accepted them
    /// and stored the generic pointer into a segment-typed object.
    ///
    /// The walk follows the object model of C11 6.7.9, not a shape guess. One
    /// braced list keeps a stack of open aggregates (`WalkFrame`), each with the
    /// index of its next subobject:
    ///
    /// * A positional item takes the next subobject of the innermost open
    ///   aggregate that still has one (p17), popping full aggregates.
    /// * An expression whose type does not match an aggregate subobject elides
    ///   the braces and initializes that subobject's first subobject (p20); a
    ///   braced list for a subobject is a new object of its own.
    /// * A designator resolves from the closest surrounding braced object (p17).
    ///   A name also finds members of unnamed struct and union members (6.7.2.2p13),
    ///   and the stack is rebuilt so the next positional item continues after the
    ///   designated subobject.
    /// * An unnamed bit-field is not a member and takes no slot (6.7.2.1p13).
    ///
    /// The only stop is a designator that does not resolve, which sema reports as
    /// an error; the walk then skips that item without guessing a target.
    fn check_initializer_element_spaces(&self, target: &CType, init: &Initializer) {
        match init {
            Initializer::Expr(expr) => self.check_element_space(target, expr),
            Initializer::List(items) => self.check_braced_element_spaces(target, items),
        }
    }

    /// One braced list for the object `target` (see `check_initializer_element_spaces`).
    fn check_braced_element_spaces(&self, target: &CType, items: &[InitializerItem]) {
        if !target.is_struct_or_union() && !matches!(target, CType::Array(..)) {
            // A scalar in braces (`gp_t x = { q };`) initializes from its first element.
            if let Some(first) = items.first() {
                self.check_initializer_element_spaces(target, &first.init);
            }
            return;
        }
        let mut stack = vec![WalkFrame {
            ty: target.clone(),
            next: 0,
        }];
        for item in items {
            if item.designators.is_empty() {
                if let Some(slot) = self.take_initializer_slot(&mut stack) {
                    self.check_slot_element_spaces(&mut stack, &slot, &item.init);
                }
            } else if let Some((frames, leaf)) = self.designated_slot(target, &item.designators) {
                stack = frames;
                self.check_slot_element_spaces(&mut stack, &leaf, &item.init);
            }
        }
    }

    /// The type of the next subobject a positional initializer takes. Full
    /// aggregates are popped, so the next slot belongs to the enclosing object.
    /// `None` when the braced object is full: an excess element that sema diagnoses.
    fn take_initializer_slot(&self, stack: &mut Vec<WalkFrame>) -> Option<CType> {
        loop {
            let top = stack.last_mut()?;
            if top.next < self.initializer_slot_count(&top.ty) {
                let index = top.next;
                top.next += 1;
                return self.initializer_slot_type(&top.ty, index);
            }
            if stack.len() == 1 {
                return None;
            }
            stack.pop();
        }
    }

    /// One initializer for the subobject `slot`. A braced list is a new object.
    /// An expression initializes `slot` whole when `slot` is a scalar or the
    /// expression has the same aggregate type (or is an array, e.g. a string);
    /// otherwise the braces are elided and the expression initializes the first
    /// subobject of `slot`, recursively (p20). An unknown expression type is
    /// treated as a whole initializer, which can only skip a check.
    fn check_slot_element_spaces(
        &self,
        stack: &mut Vec<WalkFrame>,
        slot: &CType,
        init: &Initializer,
    ) {
        match init {
            Initializer::List(items) => self.check_braced_element_spaces(slot, items),
            Initializer::Expr(expr) => {
                if self.expr_initializes_whole(slot, expr) {
                    self.check_element_space(slot, expr);
                    return;
                }
                stack.push(WalkFrame {
                    ty: slot.clone(),
                    next: 0,
                });
                if let Some(inner) = self.take_initializer_slot(stack) {
                    self.check_slot_element_spaces(stack, &inner, init);
                }
            }
        }
    }

    /// Whether `expr` initializes an aggregate `slot` as a whole, without brace
    /// elision: the expression has the aggregate's own type or is an array.
    fn expr_initializes_whole(&self, slot: &CType, expr: &Expr) -> bool {
        if !slot.is_struct_or_union() && !matches!(slot, CType::Array(..)) {
            return true;
        }
        let checker = self.expr_type_checker();
        match checker.infer_expr_ctype(expr) {
            Some(CType::Struct(_) | CType::Union(_) | CType::Array(..)) | None => true,
            Some(_) => false,
        }
    }

    /// The number of subobjects a positional initializer can take in `ty`:
    /// array elements (unbounded when the length is unknown), the
    /// initializable members of a struct, and the first member of a union.
    fn initializer_slot_count(&self, ty: &CType) -> usize {
        match ty {
            CType::Array(_, Some(len)) => *len,
            CType::Array(_, None) => usize::MAX,
            CType::Struct(key) | CType::Union(key) => {
                let layouts = self.result.type_context.borrow_struct_layouts();
                match layouts.get(&**key) {
                    Some(layout) if layout.is_union => 1,
                    Some(layout) => layout
                        .fields
                        .iter()
                        .filter(|f| is_initializer_slot(f))
                        .count(),
                    None => 0,
                }
            }
            _ => 0,
        }
    }

    /// The type of subobject `index` of `ty` (see `initializer_slot_count`).
    fn initializer_slot_type(&self, ty: &CType, index: usize) -> Option<CType> {
        match ty {
            CType::Array(elem, _) => Some((**elem).clone()),
            CType::Struct(key) | CType::Union(key) => {
                let layouts = self.result.type_context.borrow_struct_layouts();
                let layout = layouts.get(&**key)?;
                layout
                    .fields
                    .iter()
                    .filter(|f| is_initializer_slot(f))
                    .nth(index)
                    .map(|f| f.ty.clone())
            }
            _ => None,
        }
    }

    /// The index path, from a record to a named member, through unnamed
    /// struct and union members (C11 6.7.2.2p13). `None` if no member matches.
    fn designated_field_path(&self, ty: &CType, name: &str) -> Option<Vec<usize>> {
        let (CType::Struct(key) | CType::Union(key)) = ty else {
            return None;
        };
        let layouts = self.result.type_context.borrow_struct_layouts();
        let layout = layouts.get(&**key)?;
        let slots: Vec<&StructFieldLayout> = layout
            .fields
            .iter()
            .filter(|f| is_initializer_slot(f))
            .collect();
        if let Some(index) = slots.iter().position(|f| f.name == name) {
            return Some(vec![index]);
        }
        for (index, field) in slots.iter().enumerate() {
            if field.name.is_empty() && field.ty.is_struct_or_union() {
                if let Some(mut rest) = self.designated_field_path(&field.ty, name) {
                    rest.insert(0, index);
                    return Some(rest);
                }
            }
        }
        None
    }

    /// Resolves one designator list against the braced object `target`. Returns
    /// the rebuilt stack (one frame per object on the path, each with the index
    /// after the designated subobject) and the designated subobject's type.
    fn designated_slot(
        &self,
        target: &CType,
        designators: &[Designator],
    ) -> Option<(Vec<WalkFrame>, CType)> {
        let mut frames = Vec::new();
        let mut cur = target.clone();
        for designator in designators {
            match designator {
                Designator::Field(name) => {
                    for index in self.designated_field_path(&cur, name)? {
                        frames.push(WalkFrame {
                            ty: cur.clone(),
                            next: index + 1,
                        });
                        cur = self.initializer_slot_type(&cur, index)?;
                    }
                }
                Designator::Index(index) => {
                    let index = usize::try_from(self.eval_const_expr(index)?).ok()?;
                    frames.push(WalkFrame {
                        ty: cur.clone(),
                        next: index + 1,
                    });
                    cur = self.array_element_type(&cur)?;
                }
                Designator::Range(_, hi) => {
                    let hi = usize::try_from(self.eval_const_expr(hi)?).ok()?;
                    frames.push(WalkFrame {
                        ty: cur.clone(),
                        next: hi + 1,
                    });
                    cur = self.array_element_type(&cur)?;
                }
            }
        }
        Some((frames, cur))
    }

    fn array_element_type(&self, ty: &CType) -> Option<CType> {
        match ty {
            CType::Array(elem, _) => Some((**elem).clone()),
            _ => None,
        }
    }

    /// The scalar conversion check for one element expression (see
    /// `check_initializer_element_spaces`). Only pointer targets have a
    /// named-space conversion to check.
    fn check_element_space(&self, target: &CType, expr: &Expr) {
        if !matches!(target, CType::Pointer(..)) {
            return;
        }
        let checker = self.expr_type_checker();
        let Some(from) = checker.infer_expr_ctype(expr) else {
            return;
        };
        let from = self.conversion_source_ctype(expr, from);
        let null_constant = self.is_void_null_pointer_constant(expr);
        self.check_address_space_conversion(&from, target, expr.span(), null_constant);
    }

    /// Whether an integer expression is structurally constrained to 0 or 1.
    /// GCC's `-Wsign-compare` does not warn when one side is a boolean-valued
    /// comparison (including casts/bitwise combinations of comparisons).
    fn is_boolean_result_expr(expr: &Expr, ty: &CType) -> bool {
        if matches!(ty, CType::Bool) {
            return true;
        }
        match expr {
            Expr::BinaryOp(op, ..) if op.is_comparison() => true,
            Expr::UnaryOp(UnaryOp::LogicalNot, ..) => true,
            Expr::Cast(_, inner, ..) => Self::is_boolean_result_expr(inner, ty),
            Expr::BinaryOp(BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor, lhs, rhs, _) => {
                Self::is_boolean_result_expr(lhs, ty) && Self::is_boolean_result_expr(rhs, ty)
            }
            Expr::Conditional(_, then_expr, else_expr, _) => {
                Self::is_boolean_result_expr(then_expr, ty)
                    && Self::is_boolean_result_expr(else_expr, ty)
            }
            Expr::GnuConditional(condition, else_expr, _) => {
                Self::is_boolean_result_expr(condition, ty)
                    && Self::is_boolean_result_expr(else_expr, ty)
            }
            Expr::Comma(_, rhs, _) => Self::is_boolean_result_expr(rhs, ty),
            _ => false,
        }
    }

    /// GCC `-Wsign-compare`: diagnose mixed-sign comparisons only when the
    /// common arithmetic type is unsigned and neither a non-negative constant
    /// nor a boolean-valued operand makes the comparison harmless. GCC
    /// suppresses e.g. `unsigned_value < 13` and `signed_value == 0U`, but not
    /// a negative signed constant or a signed variable.
    fn warn_sign_compare(&self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) {
        if !matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ) {
            return;
        }
        let warning_kind = crate::common::error::WarningKind::SignCompare;
        if !self.diagnostics.borrow().warning_enabled(warning_kind) {
            return;
        }
        let checker = self.expr_type_checker();
        let (Some(lhs_type), Some(rhs_type)) =
            (checker.infer_expr_ctype(lhs), checker.infer_expr_ctype(rhs))
        else {
            return;
        };
        if !lhs_type.is_integer() || !rhs_type.is_integer() {
            return;
        }

        let lhs_promoted = lhs_type.integer_promoted();
        let rhs_promoted = rhs_type.integer_promoted();
        if lhs_promoted.is_signed() == rhs_promoted.is_signed() {
            return;
        }
        let common_type = CType::usual_arithmetic_conversion(&lhs_promoted, &rhs_promoted);
        if !common_type.is_integer() || common_type.is_signed() {
            return;
        }

        if Self::is_boolean_result_expr(lhs, &lhs_type)
            || Self::is_boolean_result_expr(rhs, &rhs_type)
        {
            return;
        }

        let lhs_constant = checker
            .is_integer_constant_expr(lhs)
            .then(|| self.eval_const_expr_value(lhs))
            .flatten()
            .and_then(IrConst::to_i128);
        let rhs_constant = checker
            .is_integer_constant_expr(rhs)
            .then(|| self.eval_const_expr_value(rhs))
            .flatten()
            .and_then(IrConst::to_i128);

        let unsigned_mask = |ty: &CType| -> Option<u128> {
            let bits = ty.size().checked_mul(8)?;
            match bits {
                0 => None,
                128.. => Some(u128::MAX),
                _ => Some((1u128 << bits) - 1),
            }
        };
        let signed_max = |ty: &CType| -> Option<u128> {
            let bits = ty.size().checked_mul(8)?;
            match bits {
                0 => None,
                128.. => Some(i128::MAX as u128),
                _ => Some((1u128 << (bits - 1)) - 1),
            }
        };

        let nonnegative_signed_constant_fits = lhs_promoted.is_signed()
            && lhs_constant.is_some_and(|value| {
                value >= 0 && unsigned_mask(&rhs_promoted).is_some_and(|max| (value as u128) <= max)
            })
            || rhs_promoted.is_signed()
                && rhs_constant.is_some_and(|value| {
                    value >= 0
                        && unsigned_mask(&lhs_promoted).is_some_and(|max| (value as u128) <= max)
                });
        let unsigned_constant_fits_signed = lhs_promoted.is_signed()
            && rhs_promoted.is_unsigned()
            && rhs_constant.is_some_and(|value| {
                unsigned_mask(&rhs_type)
                    .zip(signed_max(&lhs_promoted))
                    .is_some_and(|(mask, max)| ((value as u128) & mask) <= max)
            })
            || rhs_promoted.is_signed()
                && lhs_promoted.is_unsigned()
                && lhs_constant.is_some_and(|value| {
                    unsigned_mask(&lhs_type)
                        .zip(signed_max(&rhs_promoted))
                        .is_some_and(|(mask, max)| ((value as u128) & mask) <= max)
                });
        if nonnegative_signed_constant_fits || unsigned_constant_fits_signed {
            return;
        }

        self.diagnostics.borrow_mut().warning_with_kind(
            "comparison of integer expressions of different signedness",
            span,
            warning_kind,
        );
    }

    /// `==`, `!=`, `<`, ... and `-` between pointers whose pointees live in
    /// different named address spaces. GCC 14.2 errors with "comparison of pointers to
    /// disjoint address spaces" (`gp == q`, `gp == fp`, `gp == arr`), and
    /// accepts a `(void *)0` operand only for `==` and `!=`. Only the top-level
    /// pointee space is compared, as in GCC: `int __seg_gs **` against
    /// `int **` is accepted.
    fn check_pointer_comparison_spaces(&self, op: &BinOp, lhs: &Expr, rhs: &Expr, span: Span) {
        if !matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Sub
        ) {
            return;
        }
        // GCC 14.2 exempts a `(void *)0` operand only for `==` and `!=`: the
        // relational forms are still "comparison of pointers to disjoint
        // address spaces" (`gp < (void *)0`, `(void *)0 >= gp`).
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && (self.is_void_null_pointer_constant(lhs) || self.is_void_null_pointer_constant(rhs))
        {
            return;
        }
        let checker = self.expr_type_checker();
        let (Some(lt), Some(rt)) = (checker.infer_expr_ctype(lhs), checker.infer_expr_ctype(rhs))
        else {
            return;
        };
        // An array operand decays to a pointer to its element; a generic
        // array is a pointer into the default space.
        let decay = |ct: CType| match ct {
            CType::Array(elem, _) => CType::Pointer(elem, AddressSpace::Default),
            other => other,
        };
        let lt = decay(self.conversion_source_ctype(lhs, lt));
        let rt = decay(self.conversion_source_ctype(rhs, rt));
        if let (CType::Pointer(_, ls), CType::Pointer(_, rs)) = (&lt, &rt) {
            if ls != rs {
                let what = if *op == BinOp::Sub {
                    format!(
                        "invalid operands to binary - (pointers into disjoint address spaces: '{}' and '{}')",
                        lt, rt
                    )
                } else {
                    format!(
                        "comparison of pointers to disjoint address spaces ('{}' and '{}')",
                        lt, rt
                    )
                };
                self.diagnostics.borrow_mut().error(what, span);
            }
        }
    }

    /// `c ? p : q` with pointer operands in different named address spaces is
    /// an error ("pointers to disjoint address spaces used in conditional
    /// expression", GCC 14.2). A null pointer constant operand is exempt, as in
    /// `c ? gp : (void *)0`, and an array decays to a pointer as usual.
    fn check_conditional_pointer_spaces(&self, then_expr: &Expr, else_expr: &Expr, span: Span) {
        if self.is_void_null_pointer_constant(then_expr)
            || self.is_void_null_pointer_constant(else_expr)
        {
            return;
        }
        let checker = self.expr_type_checker();
        let (Some(tt), Some(et)) = (
            checker.infer_expr_ctype(then_expr),
            checker.infer_expr_ctype(else_expr),
        ) else {
            return;
        };
        let decay = |ct: CType| match ct {
            CType::Array(elem, _) => CType::Pointer(elem, AddressSpace::Default),
            other => other,
        };
        let tt = decay(self.conversion_source_ctype(then_expr, tt));
        let et = decay(self.conversion_source_ctype(else_expr, et));
        if let (CType::Pointer(_, ts), CType::Pointer(_, es)) = (&tt, &et) {
            if ts != es {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "pointers to disjoint address spaces used in conditional expression ('{}' and '{}')",
                        tt, et
                    ),
                    span,
                );
            }
        }
    }

    /// The type an expression has for the implicit-conversion address-space
    /// rule. An array in a named space decays to a pointer into that space,
    /// wherever the array is written: an identifier
    /// (`long __seg_gs arr[4]; long *p = arr;`), a member of a named object
    /// (`struct S __seg_gs g; long *p = g.a;`), or a `+`/`-` on either. The
    /// decayed pointer converts `%gs` memory to generic memory. Other
    /// expressions keep their type.
    fn conversion_source_ctype(&self, expr: &Expr, ct: CType) -> CType {
        match &ct {
            CType::Array(elem, _) => {
                let space = self.array_expr_space(expr);
                if space != AddressSpace::Default {
                    return CType::Pointer(elem.clone(), space);
                }
            }
            CType::Pointer(elem, AddressSpace::Default) => {
                if let Expr::BinaryOp(BinOp::Add | BinOp::Sub, lhs, rhs, _) = expr {
                    for operand in [lhs, rhs] {
                        if self
                            .expr_ctype(operand)
                            .is_some_and(|t| matches!(t, CType::Array(..)))
                        {
                            let space = self.array_expr_space(operand);
                            if space != AddressSpace::Default {
                                return CType::Pointer(elem.clone(), space);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        ct
    }

    /// The type of an expression, by the same checker the call sites use.
    fn expr_ctype(&self, expr: &Expr) -> Option<CType> {
        let checker = self.expr_type_checker();
        checker.infer_expr_ctype(expr)
    }

    /// The named address space an array-valued expression lives in, or
    /// `Default`. An identifier answers with its own declared space; a member
    /// of an object answers with the object's space (the member inherits it);
    /// a pointer member answers with the pointee space of its base.
    fn array_expr_space(&self, expr: &Expr) -> AddressSpace {
        match expr {
            Expr::Identifier(name, _) => self
                .symbol_table
                .lookup(name)
                .map_or(AddressSpace::Default, |sym| sym.address_space),
            Expr::MemberAccess(base, _, _) => self.object_expr_space(base),
            Expr::PointerMemberAccess(base, _, _) => self.pointee_space(base),
            Expr::ArraySubscript(base, _, _) => self.element_space(base),
            _ => AddressSpace::Default,
        }
    }

    /// The named space of an object expression (the base of a `.` access).
    fn object_expr_space(&self, expr: &Expr) -> AddressSpace {
        match expr {
            Expr::Identifier(..) | Expr::MemberAccess(..) | Expr::ArraySubscript(..) => {
                self.array_expr_space(expr)
            }
            Expr::PointerMemberAccess(base, _, _) => self.pointee_space(base),
            _ => AddressSpace::Default,
        }
    }

    /// The named space of an element of `base`: an array's element lives in
    /// the array's space; a pointer's element lives in its pointee space.
    fn element_space(&self, base: &Expr) -> AddressSpace {
        match self.expr_ctype(base) {
            Some(CType::Array(..)) => self.array_expr_space(base),
            Some(CType::Pointer(_, space)) => space,
            _ => AddressSpace::Default,
        }
    }

    /// The pointee space of a pointer-valued expression (the base of a `->`).
    fn pointee_space(&self, expr: &Expr) -> AddressSpace {
        match self.expr_ctype(expr) {
            Some(CType::Pointer(_, space)) => space,
            _ => AddressSpace::Default,
        }
    }

    /// An explicit cast that *removes* or *switches* a named address space is
    /// accepted (a deliberate conversion) but GCC 14.2 warns about it. One
    /// direction is silent in GCC: a *null pointer constant* converted into a
    /// named address space — the canonical per-cpu sentinel
    /// `(typeof(&percpu_var))(void *)0` inside -Werror kernel builds. Casting
    /// a real generic pointer into a named space does warn, and named -> named
    /// warns even through a null constant (verified against GCC 12/14).
    /// Wording follows GCC's two directions.
    fn check_explicit_cast_address_space(&self, cast: &Expr, inner: &Expr, span: Span) {
        let checker = self.expr_type_checker();
        let (Some(CType::Pointer(_, from_space)), Some(CType::Pointer(_, to_space))) = (
            checker.infer_expr_ctype(inner),
            checker.infer_expr_ctype(cast),
        ) else {
            return;
        };
        if from_space == to_space {
            return;
        }
        // GCC: (T __seg_gs *)(void *)0 is silent; only a genuinely pointer-
        // valued generic operand entering a named space warns.
        if from_space == AddressSpace::Default
            && to_space != AddressSpace::Default
            && self.is_void_null_pointer_constant(inner)
        {
            return;
        }
        let message = if to_space == AddressSpace::Default {
            format!(
                "cast to generic address space pointer from disjoint '{}' address space pointer",
                from_space
            )
        } else if from_space == AddressSpace::Default {
            format!(
                "cast to '{}' address space pointer from disjoint generic address space pointer",
                to_space
            )
        } else {
            format!(
                "cast to '{}' address space pointer from disjoint '{}' address space pointer",
                to_space, from_space
            )
        };
        self.diagnostics.borrow_mut().warning(message, span);
    }

    /// Scalar <-> vector conversions require `sizeof` equality. GCC and
    /// Clang diagnose every size mismatch in both directions: explict casts
    /// get "cannot convert a value of type 'int' to vector type ... which
    /// has different size" / "invalid conversion between vector type ...
    /// and integer type ... of different size" (applied to explicit casts
    /// too), and implicit assignments/inits get the plain "incompatible
    /// types" error. Equal-size conversions are legal and are the
    /// GNU-documented raw bit-reinterpretation in both directions.
    /// Arbitrary probe (both oracles agree): `(int)` of an 8-byte
    /// `vector_size(8)` double is rejected; `(int)` of a `vector_size(4)`
    /// float reinterprets lane bits.
    fn is_vector_scalar_size_mismatch(from_ty: &CType, to_ty: &CType) -> bool {
        // Struct/union sizes are 0 from CType::size() (they need a layout
        // context); scalar-vs-vector comparisons never involve aggregates on
        // either side because one arm is always a vector below.
        let one_vector = from_ty.is_vector() != to_ty.is_vector();
        if !one_vector {
            return false;
        }
        // The other (non-vector) side must be an arithmetic scalar: integers,
        // reals, and _Complex. (GCC's front end also accepts equal-size
        // pointer<->vector raw conversions, but rejecting only size
        // mismatches for arithmetic types is the GCC/Clang semantics at
        // issue here; pointer opposite sides are outside this rule.)
        let other = if from_ty.is_vector() { to_ty } else { from_ty };
        let other_scalar = other.is_integer() || other.is_floating() || other.is_complex();
        other_scalar && from_ty.size() != to_ty.size()
    }

    fn check_assignment_compatibility(
        &self,
        from_ty: &CType,
        to_ty: &CType,
        span: Span,
        is_explicit_cast: bool,
        null_constant: bool,
    ) {
        self.check_pointer_float_conversion(from_ty, to_ty, span);
        let incompatible = match (from_ty, to_ty) {
            (CType::Struct(a), CType::Struct(b)) | (CType::Union(a), CType::Union(b)) => {
                a != b && !a.starts_with("__anon_") && !b.starts_with("__anon_")
            }
            (CType::Struct(_), _)
            | (CType::Union(_), _)
            | (_, CType::Struct(_))
            | (_, CType::Union(_)) => true,
            // Vectors of different sizes never convert implicitly (GCC:
            // "incompatible types when assigning"; Clang: "assigning to
            // 'float2' from incompatible type 'float4'"), including a
            // multi-component swizzle target (`v.xy = v`). Same-size
            // vectors with different lanes are GCC's lax conversion and
            // a scalar converts to an ext vector by splat; both stay
            // accepted.
            (
                CType::Vector(..) | CType::ExtVector(..),
                CType::Vector(..) | CType::ExtVector(..),
            ) => from_ty.size() != to_ty.size(),
            // NOTE: `match (from_ty, to_ty)` — the FIRST element of each
            // tuple pattern is the CONVERSION SOURCE, the SECOND is the
            // DESTINATION type. Keep that orientation in every vector arm
            // below; a previous iteration bound the destination in the
            // first slot and the diagnostic silently never fired.
            //
            // Implicit scalar -> ext vector: the documented Clang splat
            // ("Scalar right operand: Clang splats it over the named
            // lanes"). Stay accepted.
            (from, CType::ExtVector(..))
                if from.is_integer()
                    || from.is_floating()
                    || from.is_soft_float()
                    || from.is_complex() =>
            {
                false
            }
            // Implicit scalar -> plain GNU vector: oracles reject at ANY
            // size ("incompatible types when assigning") — including the
            // same-size case. Accepting it also miscompiled: the scalar
            // rvalue reached vector store lowering as an ADDRESS (witness:
            // `b = i;` for `vector_size(4)` float emitted `movl (%rax)`
            // with %rax == 5 and faulted at runtime).
            (from, CType::Vector(..))
                if from.is_integer()
                    || from.is_floating()
                    || from.is_soft_float()
                    || from.is_complex() =>
            {
                true
            }
            // Implicit vector -> scalar (plain or ext): rejected by both
            // oracles at any size (only explicit equal-size casts are
            // legal: the GNU raw bit reinterpretation).
            (from, to)
                if from.is_vector()
                    && (to.is_integer()
                        || to.is_floating()
                        || to.is_soft_float()
                        || to.is_complex()) =>
            {
                true
            }
            _ => false,
        };
        if incompatible {
            self.diagnostics.borrow_mut().error(
                format!(
                    "incompatible types (have '{}' but expected '{}')",
                    from_ty, to_ty
                ),
                span,
            );
        }
        self.check_address_space_conversion(from_ty, to_ty, span, null_constant);
        // GCC/clang suppress the incompatible-pointer-types diagnostic when the
        // source is an EXPLICIT cast (C-style `(T*)expr`): the cast is an
        // intentional conversion, so emitting a hard error here breaks
        // macro-heavy kernel code that casts through `typeof` (e.g. the xchg
        // macro in include/net/sock.h `__ret = (typeof(*(dst)) *)(dst);`
        // expands `dst` (struct dst_entry *) through an int-typed
        // sk_dst_cache xchg, and the cast authoritatively reinterprets it).
        // Only IMPLICIT conversions (assignment / return / argument passing
        // without a cast) get the diagnostic, matching GCC's `-Werror=
        // incompatible-pointer-types` behaviour the kernel relies on.
        if !is_explicit_cast {
            if let (CType::Pointer(from, _), CType::Pointer(to, _)) = (from_ty, to_ty) {
                if !matches!(from.as_ref(), CType::Void)
                    && !matches!(to.as_ref(), CType::Void)
                    && !Self::pointee_types_compatible(from, to)
                {
                    // Emitted as a WARNING (not a hard error) with kind
                    // `IncompatiblePointerTypes`. lccc's type inference for
                    // macro-heavy code (`typeof` + statement-expressions +
                    // `instrument_atomic_read_write` / `xchg` machinery, as in
                    // include/net/sock.h) produces false-positives that GCC
                    // does not flag, so promoting to a hard error (the
                    // kernel's `-Werror=incompatible-pointer-types`) would
                    // break legitimate kernel builds. The warning is still
                    // VISIBLE in the build log (not silently dead); it just
                    // does not fail the compile until lccc's macro type
                    // inference matches GCC's behaviour. `-Werror=
                    // incompatible-pointer-types` is intentionally NOT
                    // registered in `from_flag_name` so it cannot promote
                    // this warning to an error.
                    self.diagnostics.borrow_mut().warning_with_kind(
                        format!(
                            "incompatible pointer types (have '{}' but expected '{}')",
                            from_ty, to_ty
                        ),
                        span,
                        crate::common::error::WarningKind::IncompatiblePointerTypes,
                    );
                }
            }
        }
    }

    /// Check for invalid implicit conversions between pointer and floating-point types.
    /// C11 6.5.16.1p1: pointer types cannot be implicitly converted to/from floating-point types.
    /// This catches cases like `float f = ptr;`, `func_taking_float(ptr)`, etc.
    fn check_pointer_float_conversion(&self, from_ty: &CType, to_ty: &CType, span: Span) {
        let from_is_ptr = from_ty.is_pointer_like() || matches!(from_ty, CType::Function(_));
        let to_is_ptr = to_ty.is_pointer_like() || matches!(to_ty, CType::Function(_));
        let from_is_float = from_ty.is_floating() || from_ty.is_complex();
        let to_is_float = to_ty.is_floating() || to_ty.is_complex();

        if (from_is_ptr && to_is_float) || (from_is_float && to_is_ptr) {
            self.diagnostics.borrow_mut().error(
                format!(
                    "incompatible types (have '{}' but expected '{}')",
                    from_ty, to_ty
                ),
                span,
            );
        }
    }

    /// Check if two pointee types are compatible for pointer arithmetic.
    /// This is a simplified check: types are compatible if they are structurally
    /// the same (ignoring qualifiers at the top level).
    fn pointee_types_compatible(a: &CType, b: &CType) -> bool {
        match (a, b) {
            // Same struct/union name -> compatible
            (CType::Struct(ka), CType::Struct(kb)) => ka == kb,
            (CType::Union(ka), CType::Union(kb)) => ka == kb,
            // Pointers: recurse on pointee types
            (CType::Pointer(pa, _), CType::Pointer(pb, _)) => {
                Self::pointee_types_compatible(pa, pb)
            }
            // Arrays: compare element types
            (CType::Array(ea, _), CType::Array(eb, _)) => Self::pointee_types_compatible(ea, eb),
            // Function types: just check broad compatibility
            (CType::Function(_), CType::Function(_)) => {
                // Simplification: treat all function pointers as compatible
                true
            }
            // Enum types are integer types in C (C11 6.7.2.2p4), so enum pointers
            // are compatible with integer pointers of the same size for pointer
            // arithmetic. This is needed for QEMU's VMSTATE_UINT32 macro pattern:
            // (uint32_t*)0 - (typeof(enum_field)*)0
            (CType::Enum(e), other) | (other, CType::Enum(e)) if other.is_integer() => {
                e.packed_size() == other.size()
            }
            // GNU C: pointers to the character types (char, signed char,
            // unsigned char) are mutually assignment-compatible. GCC emits
            // only a `-Wpointer-sign` warning for the mismatch — it never
            // classifies it as `-Wincompatible-pointer-types`/hard error,
            // even under `-Werror=incompatible-pointer-types`. The kernel
            // relies on this constantly (e.g. `const char *` -> `const
            // unsigned char *` in fs/namei.c `nd->last.name = name;`).
            // Any pair of the three character types, including the identity
            // aliases (`char *` vs `signed char *` on x86 is `-Wpointer-sign`).
            (a, b) if a.is_char_type() && b.is_char_type() => true,
            // GCC -Wpointer-sign: pointers to integer types of the SAME SIZE
            // are assignment-compatible regardless of signedness — the
            // difference is a warning at most, never
            // `-Wincompatible-pointer-types`. The kernel passes
            // -Wno-pointer-sign and relies on this constantly (e.g.
            // percpu-rwsem.c assigns `alloc_percpu(int)` — a `__seg_gs int *`
            // — to the `unsigned int __percpu *read_count` field).
            (a, b) if a.is_integer() && b.is_integer() && a.size() == b.size() => true,
            // For basic types, use equality (ignoring qualifiers)
            _ => a == b,
        }
    }

    fn analyze_initializer(&mut self, init: &Initializer) {
        match init {
            Initializer::Expr(expr) => {
                self.analyze_expr(expr);
            }
            Initializer::List(items) => {
                for item in items {
                    self.analyze_initializer(&item.init);
                }
            }
        }
    }

    /// Check if a sizeof type argument refers to an incomplete struct/union.
    /// Reports an error like GCC: "invalid application of 'sizeof' to incomplete type"
    fn check_sizeof_incomplete_type(&self, ts: &TypeSpecifier, span: Span) {
        match ts {
            TypeSpecifier::Struct(Some(tag), None, ..) => {
                // Reference to a named struct with no inline body.
                // Check if it was previously defined (key format: "struct.tag").
                // Deliberately the BASE key, not resolve_record_key: this asks
                // whether the tag has been defined at all, and the definition
                // path records a variant under both its own key and the base
                // key, so the base is exactly the right question here. The size
                // itself comes from const_eval's sizeof, which does resolve.
                let key = format!("struct.{}", tag);
                if !self.defined_structs.borrow().contains(key.as_str()) {
                    self.diagnostics.borrow_mut().error(
                        format!("storage size of 'struct {}' isn't known", tag),
                        span,
                    );
                }
            }
            TypeSpecifier::Union(Some(tag), None, ..) => {
                // Base key on purpose; see the struct arm above.
                let key = format!("union.{}", tag);
                if !self.defined_structs.borrow().contains(key.as_str()) {
                    self.diagnostics
                        .borrow_mut()
                        .error(format!("storage size of 'union {}' isn't known", tag), span);
                }
            }
            // Pointers to incomplete types are fine - sizeof gives pointer size
            TypeSpecifier::Pointer(_, _) => {}
            // Arrays of incomplete element types - check the element type
            TypeSpecifier::Array(elem, _) => {
                self.check_sizeof_incomplete_type(elem, span);
            }
            _ => {}
        }
    }

    // === Type conversion utilities ===

    /// CType of a function parameter, including C11 6.7.6.3 adjustments
    /// (array→pointer, function→pointer-to-function) and the parser's
    /// `fptr_params` encoding of `T (*name)(args)`.
    ///
    /// Mirrors `Lowerer::param_ctype`. Using only `type_spec` typed
    /// `int (*xStress)(void*, PgHdr*)` as `int *` and rejected
    /// `p->xStress = xStress` in the SQLite amalgamation.
    fn param_decl_ctype(&self, param: &crate::frontend::parser::ast::ParamDecl) -> CType {
        // One implementation, in `type_builder`, shared with the prototype path
        // so a signature cannot depend on whether the body has been seen. See
        // `param_decl_to_ctype` for why the duplication was a live defect.
        type_builder::param_decl_to_ctype(self, param)
    }

    /// Convert an AST TypeSpecifier to a CType.
    /// Delegates to the shared `TypeConvertContext::resolve_type_spec_to_ctype` default
    /// method, which handles all 22 primitive types and delegates struct/union/enum/typedef
    /// to sema-specific trait methods.
    fn type_spec_to_ctype(&self, spec: &TypeSpecifier) -> CType {
        use crate::common::type_builder::TypeConvertContext;
        self.resolve_type_spec_to_ctype(spec)
    }

    fn convert_struct_fields(
        &self,
        fields: &[StructFieldDecl],
    ) -> Vec<crate::common::types::StructField> {
        fields
            .iter()
            .filter_map(|f| {
                let ty = if f.derived.is_empty() {
                    self.type_spec_to_ctype(&f.type_spec)
                } else {
                    // Delegate to shared build_full_ctype for correct inside-out type construction
                    type_builder::build_full_ctype(self, &f.type_spec, &f.derived)
                };
                // A by-value record (including an array element) must be
                // complete at this point. Pointers to incomplete records are
                // legal and deliberately stop this walk. Do not install a
                // self-referential layout after reporting the constraint.
                let mut object = &ty;
                while let CType::Array(inner, _) = object {
                    object = inner;
                }
                if let CType::Struct(key) | CType::Union(key) = object {
                    if !self.defined_structs.borrow().contains(key.as_ref())
                        && !self
                            .result
                            .type_context
                            .borrow_struct_layouts()
                            .get(key.as_ref())
                            .is_some_and(|layout| layout.is_complete)
                    {
                        self.diagnostics.borrow_mut().error(
                            format!(
                                "field '{}' has incomplete type",
                                f.name.as_deref().unwrap_or("<anonymous>")
                            ),
                            f.span,
                        );
                        return None;
                    }
                }
                let name = f.name.clone().unwrap_or_default();
                let bit_width = f
                    .bit_width
                    .as_ref()
                    .and_then(|bw| self.eval_const_expr(bw).map(|v| v as u32));
                // Merge per-field alignment with typedef alignment.
                // If the field's type is a typedef with __aligned__, that alignment
                // must be applied even when the field itself has no explicit alignment.
                let alignas_type = f.alignas_type.as_ref().map(|ts| {
                    let natural = self
                        .type_spec_to_ctype(ts)
                        .align_ctx(&*self.result.type_context.borrow_struct_layouts());
                    natural.max(self.resolve_typedef_alignment(ts).unwrap_or(0))
                });
                let field_alignment =
                    crate::common::types::strictest_alignment([f.alignment, alignas_type]);
                // A typedef-level `aligned` REPLACES the type's alignment
                // (it may lower it); see `StructField::type_align`.
                let type_align = self.resolve_typedef_alignment(&f.type_spec);
                // C11 6.7.2.1p13: unnamed non-aggregate members are constraint
                // violations GCC ignores entirely; drop from layout + init.
                if f.name.is_none()
                    && bit_width.is_none()
                    && !matches!(
                        ty,
                        crate::common::types::CType::Struct(_)
                            | crate::common::types::CType::Union(_)
                    )
                {
                    return None;
                }
                Some(crate::common::types::StructField {
                    name,
                    ty,
                    bit_width,
                    alignment: field_alignment,
                    type_align,
                    is_packed: f.is_packed,
                    base_volatile: f.base_volatile,
                    base_const: f.base_const,
                    object_volatile: f.object_volatile,
                    object_const: f.object_const,
                    counted_by: f.counted_by.clone(),
                    pointer_quals: {
                        let mut quals = self.base_typedef_pointer_quals(&f.type_spec);
                        quals.extend(type_builder::declarator_pointer_quals(&f.derived));
                        quals
                    },
                })
            })
            .collect()
    }

    fn find_sibling_field_ctype<'a>(
        &self,
        fields: &'a [StructFieldDecl],
        target: &str,
    ) -> Option<CType> {
        for f in fields {
            if f.name.as_deref() == Some(target) {
                let ty = if f.derived.is_empty() {
                    self.type_spec_to_ctype(&f.type_spec)
                } else {
                    type_builder::build_full_ctype(self, &f.type_spec, &f.derived)
                };
                return Some(ty);
            }
            if f.name.is_none() {
                if let TypeSpecifier::Struct(_, Some(sub), ..)
                | TypeSpecifier::Union(_, Some(sub), ..) = &f.type_spec
                {
                    if let Some(ty) = self.find_sibling_field_ctype(sub, target) {
                        return Some(ty);
                    }
                }
            }
        }
        None
    }

    fn check_c2y_builtin_call(&mut self, name: &str, args: &[Expr], span: Span) {
        use crate::frontend::sema::stdbit;
        if let Some(expected) = stdbit::stdc_bit_arity(name) {
            if args.len() != expected {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "{} arguments to function '{}' (expected {}, have {})",
                        if args.len() < expected {
                            "too few"
                        } else {
                            "too many"
                        },
                        name,
                        expected,
                        args.len()
                    ),
                    span,
                );
                return;
            }
            if let Some(arg0_ct) = self.expr_type_checker().infer_expr_ctype(&args[0]) {
                let decayed = arg0_ct.decayed_value_ctype();
                if !stdbit::is_valid_stdc_bit_arg_ctype(&decayed) {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "argument 1 in call to function '{}' has type '{}', expected unsigned integer type (not '_Bool' or 'char')",
                            name, decayed
                        ),
                        args[0].span(),
                    );
                }
            }
            if expected == 2 {
                if let Some(arg1_ct) = self.expr_type_checker().infer_expr_ctype(&args[1]) {
                    let decayed = arg1_ct.decayed_value_ctype();
                    if !decayed.is_integer() || matches!(decayed, CType::Bool) {
                        self.diagnostics.borrow_mut().error(
                            format!(
                                "argument 2 in call to function '{}' must have non-boolean integer type",
                                name
                            ),
                            args[1].span(),
                        );
                    }
                }
            }
            return;
        }
        match name {
            "__builtin_counted_by_ref" => {
                if args.len() != 1 {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "wrong number of arguments to '{}' (expected 1, have {})",
                            name,
                            args.len()
                        ),
                        span,
                    );
                    return;
                }
                if !matches!(
                    args[0],
                    Expr::MemberAccess(..) | Expr::PointerMemberAccess(..)
                ) {
                    self.diagnostics.borrow_mut().error(
                        "argument to '__builtin_counted_by_ref' must be a structure member access"
                            .to_string(),
                        args[0].span(),
                    );
                } else if let Some(ct) = self.expr_type_checker().infer_expr_ctype(&args[0]) {
                    let ok = matches!(ct, CType::Array(..) | CType::Pointer(..));
                    if !ok {
                        self.diagnostics.borrow_mut().error(
                            "argument to '__builtin_counted_by_ref' must be an array or pointer member"
                                .to_string(),
                            args[0].span(),
                        );
                    }
                }
            }
            "__builtin_assoc_barrier" => {
                if args.len() != 1 {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "wrong number of arguments to '{}' (expected 1, have {})",
                            name,
                            args.len()
                        ),
                        span,
                    );
                }
            }
            "__builtin_clear_padding" => {
                if args.len() != 1 {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "wrong number of arguments to '{}' (expected 1, have {})",
                            name,
                            args.len()
                        ),
                        span,
                    );
                    return;
                }
                let tc = self.expr_type_checker();
                if let Some(ct) = tc.infer_expr_ctype(&args[0]) {
                    match ct.decayed_value_ctype() {
                        CType::Pointer(pointee, _) => {
                            if crate::common::qualifiers::expr_const_base(&args[0], &tc) {
                                self.diagnostics.borrow_mut().error(
                                    "argument 1 in call to '__builtin_clear_padding' has pointer to 'const' type"
                                        .to_string(),
                                    args[0].span(),
                                );
                            } else {
                                let incomplete = match pointee.as_ref() {
                                    CType::Void | CType::Array(_, None) => true,
                                    CType::Struct(k) | CType::Union(k) => {
                                        !self.defined_structs.borrow().contains(&**k)
                                    }
                                    _ => false,
                                };
                                if incomplete {
                                    self.diagnostics.borrow_mut().error(
                                        "argument 1 in call to '__builtin_clear_padding' points to incomplete type"
                                            .to_string(),
                                        args[0].span(),
                                    );
                                }
                            }
                        }
                        _ => {
                            self.diagnostics.borrow_mut().error(
                                "argument 1 in call to '__builtin_clear_padding' must be a pointer"
                                    .to_string(),
                                args[0].span(),
                            );
                        }
                    }
                }
            }
            _ => {
                self.check_gcc16_builtin_call(name, args, span);
            }
        }
    }

    fn check_gcc16_builtin_call(&mut self, name: &str, args: &[Expr], span: Span) {
        use crate::frontend::sema::gcc16_builtins;
        if gcc16_builtins::is_gcc16_generic_bit_builtin(name) {
            let max_args = if gcc16_builtins::generic_bit_allows_fallback(name) {
                2
            } else {
                1
            };
            if args.is_empty() || args.len() > max_args {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "{} arguments to function '{}'",
                        if args.is_empty() {
                            "too few"
                        } else {
                            "too many"
                        },
                        name
                    ),
                    span,
                );
                return;
            }
            let tc = self.expr_type_checker();
            if let Some(ct0) = tc.infer_expr_ctype(&args[0]) {
                let d0 = ct0.decayed_value_ctype();
                if let Err(msg) = gcc16_builtins::validate_generic_bit_arg0(name, &d0) {
                    self.diagnostics.borrow_mut().error(msg, args[0].span());
                    return;
                }
            }
            if args.len() == 2 {
                if let Some(ct1) = tc.infer_expr_ctype(&args[1]) {
                    let d1 = ct1.decayed_value_ctype();
                    if let Err(msg) = gcc16_builtins::validate_generic_bit_fallback_arg(name, &d1) {
                        self.diagnostics.borrow_mut().error(msg, args[1].span());
                    }
                }
            }
            return;
        }
        if let Some((elem_ct, _, _)) = gcc16_builtins::carry_builtin_info(name) {
            if args.len() != 4 {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "{} arguments to function '{}' (expected 4, have {})",
                        if args.len() < 4 {
                            "too few"
                        } else {
                            "too many"
                        },
                        name,
                        args.len()
                    ),
                    span,
                );
                return;
            }
            let tc = self.expr_type_checker();
            for (idx, arg) in args.iter().take(3).enumerate() {
                if let Some(ct) = tc.infer_expr_ctype(arg) {
                    if !ct.decayed_value_ctype().is_integer() {
                        self.diagnostics.borrow_mut().error(
                            format!(
                                "argument {} in call to '{}' must have integer type",
                                idx + 1,
                                name
                            ),
                            arg.span(),
                        );
                    }
                }
            }
            if let Some(ct3) = tc.infer_expr_ctype(&args[3]) {
                match ct3.decayed_value_ctype() {
                    CType::Pointer(pointee, _) => {
                        if crate::common::qualifiers::expr_const_base(&args[3], &tc) {
                            self.diagnostics.borrow_mut().error(
                                format!(
                                    "argument 4 in call to '{}' has pointer to 'const' type",
                                    name
                                ),
                                args[3].span(),
                            );
                        } else if *pointee != elem_ct
                            && !(matches!(*pointee, CType::PlainUChar)
                                && matches!(elem_ct, CType::UChar))
                        {
                            self.diagnostics.borrow_mut().error(
                                format!(
                                    "incompatible pointer type '{}' for argument 4 of '{}' (expected '{} *')",
                                    ct3, name, elem_ct
                                ),
                                args[3].span(),
                            );
                        }
                    }
                    _ => {
                        self.diagnostics.borrow_mut().error(
                            format!("argument 4 in call to '{}' must be a pointer", name),
                            args[3].span(),
                        );
                    }
                }
            }
            return;
        }
        if gcc16_builtins::is_gcc16_bswap_bitrev_builtin(name) {
            let expected = if gcc16_builtins::rotate_builtin_info(name).is_some() {
                2usize
            } else {
                1usize
            };
            if args.len() != expected {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "{} arguments to function '{}' (expected {}, have {})",
                        if args.len() < expected {
                            "too few"
                        } else {
                            "too many"
                        },
                        name,
                        expected,
                        args.len()
                    ),
                    span,
                );
                return;
            }
            for (idx, arg) in args.iter().enumerate() {
                if let Some(ct) = self.expr_type_checker().infer_expr_ctype(arg) {
                    if !ct.decayed_value_ctype().is_integer() {
                        self.diagnostics.borrow_mut().error(
                            format!(
                                "argument {} in call to '{}' must have integer type",
                                idx + 1,
                                name
                            ),
                            arg.span(),
                        );
                    }
                }
            }
            return;
        }
        match name {
            "__builtin_issignaling" => {
                if args.len() != 1 {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "{} arguments to function '{}' (expected 1, have {})",
                            if args.is_empty() {
                                "too few"
                            } else {
                                "too many"
                            },
                            name,
                            args.len()
                        ),
                        span,
                    );
                    return;
                }
                if let Some(ct) = self.expr_type_checker().infer_expr_ctype(&args[0]) {
                    let d = ct.decayed_value_ctype();
                    if !(d.is_floating() || d.is_soft_float()) || d.is_complex() {
                        self.diagnostics.borrow_mut().error(
                            "non-floating-point argument in call to function '__builtin_issignaling'"
                                .to_string(),
                            args[0].span(),
                        );
                    }
                }
            }
            "__builtin_iseqsig" => {
                if args.len() != 2 {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "{} arguments to function '{}' (expected 2, have {})",
                            if args.len() < 2 {
                                "too few"
                            } else {
                                "too many"
                            },
                            name,
                            args.len()
                        ),
                        span,
                    );
                    return;
                }
                let tc = self.expr_type_checker();
                if let (Some(c0), Some(c1)) =
                    (tc.infer_expr_ctype(&args[0]), tc.infer_expr_ctype(&args[1]))
                {
                    let d0 = c0.decayed_value_ctype();
                    let d1 = c1.decayed_value_ctype();
                    let real_fp =
                        |t: &CType| (t.is_floating() || t.is_soft_float()) && !t.is_complex();
                    let real_arith = |t: &CType| t.is_integer() || real_fp(t);
                    let any_real_fp = real_fp(&d0) || real_fp(&d1);
                    if !real_arith(&d0) || !real_arith(&d1) || !any_real_fp {
                        self.diagnostics.borrow_mut().error(
                            "non-floating-point arguments in call to function '__builtin_iseqsig'"
                                .to_string(),
                            span,
                        );
                    }
                }
            }
            "__builtin_speculation_safe_value" => {
                if args.is_empty() || args.len() > 2 {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "{} arguments to function '{}'",
                            if args.is_empty() {
                                "too few"
                            } else {
                                "too many"
                            },
                            name
                        ),
                        span,
                    );
                    return;
                }
                let tc = self.expr_type_checker();
                if let Some(c0) = tc.infer_expr_ctype(&args[0]) {
                    let d0 = c0.decayed_value_ctype();
                    if !d0.is_integer() && !matches!(d0, CType::Pointer(..)) {
                        self.diagnostics.borrow_mut().error(
                            "operand of '__builtin_speculation_safe_value' must have integral or pointer type"
                                .to_string(),
                            args[0].span(),
                        );
                        return;
                    }
                    if args.len() == 2 {
                        if let Some(c1) = tc.infer_expr_ctype(&args[1]) {
                            let d1 = c1.decayed_value_ctype();
                            let ok = if d0.is_integer() {
                                d1.is_integer()
                            } else {
                                matches!(d1, CType::Pointer(..))
                                    || tc.is_null_pointer_constant(&args[1])
                            };
                            if !ok {
                                self.diagnostics.borrow_mut().error(
                                    "incompatible second argument to '__builtin_speculation_safe_value'"
                                        .to_string(),
                                    args[1].span(),
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn analyze_compound_literal_expr(
        &mut self,
        cl_expr: &Expr,
        ts: &TypeSpecifier,
        init: &Initializer,
        span: Span,
        storage: &crate::frontend::parser::ast::CompoundLiteralStorage,
    ) {
        self.check_void_members(ts, span);
        let empty = matches!(init, Initializer::List(items) if items.is_empty());
        let role = VoidRole::CompoundLiteral { empty };
        self.report_void_declarator(ts, &[], None, role, span);
        if self.symbol_table.at_file_scope() && storage.is_register {
            self.diagnostics.borrow_mut().error(
                "file-scope compound literal cannot have 'register' storage class".to_string(),
                span,
            );
        }
        if !self.symbol_table.at_file_scope() && storage.is_thread_local && !storage.is_static {
            self.diagnostics.borrow_mut().error(
                "block-scope 'thread_local' compound literal requires 'static'".to_string(),
                span,
            );
        }
        self.analyze_initializer(init);
        let ct = self.type_spec_to_ctype(ts);
        self.check_initializer_element_spaces(&ct, init);
        if storage.is_static && !self.symbol_table.at_file_scope() {
            let bad = {
                let tc = self.expr_type_checker();
                let eval = |e: &Expr| self.eval_initializer_value(e);
                let ctype = |e: &Expr| tc.infer_expr_ctype(e);
                let select = |controlling: &Expr, associations: &[GenericAssociation]| {
                    tc.generic_selection_index(controlling, associations)
                };
                super::static_init::StaticInitChecker::new(
                    &self.symbol_table,
                    &eval,
                    &ctype,
                    &select,
                )
                .non_constant_leaves_for(init, &ct)
            };
            for bad_span in bad {
                self.diagnostics
                    .borrow_mut()
                    .error("initializer element is not constant".to_string(), bad_span);
            }
        }
        if storage.is_constexpr {
            self.check_constexpr_compound_literal(cl_expr, ts, init, storage.is_static);
        }
    }

    fn check_address_of_register_compound_literal(&self, inner: &Expr, span: Span) {
        let mut cur = inner;
        while let Expr::MemberAccess(base, _, _) = cur {
            cur = base;
        }
        if let Expr::CompoundLiteral(_, _, _, st) = cur {
            if st.is_register {
                self.diagnostics.borrow_mut().error(
                    "address of register compound literal requested".to_string(),
                    span,
                );
            }
        }
    }

    /// Exact source spelling of `expr`, used for GCC-compatible
    /// `read-only location '<expr>'` diagnostics. Falls back to a neutral
    /// placeholder when no source manager is attached (synthetic spans in
    /// unit tests) or the span does not resolve to a single-line slice.
    fn expr_source_text(&self, expr: &Expr) -> String {
        // `Expr::span()` stores the *operator* token for some composites
        // (the parser records `*` for `Deref` and `[` for subscripting),
        // so cover the operands too: `*q` is `*`+inner, `a[i]` is base+index.
        let mut span = expr.span();
        fn cover(expr: &Expr, span: &mut Span) {
            let child = expr.span();
            if child.file_id == span.file_id {
                span.start = span.start.min(child.start);
                span.end = span.end.max(child.end);
            }
            match expr {
                Expr::Deref(inner, _) => cover(inner, span),
                Expr::ArraySubscript(base, index, _) => {
                    cover(base, span);
                    cover(index, span);
                }
                _ => {}
            }
        }
        cover(expr, &mut span);
        let Some(sm) = self.diagnostics.borrow().share_source_manager() else {
            return "<expr>".to_string();
        };
        let content = sm.get_content(span.file_id);
        let (start, mut end) = (span.start as usize, span.end as usize);
        // Subscript spans start at `[` and end at the index expression;
        // include the closing `]` (skipping whitespace) when present.
        if matches!(expr, Expr::ArraySubscript(..)) {
            let mut probe = end;
            while probe < content.len() && content.as_bytes()[probe].is_ascii_whitespace() {
                probe += 1;
            }
            if content.as_bytes().get(probe) == Some(&b']') {
                end = probe + 1;
            }
        }
        if start <= end
            && end <= content.len()
            && content.is_char_boundary(start)
            && content.is_char_boundary(end)
        {
            let text = &content[start..end];
            if !text.is_empty() && !text.contains('\n') {
                return text.to_string();
            }
        }
        "<expr>".to_string()
    }

    fn member_constness(&self, base: &Expr, field: &str, indirect: bool) -> (bool, bool) {
        let Some(CType::Struct(key) | CType::Union(key)) = self.member_base_ctype(base, indirect)
        else {
            return (false, false);
        };
        let layouts = self.result.type_context.borrow_struct_layouts();
        layouts
            .get(key.as_ref())
            .and_then(|layout| layout.field_constness(field, &*layouts))
            .map_or((false, false), |(base, object, _)| (base, object))
    }

    /// Per-level pointer qualifiers of the member `field` selected from `base`
    /// (`base.field` when `indirect` is false, `base->field` otherwise).
    fn member_pointer_quals(&self, base: &Expr, field: &str, indirect: bool) -> Option<Vec<u8>> {
        let Some(CType::Struct(key) | CType::Union(key)) = self.member_base_ctype(base, indirect)
        else {
            return None;
        };
        self.result
            .type_context
            .borrow_struct_layouts()
            .get(key.as_ref())
            .and_then(|layout| {
                layout
                    .field_pointer_quals(field, &*self.result.type_context.borrow_struct_layouts())
            })
    }

    fn readonly_base_qualifier(&self, expr: &Expr) -> bool {
        match expr {
            Expr::MemberAccess(base, field, _) => self.member_constness(base, field, false).0,
            Expr::PointerMemberAccess(base, field, _) => self.member_constness(base, field, true).0,
            _ => crate::common::qualifiers::expr_const_base(expr, &self.expr_type_checker()),
        }
    }

    /// Whether the POINTEE the pointer expression `ptr` designates is
    /// const-qualified (`const int *q` -> `*q` and `q->f` are read-only;
    /// `int *const p` -> the pointer is read-only but `*p` is writable).
    /// This is exactly `common::qualifiers`' base-constness model, the one
    /// `_Generic` selection already uses.
    fn pointee_readonly(&self, ptr: &Expr) -> bool {
        // Walk the dereference chain down to its root so the verdict can be
        // exact. `CType::Pointer` carries no qualifiers, so the per-level
        // `const` bits come from the declaration that produced the root's
        // type: symbols, members, cast type names, compound literals and
        // function return types each record them (index `L-1` is pointer level
        // `L`). With the root's pointer depth known, the object `*^k root`
        // designates is level `depth - k`; level 0 is the base object, whose
        // qualifier is the separate base-const record. An array-typed root
        // (`const int *a[2]; *a[0] = 1`) supplies the ELEMENT's depth, since
        // its subscript already consumed a dereference. When nothing recorded
        // the levels the check stays silent rather than guessing: a missing
        // diagnostic never breaks a legal program, a wrong one does.
        let tc = self.expr_type_checker();
        let mut derefs = 0usize;
        let mut root = ptr;
        loop {
            match root {
                Expr::Deref(inner, _) => {
                    derefs += 1;
                    root = inner;
                }
                Expr::ArraySubscript(base, index, _) => {
                    derefs += 1;
                    root = if tc.infer_expr_ctype(base).is_some_and(|ty| ty.is_integer()) {
                        index
                    } else {
                        base
                    };
                }
                // `*(p + i)` is the same lvalue `p[i]` names, but `p + i`
                // itself still designates a POINTER: the dereference belongs to
                // the enclosing `*`, so the chain only unwraps the offset.
                Expr::BinaryOp(op, lhs, rhs, _) if matches!(op, BinOp::Add | BinOp::Sub) => {
                    root = if tc.infer_expr_ctype(lhs).is_some_and(|ty| ty.is_integer()) {
                        rhs
                    } else {
                        lhs
                    };
                }
                _ => break,
            }
        }
        let mut k = derefs; // this call asks about `*^k root`'s successor
        let mut depth = pointer_depth_of(tc.infer_expr_ctype(root).as_ref());
        if depth == 0 {
            // An array-typed root (`const int *a[2]; *a[0] = 1`): the chain's
            // subscript already consumed the array layer, so the ELEMENT type
            // supplies the pointer depth and one dereference is accounted for.
            if let Some(CType::Array(element, _)) = tc.infer_expr_ctype(root) {
                if k >= 1 {
                    depth = pointer_depth_of(Some(&element));
                    k -= 1;
                }
            }
        }
        // A symbol that recorded its per-level qualifiers decides exactly:
        // the k-th dereference of `root` designates pointer level `k + 1`, and
        // only the level beyond every star (`k + 1 == depth`) designates the
        // base object. Levels in between are invisible to `CType`
        // (`int *const *p` has `const` on level 1, so `*p = 0` is a
        // modification of a read-only object).
        if depth > 0 {
            // Where the per-level qualifiers of the root's type are recorded:
            // objects in the symbol table, members in their layout.
            let levels: Option<Vec<u8>> = match root {
                Expr::Identifier(name, _) => {
                    self.symbol_table.pointer_quals(name).map(<[u8]>::to_vec)
                }
                Expr::MemberAccess(base, field, _) => self.member_pointer_quals(base, field, false),
                Expr::PointerMemberAccess(base, field, _) => {
                    self.member_pointer_quals(base, field, true)
                }
                // A cast type name keeps its own per-level `const`
                // (`*(int *const *)p = 0`); `CType` cannot carry it.
                Expr::Cast(_, _, _, quals) if quals.pointer_const_levels != 0 => {
                    Some(const_levels_to_cvr(quals.pointer_const_levels))
                }
                // A compound literal of such a type is an lvalue and records
                // its levels the same way (`*((int *const *){0}) = 0`).
                Expr::CompoundLiteral(_, _, _, storage) if storage.pointer_const_levels != 0 => {
                    Some(const_levels_to_cvr(storage.pointer_const_levels))
                }
                // A function's return type keeps its own level qualifiers
                // (`int *const *g(void)`), so `*g() = 0` is a modification of a
                // read-only object.
                Expr::FunctionCall(callee, _, _) => match &**callee {
                    Expr::Identifier(name, _) => self
                        .visible_function_info(name)
                        .map(|info| info.return_pointer_const_levels)
                        .filter(|mask| *mask != 0)
                        .map(const_levels_to_cvr),
                    _ => None,
                },
                _ => None,
            };
            if let Some(levels) = levels {
                // `root` has a `depth`-level pointer type, so `*root` names the
                // object of level `depth - 1` and every further dereference
                // drops one level; level 0 is the base object.
                let Some(target) = (depth as usize).checked_sub(k + 1) else {
                    return false;
                };
                if target == 0 {
                    return self.readonly_base_qualifier(root);
                }
                if target < depth as usize {
                    return levels
                        .get(target - 1)
                        .is_some_and(|&cvr| crate::frontend::parser::ast::cvr_is_const(cvr));
                }
                return false;
            }
        }
        match (k, depth) {
            // `*root` of a one-star root: the base specifier IS the pointee.
            (0, 1) | (0, 0) => {
                if depth == 0 {
                    // `*a` of an array: the element, i.e. the array's own
                    // top-level const (`const int a[]`, `int *const ap[]`).
                    self.lvalue_readonly(root)
                } else {
                    self.readonly_base_qualifier(root)
                }
            }
            // The chain consumes every star: the const is the base one.
            (k, depth) if depth > 0 && k + 1 == depth as usize => {
                self.readonly_base_qualifier(root)
            }
            // Middle-level const is unrecorded: do not guess (see above).
            _ => false,
        }
    }

    /// Whether assigning to (or `++`/`--` on) the lvalue `target` violates
    /// C11 6.5.16p1: the left operand must not be const-qualified.
    ///
    /// The recursion mirrors GCC 16.2's diagnostic classes: a plain
    /// identifier consults the object's TOP-LEVEL const (`const int x` and
    /// `int *const p` are read-only; `const int *q` is a writable pointer);
    /// `*e`/`e[i]` through a pointer consult the pointee (`q[i]` through
    /// `const int *q`); an array's elements follow the array's own
    /// top-level const (`const int a[]` and `int *const ap[]` have
    /// read-only elements); a dot access recurses into the object; an
    /// arrow access consults the pointee.
    fn lvalue_readonly(&self, target: &Expr) -> bool {
        let tc = self.expr_type_checker();
        match target {
            Expr::Identifier(name, _) => self.symbol_table.is_top_const(name),
            Expr::Deref(inner, _) => self.pointee_readonly(inner),
            Expr::ArraySubscript(base, index, _) => {
                let base = if tc.infer_expr_ctype(base).is_some_and(|ty| ty.is_integer()) {
                    index
                } else {
                    base
                };
                if matches!(tc.infer_expr_ctype(base), Some(CType::Array(..))) {
                    self.lvalue_readonly(base)
                } else {
                    self.pointee_readonly(base)
                }
            }
            Expr::MemberAccess(base, field, _) => {
                self.lvalue_readonly(base) || self.member_constness(base, field, false).1
            }
            Expr::PointerMemberAccess(base, field, _) => {
                self.pointee_readonly(base) || self.member_constness(base, field, true).1
            }
            _ => false,
        }
    }

    /// Emit GCC 16.2's read-only diagnostic for a mutation whose operand
    /// fails [`Self::lvalue_readonly`]. `verb` is the measured GCC word:
    /// `assignment` for `=`/`op=`, `increment` for pre/postfix `++`, and
    /// `decrement` for `--`, each pairing with one of the three operand
    /// classes (variable / location / member in read-only object).
    fn check_readonly_mutation(&self, target: &Expr, op_span: Span, verb: &str) {
        if !self.lvalue_readonly(target) {
            return;
        }
        let message = match target {
            Expr::Identifier(name, _) => format!("{verb} of read-only variable '{name}'"),
            Expr::MemberAccess(base, field, _) => {
                if self.lvalue_readonly(base) {
                    format!("{verb} of member '{field}' in read-only object")
                } else {
                    format!("{verb} of read-only member '{field}'")
                }
            }
            Expr::PointerMemberAccess(base, field, _) => {
                if self.pointee_readonly(base) {
                    format!("{verb} of member '{field}' in read-only object")
                } else {
                    format!("{verb} of read-only member '{field}'")
                }
            }
            _ => format!(
                "{verb} of read-only location '{}'",
                self.expr_source_text(target)
            ),
        };
        self.diagnostics.borrow_mut().error(message, op_span);
    }

    fn check_const_compound_literal_mutation(&self, target: &Expr, span: Span) {
        let mut cur = target;
        while let Expr::MemberAccess(base, _, _) | Expr::ArraySubscript(base, _, _) = cur {
            if let Expr::ArraySubscript(_, _, _) = cur {
                if !matches!(
                    self.expr_type_checker().infer_expr_ctype(base),
                    Some(CType::Array(..))
                ) {
                    return;
                }
            }
            cur = base;
        }
        if let Expr::CompoundLiteral(_, _, _, st) = cur {
            if st.is_const || st.is_constexpr {
                self.diagnostics
                    .borrow_mut()
                    .error("assignment of read-only compound literal".to_string(), span);
            }
        }
    }

    fn analyze_bit_cast_expr(&mut self, dst_ts: &TypeSpecifier, src_expr: &Expr, span: Span) {
        self.check_void_type_name(dst_ts, span);
        self.analyze_expr(src_expr);
        let dst_ct = self.type_spec_to_ctype(dst_ts);
        if matches!(dst_ct, CType::Void | CType::Function(_) | CType::Array(..)) {
            self.diagnostics.borrow_mut().error(
                format!(
                    "'__builtin_bit_cast' destination type '{}' is not trivially copyable or is an array",
                    dst_ct
                ),
                span,
            );
            return;
        }
        if let CType::Struct(k) | CType::Union(k) = &dst_ct {
            if !self.defined_structs.borrow().contains(&**k) {
                self.diagnostics.borrow_mut().error(
                    format!(
                        "'__builtin_bit_cast' destination type '{}' is incomplete",
                        dst_ct
                    ),
                    span,
                );
                return;
            }
        }
        let Some(src_raw) = self.expr_type_checker().infer_expr_ctype(src_expr) else {
            return;
        };
        if matches!(src_raw, CType::Void | CType::Function(_)) {
            self.diagnostics.borrow_mut().error(
                "'__builtin_bit_cast' source must be a complete object".to_string(),
                src_expr.span(),
            );
            return;
        }
        let layouts = self.result.type_context.borrow_struct_layouts();
        let dst_sz = dst_ct.size_ctx(&*layouts);
        let src_sz = src_raw.size_ctx(&*layouts);
        if dst_sz == 0 || src_sz == 0 || dst_sz != src_sz {
            self.diagnostics.borrow_mut().error(
                format!(
                    "'__builtin_bit_cast' source size ({src_sz}) does not match destination size ({dst_sz})"
                ),
                span,
            );
        }
    }

    fn check_counted_by_members(&self, fields: &[StructFieldDecl], is_union: bool, span: Span) {
        for f in fields {
            let Some(ref count_member) = f.counted_by else {
                continue;
            };
            if is_union {
                self.diagnostics.borrow_mut().error(
                    "'counted_by' attribute is not allowed for a union member".to_string(),
                    span,
                );
                continue;
            }
            let field_ct = if f.derived.is_empty() {
                self.type_spec_to_ctype(&f.type_spec)
            } else {
                type_builder::build_full_ctype(self, &f.type_spec, &f.derived)
            };
            let valid_target = matches!(field_ct, CType::Array(_, None))
                || matches!(
                    field_ct,
                    CType::Pointer(ref elem, _)
                        if !matches!(elem.as_ref(), CType::Void | CType::Function(_))
                );
            if !valid_target {
                self.diagnostics.borrow_mut().error(
                    "'counted_by' attribute is not allowed for a non-flexible array member field"
                        .to_string(),
                    span,
                );
                continue;
            }
            match self.find_sibling_field_ctype(fields, count_member) {
                None => {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "'counted_by' argument '{count_member}' is not a field of the same structure"
                        ),
                        span,
                    );
                }
                Some(ct) if !ct.is_integer() || matches!(ct, CType::Bool) => {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "'counted_by' argument '{count_member}' must have non-boolean integer type"
                        ),
                        span,
                    );
                }
                Some(_) => {}
            }
        }
    }

    /// Resolve the alignment override carried by a typedef, if any.
    /// For a `TypeSpecifier::TypedefName("foo")`, looks up `foo` in
    /// `typedef_alignments`.  Returns `None` when the type specifier is not
    /// a typedef name or the typedef has no alignment attribute.
    fn resolve_typedef_alignment(&self, ts: &TypeSpecifier) -> Option<usize> {
        if let TypeSpecifier::TypedefName(name) = ts {
            self.result
                .type_context
                .typedef_alignments
                .get(name)
                .copied()
        } else {
            None
        }
    }

    // === Constant expression evaluation ===

    /// The expression type checker over this analyzer's state.
    /// Base constness of a declaration's type (see `common::qualifiers`).
    /// CVR bits of each pointer level of a declarator, in CType creation
    /// order: index `L-1` is pointer level `L`, counting the level closest to
    /// the base type as 1 (`int *const *p` → level 1 is `const`).
    ///
    /// `CType::Pointer` carries no qualifiers, and this is the only place they
    /// survive the parser. Function-pointer declarators are the one shape whose
    /// entries the type builder may reorder around the `FunctionPointer` core;
    /// a function type is never an assignment target, so its levels are not
    /// consulted by the read-only check.
    /// Combine the `const` levels an alias in the return type contributes with
    /// those the declarator spells (`typedef int *const cp; cp *g(void)`).
    fn combine_return_const_levels(&self, type_spec: &TypeSpecifier, declarator: u32) -> u32 {
        let prefix = self.base_typedef_pointer_quals(type_spec);
        let prefix_mask = prefix.iter().enumerate().fold(0u32, |mask, (level, cvr)| {
            if level < 32 && crate::frontend::parser::ast::cvr_is_const(*cvr) {
                mask | (1 << level)
            } else {
                mask
            }
        });
        prefix_mask | shift_const_levels(declarator, prefix.len())
    }

    /// Whether the alias naming `type_spec`'s base declares a read-only object
    /// (`typedef int *const cp;`), so a declaration with no star of its own is
    /// read-only. See [`Self::base_typedef_pointer_quals`] for the leaf walk.
    fn base_typedef_is_top_const(&self, type_spec: &TypeSpecifier) -> bool {
        let mut leaf = type_spec;
        loop {
            match leaf {
                TypeSpecifier::Pointer(inner, _)
                | TypeSpecifier::Array(inner, _)
                | TypeSpecifier::TypeofType(inner)
                | TypeSpecifier::Vector(inner, _)
                | TypeSpecifier::ExtVector(inner, _) => leaf = inner,
                _ => break,
            }
        }
        match leaf {
            TypeSpecifier::TypedefName(name) => self.result.type_context.typedef_is_top_const(name),
            _ => false,
        }
    }

    /// Per-level pointer qualifiers inherited from the typedef named by
    /// `type_spec` (empty for any other base). An alias keeps the qualifiers of
    /// the type it names, so a declaration built on it starts with them.
    fn base_typedef_pointer_quals(&self, type_spec: &TypeSpecifier) -> Vec<u8> {
        // The type name may already be wrapped in pointers/arrays (the parser
        // folds a simple declarator into the specifier), so follow the shape
        // down to the leaf that carries the name.
        let mut leaf = type_spec;
        loop {
            match leaf {
                TypeSpecifier::Pointer(inner, _)
                | TypeSpecifier::Array(inner, _)
                | TypeSpecifier::TypeofType(inner)
                | TypeSpecifier::Vector(inner, _)
                | TypeSpecifier::ExtVector(inner, _) => leaf = inner,
                _ => break,
            }
        }
        match leaf {
            TypeSpecifier::TypedefName(name) => self
                .result
                .type_context
                .typedef_pointer_quals(name)
                .map(<[u8]>::to_vec)
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn decl_const_base(&self, type_spec: &TypeSpecifier, explicit: bool) -> bool {
        crate::common::qualifiers::type_name_const_base(
            type_spec,
            explicit,
            &self.expr_type_checker(),
        )
    }

    /// Whether `expr` is an ARRAY-typed expression, i.e. the shape 6.7.9p13
    /// forbids as an automatic array's initializer. String literals are
    /// expressly allowed (6.7.9p14) even though their type is an array, so they
    /// are excluded before any inference happens; the inference itself is
    /// deliberately last, because its memo is cold at this point in the walk
    /// and it costs a traversal of the initializer.
    fn initializer_is_array_valued(&self, expr: &Expr) -> bool {
        if matches!(
            expr,
            Expr::StringLiteral(..) | Expr::WideStringLiteral(..) | Expr::Char16StringLiteral(..)
        ) {
            return false;
        }
        self.expr_type_checker()
            .infer_expr_ctype(expr)
            .is_some_and(|t| matches!(t, CType::Array(..)))
    }

    fn expr_type_checker(&self) -> super::type_checker::ExprTypeChecker<'_> {
        super::type_checker::ExprTypeChecker {
            symbols: &self.symbol_table,
            types: &self.result.type_context,
            functions: &self.result.functions,
            expr_types: Some(&self.result.expr_types),
            builtin_policy: self.builtin_policy.clone(),
            pre_c99: self.pre_c99,
        }
    }

    /// Whether `expr` is an integer constant expression (C11 6.6p6), the
    /// condition `__builtin_choose_expr` requires (see `ice.rs`).
    fn is_integer_constant_expr(&self, expr: &Expr) -> bool {
        self.expr_type_checker().is_integer_constant_expr(expr)
    }

    /// The constant evaluator, as used by enum values, bitfield widths, array
    /// sizes, `case` labels and static-initializer folding.
    fn sema_const_eval(&self) -> SemaConstEval<'_> {
        SemaConstEval {
            types: &self.result.type_context,
            symbols: &self.symbol_table,
            functions: &self.result.functions,
            const_values: Some(&self.result.const_values),
            expr_types: Some(&self.result.expr_types),
            builtin_policy: self.builtin_policy.clone(),
            folding_initializer: false,
            pre_c99: self.pre_c99,
            named_constants_only: false,
        }
    }

    /// Try to evaluate a constant expression at compile time, keeping the
    /// value in its own type: an integer, a `float`/`double`, or a long
    /// double. Returns None if the expression cannot be evaluated.
    ///
    /// Every consumer that needs an integer -- enum values, bitfield widths,
    /// array sizes, `case` labels -- narrows with [`IrConst::to_i64`]; a
    /// consumer that only asks *whether* an expression is constant must not,
    /// or it would see a floating constant as no constant at all.
    fn eval_const_expr_value(&self, expr: &Expr) -> Option<IrConst> {
        self.sema_const_eval().eval_const_expr(expr)
    }

    /// [`Self::eval_const_expr_value`] for the initializer of an object with
    /// static storage (see [`SemaConstEval::folding_initializer`]).
    fn eval_initializer_value(&self, expr: &Expr) -> Option<IrConst> {
        SemaConstEval {
            folding_initializer: true,
            ..self.sema_const_eval()
        }
        .eval_const_expr(expr)
    }

    /// Evaluate a constant expression at compile time and narrow it to `i64`.
    fn eval_const_expr(&self, expr: &Expr) -> Option<i64> {
        self.eval_const_expr_value(expr)?.to_i64()
    }

    // === Implicit declarations ===

    /// Pre-declare common implicit functions that C programs expect.
    fn declare_implicit_functions(&mut self) {
        // Common libc functions that tests may use without headers
        let implicit_funcs = [
            ("printf", CType::Int, true),
            ("fprintf", CType::Int, true),
            ("sprintf", CType::Int, true),
            ("snprintf", CType::Int, true),
            ("puts", CType::Int, false),
            ("putchar", CType::Int, false),
            ("getchar", CType::Int, false),
            (
                "malloc",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
                false,
            ),
            (
                "calloc",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
                false,
            ),
            (
                "realloc",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
                false,
            ),
            ("free", CType::Void, false),
            ("exit", CType::Void, false),
            ("abort", CType::Void, false),
            (
                "memcpy",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
                false,
            ),
            (
                "memmove",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
                false,
            ),
            (
                "memset",
                CType::Pointer(Box::new(CType::Void), AddressSpace::Default),
                false,
            ),
            ("memcmp", CType::Int, false),
            ("strlen", CType::ULong, false),
            ("strcmp", CType::Int, false),
            (
                "strcpy",
                CType::Pointer(Box::new(CType::Char), AddressSpace::Default),
                false,
            ),
            ("atoi", CType::Int, false),
            ("atol", CType::Long, false),
            ("abs", CType::Int, false),
        ];

        for (name, ret_type, variadic) in &implicit_funcs {
            let is_noreturn = matches!(*name, "exit" | "abort" | "_Exit" | "quick_exit");
            let func_info = FunctionInfo {
                return_type: ret_type.clone(),
                params: Vec::new(),
                variadic: *variadic,
                prototype: None,
                is_defined: false,
                is_noreturn,
                return_pointer_const_levels: 0,
            };
            self.result.functions.insert(name.to_string(), func_info);
        }
    }
}

/// Member-type comparison that sees through anonymous record keys.
///
/// Every conversion of an anonymous `struct { ... }` mints a fresh
/// `__anon_struct_N` key, so two textually identical definitions carry
/// different keys and plain equality says "not corresponding" for a pair C23
/// 6.2.7 explicitly makes compatible -- which turned a valid C23 program into a
/// hard error.  Anonymous members are therefore compared by recursing into
/// their layouts; named records keep key equality, because for them the key IS
/// the identity this whole mechanism exists to establish.
///
/// Recursion terminates: an anonymous record has no tag, so it cannot appear
/// inside its own definition, and depth is bounded by the source nesting.
fn member_types_correspond(x: &CType, y: &CType, layouts: &FxHashMap<String, RcLayout>) -> bool {
    if x == y {
        return true;
    }
    let (kx, ky) = match (x, y) {
        (CType::Struct(a), CType::Struct(b)) => (a, b),
        (CType::Union(a), CType::Union(b)) => (a, b),
        _ => return false,
    };
    let is_anon = |k: &Rc<str>| k.starts_with("__anon_struct_");
    if !is_anon(kx) || !is_anon(ky) {
        return false;
    }
    match (layouts.get(&**kx), layouts.get(&**ky)) {
        (Some(lx), Some(ly)) => record_layouts_correspond(lx, ly, layouts),
        _ => false,
    }
}

/// Whether two record layouts have *corresponding* members: the N3037
/// (C23 6.2.9) relation, reduced to what a computed layout can express.
///
/// Same record kind, same member count, and per member the same name, offset,
/// type, and bit-field width and bit offset. Size and alignment follow from
/// those, but are compared too so that any unexplained difference also counts
/// as non-corresponding -- erring toward two distinct types, which is the safe
/// direction.
fn record_layouts_correspond(
    a: &StructLayout,
    b: &StructLayout,
    layouts: &FxHashMap<String, RcLayout>,
) -> bool {
    a.is_union == b.is_union
        && a.size == b.size
        && a.align == b.align
        && a.fields.len() == b.fields.len()
        && a.fields.iter().zip(&b.fields).all(|(x, y)| {
            x.name == y.name
                && x.offset == y.offset
                && x.bit_offset == y.bit_offset
                && x.bit_width == y.bit_width
                && member_types_correspond(&x.ty, &y.ty, layouts)
        })
}

/// Implement TypeConvertContext so shared type_builder functions can call back
/// into sema for type resolution and constant expression evaluation.
///
/// The 4 divergent methods handle sema-specific behavior:
/// - typedef: looks up in type_context.typedefs
/// - struct/union: converts fields and computes layout
/// - enum: returns CType::Enum with name info (preserves enum identity)
/// - typeof: returns CType::Int (sema doesn't have full expr type resolution yet)

/// `value` as the value of an expression of integer type `ty`. IR constants
/// carry no signedness, so an `unsigned` result may arrive sign-extended
/// (`-1u` folds to `I32(-1)`); 128-bit types are returned unchanged.
pub(super) fn integer_value_in_type(value: i128, ty: &CType) -> i128 {
    let ty = match ty {
        CType::Enum(e) => e.compatible_ctype(),
        other => other.clone(),
    };
    if !ty.is_integer() || matches!(ty, CType::Bool) {
        return value;
    }
    let bits = ty.size() * 8;
    if bits >= 128 {
        return value;
    }
    let low = value & ((1i128 << bits) - 1);
    if ty.is_signed() && low >> (bits - 1) & 1 == 1 {
        low - (1i128 << bits)
    } else {
        low
    }
}

/// Whether `value` is a value of the integer type `ty` (exactly representable:
/// `_Bool` takes only 0 and 1; an enum, its compatible type's values).
pub(super) fn integer_representable(value: i128, ty: &CType) -> bool {
    let ty = match ty {
        CType::Enum(e) => e.compatible_ctype(),
        other => other.clone(),
    };
    if matches!(ty, CType::Bool) {
        return value == 0 || value == 1;
    }
    let bits = ty.size() * 8;
    if bits >= 128 {
        return ty.is_signed() || value >= 0;
    }
    if ty.is_signed() {
        let min = -(1i128 << (bits - 1));
        (min..=-min - 1).contains(&value)
    } else {
        (0..(1i128 << bits)).contains(&value)
    }
}

/// A constant `value` -- the value of an expression of type `source` --
/// converted to the scalar type `ty` as an object of that type holds it
/// (C11 6.3.1.2-6.3.1.5). `None` for the types not folded (aggregates,
/// pointers, complex, decimal), and wherever folding could only be
/// approximate: a missed fold, never a wrong one.
pub(super) fn fold_to_type(
    value: IrConst,
    source: Option<&CType>,
    ty: &CType,
) -> Option<FoldedValue> {
    use crate::common::long_double::{
        f64_to_f128_bytes_lossless, f128_bytes_to_f32, f128_bytes_to_f64, i128_to_f128_bytes,
        long_double_value_bytes,
    };
    // A long double constant as the target holds it: on x87 its bytes may
    // carry a wider intermediate than the 64-bit significand.
    let long_double = |bytes: &[u8; 16]| long_double_value_bytes(bytes);
    let integer = match value {
        IrConst::D32(_) | IrConst::D64(_) => return None,
        IrConst::F32(_) | IrConst::F64(_) | IrConst::LongDouble(..) => None,
        other => {
            let v = other.to_i128()?;
            Some(source.map_or(v, |s| integer_value_in_type(v, s)))
        }
    };
    match ty {
        CType::Float => Some(FoldedValue::Float(f64::from(match (integer, value) {
            (Some(v), _) => v as f32,
            // One rounding, not two through f64.
            (None, IrConst::LongDouble(_, bytes)) => f128_bytes_to_f32(&long_double(&bytes)),
            (None, other) => other.to_f64()? as f32,
        }))),
        CType::Double => Some(FoldedValue::Float(match (integer, value) {
            (Some(v), _) => v as f64,
            (None, IrConst::LongDouble(_, bytes)) => f128_bytes_to_f64(&long_double(&bytes)),
            (None, other) => other.to_f64()?,
        })),
        CType::LongDouble => {
            let bytes = match (integer, value) {
                // Past 113 significant bits binary128 rounds, and x87 would
                // round that again.
                (Some(v), _) => {
                    let m = v.unsigned_abs();
                    if m != 0 && 128 - m.leading_zeros() - m.trailing_zeros() > 113 {
                        return None;
                    }
                    long_double(&i128_to_f128_bytes(v))
                }
                (None, IrConst::LongDouble(_, bytes)) => long_double(&bytes),
                (None, other) => f64_to_f128_bytes_lossless(other.to_f64()?),
            };
            Some(FoldedValue::LongDouble(f128_bytes_to_f64(&bytes), bytes))
        }
        CType::Bool => Some(FoldedValue::Int(i64::from(match (integer, value) {
            (Some(v), _) => v != 0,
            // Any nonzero magnitude, however small (it may underflow f64);
            // a NaN is nonzero too.
            (None, IrConst::LongDouble(_, bytes)) => {
                bytes[..15].iter().any(|&b| b != 0) || bytes[15] & 0x7f != 0
            }
            (None, other) => other.to_f64()? != 0.0,
        }))),
        _ if ty.is_integer() => {
            let v = match (integer, value) {
                (Some(v), _) => v,
                // A floating value truncates toward zero (C11 6.3.1.4p1). A
                // long double folds only when its double image is exact.
                (None, IrConst::LongDouble(_, bytes)) => {
                    let bytes = long_double(&bytes);
                    let d = f128_bytes_to_f64(&bytes);
                    if f64_to_f128_bytes_lossless(d) != bytes {
                        return None;
                    }
                    i128::from(d as i64)
                }
                (None, other) => i128::from(other.to_f64()? as i64),
            };
            if ty.size() > 8 {
                // A 128-bit object is carried in the i64 `FoldedValue`, so it
                // folds only when its exact value fits that carrier. Keeping
                // the low 64 bits of `(u128)1 << 100 | 3` made `c >> 64` read
                // the low half (tests/regression/i128_const_local_half.c). A
                // float initializer is never folded here.
                if integer.is_none() {
                    return None;
                }
                // An unsigned object: a negative source converts to a value
                // above i64::MAX, so `0 <= v` is required.
                let fits = i64::try_from(v).is_ok() && (ty.is_signed() || v >= 0);
                return fits.then_some(FoldedValue::Int(v as i64));
            }
            let (bits, _) = crate::common::const_arith::truncate_and_extend_bits(
                v as i64 as u64,
                ty.size() * 8,
                ty.is_signed(),
            );
            Some(FoldedValue::Int(bits as i64))
        }
        _ => None,
    }
}

impl type_builder::TypeConvertContext for SemanticAnalyzer {
    fn typeof_object_space(&self, expr: &Expr) -> AddressSpace {
        let checker = self.expr_type_checker();
        checker.lvalue_address_space(expr)
    }

    fn resolve_typedef(&self, name: &str) -> CType {
        if let Some(resolved) = self.result.type_context.typedefs.get(name) {
            resolved.clone()
        } else {
            CType::Int
        }
    }

    fn resolve_struct_or_union(
        &self,
        name: &Option<String>,
        fields: &Option<Vec<StructFieldDecl>>,
        is_union: bool,
        is_packed: bool,
        pragma_pack: Option<usize>,
        struct_aligned: Option<usize>,
        reverse_sso: bool,
    ) -> CType {
        let prefix = if is_union { "union" } else { "struct" };
        let struct_fields = fields
            .as_ref()
            .map(|f| self.convert_struct_fields(f))
            .unwrap_or_default();
        let max_field_align = if is_packed { Some(1) } else { pragma_pack };
        let base_key = if let Some(tag) = name {
            format!("{}.{}", prefix, tag)
        } else {
            let id = self.result.type_context.next_anon_struct_id();
            format!("__anon_struct_{}", id)
        };
        // The layout is computed BEFORE the key is settled, because deciding
        // whether this definition shadows a different one needs the members.
        let defined_layout: Option<StructLayout> = if fields.is_none() {
            None
        } else {
            let mut layout = if is_union {
                StructLayout::for_union_with_packing(
                    &struct_fields,
                    max_field_align,
                    &*self.result.type_context.borrow_struct_layouts(),
                    reverse_sso,
                )
            } else {
                StructLayout::for_struct_with_packing(
                    &struct_fields,
                    max_field_align,
                    &*self.result.type_context.borrow_struct_layouts(),
                    reverse_sso,
                )
            };
            layout.apply_struct_aligned(struct_aligned);
            Some(layout)
        };
        // C 6.7.2.3: a record definition in an inner scope declares a NEW type,
        // incompatible with any outer type of the same tag. lccc keyed records
        // by tag alone, so an inner `struct S { int c; }` and an outer
        // `struct S { char c; }` were the SAME CType -- no compatibility check
        // could ever fire, and passing one where the other was expected was
        // silent: `f(s)` returned 44 for `s.c == 300` (tests/bugs/
        // nested_tag_struct_identity.c) where GCC 16.2 rejects the program.
        // A shadowing definition therefore gets its own key. Members that
        // correspond (the N3037 / C23 6.2.9 case) keep the shared key so valid
        // C23 still compiles, and two layout-identical definitions cannot
        // miscompile whichever way they are keyed.
        // `struct S` may already denote a variant in this scope: an inner
        // definition rebinds the tag to its own key through record_alias.
        // Resolve that BEFORE anything else, so the shadow comparison and every
        // non-defining spelling agree on which type the tag currently means.
        // Two things broke without it:
        //   * the shadow test compared against the OUTERMOST layout under
        //     base_key, so a third nesting level was judged against a type that
        //     is no longer the visible one;
        //   * the non-defining arm returned base_key, so inside a shadowing
        //     scope `struct S {int c;} a; struct S b; b = a;` gave the two
        //     spellings of one type different identities and rejected a valid
        //     assignment that GCC accepts.
        // Anonymous records are exempt: their key is already unique per
        // conversion and never aliased.
        let visible: std::borrow::Cow<'_, str> = if name.is_some() {
            self.result.type_context.resolve_record_key(&base_key)
        } else {
            std::borrow::Cow::Borrowed(&base_key)
        };
        let key: String = match (&defined_layout, name) {
            (Some(new_layout), Some(_)) => {
                // Scoped so the Ref is dropped before the alias is mutated.
                let shadows_different = {
                    let layouts = self.result.type_context.borrow_struct_layouts();
                    layouts.get(visible.as_ref()).is_some_and(|prev| {
                        // An empty layout is a forward declaration, which this
                        // definition completes rather than shadows.  Note the
                        // negation: a shadow is a definition that does NOT
                        // correspond, so dropping the `!` inverts the whole
                        // mechanism (verified -- it silently restored the
                        // TAG-ID-1 miscompile and the gate went red).
                        !prev.fields.is_empty()
                            && !record_layouts_correspond(prev, new_layout, &layouts)
                    })
                };
                if shadows_different {
                    let id = self.result.type_context.next_struct_variant_id();
                    let variant = format!("{}#{}", base_key, id);
                    // While this scope lives, the tag denotes the variant, so
                    // every tag->CType site resolves to the inner type.
                    self.result
                        .type_context
                        .set_record_alias_from_ref(&base_key, &variant);
                    variant
                } else {
                    visible.into_owned()
                }
            }
            _ => visible.into_owned(),
        };
        // Track whether this struct/union has been defined (has a body in the
        // AST, e.g. `struct X { ... }` or `struct X {}`), as opposed to just
        // forward-declared (`struct X;`). This distinction is needed for the
        // incomplete type check in analyze_declaration.
        if fields.is_some() {
            let mut defined = self.defined_structs.borrow_mut();
            defined.insert(key.clone());
            if key != base_key {
                defined.insert(base_key.clone());
            }
        }
        if let Some(layout) = defined_layout {
            self.result
                .type_context
                .insert_struct_layout_scoped_from_ref(&key, layout);
        } else if self
            .result
            .type_context
            .borrow_struct_layouts()
            .get(&key)
            .is_none()
        {
            let align = struct_aligned.unwrap_or(1);
            let layout = StructLayout {
                is_complete: fields.is_some(),
                fields: Vec::new(),
                size: 0,
                align,
                is_union,
                is_transparent_union: false,
                reverse_sso: false,
                user_align: struct_aligned.is_some(),
            };
            self.result
                .type_context
                .insert_struct_layout_scoped_from_ref(&key, layout);
        }
        if is_union {
            CType::Union(key.into())
        } else {
            CType::Struct(key.into())
        }
    }

    fn resolve_enum(
        &self,
        name: &Option<String>,
        variants: &Option<Vec<EnumVariant>>,
        is_packed: bool,
    ) -> CType {
        // Reuse the completed definition visible in this lexical scope. It
        // carries both packing and variant-value information for body-less
        // references; looking up only the packed subset loses signedness.
        // Sema preserves enum identity for diagnostics. Variant processing is
        // done separately via process_enum_variants (requires &mut self).
        // We carry variant values so packed_size() can compute the correct size.
        let completed = name
            .as_ref()
            .and_then(|n| self.result.type_context.enum_types.get(n));
        // The fixed type of a definition (`enum E : T { ... }`) is attached
        // by `type_builder`; a body-less reference inherits the visible one.
        CType::Enum(type_builder::enum_type_of(
            name,
            variants,
            is_packed,
            completed,
            None,
            |v| v.value.as_ref().and_then(|e| self.eval_const_expr(e)),
        ))
    }

    fn resolve_typeof_expr(&self, expr: &Expr) -> CType {
        // Use the ExprTypeChecker to infer typeof(expr) from sema state.
        let checker = self.expr_type_checker();
        checker.infer_expr_ctype(expr).unwrap_or(CType::Int)
    }

    fn eval_array_bound_len(&self, expr: &Expr, elem: &CType) -> Option<usize> {
        self.eval_const_expr(expr).and_then(|v| {
            if v < 0 {
                // C standard requires array sizes to be positive (constraint violation).
                // Critical for autoconf AC_CHECK_SIZEOF which uses negative array sizes
                // as compile-time assertions to detect type sizes during cross-compilation.
                self.diagnostics
                    .borrow_mut()
                    .error("size of array is negative", expr.span());
                return None;
            }
            // A count that does not fit the target's `usize` cannot become an
            // element count at all: on `-m32` the conversion would truncate and
            // `char a[4294967296]` became a zero-element array. (Unreachable on
            // LP64, where `i64::MAX` fits a 64-bit `usize`.)
            if (v as u128) > usize::MAX as u128 {
                self.diagnostics
                    .borrow_mut()
                    .error("size of array is too large", expr.span());
                return None;
            }
            let n = v as usize;
            // The bound must describe an existing object: GCC's maximum object
            // size, the target's PTRDIFF_MAX bytes. Tested in u128 because the
            // product that overflows is exactly the one that must be rejected --
            // `int a[4611686018427387904]` is 2^64 bytes and used to wrap to a
            // zero-byte object that compiled and linked without a word.
            let elem_size = elem.size_ctx(&*self.result.type_context.borrow_struct_layouts());
            if (n as u128) * (elem_size as u128) > crate::common::types::max_object_bytes() as u128
            {
                self.diagnostics
                    .borrow_mut()
                    .error("size of array is too large", expr.span());
                return None;
            }
            Some(n)
        })
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Pointer compatibility for call arguments
// ---------------------------------------------------------------------------
//
// C23 6.5.2.2p7 makes "the argument type is compatible with the parameter type"
// a constraint, so passing a `struct A *` where a `struct B *` is expected is
// an error rather than a warning, and GCC 14 onward rejects it by default.
// lccc checked only arity plus pointer/float mixing at this site, which is how
// TAG-ID-1 shipped as a miscompile: `check_record_argument_compatibility`
// closed the by-value path, but a pointer to one of the two distinct same-tag
// records sailed straight through its `_ => return` arm.
//
// Every rule below was pinned against GCC 16.2 rather than read off the
// standard, because the standard's "compatible" and GCC's default diagnostics
// are not the same thing:
//
//   int *   -> void *                 accept  (6.3.2.3p2)
//   void *  -> int *                  accept  (6.3.2.3p2)
//   void ** -> int **                 REJECT  <- the exemption is top-level only
//   int **  -> void **                REJECT
//   int (*)[3] vs int (*)[4]          REJECT  (array size is part of the type)
//   int (*)(void) vs long (*)(void)   REJECT
//   int (*)(int)  vs int (*)(long)    REJECT
//   char *  -> signed char *          accept  (-Wpointer-sign is a warning)
//   char *  -> unsigned char *        accept  (-Wpointer-sign is a warning)
//   int *   -> unsigned *             accept  (-Wpointer-sign is a warning)
//   _Bool * -> int *                  REJECT  (not a sign pair)
//   long *  -> long long *            REJECT  (different types)
//   float * -> double *               REJECT
//   struct A ** vs struct B **        REJECT  (recurses to the pointee)

/// Verdict of the pointer half of the call-argument compatibility constraint.
///
/// `Unknown` is a first-class answer, not a fallback: it means the shapes
/// involved are not modelled precisely enough to decide. This check is being
/// introduced where nothing ran before, so firing on a shape it does not
/// understand would reject valid code -- a strictly worse failure than the one
/// it closes. Every arm that cannot decide says so instead of guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtrCompat {
    Compatible,
    Incompatible,
    Unknown,
}

/// Signed and unsigned variants of one rank. GCC diagnoses these with
/// `-Wpointer-sign`, which is a warning rather than a constraint violation, so
/// they must stay acceptable here or every `char *`/`unsigned char *` pair in
/// existing code would start failing the build. `_Bool` is deliberately absent:
/// GCC rejects `_Bool *` against `int *`.
///
/// The three character types are pairwise distinct (C11 6.2.5p15) and GCC
/// classes every mismatched pair as `-Wpointer-sign`, including `char` against
/// `signed char` where both are signed.
fn same_scalar_modulo_sign(a: &CType, b: &CType) -> bool {
    (a.is_char_type() && b.is_char_type() && a != b)
        || matches!(
            (a, b),
            (CType::Short, CType::UShort)
                | (CType::UShort, CType::Short)
                | (CType::Int, CType::UInt)
                | (CType::UInt, CType::Int)
                | (CType::Long, CType::ULong)
                | (CType::ULong, CType::Long)
                | (CType::LongLong, CType::ULongLong)
                | (CType::ULongLong, CType::LongLong)
                | (CType::Int128, CType::UInt128)
                | (CType::UInt128, CType::Int128)
        )
}

/// Function-type compatibility, C23 6.7.6.3p15: the return type and the
/// parameter types decide it, and *parameter names are not part of the type*.
/// `FunctionType` derives `PartialEq` over `params: Vec<(CType, Option<String>)>`,
/// so a bare `==` compares the names too -- which is wrong, and observably so:
/// `int dbl(int x)` against a parameter declared `int (*)(int)` has one named
/// parameter and one unnamed, and every callback in C is written that way.
fn function_types_compatible(a: &FunctionType, b: &FunctionType) -> bool {
    crate::common::types::ctypes_compatible(
        &CType::Function(Box::new(a.clone())),
        &CType::Function(Box::new(b.clone())),
    )
}

/// Compatibility of two pointed-to types (C23 6.7.6.1p2). Recurses through
/// pointer and array chains, which is what makes `struct S **` an error too
/// rather than only the single-level case.
/// An enumeration and the integer type it is compatible with (C11 6.7.2.2p4),
/// in either direction. Pointer-to-enum and pointer-to-that-integer are
/// pointers to compatible types, which is what makes the kernel's
/// `lookup_address(addr, &level)` — `unsigned int *` parameter, `enum pg_level`
/// argument — a valid call for GCC under -Werror=incompatible-pointer-types.
fn enum_matches_integer(a: &CType, b: &CType) -> bool {
    match (a, b) {
        (CType::Enum(e), other) | (other, CType::Enum(e)) => *other == e.compatible_ctype(),
        _ => false,
    }
}

fn pointee_compat(from: &CType, to: &CType) -> PtrCompat {
    if from == to {
        return PtrCompat::Compatible;
    }
    if same_scalar_modulo_sign(from, to) {
        return PtrCompat::Compatible;
    }
    if enum_matches_integer(from, to) {
        return PtrCompat::Compatible;
    }
    match (from, to) {
        (CType::Pointer(a, a_space), CType::Pointer(b, b_space)) => {
            // The `void *` exemption converts a `void *` EXPRESSION (6.3.2.3p2);
            // it does not make `void **` and `int **` compatible, and GCC
            // rejects both directions of that.
            match (**a == CType::Void, **b == CType::Void) {
                (true, true) => PtrCompat::Compatible,
                (true, false) | (false, true) => PtrCompat::Incompatible,
                (false, false) => {
                    if a_space != b_space {
                        PtrCompat::Incompatible
                    } else {
                        pointee_compat(a, b)
                    }
                }
            }
        }
        (CType::Array(a, a_len), CType::Array(b, b_len)) => {
            // An unsized array is compatible with any size; two sizes that
            // differ are different types.
            match (a_len, b_len) {
                (Some(x), Some(y)) if x != y => PtrCompat::Incompatible,
                _ => pointee_compat(a, b),
            }
        }
        (CType::Function(a), CType::Function(b)) => {
            if function_types_compatible(a, b) {
                PtrCompat::Compatible
            } else {
                PtrCompat::Incompatible
            }
        }
        (CType::Struct(a), CType::Struct(b)) | (CType::Union(a), CType::Union(b)) => {
            // ANON-KEY-1, the same exemption `check_record_argument_compatibility`
            // carries and for the same reason: lccc does not canonicalise
            // anonymous record keys across construction paths, so an intrinsic's
            // parameter and the caller's spelling of one typedef can hold
            // different `__anon_struct_N` keys for one type. Measured, not
            // assumed -- dropping it fails 19 corpus tests, all of them SSE/AVX
            // (`_mm_loadu_si128`, `_mm_storeu_si128`, `_mm_cmppd`). `Unknown`
            // rather than `Compatible` because the honest verdict is that the
            // key model cannot decide, and that silence must not spread to
            // named records, where the key IS the identity.
            if a.starts_with("__anon_") || b.starts_with("__anon_") {
                PtrCompat::Unknown
            } else {
                PtrCompat::Incompatible
            }
        }
        // Enums, vectors and mismatched categories: the key IS the identity and
        // `from == to` above already compared it, so anything reaching here is a
        // different type.
        _ => PtrCompat::Incompatible,
    }
}

/// The pointer half of C23 6.5.2.2p7 for a single call argument.
fn pointer_argument_compat(from: &CType, to: &CType) -> PtrCompat {
    match (from, to) {
        (CType::Pointer(from_inner, from_space), CType::Pointer(to_inner, to_space)) => {
            // The address space is checked first and for every pointee: GCC 14.2
            // rejects `void *` <-> `__seg_gs` in both directions too.
            if from_space != to_space {
                return PtrCompat::Incompatible;
            }
            if **from_inner == CType::Void || **to_inner == CType::Void {
                return PtrCompat::Compatible;
            }
            pointee_compat(from_inner, to_inner)
        }
        // An array argument decays to a pointer to its first element.
        (CType::Array(from_inner, _), CType::Pointer(to_inner, _)) => {
            if **to_inner == CType::Void {
                PtrCompat::Compatible
            } else {
                pointee_compat(from_inner, to_inner)
            }
        }
        (CType::Pointer(from_inner, _), CType::Array(to_inner, _)) => {
            if **from_inner == CType::Void {
                PtrCompat::Compatible
            } else {
                pointee_compat(from_inner, to_inner)
            }
        }
        // A function designator decays to a pointer to the function. Only that
        // decay is decided here; a function value against a non-function
        // pointer is left to the other checks rather than guessed at.
        (CType::Function(from_sig), CType::Pointer(to_inner, _)) => match &**to_inner {
            CType::Function(to_sig) => {
                if function_types_compatible(from_sig, to_sig) {
                    PtrCompat::Compatible
                } else {
                    PtrCompat::Incompatible
                }
            }
            CType::Void => PtrCompat::Compatible,
            _ => PtrCompat::Unknown,
        },
        // Not pointer-shaped. A null pointer constant arrives here as an
        // integer type and has to keep working (`f(0)`, `f(NULL)`), and scalar
        // mixing is `check_pointer_float_conversion`'s job.
        _ => PtrCompat::Unknown,
    }
}

/// One member of a `transparent_union` against an argument type (after
/// array/function decay), as GCC's `convert_for_assignment` member loop
/// decides it -- each rule measured on GCC 16.2, c17 and c23 alike:
///
/// * the same type, or an enum against its compatible integer type (`enum
///   E` for an `unsigned` member; `_Bool` for `unsigned`, `double` for
///   `float`, `long` for `int` and two distinct enums are all rejected:
///   arithmetic types are never converted here);
/// * a pointer member: a pointer argument with a compatible target or a
///   `void` target on either side -- but NOT a target differing only in
///   signedness (`char *` for `unsigned char *`, which an ordinary argument
///   only warns about) -- or a null pointer constant. Dropped qualifiers
///   are a warning there and CType carries none, so they cannot reject.
///
/// `PtrCompat::Unknown` (an unresolved pointee) accepts: a false rejection
/// breaks a valid program, a miss only loses a diagnostic.
fn transparent_member_accepts(member: &CType, arg: &CType, null_constant: bool) -> bool {
    if member == arg {
        return true;
    }
    match (member, arg) {
        (CType::Enum(e), other) | (other, CType::Enum(e))
            if other.is_integer() && !matches!(other, CType::Enum(_)) =>
        {
            e.compatible_ctype() == *other
        }
        (CType::Pointer(target, _), CType::Pointer(..) | CType::Array(..) | CType::Function(_)) => {
            let sign_only = match arg {
                CType::Pointer(a, _) | CType::Array(a, _) => same_scalar_modulo_sign(a, target),
                _ => false,
            };
            !sign_only && pointer_argument_compat(arg, member) != PtrCompat::Incompatible
        }
        (CType::Pointer(..), _) => null_constant,
        _ => false,
    }
}

/// The first pointer layer, outermost first, whose named address space
/// changes in a conversion from `from` to `to`. Each pointer layer's space is
/// the space of the memory it addresses. Arrays are stepped over on both
/// sides, so an array of `__seg_gs` pointers keeps its inner space. The
/// diagnostic prints this layer's spaces, not the outermost ones.
struct SpaceMismatch {
    /// 1-based pointer level, counting the outermost pointer as level 1.
    depth: usize,
    from_space: AddressSpace,
    to_space: AddressSpace,
}

fn first_space_mismatch(from: &CType, to: &CType) -> Option<SpaceMismatch> {
    fn walk(from: &CType, to: &CType, depth: usize) -> Option<SpaceMismatch> {
        match (from, to) {
            (CType::Pointer(f, fs), CType::Pointer(t, ts)) => {
                if fs != ts {
                    Some(SpaceMismatch {
                        depth,
                        from_space: *fs,
                        to_space: *ts,
                    })
                } else {
                    walk(f, t, depth + 1)
                }
            }
            (CType::Array(f, _), CType::Array(t, _)) => walk(f, t, depth),
            _ => None,
        }
    }
    walk(from, to, 1)
}

#[cfg(test)]
mod record_layouts_correspond_tests {
    use super::{member_types_correspond, record_layouts_correspond};
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::{CType, RcLayout, StructFieldLayout, StructLayout};
    use std::rc::Rc;

    fn layout(
        is_union: bool,
        size: usize,
        align: usize,
        fields: Vec<StructFieldLayout>,
    ) -> StructLayout {
        StructLayout {
            is_complete: true,
            fields,
            size,
            align,
            is_union,
            is_transparent_union: false,
            reverse_sso: false,
            user_align: false,
        }
    }

    fn field(name: &str, ty: CType, offset: usize) -> StructFieldLayout {
        StructFieldLayout {
            name: name.to_string(),
            ty,
            offset,
            bit_offset: None,
            bit_width: None,
            sso: crate::common::types::SsoMode::None,
            sso_storage_ty: None,
            base_volatile: false,
            base_const: false,
            object_volatile: false,
            object_const: false,
            counted_by: None,
            pointer_quals: Vec::new(),
        }
    }

    fn anon(
        key: &str,
        size: usize,
        align: usize,
        fields: Vec<StructFieldLayout>,
    ) -> (String, RcLayout) {
        (key.to_string(), Rc::new(layout(false, size, align, fields)))
    }

    #[test]
    fn complete_empty_records_are_not_forward_placeholders() {
        let layouts: FxHashMap<String, RcLayout> = FxHashMap::default();
        assert!(!StructLayout::empty().is_complete);
        assert!(!StructLayout::empty_union().is_complete);
        assert!(StructLayout::for_struct_with_packing(&[], None, &layouts, false).is_complete);
        assert!(StructLayout::for_union_with_packing(&[], None, &layouts, false).is_complete);
    }

    #[test]
    fn identical_named_records_correspond() {
        let m = FxHashMap::default();
        let a = layout(false, 4, 4, vec![field("i", CType::Int, 0)]);
        let b = layout(false, 4, 4, vec![field("i", CType::Int, 0)]);
        assert!(record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn a_different_member_name_does_not_correspond() {
        let m = FxHashMap::default();
        let a = layout(false, 4, 4, vec![field("i", CType::Int, 0)]);
        let b = layout(false, 4, 4, vec![field("j", CType::Int, 0)]);
        assert!(!record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn a_different_member_type_does_not_correspond() {
        let m = FxHashMap::default();
        let a = layout(false, 1, 1, vec![field("c", CType::Char, 0)]);
        let b = layout(false, 4, 4, vec![field("c", CType::Int, 0)]);
        assert!(!record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn a_struct_and_a_union_do_not_correspond() {
        let m = FxHashMap::default();
        let a = layout(false, 4, 4, vec![field("i", CType::Int, 0)]);
        let b = layout(true, 4, 4, vec![field("i", CType::Int, 0)]);
        assert!(!record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn a_different_bit_field_width_does_not_correspond() {
        let m = FxHashMap::default();
        let mut fa = field("b", CType::Int, 0);
        fa.bit_width = Some(3);
        let mut fb = field("b", CType::Int, 0);
        fb.bit_width = Some(5);
        let a = layout(false, 4, 4, vec![fa]);
        let b = layout(false, 4, 4, vec![fb]);
        assert!(!record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn anonymous_members_correspond_structurally_not_by_key() {
        // C23 6.2.7: two anonymous members are compatible when their members
        // correspond. Each conversion mints a fresh __anon_struct_N, so key
        // equality alone rejected valid C23 here.
        let mut m: FxHashMap<String, RcLayout> = FxHashMap::default();
        let (k1, l1) = anon("__anon_struct_1", 4, 4, vec![field("i", CType::Int, 0)]);
        let (k2, l2) = anon("__anon_struct_2", 4, 4, vec![field("i", CType::Int, 0)]);
        m.insert(k1.clone(), l1);
        m.insert(k2.clone(), l2);
        let a = layout(false, 4, 4, vec![field("", CType::Struct(k1.into()), 0)]);
        let b = layout(false, 4, 4, vec![field("", CType::Struct(k2.into()), 0)]);
        assert!(record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn anonymous_members_with_different_shapes_do_not_correspond() {
        let mut m: FxHashMap<String, RcLayout> = FxHashMap::default();
        let (k1, l1) = anon("__anon_struct_1", 4, 4, vec![field("i", CType::Int, 0)]);
        let (k2, l2) = anon("__anon_struct_2", 1, 1, vec![field("c", CType::Char, 0)]);
        m.insert(k1.clone(), l1);
        m.insert(k2.clone(), l2);
        let a = layout(false, 4, 4, vec![field("", CType::Struct(k1.into()), 0)]);
        let b = layout(false, 1, 1, vec![field("", CType::Struct(k2.into()), 0)]);
        assert!(!record_layouts_correspond(&a, &b, &m));
    }

    #[test]
    fn a_struct_member_and_a_union_member_never_correspond() {
        // Different record kinds are different types however alike they look.
        let m = FxHashMap::default();
        assert!(!member_types_correspond(
            &CType::Struct("struct.A".into()),
            &CType::Union("struct.A".into()),
            &m
        ));
    }

    #[test]
    fn named_records_keep_key_equality() {
        // For a NAMED record the key is the identity this mechanism exists to
        // establish, so two different keys must stay non-corresponding even if
        // their layouts happen to match -- otherwise the miscompile returns.
        let mut m: FxHashMap<String, RcLayout> = FxHashMap::default();
        m.insert(
            "struct.A".to_string(),
            Rc::new(layout(false, 4, 4, vec![field("i", CType::Int, 0)])),
        );
        m.insert(
            "struct.B".to_string(),
            Rc::new(layout(false, 4, 4, vec![field("i", CType::Int, 0)])),
        );
        assert!(!member_types_correspond(
            &CType::Struct("struct.A".into()),
            &CType::Struct("struct.B".into()),
            &m
        ));
    }
}

#[cfg(test)]
mod pointer_argument_compat_tests {
    use super::{
        PtrCompat, function_types_compatible, pointee_compat, pointer_argument_compat,
        transparent_member_accepts,
    };
    use crate::common::types::{AddressSpace, CType, EnumType, FunctionType};
    use std::rc::Rc;

    /// GCC 16.2's transparent_union member rule, row by row
    /// (`tests/regression/check_record_tag_identity.sh` drives the same
    /// shapes end to end).
    #[test]
    fn transparent_member_rule_matches_gcc() {
        // Arithmetic: the same type only.
        assert!(transparent_member_accepts(&CType::Int, &CType::Int, false));
        assert!(!transparent_member_accepts(
            &CType::Float,
            &CType::Double,
            false
        ));
        assert!(!transparent_member_accepts(
            &CType::Int,
            &CType::Long,
            false
        ));
        assert!(!transparent_member_accepts(
            &CType::Int,
            &CType::Short,
            false
        ));
        assert!(!transparent_member_accepts(
            &CType::UInt,
            &CType::Bool,
            false
        ));
        // Pointers: compatible or void targets, a null constant, an array.
        let lp = ptr(CType::Long);
        assert!(transparent_member_accepts(&lp, &lp, false));
        assert!(transparent_member_accepts(&lp, &ptr(CType::Void), false));
        assert!(transparent_member_accepts(
            &ptr(CType::Void),
            &ptr(CType::Int),
            false
        ));
        assert!(transparent_member_accepts(&lp, &CType::Int, true));
        assert!(transparent_member_accepts(
            &lp,
            &CType::Array(Box::new(CType::Long), Some(2)),
            false
        ));
        assert!(!transparent_member_accepts(&lp, &CType::Int, false));
        assert!(!transparent_member_accepts(&lp, &ptr(CType::Int), false));
        // A signedness-only pointee difference is rejected (an ordinary
        // argument would merely warn).
        assert!(!transparent_member_accepts(
            &ptr(CType::UChar),
            &ptr(CType::Char),
            false
        ));
        assert!(!transparent_member_accepts(
            &ptr(CType::UInt),
            &ptr(CType::Int),
            false
        ));
        // A non-pointer member never takes a null constant.
        assert!(!transparent_member_accepts(
            &CType::Float,
            &CType::Int,
            true
        ));
    }

    fn ptr(t: CType) -> CType {
        CType::Pointer(Box::new(t), AddressSpace::Default)
    }
    fn segfs(t: CType) -> CType {
        CType::Pointer(Box::new(t), AddressSpace::SegFs)
    }
    fn arr(t: CType, n: Option<usize>) -> CType {
        CType::Array(Box::new(t), n)
    }
    /// Named parameters, so that every case here also proves the comparison
    /// ignores them: C23 6.7.6.3p15 keeps names out of the type.
    fn sig(ret: CType, params: Vec<CType>, named: bool) -> CType {
        CType::Function(Box::new(FunctionType {
            return_type: ret,
            params: params
                .into_iter()
                .enumerate()
                .map(|(i, t)| (t, if named { Some(format!("p{i}")) } else { None }))
                .collect(),
            variadic: false,
            has_prototype: true,
        }))
    }
    fn record(key: &str) -> CType {
        CType::Struct(Rc::from(key))
    }

    // --- the `void *` exemption is top-level only (6.3.2.3p2) --------------
    #[test]
    fn object_pointer_converts_to_void_pointer() {
        assert_eq!(
            pointer_argument_compat(&ptr(CType::Int), &ptr(CType::Void)),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn void_pointer_converts_to_object_pointer() {
        assert_eq!(
            pointer_argument_compat(&ptr(CType::Void), &ptr(CType::Char)),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn double_void_pointer_is_not_double_int_pointer() {
        // The single most important case in this module: folding the void
        // exemption into the recursion would accept both of these, and GCC
        // rejects both directions.
        assert_eq!(
            pointer_argument_compat(&ptr(ptr(CType::Void)), &ptr(ptr(CType::Int))),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(&ptr(ptr(CType::Int)), &ptr(ptr(CType::Void))),
            PtrCompat::Incompatible
        );
    }

    // --- sign pairs are -Wpointer-sign, a warning, so not an error ---------
    #[test]
    fn signed_unsigned_pairs_of_one_rank_are_compatible() {
        for (a, b) in [
            (CType::Char, CType::UChar),
            // the identity aliases: distinct types, still only -Wpointer-sign
            (CType::Char, CType::SChar),
            (CType::SChar, CType::UChar),
            (CType::PlainUChar, CType::UChar),
            (CType::PlainUChar, CType::Char),
            (CType::Short, CType::UShort),
            (CType::Int, CType::UInt),
            (CType::Long, CType::ULong),
            (CType::LongLong, CType::ULongLong),
            (CType::Int128, CType::UInt128),
        ] {
            assert_eq!(
                pointer_argument_compat(&ptr(a.clone()), &ptr(b.clone())),
                PtrCompat::Compatible,
                "{a:?} -> {b:?}"
            );
            assert_eq!(
                pointer_argument_compat(&ptr(b.clone()), &ptr(a.clone())),
                PtrCompat::Compatible,
                "{b:?} -> {a:?}"
            );
        }
    }

    #[test]
    fn bool_is_not_part_of_the_sign_pairs() {
        assert_eq!(
            pointer_argument_compat(&ptr(CType::Bool), &ptr(CType::Int)),
            PtrCompat::Incompatible
        );
    }

    #[test]
    fn different_ranks_are_not_a_sign_pair() {
        assert_eq!(
            pointer_argument_compat(&ptr(CType::Long), &ptr(CType::LongLong)),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(&ptr(CType::Float), &ptr(CType::Double)),
            PtrCompat::Incompatible
        );
    }

    // --- pointee identity ---------------------------------------------------
    #[test]
    fn unrelated_scalar_pointees_are_incompatible() {
        assert_eq!(
            pointer_argument_compat(&ptr(CType::Int), &ptr(CType::Char)),
            PtrCompat::Incompatible
        );
    }

    #[test]
    fn pointers_to_distinct_same_tag_records_are_incompatible() {
        // The TAG-ID-1 hole: by value was checked, through a pointer was not.
        assert_eq!(
            pointer_argument_compat(&record("struct.S#1"), &record("struct.S")),
            PtrCompat::Unknown, // bare records are the record check's job
        );
        assert_eq!(
            pointer_argument_compat(&ptr(record("struct.S#1")), &ptr(record("struct.S"))),
            PtrCompat::Incompatible
        );
    }

    #[test]
    fn pointers_to_records_of_different_tags_are_incompatible() {
        assert_eq!(
            pointer_argument_compat(&ptr(record("struct.A")), &ptr(record("struct.B"))),
            PtrCompat::Incompatible
        );
    }

    #[test]
    fn struct_and_union_with_the_same_name_are_incompatible() {
        assert_eq!(
            pointer_argument_compat(
                &ptr(record("struct.A")),
                &ptr(CType::Union(Rc::from("union.A")))
            ),
            PtrCompat::Incompatible
        );
    }

    #[test]
    fn nested_record_pointers_recurse_to_the_ultimate_pointee() {
        assert_eq!(
            pointer_argument_compat(&ptr(ptr(record("struct.A"))), &ptr(ptr(record("struct.B")))),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(&ptr(ptr(record("struct.A"))), &ptr(ptr(record("struct.A")))),
            PtrCompat::Compatible
        );
    }

    // --- array pointees -----------------------------------------------------
    #[test]
    fn array_pointee_size_is_part_of_the_type() {
        assert_eq!(
            pointer_argument_compat(
                &ptr(arr(CType::Int, Some(3))),
                &ptr(arr(CType::Int, Some(4)))
            ),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(
                &ptr(arr(CType::Int, Some(3))),
                &ptr(arr(CType::Int, Some(3)))
            ),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn unsized_array_pointee_matches_any_size() {
        assert_eq!(
            pointer_argument_compat(&ptr(arr(CType::Int, None)), &ptr(arr(CType::Int, Some(9)))),
            PtrCompat::Compatible
        );
    }

    // --- decay --------------------------------------------------------------
    #[test]
    fn array_argument_decays_to_pointer_to_element() {
        assert_eq!(
            pointer_argument_compat(
                &arr(record("struct.S#1"), Some(4)),
                &ptr(record("struct.S"))
            ),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(&arr(CType::Int, Some(4)), &ptr(CType::Int)),
            PtrCompat::Compatible
        );
        assert_eq!(
            pointer_argument_compat(&arr(CType::Int, Some(4)), &ptr(CType::Void)),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn function_designator_decays_to_pointer_to_function() {
        assert_eq!(
            pointer_argument_compat(
                &sig(CType::Int, vec![CType::Int], true),
                &ptr(sig(CType::Int, vec![CType::Int], false))
            ),
            PtrCompat::Compatible
        );
        assert_eq!(
            pointer_argument_compat(
                &sig(CType::Long, vec![CType::Int], false),
                &ptr(sig(CType::Int, vec![CType::Int], false))
            ),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(&sig(CType::Int, vec![CType::Int], false), &ptr(CType::Void)),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn parameter_names_do_not_decide_function_compatibility() {
        // Regression guard for the callback idiom: `int dbl(int x)` against a
        // parameter declared `int (*)(int)` differs only in the name.
        let named = sig(CType::Int, vec![CType::Int], true);
        let unnamed = sig(CType::Int, vec![CType::Int], false);
        if let (CType::Function(a), CType::Function(b)) = (&named, &unnamed) {
            assert!(function_types_compatible(a, b));
            assert_ne!(a, b, "the derived PartialEq does compare names");
        } else {
            panic!("expected function types");
        }
    }

    #[test]
    fn function_signature_differences_do_decide_compatibility() {
        let a = sig(CType::Int, vec![CType::Int], false);
        let b = sig(CType::Int, vec![CType::Long], false);
        if let (CType::Function(x), CType::Function(y)) = (&a, &b) {
            assert!(!function_types_compatible(x, y));
        } else {
            panic!("expected function types");
        }
    }

    // --- silence where the check has no business guessing -------------------
    #[test]
    fn null_pointer_constants_stay_silent() {
        // `f(0)` and `f(NULL)` reach here as an integer type; diagnosing them
        // would break the single most common way to write a null pointer.
        assert_eq!(
            pointer_argument_compat(&CType::Int, &ptr(CType::Char)),
            PtrCompat::Unknown
        );
        assert_eq!(
            pointer_argument_compat(&CType::Long, &ptr(CType::Void)),
            PtrCompat::Unknown
        );
    }

    #[test]
    fn scalar_arguments_are_not_this_checks_concern() {
        assert_eq!(
            pointer_argument_compat(&CType::Double, &CType::Int),
            PtrCompat::Unknown
        );
    }

    #[test]
    fn address_spaces_do_not_mix() {
        assert_eq!(
            pointer_argument_compat(&segfs(CType::Int), &ptr(CType::Int)),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointer_argument_compat(&segfs(CType::Int), &segfs(CType::Int)),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn enum_is_compatible_with_its_underlying_integer_pointer() {
        // C11 6.7.2.2p4 + GCC's choice: an all-non-negative enum is compatible
        // with `unsigned int`. The kernel relies on it:
        // arch/x86/mm/pat/set_memory.c passes `enum pg_level *` where
        // `unsigned int *` is expected, and GCC accepts that under
        // -Werror=incompatible-pointer-types.
        let pg_level_ty = EnumType {
            name: Some("pg_level".to_string()),
            variants: vec![
                ("PG_LEVEL_NONE".to_string(), 0),
                ("PG_LEVEL_4K".to_string(), 1),
                ("PG_LEVEL_2M".to_string(), 2),
                ("PG_LEVEL_1G".to_string(), 3),
            ],
            is_packed: false,
            fixed: None,
        };
        assert_eq!(pg_level_ty.compatible_ctype(), CType::UInt);
        let pg_level = CType::Enum(pg_level_ty);
        assert_eq!(
            pointee_compat(&pg_level, &CType::UInt),
            PtrCompat::Compatible
        );
        assert_eq!(
            pointee_compat(&CType::UInt, &pg_level),
            PtrCompat::Compatible
        );
        // ... and the full argument check follows from that.
        assert_eq!(
            pointer_argument_compat(
                &CType::Pointer(Box::new(pg_level.clone()), AddressSpace::Default),
                &CType::Pointer(Box::new(CType::UInt), AddressSpace::Default)
            ),
            PtrCompat::Compatible
        );
        // A negative enumerator makes the compatible type `int`, so the
        // unsigned form must NOT be accepted for that enum.
        let signed_enum = CType::Enum(EnumType {
            name: Some("signed_e".to_string()),
            variants: vec![("A".to_string(), -1), ("B".to_string(), 0)],
            is_packed: false,
            fixed: None,
        });
        assert_eq!(
            pointee_compat(&signed_enum, &CType::UInt),
            PtrCompat::Incompatible
        );
        assert_eq!(
            pointee_compat(&signed_enum, &CType::Int),
            PtrCompat::Compatible
        );
    }

    #[test]
    fn pointee_compat_treats_identical_types_as_compatible() {
        assert_eq!(
            pointee_compat(&CType::Double, &CType::Double),
            PtrCompat::Compatible
        );
    }
    #[test]
    fn anonymous_record_pointees_are_undecidable_not_incompatible() {
        // ANON-KEY-1: `_mm_loadu_si128` and its caller can hold different
        // `__anon_struct_N` keys for one typedef, so this must stay silent.
        assert_eq!(
            pointer_argument_compat(
                &ptr(record("__anon_struct_3")),
                &ptr(record("__anon_struct_9"))
            ),
            PtrCompat::Unknown
        );
        assert_eq!(
            pointer_argument_compat(&ptr(record("__anon_struct_3")), &ptr(record("struct.S"))),
            PtrCompat::Unknown
        );
        // ... and the silence must not spread to named records.
        assert_eq!(
            pointer_argument_compat(&ptr(record("struct.A")), &ptr(record("struct.B"))),
            PtrCompat::Incompatible
        );
    }
}

#[cfg(test)]
mod kr_prototype_tests {
    use super::SemanticAnalyzer;
    use crate::common::types::CType;
    use crate::frontend::lexer::Lexer;
    use crate::frontend::parser::Parser;

    /// Which half retypes `main`'s call in the prototype-only variant? This
    /// pins the TU record itself: a block-scope `auto` prototype must leave the
    /// file-scope record alone even when no nested definition exists.
    #[test]
    fn auto_prototype_without_definition_leaves_the_file_record_alone() {
        let (analyzer, result) = analyze(
            "int f(int x) { return x + 1; }\n\
             int g(void) { auto double f(double); return 0; }\n\
             int main(void) { return f(41) != 42; }",
        );
        result.expect("a nested declaration must not be rejected");
        let info = analyzer
            .result
            .functions
            .get("f")
            .expect("file-scope f is recorded");
        let got = info
            .prototype
            .as_ref()
            .map(|p| format!("{:?}", p.return_type))
            .unwrap_or_else(|| "<none>".to_string());
        eprintln!("LADDER functions[\"f\"].prototype.return_type = {got}");
        assert_eq!(got, "Int", "the outer `f` must keep its own prototype");
    }

    /// The parser half of the F01 law, pinned at the source of the fact: GNU
    /// `auto` on a block-scope function declaration must survive to the
    /// declarator's attributes, because that flag -- not a type comparison --
    /// is what tells sema "this is a nested function of its own, do not merge
    /// it into the file-scope entity". The scanners that consume the token
    /// live in `frontend/parser/types.rs`; every one of them has to record it,
    /// which is exactly what was missing when the guard "looked right" and the
    /// miscompile stayed (measured: `main`'s `f(41)` emitted as a double call).
    #[test]
    fn auto_on_a_block_scope_prototype_reaches_the_declarator_attrs() {
        use crate::frontend::parser::ast::{BlockItem, ExternalDecl};
        let src = "int g(void) { auto double f(double); auto int h(void), k(void); return 0; }";
        let tokens = Lexer::new(src, 0).tokenize();
        let mut parser = Parser::new(tokens);
        let tu = parser.parse();
        assert_eq!(parser.error_count, 0, "fixture must parse");
        let mut seen: Vec<(String, bool)> = Vec::new();
        for d in &tu.decls {
            if let ExternalDecl::FunctionDef(f) = d {
                for item in &f.body.items {
                    if let BlockItem::Declaration(decl) = item {
                        for id in &decl.declarators {
                            seen.push((id.name.clone(), id.attrs.is_nested_fn()));
                        }
                    }
                }
            }
        }
        assert_eq!(
            seen.len(),
            3,
            "the fixture has three block-scope declarators; got {seen:?}"
        );
        assert!(
            seen.iter().all(|(_, nested)| *nested),
            "`auto` must be recorded on every declarator of the statement; got {seen:?}"
        );
    }

    /// P1-3 witness 1 -- the direction that *deletes* code.  A nested entity that
    /// returns must not be read as `noreturn` because a file-scope function of the
    /// same basename is one, and the outer record must keep its own answer.  Both
    /// halves are pinned: the fix is a scoped record, and a fix that merely made
    /// the nested lookup return `false` for every shadowed name would pass one
    /// half and break the other.
    #[test]
    fn nested_returning_definition_never_inherits_the_outer_noreturn() {
        let (analyzer, result) = analyze(
            "extern void die(void) __attribute__((noreturn));\n\
             int g(void) { auto void die(void) { return; } die(); return 0; }\n",
        );
        result.expect("a nested definition may shadow an outer noreturn function");
        let outer = analyzer
            .result
            .functions
            .get("die")
            .expect("file-scope die is recorded");
        let nested = analyzer
            .result
            .functions
            .get("g.die")
            .expect("the nested entity has its own record");
        assert!(outer.is_noreturn, "the outer entity is noreturn");
        assert!(
            !nested.is_noreturn,
            "the nested entity returns; inheriting the flag would delete the code after the call"
        );
    }

    /// P1-3 witness 2 -- the direction the first fix missed.  A prototype-only
    /// nested declaration had no record at all, so a scoped lookup fell back to
    /// the file-scope entity and *lost* a real `noreturn`.  GCC refuses to call
    /// such a function ("nested function declared but never defined"), so the
    /// assertion is on the record the read side consults rather than on a call.
    #[test]
    fn nested_noreturn_prototype_records_its_own_entity() {
        let (analyzer, result) = analyze(
            "int f(void) { return 1; }\n\
             int g(void) { auto void f(void) __attribute__((noreturn)); (void)0; return 0; }\n",
        );
        result.expect("a block-scope nested prototype is accepted");
        let outer = analyzer
            .result
            .functions
            .get("f")
            .expect("file-scope f is recorded");
        let nested = analyzer
            .result
            .functions
            .get("g.f")
            .expect("the nested prototype creates its entity record");
        assert!(nested.is_noreturn, "the nested entity is the noreturn one");
        assert!(!nested.is_defined, "nothing here defines it");
        assert!(!outer.is_noreturn, "the outer f is unaffected either way");
    }

    /// P1-3 witness 3: a nested definition must merge the facts of a preceding
    /// prototype the way the file-scope path does (`|=`).  Overwriting the record
    /// at definition time silently dropped `noreturn`, which is how a
    /// declaration/definition split -- legal C, and what every header produces --
    /// turned a real noreturn call back into a falling-through one.
    #[test]
    fn nested_definition_merges_the_prototype_that_preceded_it() {
        let (analyzer, result) = analyze(
            "int g(void) {\n\
             auto void n(void) __attribute__((noreturn));\n\
             auto void n(void) { for (;;) {} }\n\
             n();\n\
             }\n",
        );
        result.expect("a noreturn nested function is valid C");
        let nested = analyzer
            .result
            .functions
            .get("g.n")
            .expect("the nested entity is recorded");
        assert!(
            nested.is_noreturn,
            "the prototype's noreturn survives the definition"
        );
        assert!(nested.is_defined, "the definition is what defines it");
    }

    fn analyze(source: &str) -> (SemanticAnalyzer, Result<(), usize>) {
        let tokens = Lexer::new(source, 0).tokenize();
        let mut parser = Parser::new(tokens);
        let tu = parser.parse();
        assert_eq!(parser.error_count, 0, "fixture must parse");
        let mut sema = SemanticAnalyzer::new();
        // Mirror the driver's GCC 16.2 dialect-default severity policy
        // (`-Wint-conversion`, `-Wincompatible-pointer-types` and friends
        // are errors from C99 on), so fixture verdicts equal what the
        // compiler does under its default dialect instead of depending on
        // whether a particular constraint already had a kind.
        let mut config = crate::common::error::WarningConfig::new();
        config.apply_gcc162_c99_default_errors();
        sema.diagnostics.borrow_mut().set_warning_config(config);
        let result = sema.analyze(&tu);
        (sema, result)
    }

    #[test]
    fn body_parameter_information_is_not_a_call_prototype() {
        let (sema, result) = analyze(
            "int old(p) short *p; { return *p; }\n\
             int use(void) { short a[2]; return old(&a); }",
        );
        // The mismatched pointer is deliberate: without a prototype this is
        // not a call-site constraint. It is NOT executed as a defined-C test.
        assert!(result.is_ok());
        let info = &sema.result.functions["old"];
        assert_eq!(info.params.len(), 1, "lowering still needs the body types");
        assert!(info.prototype.is_none());
        assert!(info.call_type().params.is_empty());
    }

    #[test]
    fn known_kr_definition_does_not_enforce_prototype_arity() {
        for call in ["old()", "old(1, 2, 3)"] {
            let source =
                format!("int old(x) int x; {{ return x; }} int use(void) {{ return {call}; }}");
            assert!(analyze(&source).1.is_ok());
        }
    }

    #[test]
    fn prior_prototype_survives_kr_definition_with_body_types_intact() {
        let (sema, result) = analyze("double old(double); double old(x) float x; { return x; }");
        assert!(result.is_ok());
        let info = &sema.result.functions["old"];
        assert_eq!(info.params[0].0, CType::Float);
        assert_eq!(info.prototype.as_ref().unwrap().params[0].0, CType::Double);
        assert!(
            analyze(
                "int old(int); int old(x) int x; { return x; } int use(void) { return old(); }"
            )
            .1
            .is_err()
        );
    }

    #[test]
    fn later_prototype_refines_kr_definition_without_erasing_body_types() {
        let (sema, result) = analyze("double old(x) float x; { return x; } double old(double);");
        assert!(result.is_ok());
        let info = &sema.result.functions["old"];
        assert_eq!(info.params[0].0, CType::Float);
        assert_eq!(info.prototype.as_ref().unwrap().params[0].0, CType::Double);
        assert!(
            analyze(
                "int old(x) int x; { return x; } int old(int); int use(void) { return old(); }"
            )
            .1
            .is_err()
        );
    }

    /// The blocker the audit opened this PR for: a `const` on a middle pointer
    /// level is invisible in `CType`, so the read-only check has to read it
    /// from the declarator record. Everything below was measured with
    /// gcc 16.2.0 and clang 23.1.4.
    #[test]
    fn middle_pointer_levels_keep_their_const() {
        // `*p` designates a const pointer object: assigning to it is a
        // constraint violation (C11 6.5.16p1).
        for bad in [
            "void f(int *const *p){*p=0;}",
            "void f(int *const *p){(*p)++;}",
            "void f(int *const *p){p[0]=0;}",
            "void f(int *const *p){*(p+0)=0;}",
            "void f(int **const *p){*p=0;}",
            "void f(int *const **p){**p=0;}",
            "void f(void){int *const *p=0;*p=0;}",
            "struct S{int *const *p;};void f(struct S*s){*s->p=0;}",
            "typedef int *const cp;void f(cp *p){*p=0;}",
            "typedef int *const cp;void f(void){cp x=0;x=0;}",
            "typedef int *const cp;void f(cp p){p=0;}",
            "typedef int *const cp;typedef cp *cpp;void f(cpp p){*p=0;}",
            "void f(void){const int *a[2]={0};*a[0]=1;}",
            "void f(void){int *const a[2]={0};a[0]=0;}",
            "void f(void){int (*const fp)(void)=0;fp=0;}",
            "void f(void*p){*(int *const *)p=0;}",
            "void f(void*p){*(int *const *const *)p=0;}",
            "void f(void*p){**(int *const **)p=0;}",
            "int *const *g(void);void f(void){*g()=0;}",
            "typedef int *const cp;cp *g(void);void f(void){*g()=0;}",
            "void f(void){*((int *const *){0})=0;}",
        ] {
            assert!(analyze(bad).1.is_err(), "{bad}");
        }
        // Legal forms stay legal: only the qualified level is read-only, and a
        // `const` on a pointee does not make the pointer itself read-only.
        for good in [
            "void f(int *const *p){**p=1;}",
            "void f(int *const *p){p=0;}",
            "void f(int **const p){*p=0;}",
            "void f(int *const **p){*p=0;}",
            "void f(int *const p){*p=1;}",
            "void f(const int **p){*p=0;}",
            "void f(int *volatile *p){*p=0;}",
            "typedef int *const cp;void f(cp p){*p=0;}",
            "typedef int *const cp;void f(cp *p){**p=0;}",
            "struct S{int *const *p;};void f(struct S*s){**s->p=1;}",
            "void f(void){int *a[2]={0};*a[0]=1;}",
            "void f(void){int *a[2]={0};a[0]=0;}",
            "void f(void){int (*fp)(void)=0;fp=0;}",
            "void f(void*p){**(int *const *)p=0;}",
            "void f(void*p){*(int **)p=0;}",
            "int **g(void);void f(void){*g()=0;}",
            "int *const *g(void);void f(void){**g()=0;}",
            "void f(void){*((int **){0})=0;}",
        ] {
            assert!(analyze(good).1.is_ok(), "{good}");
        }
    }

    #[test]
    fn real_prototypes_keep_pointer_constraints() {
        assert!(
            analyze("int typed(short *); int use(void) { short a[2]; return typed(&a); }")
                .1
                .is_err()
        );
        assert!(
            analyze(
                "int typed(short *p) { return *p; } int use(void) { short a[2]; return typed(&a); }"
            )
            .1
            .is_err()
        );
    }

    #[test]
    fn unrelated_outer_prototype_does_not_type_a_nested_kr_definition() {
        assert!(analyze("int old(short *); int outer(void) { int old(p) short *p; { return *p; } short a[2]; return old(&a); }").1.is_ok());
        assert!(
            analyze(
                "int old(int, int); int outer(void) { int old(int x) { return x; } return old(1); }"
            )
            .1
            .is_ok()
        );
    }

    #[test]
    fn unspecified_redeclaration_keeps_prior_prototype() {
        assert!(
            analyze("int f(int); int f(); int use(void) { return f(); }")
                .1
                .is_err()
        );
    }

    /// A block-scope function declaration must describe only that block.
    ///
    /// The shape that goes wrong is a GNU nested function sharing a basename
    /// with a file-scope one: `int f(int)` at file scope, then inside `g` the
    /// declaration `auto double f(double);`. That declaration is `g`'s view of a
    /// *different entity*, yet it wrote the translation-unit record for `f` --
    /// `prototype` included, since a declaration carrying one overwrites it
    /// unconditionally -- so the file-scope definition was later lowered with the
    /// nested function's parameter types. The `is_global` flag that distinguishes
    /// the two cases was already threaded into `analyze_declaration` and ignored.
    #[test]
    fn nested_block_scope_declaration_does_not_retype_the_outer_function() {
        let (analyzer, result) = analyze(
            "int f(int x);\n\
             int g(void) { auto double f(double); double f(double x) { return x * 2; } return (int) f(1.5); }",
        );
        result.expect("a nested declaration must not be rejected");
        let info = analyzer
            .result
            .functions
            .get("f")
            .expect("file-scope f is recorded");
        let proto = info
            .prototype
            .as_ref()
            .expect("`int f(int x);` leaves a prototype");
        assert_eq!(
            proto.return_type,
            CType::Int,
            "a block-scope `auto double f(double);` must not replace the outer return type"
        );
        assert_eq!(
            proto.params.len(),
            1,
            "and must not replace the outer parameter list either"
        );
        assert!(
            matches!(proto.params[0].0, CType::Int),
            "outer parameter type came out as {:?}",
            proto.params[0].0
        );
        assert!(
            analyzer.result.functions.contains_key("g.f"),
            "the nested definition registers under its mangled key; got {:?}",
            analyzer.result.functions.keys().collect::<Vec<_>>()
        );
    }

    /// …but a block-scope declaration *without* a prototype does read the
    /// translation-unit record, because C11 6.2.1p4 makes the visible prototype the
    /// type of the declared identifier. Pinned so the fix above is not widened into
    /// "block-scope declarations never consult the table", which would drop
    /// `noreturn` for redeclarations of an external function made inside a body.
    #[test]
    fn block_scope_declaration_without_prototype_absorbs_the_visible_one() {
        let (analyzer, result) =
            analyze("void die(void) __attribute__((noreturn));\nint g(void) { die(); return 0; }");
        result.expect("absorbing a visible prototype is valid C");
        assert!(
            analyzer
                .result
                .functions
                .get("die")
                .expect("die is recorded")
                .is_noreturn,
            "an attribute set at file scope survives calls made from inside a body"
        );
    }

    #[test]
    fn nested_and_indirect_kr_calls_do_not_acquire_body_prototypes() {
        assert!(analyze(
            "int outer(void) { int old(p) short *p; { return *p; } short a[2]; return old(&a); }"
        )
        .1
        .is_ok());
        assert!(analyze(
            "int old(p) short *p; { return *p; } int use(void) { short a[2]; return (&old)(&a); }"
        )
        .1
        .is_ok());
    }
}

/// Whether `init_decl` declares a const-qualified object (C11 6.7.3p5): the
/// base type is const when the declarator has no pointer, and the outermost
/// pointer is const when it has one (`const int *p` is not a const object).
/// Shift a declarator's return-level mask past the pointer levels an alias in
/// the return type contributes (`typedef int *const cp; cp *g(void)` has the
/// alias's level below the declarator's star).
fn shift_const_levels(mask: u32, by: usize) -> u32 {
    if by >= 32 { 0 } else { mask << by }
}

/// `const` bits of a FUNCTION's return-type pointer levels, read from its
/// declarator. The return type's levels are the pointer declarators before the
/// function core, in creation order; `CType` cannot carry their qualifiers, so
/// this is what lets `*g() = 0` for `int *const *g(void)` be rejected.
/// Declarators that wrap the function in a function pointer reorder those
/// layers and report nothing rather than a wrong level.
fn declarator_return_const_levels(derived: &[DerivedDeclarator], return_type: &CType) -> u32 {
    if derived
        .iter()
        .any(|d| matches!(d, DerivedDeclarator::FunctionPointer(..)))
    {
        return 0;
    }
    let depth = pointer_depth_of(Some(return_type));
    let mut mask = 0u32;
    let mut level = 0usize;
    for d in derived {
        match d {
            DerivedDeclarator::Pointer(_, cvr) => {
                if level >= depth {
                    break;
                }
                if level < 32 && crate::frontend::parser::ast::cvr_is_const(*cvr) {
                    mask |= 1 << level;
                }
                level += 1;
            }
            DerivedDeclarator::Array(_) => {}
            _ => break,
        }
    }
    mask
}

/// Expand a bitmask of `const` pointer levels into the per-level CVR vector the
/// read-only check indexes (bit `L-1` set means level `L` is `const`).
fn const_levels_to_cvr(mask: u32) -> Vec<u8> {
    (0..32)
        .map(|level| {
            if mask & (1 << level) != 0 {
                crate::frontend::parser::ast::CVR_CONST
            } else {
                0
            }
        })
        .collect()
}

/// Number of consecutive pointer levels in `ty` (0 for anything that is not a
/// pointer, including arrays and functions).
fn pointer_depth_of(ty: Option<&CType>) -> usize {
    let mut inner = match ty {
        Some(CType::Pointer(inner, _)) => &**inner,
        _ => return 0,
    };
    let mut depth = 1usize;
    while let CType::Pointer(next, _) = inner {
        depth += 1;
        inner = next;
    }
    depth
}

fn declares_const_object(decl: &Declaration, init_decl: &InitDeclarator) -> bool {
    let has_pointer = init_decl
        .derived
        .iter()
        .any(|d| matches!(d, DerivedDeclarator::Pointer(..)));
    // A volatile object is never a folded constant: every read of it is an
    // access that must reach memory (C11 6.7.3p7), so `const volatile` objects
    // are not recorded here. For a pointer declarator only the object's own
    // qualifier counts (`volatile int *p` is a non-volatile pointer).
    let volatile_object = if has_pointer {
        init_decl.attrs.object_volatile
    } else {
        decl.is_volatile() || init_decl.attrs.object_volatile
    };
    if volatile_object {
        return false;
    }
    if has_pointer {
        decl.is_pointer_const()
    } else {
        decl.is_const()
    }
}

/// An open aggregate in a braced initializer: its type and the index of the
/// next subobject a positional initializer takes (see `check_braced_element_spaces`).
struct WalkFrame {
    ty: CType,
    next: usize,
}

/// A member that takes an initializer slot. Unnamed bit-fields do not (C11 6.7.2.1p13).
fn is_initializer_slot(field: &StructFieldLayout) -> bool {
    !(field.name.is_empty() && field.bit_width.is_some())
}

#[cfg(test)]
mod builtin_return_flow_tests {
    use super::SemanticAnalyzer;
    use crate::frontend::lexer::Lexer;
    use crate::frontend::parser::Parser;

    fn warnings(body: &str) -> usize {
        let source = format!("int f(void *block) {{ {body} }}");
        let tokens = Lexer::new(&source, 0).tokenize();
        let mut parser = Parser::new(tokens);
        let tu = parser.parse();
        assert_eq!(parser.error_count, 0, "{body}");
        let mut analyzer = SemanticAnalyzer::new();
        assert!(analyzer.analyze(&tu).is_ok(), "{body}");
        analyzer.diagnostics.borrow().warning_count()
    }

    #[test]
    fn nonlocal_return_builtins_terminate_fallthrough_analysis() {
        // Lowering already terminates these paths. Their semantic classification
        // must agree, including the existing cast/comma expression wrappers.
        for body in [
            "__builtin_return(block);",
            "(void)__builtin_return(block);",
            "((void)0, __builtin_return(block));",
            "if (block) __builtin_return(block); else __builtin_return(block);",
            "__builtin_longjmp(block, 1);",
            "(void)__builtin_longjmp(block, 1);",
        ] {
            assert_eq!(warnings(body), 0, "{body}");
        }
    }

    #[test]
    fn ordinary_builtin_results_and_conditional_paths_still_warn() {
        for body in [
            "__builtin_return_address(0);",
            "__builtin_apply_args();",
            "__builtin_setjmp(block);",
            "if (block) __builtin_return(block);",
            "if (block) __builtin_longjmp(block, 1);",
        ] {
            assert_eq!(warnings(body), 1, "{body}");
        }
    }
}

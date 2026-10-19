use crate::common::source::Span;
use crate::common::types::{AddressSpace, CType, ExtVectorLayout};

/// A complete translation unit (one C source file).
#[derive(Debug)]
pub struct TranslationUnit {
    pub decls: Vec<ExternalDecl>,
}

/// Top-level declarations in a translation unit.
#[derive(Debug)]
pub enum ExternalDecl {
    FunctionDef(FunctionDef),
    Declaration(Declaration),
    /// Top-level asm("...") directive - emitted verbatim in assembly output.
    TopLevelAsm(String),
    /// A `_Static_assert` the parser could not decide; sema evaluates it.
    StaticAssert(StaticAssert),
}

/// A `_Static_assert` whose condition needs type information (`sizeof`,
/// `_Generic`, enumerator types, ...). The parser decides what it can; the
/// rest reach sema, which evaluates them in lexical order.
#[derive(Debug, Clone)]
pub struct StaticAssert {
    pub cond: Expr,
    pub message: Option<String>,
    pub span: Span,
}

/// Attributes that can be applied to function definitions via storage-class
/// specifiers (static, inline, extern) and GCC __attribute__((...)) syntax.
///
/// Boolean attributes are stored as a packed bitfield (`flags`) for memory
/// efficiency — 17 booleans collapse from 17 bytes into 4 bytes. Accessor
/// methods provide the same API as the old struct fields.
///
/// Non-boolean attributes (`section`, `visibility`) remain as `Option<String>`.
#[derive(Clone, Default)]
pub struct FunctionAttributes {
    /// Packed boolean flags — see `FuncAttrFlag` constants.
    flags: u32,
    /// __attribute__((section("..."))) - place in specific ELF section
    pub section: Option<String>,
    /// __attribute__((visibility("hidden"|"default"|...)))
    pub visibility: Option<String>,
    /// __attribute__((symver("name@@VERSION"))) - symbol version alias
    pub symver: Option<String>,
    /// __attribute__((__diagnose_as(__builtin_NAME, ...))) - builtin alias
    /// (mirrored from DeclAttributes for definitions; see expr_calls.rs).
    pub diagnose_as: Option<String>,
    /// __attribute__((regparm(N))) - pass N integer args in EAX/EDX/ECX.
    /// None = attribute absent; Some(N) with N in 0..=3 (GCC semantics:
    /// bare `regparm` means N = 3; the global -mregparm flag applies only
    /// when the attribute is absent).
    pub regparm: Option<u8>,
}

/// Bit masks for boolean flags in `FunctionAttributes::flags`.
///
/// Each attribute occupies one bit, allowing cheap test/set/clear operations.
/// New attributes can be added by defining the next power-of-two constant.
pub mod func_attr_flag {
    pub const STATIC: u32 = 1 << 0;
    pub const INLINE: u32 = 1 << 1;
    /// `extern` storage class on the function definition.
    pub const EXTERN: u32 = 1 << 2;
    /// `__attribute__((gnu_inline))` — forces GNU89 inline semantics.
    pub const GNU_INLINE: u32 = 1 << 3;
    /// `__attribute__((always_inline))` — must always be inlined.
    pub const ALWAYS_INLINE: u32 = 1 << 4;
    /// `__attribute__((noinline))` — must never be inlined.
    pub const NOINLINE: u32 = 1 << 5;
    /// `__attribute__((constructor))` — run before main.
    pub const CONSTRUCTOR: u32 = 1 << 6;
    /// `__attribute__((destructor))` — run after main.
    pub const DESTRUCTOR: u32 = 1 << 7;
    /// `__attribute__((weak))` — emit as a weak symbol.
    pub const WEAK: u32 = 1 << 8;
    /// `__attribute__((used))` — prevent dead code elimination.
    pub const USED: u32 = 1 << 9;
    /// `__attribute__((fastcall))` — i386 fastcall convention (first 2 int args in ecx/edx).
    pub const FASTCALL: u32 = 1 << 10;
    /// `__attribute__((naked))` — emit no prologue/epilogue; function body is pure asm.
    pub const NAKED: u32 = 1 << 11;
    /// `__attribute__((noreturn))` or `_Noreturn` — function never returns.
    pub const NORETURN: u32 = 1 << 12;
    /// `__attribute__((no_instrument_function))` — skip mcount/__fentry__ prologue.
    pub const NO_INSTRUMENT: u32 = 1 << 13;
    /// `__attribute__((pure))` — read-only memory access, no observable side effects.
    pub const PURE: u32 = 1 << 14;
    /// `__attribute__((const))` — no memory access, no observable side effects.
    pub const CONST_ATTR: u32 = 1 << 15;
    /// `__attribute__((cold))` — the function is unlikely to execute; the
    /// lowerer places it in `.text.unlikely` unless `section(...)` says
    /// otherwise, and the loop-alignment pass skips its loops.
    pub const COLD: u32 = 1 << 16;
}

/// The declaration *specifiers* of a function definition: the part of its
/// attribute set that a `__attribute__` list cannot spell.  Grouped so that
/// assembling `FunctionAttributes` is one call whose signature is complete,
/// instead of a list of setters each caller keeps by hand.
#[derive(Clone, Copy, Default, Debug)]
pub struct DefSpecifiers {
    pub static_: bool,
    pub inline_: bool,
    pub extern_: bool,
    pub gnu_inline: bool,
    pub always_inline: bool,
    pub noinline: bool,
}

impl FunctionAttributes {
    /// Create a new `FunctionAttributes` with all flags cleared.
    pub fn new() -> Self {
        Self::default()
    }

    /// The attributes of a function *definition*, assembled from the parser's
    /// collected declaration state in one place.
    ///
    /// The file-scope path and the nested-function path each enumerated the flags
    /// they happened to copy, and the lists drifted: the nested one carried six
    /// of these twenty-one, so `auto void f(void) __attribute__((noreturn))
    /// { for (;;) {} }` defined a function that analysis did not consider
    /// noreturn -- with `pure`, `const` and `cold` dropped the same way, which is
    /// both a wrong diagnostic answer and a lost optimization.  Sharing the
    /// assembly is what stops the next attribute from being forgotten; a second
    /// hand-kept list would only re-open it.
    ///
    /// `no_instrument` is an argument rather than a read of `decl` because the
    /// file-scope caller merges the attribute with a prior declaration's (`S13`),
    /// and that merge needs state `decl` does not have.
    pub fn for_definition(decl: &DeclAttributes, spec: DefSpecifiers, no_instrument: bool) -> Self {
        let mut attrs = Self::new();
        attrs.set_static(spec.static_);
        attrs.set_inline(spec.inline_);
        attrs.set_extern(spec.extern_);
        attrs.set_gnu_inline(spec.gnu_inline);
        attrs.set_always_inline(spec.always_inline);
        attrs.set_noinline(spec.noinline);
        attrs.set_constructor(decl.is_constructor());
        attrs.set_destructor(decl.is_destructor());
        attrs.set_weak(decl.is_weak());
        attrs.set_used(decl.is_used());
        attrs.set_fastcall(decl.is_fastcall());
        attrs.regparm = decl.regparm;
        attrs.set_naked(decl.is_naked());
        attrs.set_no_instrument(no_instrument);
        attrs.set_noreturn(decl.is_noreturn());
        attrs.set_pure(decl.is_pure());
        attrs.set_const_attr(decl.is_const_attr());
        attrs.set_cold(decl.is_cold());
        attrs.section = decl.section.clone();
        attrs.visibility = decl.visibility.clone();
        attrs.symver = decl.symver.clone();
        attrs.diagnose_as = decl.diagnose_as.clone();
        attrs
    }

    // --- flag getters ---

    #[inline]
    pub fn is_static(&self) -> bool {
        self.flags & func_attr_flag::STATIC != 0
    }
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.flags & func_attr_flag::INLINE != 0
    }
    #[inline]
    pub fn is_extern(&self) -> bool {
        self.flags & func_attr_flag::EXTERN != 0
    }
    #[inline]
    pub fn is_gnu_inline(&self) -> bool {
        self.flags & func_attr_flag::GNU_INLINE != 0
    }
    #[inline]
    pub fn is_always_inline(&self) -> bool {
        self.flags & func_attr_flag::ALWAYS_INLINE != 0
    }
    #[inline]
    pub fn is_noinline(&self) -> bool {
        self.flags & func_attr_flag::NOINLINE != 0
    }
    #[inline]
    pub fn is_constructor(&self) -> bool {
        self.flags & func_attr_flag::CONSTRUCTOR != 0
    }
    #[inline]
    pub fn is_destructor(&self) -> bool {
        self.flags & func_attr_flag::DESTRUCTOR != 0
    }
    #[inline]
    pub fn is_weak(&self) -> bool {
        self.flags & func_attr_flag::WEAK != 0
    }
    #[inline]
    pub fn is_used(&self) -> bool {
        self.flags & func_attr_flag::USED != 0
    }
    #[inline]
    pub fn is_fastcall(&self) -> bool {
        self.flags & func_attr_flag::FASTCALL != 0
    }
    #[inline]
    pub fn is_naked(&self) -> bool {
        self.flags & func_attr_flag::NAKED != 0
    }
    #[inline]
    pub fn is_noreturn(&self) -> bool {
        self.flags & func_attr_flag::NORETURN != 0
    }
    #[inline]
    pub fn is_no_instrument(&self) -> bool {
        self.flags & func_attr_flag::NO_INSTRUMENT != 0
    }
    #[inline]
    pub fn is_pure(&self) -> bool {
        self.flags & func_attr_flag::PURE != 0
    }
    #[inline]
    pub fn is_const_attr(&self) -> bool {
        self.flags & func_attr_flag::CONST_ATTR != 0
    }
    #[inline]
    pub fn is_cold(&self) -> bool {
        self.flags & func_attr_flag::COLD != 0
    }

    // --- flag setters ---

    #[inline]
    pub fn set_static(&mut self, v: bool) {
        self.set_flag(func_attr_flag::STATIC, v)
    }
    #[inline]
    pub fn set_inline(&mut self, v: bool) {
        self.set_flag(func_attr_flag::INLINE, v)
    }
    #[inline]
    pub fn set_extern(&mut self, v: bool) {
        self.set_flag(func_attr_flag::EXTERN, v)
    }
    #[inline]
    pub fn set_gnu_inline(&mut self, v: bool) {
        self.set_flag(func_attr_flag::GNU_INLINE, v)
    }
    #[inline]
    pub fn set_always_inline(&mut self, v: bool) {
        self.set_flag(func_attr_flag::ALWAYS_INLINE, v)
    }
    #[inline]
    pub fn set_noinline(&mut self, v: bool) {
        self.set_flag(func_attr_flag::NOINLINE, v)
    }
    #[inline]
    pub fn set_constructor(&mut self, v: bool) {
        self.set_flag(func_attr_flag::CONSTRUCTOR, v)
    }
    #[inline]
    pub fn set_destructor(&mut self, v: bool) {
        self.set_flag(func_attr_flag::DESTRUCTOR, v)
    }
    #[inline]
    pub fn set_weak(&mut self, v: bool) {
        self.set_flag(func_attr_flag::WEAK, v)
    }
    #[inline]
    pub fn set_used(&mut self, v: bool) {
        self.set_flag(func_attr_flag::USED, v)
    }
    #[inline]
    pub fn set_fastcall(&mut self, v: bool) {
        self.set_flag(func_attr_flag::FASTCALL, v)
    }
    #[inline]
    pub fn set_naked(&mut self, v: bool) {
        self.set_flag(func_attr_flag::NAKED, v)
    }
    #[inline]
    pub fn set_noreturn(&mut self, v: bool) {
        self.set_flag(func_attr_flag::NORETURN, v)
    }
    #[inline]
    pub fn set_no_instrument(&mut self, v: bool) {
        self.set_flag(func_attr_flag::NO_INSTRUMENT, v)
    }
    #[inline]
    pub fn set_pure(&mut self, v: bool) {
        self.set_flag(func_attr_flag::PURE, v)
    }
    #[inline]
    pub fn set_const_attr(&mut self, v: bool) {
        self.set_flag(func_attr_flag::CONST_ATTR, v)
    }
    #[inline]
    pub fn set_cold(&mut self, v: bool) {
        self.set_flag(func_attr_flag::COLD, v)
    }

    #[inline]
    fn set_flag(&mut self, mask: u32, v: bool) {
        if v {
            self.flags |= mask;
        } else {
            self.flags &= !mask;
        }
    }
}

impl std::fmt::Debug for FunctionAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FunctionAttributes")
            .field("is_static", &self.is_static())
            .field("is_inline", &self.is_inline())
            .field("is_extern", &self.is_extern())
            .field("is_gnu_inline", &self.is_gnu_inline())
            .field("is_always_inline", &self.is_always_inline())
            .field("is_noinline", &self.is_noinline())
            .field("is_constructor", &self.is_constructor())
            .field("is_destructor", &self.is_destructor())
            .field("is_weak", &self.is_weak())
            .field("is_used", &self.is_used())
            .field("is_fastcall", &self.is_fastcall())
            .field("regparm", &self.regparm)
            .field("is_naked", &self.is_naked())
            .field("is_noreturn", &self.is_noreturn())
            .field("is_cold", &self.is_cold())
            .field("section", &self.section)
            .field("visibility", &self.visibility)
            .finish()
    }
}

/// A function definition.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub return_type: TypeSpecifier,
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub variadic: bool,
    pub body: CompoundStmt,
    /// Function attributes (storage class, inline hints, GCC __attribute__).
    pub attrs: FunctionAttributes,
    /// Alignment requested on this definition -- `__attribute__((aligned(N)))`
    /// or `_Alignas(N)` written on the DEFINITION rather than on a prototype.
    ///
    /// GCC honours both channels and takes the maximum; this field is the
    /// definition channel, which previously had no path to codegen: alignment
    /// reaches the backend through `IrModule::function_alignments`, keyed by the
    /// emitted function name, and only `lower_global_decl` (prototypes) ever
    /// wrote to it.  A nested function's emitted name is mangled
    /// (`parent.inner`), which is why the key at the definition end has to be
    /// taken from the built `IrFunction` rather than from the C name.
    pub alignment: Option<usize>,
    pub is_kr: bool,
    pub span: Span,
}

/// A parameter declaration.
#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub type_spec: TypeSpecifier,
    pub name: Option<String>,
    /// Direct alignment override written IN the parameter list:
    /// `int x __attribute__((aligned(32)))` or `_Alignas(32) int x`.
    /// The typedef spelling rides on `type_spec` (TypedefName); the direct
    /// spelling lands here — without it the alignment is silently dropped
    /// while the spelling is accepted (silent misalignment of `&x`).
    pub alignment: Option<usize>,
    /// `_Alignas(type-name)` form: resolved by the lowerer via alignof_type.
    pub alignas_type: Option<TypeSpecifier>,
    /// For function pointer parameters, the parameter types of the pointed-to function.
    /// E.g., for `float (*func)(float, float)`, this holds the two float param decls.
    pub fptr_params: Option<Vec<ParamDecl>>,
    /// Whether the pointed-to function accepts a variadic tail. Kept separate
    /// because CType erases the parser's explicit function-pointer declarator.
    pub fptr_variadic: bool,
    /// Whether this parameter's base type has a `const` qualifier.
    /// Used by _Generic matching to distinguish e.g. `const int *` from `int *`.
    pub is_const: bool,
    /// Whether this parameter's base type has a `volatile` qualifier.
    /// `volatile T *p` decays to a pointer whose POINTEE is volatile: every
    /// `*p` access is an observable side effect (C11 5.1.2.3).  CType does
    /// not track qualifiers, so the flag rides on ParamDecl.
    pub is_volatile: bool,
    /// Whether the adjusted pointer parameter is restrict-qualified. Unlike
    /// const, this is an optimization contract: accesses based on distinct
    /// restrict parameters may be proven non-aliasing within the function.
    pub is_restrict: bool,
    /// VLA size expressions from the outermost array dimension that was decayed to pointer.
    /// E.g., for `void foo(int a, int b[a++])`, the expression `a++` is stored here
    /// so its side effects can be evaluated at function entry during IR lowering.
    pub vla_size_exprs: Vec<Expr>,
    /// Declared with an array declarator whose outermost dimension was
    /// adjusted to a pointer (C11 6.7.6.3p7): `T a[2]` arrives as `T *`.
    /// The constraints on the unadjusted type still apply -- `void a[2]` is
    /// an array of `void` -- so sema needs to know the adjustment happened.
    pub decayed_array: bool,
    /// The number of `*` inside the parenthesized declarator for function pointer params.
    /// `(*fp)` has depth 1, `(**fpp)` has depth 2. A depth >= 2 means pointer-to-function-
    /// pointer. This distinguishes `int (**fpp)(int)` from `void *(*fp)(size_t)` which
    /// otherwise have identical CType representations.
    pub fptr_inner_ptr_depth: u32,
}

/// Bit masks for boolean flags in `Declaration::flags`.
///
/// Each flag occupies one bit of a `u16`, matching the pattern used by
/// `FunctionAttributes` and `DeclAttributes`.  New flags can be added by
/// defining the next power-of-two constant.
pub mod decl_flag {
    /// `static` storage class.
    pub const STATIC: u16 = 1 << 0;
    /// `extern` storage class.
    pub const EXTERN: u16 = 1 << 1;
    /// `typedef` storage class.
    pub const TYPEDEF: u16 = 1 << 2;
    /// `const` type qualifier.
    pub const CONST: u16 = 1 << 3;
    /// `volatile` type qualifier.
    pub const VOLATILE: u16 = 1 << 4;
    /// GCC `-fcommon` tentative definition (no initialiser, no `extern`).
    pub const COMMON: u16 = 1 << 5;
    /// `_Thread_local` or `__thread` storage class.
    pub const THREAD_LOCAL: u16 = 1 << 6;
    /// `__attribute__((transparent_union))` applied to a typedef.
    pub const TRANSPARENT_UNION: u16 = 1 << 7;
    /// `inline` function specifier on a declaration (not a definition).
    /// Used to implement C99 6.7.4p7: a function provides an external definition
    /// only if not ALL file-scope declarations include `inline`.
    pub const INLINE: u16 = 1 << 8;
    /// The POINTER itself is const (`T *const p`), as opposed to `CONST`
    /// which describes the pointee for pointer declarators.
    pub const POINTER_CONST: u16 = 1 << 9;
    /// C23 `constexpr` storage-class specifier (6.7.1).
    pub const CONSTEXPR: u16 = 1 << 10;
    /// `constexpr` came after `_Thread_local` or `auto` (diagnostic order).
    pub const CONSTEXPR_LAST: u16 = 1 << 11;
}

/// A variable/type declaration.
///
/// Boolean storage-class / qualifier / attribute flags are stored as a packed
/// bitfield (`flags`) for memory efficiency — 9 booleans collapse from 9 bytes
/// into 2 bytes.  Accessor methods provide the same API as the old struct
/// fields.
#[derive(Clone)]
pub struct Declaration {
    pub type_spec: TypeSpecifier,
    pub declarators: Vec<InitDeclarator>,
    /// Packed boolean flags — see `decl_flag` constants.
    flags: u16,
    /// Alignment override from _Alignas(N) or __attribute__((aligned(N))).
    pub alignment: Option<usize>,
    /// Type specifier from _Alignas(type) that the lowerer should resolve for alignment.
    /// The parser can't resolve typedef names, so when _Alignas takes a type argument,
    /// we store the type specifier here for proper resolution during lowering.
    pub alignas_type: Option<TypeSpecifier>,
    /// Type from `__attribute__((aligned(sizeof(type))))`.  The parser can't compute
    /// sizeof for struct/union types accurately, so we capture the type here and let
    /// sema/lowerer recompute `sizeof(type)` with full layout information.
    pub alignment_sizeof_type: Option<TypeSpecifier>,
    /// Address space qualifier on the variable itself (not on a pointer).
    /// E.g., `extern const struct pcpu_hot __seg_gs const_pcpu_hot;` has SegGs.
    /// Used for x86 per-CPU variable access with %gs:/%fs: segment prefixes.
    pub address_space: AddressSpace,
    /// GCC __attribute__((vector_size(N))): total vector size in bytes.
    /// When present on a typedef, wraps the base type in CType::Vector.
    pub vector_size: Option<usize>,
    /// Clang __attribute__((ext_vector_type(N))): number of vector elements.
    /// Resolved to total bytes in lowering via N * sizeof(element_type).
    pub ext_vector_nelem: Option<usize>,
    /// Deferred vector_size(EXPR): EXPR was not parse-time evaluable (e.g.
    /// `16 * sizeof(some_typedef)`); resolved in sema where typedef sizes are
    /// known.
    pub vector_size_expr: Option<Box<Expr>>,
    pub span: Span,
}

impl Declaration {
    // --- flag getters ---

    #[inline]
    pub fn is_static(&self) -> bool {
        self.flags & decl_flag::STATIC != 0
    }
    #[inline]
    pub fn is_extern(&self) -> bool {
        self.flags & decl_flag::EXTERN != 0
    }
    #[inline]
    pub fn is_typedef(&self) -> bool {
        self.flags & decl_flag::TYPEDEF != 0
    }
    #[inline]
    pub fn is_const(&self) -> bool {
        self.flags & decl_flag::CONST != 0
    }
    #[inline]
    pub fn is_volatile(&self) -> bool {
        self.flags & decl_flag::VOLATILE != 0
    }
    #[inline]
    pub fn is_common(&self) -> bool {
        self.flags & decl_flag::COMMON != 0
    }
    #[inline]
    pub fn is_thread_local(&self) -> bool {
        self.flags & decl_flag::THREAD_LOCAL != 0
    }
    #[inline]
    pub fn is_transparent_union(&self) -> bool {
        self.flags & decl_flag::TRANSPARENT_UNION != 0
    }
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.flags & decl_flag::INLINE != 0
    }

    // --- flag setters ---

    #[inline]
    pub fn set_static(&mut self, v: bool) {
        self.set_flag(decl_flag::STATIC, v)
    }
    #[inline]
    pub fn set_extern(&mut self, v: bool) {
        self.set_flag(decl_flag::EXTERN, v)
    }
    #[inline]
    pub fn set_typedef(&mut self, v: bool) {
        self.set_flag(decl_flag::TYPEDEF, v)
    }
    /// Attach the deferred (parse-time unevaluable) vector_size expression.
    #[inline]
    pub fn set_vector_size_expr(&mut self, e: Option<Box<Expr>>) {
        self.vector_size_expr = e;
    }
    #[inline]
    pub fn set_const(&mut self, v: bool) {
        self.set_flag(decl_flag::CONST, v)
    }
    #[inline]
    pub fn set_volatile(&mut self, v: bool) {
        self.set_flag(decl_flag::VOLATILE, v)
    }
    #[inline]
    pub fn set_pointer_const(&mut self, v: bool) {
        self.set_flag(decl_flag::POINTER_CONST, v)
    }
    #[inline]
    pub fn set_constexpr(&mut self, v: bool) {
        self.set_flag(decl_flag::CONSTEXPR, v)
    }
    /// C23 `constexpr`: a named constant (6.7.1p5-p6, 6.6p7-p8).
    #[inline]
    pub fn is_constexpr(&self) -> bool {
        self.flags & decl_flag::CONSTEXPR != 0
    }
    #[inline]
    pub fn set_constexpr_last(&mut self, v: bool) {
        self.set_flag(decl_flag::CONSTEXPR_LAST, v)
    }
    /// `constexpr` followed `_Thread_local` or `auto` in the specifiers.
    #[inline]
    pub fn is_constexpr_last(&self) -> bool {
        self.flags & decl_flag::CONSTEXPR_LAST != 0
    }
    /// True for `T *const p`: the pointer object itself is read-only.
    #[inline]
    pub fn is_pointer_const(&self) -> bool {
        self.flags & decl_flag::POINTER_CONST != 0
    }
    #[inline]
    pub fn set_common(&mut self, v: bool) {
        self.set_flag(decl_flag::COMMON, v)
    }
    #[inline]
    pub fn set_thread_local(&mut self, v: bool) {
        self.set_flag(decl_flag::THREAD_LOCAL, v)
    }
    #[inline]
    pub fn set_transparent_union(&mut self, v: bool) {
        self.set_flag(decl_flag::TRANSPARENT_UNION, v)
    }
    #[inline]
    pub fn set_inline(&mut self, v: bool) {
        self.set_flag(decl_flag::INLINE, v)
    }

    #[inline]
    fn set_flag(&mut self, mask: u16, v: bool) {
        if v {
            self.flags |= mask;
        } else {
            self.flags &= !mask;
        }
    }

    /// Create a `Declaration` with all flags cleared.  Non-boolean fields must
    /// be supplied; boolean qualifiers are set afterwards via the `set_*` methods.
    pub fn new(
        type_spec: TypeSpecifier,
        declarators: Vec<InitDeclarator>,
        alignment: Option<usize>,
        alignas_type: Option<TypeSpecifier>,
        alignment_sizeof_type: Option<TypeSpecifier>,
        address_space: AddressSpace,
        vector_size: Option<usize>,
        ext_vector_nelem: Option<usize>,
        span: Span,
    ) -> Self {
        Self {
            type_spec,
            declarators,
            flags: 0,
            alignment,
            alignas_type,
            alignment_sizeof_type,
            address_space,
            vector_size,
            ext_vector_nelem,
            vector_size_expr: None,
            span,
        }
    }

    /// Create an empty declaration (used for skipped constructs like asm directives).
    pub fn empty() -> Self {
        Self::new(
            TypeSpecifier::Void,
            Vec::new(),
            None,
            None,
            None,
            AddressSpace::Default,
            None,
            None,
            Span::dummy(),
        )
    }

    /// Resolve a declaration-level vector attribute without conflating its
    /// two layout contracts: `vector_size` is an explicit byte width, while
    /// `ext_vector_type` is an explicit logical lane count with target padding.
    pub fn resolve_vector_ctype(&self, element: CType) -> Option<CType> {
        if let Some(bytes) = self.vector_size {
            Some(CType::Vector(Box::new(element), bytes))
        } else if let Some(lanes) = self.ext_vector_nelem {
            ExtVectorLayout::new(&element, lanes)
                .map(|layout| CType::ExtVector(Box::new(element), layout))
        } else {
            None
        }
    }

    /// Resolve the unpadded element payload size for legacy consumers that
    /// only need a byte count. New type/layout code should use
    /// `resolve_vector_ctype` so ext-vector lane count and padding are retained.
    pub fn resolve_vector_size(&self, elem_size: usize) -> Option<usize> {
        if let Some(vs) = self.vector_size {
            Some(vs)
        } else {
            self.ext_vector_nelem.and_then(|n| n.checked_mul(elem_size))
        }
    }
}

impl std::fmt::Debug for Declaration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Declaration")
            .field("type_spec", &self.type_spec)
            .field("declarators", &self.declarators)
            .field("is_static", &self.is_static())
            .field("is_extern", &self.is_extern())
            .field("is_typedef", &self.is_typedef())
            .field("is_const", &self.is_const())
            .field("is_volatile", &self.is_volatile())
            .field("is_common", &self.is_common())
            .field("is_thread_local", &self.is_thread_local())
            .field("is_transparent_union", &self.is_transparent_union())
            .field("alignment", &self.alignment)
            .field("alignas_type", &self.alignas_type)
            .field("alignment_sizeof_type", &self.alignment_sizeof_type)
            .field("address_space", &self.address_space)
            .field("vector_size", &self.vector_size)
            .field("ext_vector_nelem", &self.ext_vector_nelem)
            .field("vector_size_expr", &self.vector_size_expr)
            .field("span", &self.span)
            .finish()
    }
}

/// Attributes that can be applied to individual declarators via GCC
/// __attribute__((...)) syntax.
///
/// Boolean attributes are stored as a packed bitfield (`flags`). Non-boolean
/// attributes (`alias_target`, `visibility`, `section`, `asm_register`,
/// `cleanup_fn`) remain as `Option<String>`.
#[derive(Clone, Default)]
pub struct DeclAttributes {
    /// Packed boolean flags — see `decl_attr_flag` constants.
    flags: u16,
    /// Named address space written after the declarator's last `*`
    /// (`long *__seg_gs p`): the space of the object itself, not its pointee.
    pub object_space: AddressSpace,
    /// Whether the declared object is itself `volatile`: the qualifier written
    /// after the declarator's last `*`, or the specifier's qualifier when the
    /// declarator has no derivations. A typedef recorded with this flag makes
    /// each object declared through it volatile, as its written text would.
    pub object_volatile: bool,
    /// Whether the declared object is itself `restrict`- or `_Atomic`-
    /// qualified, by the same rule as `object_volatile` (an array's object
    /// qualifier is its elements'). Atomic-ness through a typedef name is not
    /// tracked.
    pub object_restrict: bool,
    pub object_atomic: bool,
    /// Whether the declaration's specifiers are `volatile`-qualified (the base
    /// qualifier, not the declarator's own). A typedef recorded with this flag
    /// qualifies each use of the name (see `Parser::typedef_base_volatile`).
    pub base_volatile: bool,
    /// __attribute__((alias("target"))) - this symbol is an alias for target
    pub alias_target: Option<String>,
    /// __attribute__((visibility("hidden"))) etc.
    pub visibility: Option<String>,
    /// __attribute__((section("..."))) - place in specific ELF section
    pub section: Option<String>,
    /// register var __asm__("regname") - pin to specific register for inline asm
    pub asm_register: Option<String>,
    /// __attribute__((cleanup(func))) - call func(&var) when var goes out of scope.
    /// Used for RAII-style cleanup (e.g., Linux kernel guard()/scoped_guard() for mutex_unlock).
    pub cleanup_fn: Option<String>,
    /// __attribute__((symver("name@@VERSION"))) - symbol version alias
    pub symver: Option<String>,
    /// __attribute__((__diagnose_as(__builtin_NAME, ...))) - the declared
    /// function is an analyzer alias of builtin NAME; calls that survive
    /// inlining fold to the builtin (kernel fortify-string family).
    pub diagnose_as: Option<String>,
    /// __attribute__((regparm(N))) on a function declaration.
    pub regparm: Option<u8>,
}

/// Bit masks for boolean flags in `DeclAttributes::flags`.
pub mod decl_attr_flag {
    /// `__attribute__((constructor))` — run before main.
    pub const CONSTRUCTOR: u16 = 1 << 0;
    /// `__attribute__((destructor))` — run after main.
    pub const DESTRUCTOR: u16 = 1 << 1;
    /// `__attribute__((weak))` — emit as a weak symbol.
    pub const WEAK: u16 = 1 << 2;
    /// `__attribute__((error("...")))` or `__attribute__((warning("...")))`.
    pub const ERROR_ATTR: u16 = 1 << 3;
    /// `__attribute__((noreturn))` or `_Noreturn`.
    pub const NORETURN: u16 = 1 << 4;
    /// `__attribute__((used))` — prevent dead code elimination.
    pub const USED: u16 = 1 << 5;
    /// `__attribute__((fastcall))` — i386 fastcall calling convention.
    pub const FASTCALL: u16 = 1 << 6;
    /// `__attribute__((naked))` — emit no prologue/epilogue.
    pub const NAKED: u16 = 1 << 7;
    /// `__attribute__((no_instrument_function))` — skip mcount/__fentry__ prologue.
    pub const NO_INSTRUMENT: u16 = 1 << 8;
    /// `__attribute__((pure))` — read-only memory access, no observable side effects.
    pub const PURE: u16 = 1 << 9;
    /// `__attribute__((const))` — no memory access, no observable side effects.
    pub const CONST_ATTR: u16 = 1 << 10;
    /// `__attribute__((cold))` — the function is unlikely to execute.
    /// Meaningful on functions only; ignored for variables like GCC.
    pub const COLD: u16 = 1 << 11;
    /// GNU `auto` at block scope, i.e. a nested function (see
    /// `parsed_attr_flag::NESTED_FN`). Not an attribute spelling: there is no
    /// `__attribute__((auto))`, only the storage-class keyword.
    pub const NESTED_FN: u16 = 1 << 12;
}

impl DeclAttributes {
    // --- flag getters ---

    #[inline]
    pub fn is_constructor(&self) -> bool {
        self.flags & decl_attr_flag::CONSTRUCTOR != 0
    }
    #[inline]
    pub fn is_destructor(&self) -> bool {
        self.flags & decl_attr_flag::DESTRUCTOR != 0
    }
    #[inline]
    pub fn is_weak(&self) -> bool {
        self.flags & decl_attr_flag::WEAK != 0
    }
    #[inline]
    pub fn is_error_attr(&self) -> bool {
        self.flags & decl_attr_flag::ERROR_ATTR != 0
    }
    #[inline]
    pub fn is_noreturn(&self) -> bool {
        self.flags & decl_attr_flag::NORETURN != 0
    }
    #[inline]
    pub fn is_nested_fn(&self) -> bool {
        self.flags & decl_attr_flag::NESTED_FN != 0
    }
    #[inline]
    pub fn is_used(&self) -> bool {
        self.flags & decl_attr_flag::USED != 0
    }
    #[inline]
    pub fn is_fastcall(&self) -> bool {
        self.flags & decl_attr_flag::FASTCALL != 0
    }
    #[inline]
    pub fn is_naked(&self) -> bool {
        self.flags & decl_attr_flag::NAKED != 0
    }
    #[inline]
    pub fn is_no_instrument(&self) -> bool {
        self.flags & decl_attr_flag::NO_INSTRUMENT != 0
    }
    #[inline]
    pub fn is_pure(&self) -> bool {
        self.flags & decl_attr_flag::PURE != 0
    }
    #[inline]
    pub fn is_const_attr(&self) -> bool {
        self.flags & decl_attr_flag::CONST_ATTR != 0
    }
    #[inline]
    pub fn is_cold(&self) -> bool {
        self.flags & decl_attr_flag::COLD != 0
    }

    // --- flag setters ---

    #[inline]
    pub fn set_constructor(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::CONSTRUCTOR, v)
    }
    #[inline]
    pub fn set_destructor(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::DESTRUCTOR, v)
    }
    #[inline]
    pub fn set_weak(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::WEAK, v)
    }
    #[inline]
    pub fn set_error_attr(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::ERROR_ATTR, v)
    }
    #[inline]
    pub fn set_noreturn(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::NORETURN, v)
    }
    #[inline]
    pub fn set_nested_fn(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::NESTED_FN, v)
    }
    #[inline]
    pub fn set_used(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::USED, v)
    }
    #[inline]
    pub fn set_fastcall(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::FASTCALL, v)
    }
    #[inline]
    pub fn set_naked(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::NAKED, v)
    }
    #[inline]
    pub fn set_no_instrument(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::NO_INSTRUMENT, v)
    }
    #[inline]
    pub fn set_pure(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::PURE, v)
    }
    #[inline]
    pub fn set_const_attr(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::CONST_ATTR, v)
    }
    #[inline]
    pub fn set_cold(&mut self, v: bool) {
        self.set_flag(decl_attr_flag::COLD, v)
    }

    #[inline]
    fn set_flag(&mut self, mask: u16, v: bool) {
        if v {
            self.flags |= mask;
        } else {
            self.flags &= !mask;
        }
    }
}

impl std::fmt::Debug for DeclAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclAttributes")
            .field("is_constructor", &self.is_constructor())
            .field("is_destructor", &self.is_destructor())
            .field("is_weak", &self.is_weak())
            .field("is_error_attr", &self.is_error_attr())
            .field("is_noreturn", &self.is_noreturn())
            .field("is_used", &self.is_used())
            .field("alias_target", &self.alias_target)
            .field("regparm", &self.regparm)
            .field("visibility", &self.visibility)
            .field("section", &self.section)
            .field("asm_register", &self.asm_register)
            .field("cleanup_fn", &self.cleanup_fn)
            .finish()
    }
}

/// A declarator with optional initializer.
#[derive(Debug, Clone)]
pub struct InitDeclarator {
    pub name: String,
    pub derived: Vec<DerivedDeclarator>,
    pub init: Option<Initializer>,
    /// Declarator attributes (GCC __attribute__, asm register, etc.).
    pub attrs: DeclAttributes,
    pub span: Span,
}

/// Derived parts of a declarator (pointers, arrays, function params).
#[derive(Debug, Clone)]
pub enum DerivedDeclarator {
    /// A `*`. The address space is the qualifier written immediately after this
    /// `*` (`long *__seg_gs *p`: the first star carries `__seg_gs`). It qualifies
    /// the storage of the pointer that the NEXT pointer level points to, so
    /// `p` points at a segment pointer. The last star's space is the declared
    /// object's own space and is carried separately (`DeclAttributes::object_space`).
    Pointer(AddressSpace),
    Array(Option<Box<Expr>>),
    Function(Vec<ParamDecl>, bool), // params, variadic
    /// Function pointer: (*name)(params) - distinguishes from pointer-to-return-type
    FunctionPointer(Vec<ParamDecl>, bool), // params, variadic
}

impl DerivedDeclarator {
    /// Whether a declarator with this derived list declares a FUNCTION (as
    /// opposed to an object of pointer/array/function-pointer type).
    ///
    /// The parser's encoding (`combine_declarator_parts`) always places the
    /// declared entity's own parameter list LAST: `int f(int)` → `[Function]`,
    /// `int *f(int)` → `[Pointer, Function]`, and a function returning a
    /// function pointer, `void (*f(int))(void)` →
    /// `[Pointer, FunctionPointer(void), Function(int)]`.  Function-pointer
    /// objects end in `FunctionPointer` (`int (*fp)(int)`), a pointer
    /// (`int (**fpp)(int)`), or an array (`int (*fps[3])(int)`).
    ///
    /// This is the single canonical predicate.  The historical spelling
    /// "contains a Function and no FunctionPointer" misclassified every
    /// function returning a function pointer as an OBJECT, so a plain
    /// prototype such as SQLite's
    /// `void (*sqlite3OsDlSym(sqlite3_vfs*, void*, const char*))(void);`
    /// emitted a tentative definition (`B sqlite3OsDlSym`) in every TU that
    /// included it and the link failed with "multiple definition"
    /// (regression `fn_returning_fnptr_prototype.c`).
    pub fn declares_function(derived: &[DerivedDeclarator]) -> bool {
        matches!(derived.last(), Some(DerivedDeclarator::Function(_, _)))
    }

    /// Pointer indirection depth of the RETURN type a derived list encodes.
    ///
    /// For a FUNCTION declarator (the entity's own parameter list last) the
    /// return type is everything before it: every `Pointer` in that prefix
    /// is one indirection of the return (`int **f(int)` -> 2, and
    /// `void (*(*f(int))(void))(int)` — a function returning a pointer to
    /// a function pointer — -> 2, where a LEADING-run count would
    /// undercount to 1 because a `FunctionPointer` precedes the second
    /// `Pointer`). For non-function declarators the whole list belongs to
    /// the entity's own type and every `Pointer` counts.
    ///
    /// This is the single canonical depth oracle: the sema function-typedef
    /// path and the block-scope function-meta lowering both used private
    /// counts (a leading-run `take_while` and a hand-rolled prefix slice)
    /// that agree on the common shapes but diverge exactly on the
    /// function-pointer-in-return-type shapes this session made
    /// representable.
    pub fn return_pointer_depth(derived: &[DerivedDeclarator]) -> usize {
        let slice = if Self::declares_function(derived) {
            &derived[..derived.len().saturating_sub(1)]
        } else {
            derived
        };
        slice
            .iter()
            .filter(|d| matches!(d, DerivedDeclarator::Pointer(_)))
            .count()
    }
}

#[cfg(test)]
mod declarator_law_tests {
    use super::DerivedDeclarator;
    use super::DerivedDeclarator::*;
    use super::ParamDecl;
    use super::TypeSpecifier;

    fn param() -> ParamDecl {
        ParamDecl {
            type_spec: TypeSpecifier::Int,
            name: None,
            alignment: None,
            alignas_type: None,
            fptr_params: None,
            fptr_variadic: false,
            is_const: false,
            is_volatile: false,
            is_restrict: false,
            vla_size_exprs: Vec::new(),
            decayed_array: false,
            fptr_inner_ptr_depth: 0,
        }
    }

    fn fnp(params: usize) -> DerivedDeclarator {
        DerivedDeclarator::FunctionPointer(vec![param(); params], false)
    }

    fn func(params: usize) -> DerivedDeclarator {
        DerivedDeclarator::Function(vec![param(); params], false)
    }

    /// The canonical predicate pair: `declares_function` keys on the
    /// entity's OWN parameter list being last; `return_pointer_depth`
    /// counts the Pointer indirections of the return type (the prefix for
    /// functions, the whole list otherwise). Every row mirrors the parser's
    /// `combine_declarator_parts` encoding documented on
    /// `DerivedDeclarator::declares_function`.
    #[test]
    fn declares_function_and_return_pointer_depth_laws() {
        // (derived, declares_function, return_pointer_depth)
        let rows: Vec<(Vec<DerivedDeclarator>, bool, usize)> = vec![
            // int f(void) — plain function.
            (vec![func(0)], true, 0),
            // int *f(int) — pointer return.
            (
                vec![
                    Pointer(crate::common::types::AddressSpace::Default),
                    func(1),
                ],
                true,
                1,
            ),
            // int **f(int) — two indirections.
            (
                vec![
                    Pointer(crate::common::types::AddressSpace::Default),
                    Pointer(crate::common::types::AddressSpace::Default),
                    func(1),
                ],
                true,
                2,
            ),
            // void (*f(int))(void) — function RETURNING a function pointer
            // (SQLite's sqlite3OsDlSym shape): the prefix has ONE Pointer.
            (
                vec![
                    Pointer(crate::common::types::AddressSpace::Default),
                    fnp(0),
                    func(1),
                ],
                true,
                1,
            ),
            // void (*(*f(int))(void))(int) — function returning a POINTER
            // TO a function pointer: the prefix interleaves a
            // FunctionPointer between two Pointers, exactly the shape a
            // leading-run count undercounts (1 instead of 2).
            (
                vec![
                    Pointer(crate::common::types::AddressSpace::Default),
                    fnp(0),
                    Pointer(crate::common::types::AddressSpace::Default),
                    fnp(1),
                    func(1),
                ],
                true,
                2,
            ),
            // int (*fp)(int) — function-pointer OBJECT: not a function,
            // whole-list depth 1 (the typedef return is depth-1 = 0: int).
            (
                vec![Pointer(crate::common::types::AddressSpace::Default), fnp(1)],
                false,
                1,
            ),
            // int *(*fp)(int) — fn-ptr object returning int*: depth 2,
            // typedef return depth 1.
            (
                vec![
                    Pointer(crate::common::types::AddressSpace::Default),
                    Pointer(crate::common::types::AddressSpace::Default),
                    fnp(1),
                ],
                false,
                2,
            ),
            // int (*fps[3])(int) — array of fn-ptrs: not a function.
            (vec![Array(None), fnp(1)], false, 0),
            // Plain pointer object.
            (
                vec![Pointer(crate::common::types::AddressSpace::Default)],
                false,
                1,
            ),
            // Empty declarator (abstract params of a bare type).
            (vec![], false, 0),
        ];
        for (i, (derived, is_fn, depth)) in rows.iter().enumerate() {
            assert_eq!(
                DerivedDeclarator::declares_function(derived),
                *is_fn,
                "row {i} ({derived:?}): declares_function"
            );
            assert_eq!(
                DerivedDeclarator::return_pointer_depth(derived),
                *depth,
                "row {i} ({derived:?}): return_pointer_depth"
            );
        }
    }
}

/// An initializer.
#[derive(Debug, Clone)]
pub enum Initializer {
    Expr(Expr),
    List(Vec<InitializerItem>),
}

/// An item in an initializer list.
#[derive(Debug, Clone)]
pub struct InitializerItem {
    pub designators: Vec<Designator>,
    pub init: Initializer,
}

/// A designator in an initializer.
#[derive(Debug, Clone)]
pub enum Designator {
    Index(Expr),
    /// GCC range designator: [lo ... hi]
    Range(Expr, Expr),
    Field(String),
}

/// Type specifiers.
#[derive(Debug, Clone)]
pub enum TypeSpecifier {
    Void,
    /// Signed 8-bit char; see `CType::Char` (plain on signed-char targets,
    /// explicit `signed char` on unsigned-char targets).
    Char,
    /// Explicit `signed char` on signed-char targets (`CType::SChar`).
    SignedChar,
    /// Plain `char` on unsigned-char targets (`CType::PlainUChar`).
    PlainUnsignedChar,
    Short,
    Int,
    Long,
    LongLong,
    Float,
    Double,
    LongDouble,
    /// _Float128 / __float128 (IEEE binary128, distinct from long double).
    Float128,
    /// C23 decimal floating point (_Decimal32/64/128, BID encoding).
    Decimal32,
    Decimal64,
    Decimal128,
    #[expect(dead_code)] // Matched in type resolution but not currently emitted by parser
    Signed,
    #[expect(dead_code)] // Matched in type resolution but not currently emitted by parser
    Unsigned,
    UnsignedChar,
    UnsignedShort,
    UnsignedInt,
    UnsignedLong,
    UnsignedLongLong,
    Int128,
    UnsignedInt128,
    Bool,
    ComplexFloat,
    ComplexDouble,
    ComplexLongDouble,
    /// Struct: (name, fields, is_packed, max_field_align from #pragma pack, struct-level aligned attribute,
    /// scalar_storage_order: Some(true) = big-endian, Some(false) = little-endian, None = default/native)
    Struct(
        Option<String>,
        Option<Vec<StructFieldDecl>>,
        bool,
        Option<usize>,
        Option<usize>,
        Option<bool>,
    ),
    /// Union: (name, fields, is_packed, max_field_align from #pragma pack, struct-level aligned attribute,
    /// scalar_storage_order: Some(true) = big-endian, Some(false) = little-endian, None = default/native)
    Union(
        Option<String>,
        Option<Vec<StructFieldDecl>>,
        bool,
        Option<usize>,
        Option<usize>,
        Option<bool>,
    ),
    /// Enum: (name, variants, is_packed, C23 fixed underlying type `: T`)
    Enum(
        Option<String>,
        Option<Vec<EnumVariant>>,
        bool,
        Option<Box<TypeSpecifier>>,
    ),
    TypedefName(String),
    Pointer(Box<TypeSpecifier>, AddressSpace),
    Array(Box<TypeSpecifier>, Option<Box<Expr>>),
    /// Function pointer type from cast/sizeof: return_type, params, variadic
    /// E.g., `(jv (*)(void*, jv))` produces FunctionPointer(jv, [void*, jv], false)
    FunctionPointer(Box<TypeSpecifier>, Vec<ParamDecl>, bool),
    /// Bare function type (not a pointer): return_type, params, variadic
    /// This is produced by ctype_to_type_spec for CType::Function, used in typeof
    /// resolution. Unlike FunctionPointer which represents Pointer(Function(...)),
    /// this represents just Function(...) without the pointer wrapper.
    BareFunction(Box<TypeSpecifier>, Vec<ParamDecl>, bool),
    /// typeof(expr) - GCC extension: type of an expression. The flag is true for
    /// `typeof_unqual`, which also drops the expression's named address space.
    Typeof(Box<Expr>, bool),
    /// typeof(type-name) - GCC extension: type from a type name
    TypeofType(Box<TypeSpecifier>),
    /// __auto_type - GCC extension: type inferred from initializer expression
    AutoType,
    /// C23 `nullptr_t`, produced by `typeof(nullptr)` round-trips.
    NullPtrT,
    /// __attribute__((vector_size(N))) applied directly in cast/compound-literal/sizeof.
    /// Wraps the base element type and total vector size in bytes.
    /// E.g., `(__attribute__((vector_size(16))) float){...}` becomes Vector(Float, 16).
    Vector(Box<TypeSpecifier>, usize),
    /// __attribute__((ext_vector_type(N))) applied directly in a type name.
    /// The second field is the logical lane count, not the padded byte size.
    ExtVector(Box<TypeSpecifier>, usize),
}

/// A field declaration in a struct/union.
#[derive(Debug, Clone)]
pub struct StructFieldDecl {
    pub type_spec: TypeSpecifier,
    pub name: Option<String>,
    pub bit_width: Option<Box<Expr>>,
    /// Derived declarator parts (pointers, arrays, function pointers) from the declarator.
    /// For simple fields like `int x` or `int *p`, this is empty (the pointer is in type_spec).
    /// For complex declarators like `void (*(*fp)(int))(void)`, this carries the full
    /// derived declarator chain that must be applied to type_spec to get the final type.
    pub derived: Vec<DerivedDeclarator>,
    /// Per-field alignment from _Alignas(N) or __attribute__((aligned(N))).
    pub alignment: Option<usize>,
    /// `_Alignas(type-name)` whose alignment the parser cannot know exactly
    /// (typedef, enum, ...). The layout builders resolve it and take the
    /// strictest of it and `alignment` (C11 6.7.5p6).
    pub alignas_type: Option<TypeSpecifier>,
    /// Per-field __attribute__((packed)) - forces this field's alignment to 1.
    pub is_packed: bool,
    /// `volatile` on the field's base type (`volatile u32 ctrl`, `volatile int a[4]`):
    /// the element or pointee access is volatile. Mirrors `DeclAttributes::base_volatile`.
    pub base_volatile: bool,
    /// `volatile` on the field object itself (`int *volatile p`). Mirrors
    /// `DeclAttributes::object_volatile`.
    pub object_volatile: bool,
}

/// An enum variant.
#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub value: Option<Box<Expr>>,
    /// The enumerator's identifier.
    pub span: Span,
}

/// A compound statement (block).
#[derive(Debug, Clone)]
pub struct CompoundStmt {
    pub items: Vec<BlockItem>,
    /// GNU __label__ declarations: local label names scoped to this block.
    /// When non-empty, label definitions and gotos within this block use
    /// scope-qualified names to avoid collisions (e.g., in statement expressions).
    pub local_labels: Vec<String>,
}

/// Items within a block.
#[derive(Debug, Clone)]
pub enum BlockItem {
    Declaration(Declaration),
    Statement(Stmt),
    /// GNU C nested function definition (GCC extension): a function defined
    /// at block scope that may reference enclosing-function locals through a
    /// static chain. Reuses `FunctionDef`; the item is lowered to a separate
    /// IR function with a mangled `parent.name` symbol.
    NestedFunction(FunctionDef),
    /// A `_Static_assert` the parser could not decide; sema evaluates it.
    StaticAssert(StaticAssert),
}

/// Statements.
#[derive(Debug, Clone)]
pub enum Stmt {
    Expr(Option<Expr>),
    Return(Option<Expr>, Span),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>, Span),
    While(Expr, Box<Stmt>, Span),
    DoWhile(Box<Stmt>, Expr, Span),
    For(
        Option<Box<ForInit>>,
        Option<Expr>,
        Option<Expr>,
        Box<Stmt>,
        Span,
    ),
    Compound(CompoundStmt),
    Break(Span),
    Continue(Span),
    Switch(Expr, Box<Stmt>, Span),
    Case(Expr, Box<Stmt>, Span),
    /// GNU case range: `case low ... high:` (GCC extension)
    CaseRange(Expr, Expr, Box<Stmt>, Span),
    Default(Box<Stmt>, Span),
    Goto(String, Span),
    /// Computed goto: goto *expr (GCC extension, labels-as-values)
    GotoIndirect(Box<Expr>, Span),
    Label(String, Box<Stmt>, Span),
    /// A declaration in statement position (C23: declarations allowed after labels,
    /// and in other statement contexts like `case`/`default`).
    Declaration(Declaration),
    InlineAsm {
        template: String,
        outputs: Vec<AsmOperand>,
        inputs: Vec<AsmOperand>,
        clobbers: Vec<String>,
        /// Goto labels for asm goto (e.g., `asm goto("..." : : : : label1, label2)`)
        goto_labels: Vec<String>,
    },
}

impl Stmt {
    /// Calls `f` on each expression this statement evaluates itself -- not
    /// those of nested statements, nor of a `for` clause's declaration (a
    /// declaration's own expressions come from `InitDeclarator::for_each_expr`).
    pub fn for_each_own_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        match self {
            Stmt::Expr(Some(e)) | Stmt::Return(Some(e), _) => f(e),
            Stmt::If(cond, ..)
            | Stmt::While(cond, ..)
            | Stmt::DoWhile(_, cond, _)
            | Stmt::Switch(cond, ..)
            | Stmt::Case(cond, ..) => f(cond),
            Stmt::CaseRange(lo, hi, ..) => {
                f(lo);
                f(hi);
            }
            Stmt::For(init, cond, inc, ..) => {
                if let Some(init) = init {
                    if let ForInit::Expr(e) = init.as_ref() {
                        f(e);
                    }
                }
                cond.iter().chain(inc).for_each(&mut *f);
            }
            Stmt::GotoIndirect(e, _) => f(e),
            Stmt::InlineAsm {
                outputs, inputs, ..
            } => outputs.iter().chain(inputs).for_each(|op| f(&op.expr)),
            Stmt::Expr(None)
            | Stmt::Return(None, _)
            | Stmt::Compound(_)
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Default(..)
            | Stmt::Goto(..)
            | Stmt::Label(..)
            | Stmt::Declaration(_) => {}
        }
    }

    /// Extract the source span from this statement, if available.
    /// Used by IR lowering to propagate source locations for debug info.
    pub fn span(&self) -> Option<Span> {
        match self {
            Stmt::Return(_, span)
            | Stmt::If(_, _, _, span)
            | Stmt::While(_, _, span)
            | Stmt::DoWhile(_, _, span)
            | Stmt::For(_, _, _, _, span)
            | Stmt::Break(span)
            | Stmt::Continue(span)
            | Stmt::Switch(_, _, span)
            | Stmt::Case(_, _, span)
            | Stmt::CaseRange(_, _, _, span)
            | Stmt::Default(_, span)
            | Stmt::Goto(_, span)
            | Stmt::GotoIndirect(_, span)
            | Stmt::Label(_, _, span) => Some(*span),
            Stmt::Expr(Some(expr)) => Some(expr.span()),
            Stmt::Declaration(decl) => Some(decl.span),
            Stmt::Expr(None) | Stmt::Compound(_) | Stmt::InlineAsm { .. } => None,
        }
    }
}

/// An operand in an inline asm statement.
#[derive(Debug, Clone)]
pub struct AsmOperand {
    /// Symbolic name (e.g., [name]) if present
    pub name: Option<String>,
    /// Constraint string (e.g., "=r", "+r", "r")
    pub constraint: String,
    /// The C expression for the operand
    pub expr: Expr,
}

/// For loop initializer.
#[derive(Debug, Clone)]
pub enum ForInit {
    Declaration(Declaration),
    Expr(Expr),
}

/// A type-safe identity key for AST `Expr` nodes, used as a HashMap key for
/// expression type and constant value caches.
///
/// Currently derived from the heap address of the Expr node (`&Expr as *const _
/// as usize`), which is stable because the AST is allocated once during parsing
/// and not moved before lowering consumes it.  Wrapping the raw address in a
/// newtype provides:
/// - **Type safety**: prevents accidental mixing with other `usize` values.
/// - **Self-documenting API**: call sites use `expr.id()` instead of a raw cast.
/// - **Future-proofing**: the underlying representation can be changed to a
///   counter-based scheme (assigned during parsing) without touching call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExprId(usize);

impl ExprId {
    /// Create an ExprId from a reference to an Expr node.
    #[inline]
    pub fn of(expr: &Expr) -> Self {
        ExprId(expr as *const Expr as usize)
    }
}

/// Qualifiers of a cast's type name, read from its tokens by the parser.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CastQualifiers {
    /// A `const` or `volatile` appears before the type's first top-level `*`
    /// (`(const void *)0`). The null-pointer-constant rule excludes these.
    pub pointee_qualified: bool,
    /// A `const` appears before the type's first top-level `*`: the base
    /// type is const (`(const char *)s`). See `common::qualifiers`.
    pub base_const: bool,
    /// The type is a single-level pointer whose pointee is `volatile`
    /// (`(volatile u32 *)addr`, the MMIO idiom): `*cast` is a volatile access.
    pub pointee_volatile: bool,
}

/// Expressions.
#[derive(Debug, Clone)]
pub enum Expr {
    IntLiteral(i64, Span),
    UIntLiteral(u64, Span),
    LongLiteral(i64, Span),
    ULongLiteral(u64, Span),
    LongLongLiteral(i64, Span), // ll/LL suffix (signed long long, always 64-bit)
    ULongLongLiteral(u64, Span), // ull/ULL suffix (unsigned long long, always 64-bit)
    FloatLiteral(f64, Span),    // double literal (no suffix)
    FloatLiteralF32(f64, Span), // float literal (f/F suffix)
    /// Long double literal (l/L suffix). Stores (f64_approx, f128_bytes, span).
    /// f128_bytes is IEEE 754 binary128 with full 112-bit mantissa precision.
    FloatLiteralLongDouble(f64, [u8; 16], Span),
    /// _Float128 literal (Q/q/f128 suffix): value + IEEE binary128 bytes.
    FloatLiteralF128(f64, [u8; 16], Span),
    /// C23 decimal FP literal (df/dd/dl suffix): (width 32|64|128, BID bits
    /// little-endian, span). Unary minus is applied as a separate expression
    /// and folded by const evaluation (BID sign flip).
    FloatLiteralDecimal(u8, [u8; 16], Span),
    /// Imaginary literal: value * I (double imaginary, e.g. 1.0i)
    ImaginaryLiteral(f64, Span),
    /// Float imaginary literal (e.g. 1.0fi)
    ImaginaryLiteralF32(f64, Span),
    /// Long double imaginary literal (e.g. 1.0Li). Stores (f64_approx, f128_bytes, span).
    ImaginaryLiteralLongDouble(f64, [u8; 16], Span),
    StringLiteral(String, Span),
    /// Wide string literal (L"...") - each char is a wchar_t (32-bit int)
    WideStringLiteral(String, Span),
    /// char16_t string literal (u"...") - each char is a char16_t (16-bit unsigned)
    Char16StringLiteral(String, Span),
    CharLiteral(char, Span),
    Identifier(String, Span),
    BinaryOp(BinOp, Box<Expr>, Box<Expr>, Span),
    UnaryOp(UnaryOp, Box<Expr>, Span),
    PostfixOp(PostfixOp, Box<Expr>, Span),
    Assign(Box<Expr>, Box<Expr>, Span),
    CompoundAssign(BinOp, Box<Expr>, Box<Expr>, Span),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>, Span),
    /// GNU extension: `cond ? : else_expr` - condition is evaluated once and used as the
    /// then-value if truthy. Semantically: `({ auto __tmp = cond; __tmp ? __tmp : else_expr; })`
    GnuConditional(Box<Expr>, Box<Expr>, Span),
    FunctionCall(Box<Expr>, Vec<Expr>, Span),
    ArraySubscript(Box<Expr>, Box<Expr>, Span),
    MemberAccess(Box<Expr>, String, Span),
    PointerMemberAccess(Box<Expr>, String, Span),
    /// `(type) operand`. The qualifiers record the pointee's `const`/`volatile`
    /// (`(const void *)`): C23 6.3.2.3p3 admits only an UNQUALIFIED `void *`
    /// cast as a null pointer constant, so that form is not one even when its
    /// operand is `0`. `pointee_volatile` makes `*(volatile T *)p` volatile.
    Cast(TypeSpecifier, Box<Expr>, Span, CastQualifiers),
    CompoundLiteral(TypeSpecifier, Box<Initializer>, Span),
    StmtExpr(CompoundStmt, Span),
    Sizeof(Box<SizeofArg>, Span),
    /// __builtin_va_arg(ap, type): extract next variadic argument of given type
    VaArg(Box<Expr>, TypeSpecifier, Span),
    /// __builtin_convertvector(expr, type): element-wise vector conversion
    /// (int<->float lanes). Second argument is a TYPE, so it needs its own
    /// AST node like VaArg.
    ConvertVector(Box<Expr>, TypeSpecifier, Span),
    /// _Alignof(type) - C11 standard, returns minimum ABI alignment
    Alignof(TypeSpecifier, Span),
    /// _Alignof(expr) - alignment of expression's type (via _Alignof macro path)
    AlignofExpr(Box<Expr>, Span),
    /// __alignof(type) / __alignof__(type) - GCC extension, returns preferred alignment
    /// On i686: __alignof__(long long) == 8, _Alignof(long long) == 4
    GnuAlignof(TypeSpecifier, Span),
    /// __alignof__(expr) - GCC extension, preferred alignment of expression's type
    GnuAlignofExpr(Box<Expr>, Span),
    Comma(Box<Expr>, Box<Expr>, Span),
    AddressOf(Box<Expr>, Span),
    Deref(Box<Expr>, Span),
    /// _Generic(controlling_expr, type1: expr1, type2: expr2, ..., default: exprN)
    GenericSelection(Box<Expr>, Vec<GenericAssociation>, Span),
    /// A type name as the controlling operand of `_Generic` (C2y N3260,
    /// accepted by GCC 16.2 and Clang 23.1.3 in C23 mode). It is never
    /// evaluated and, unlike an expression operand, undergoes no lvalue
    /// conversion: `_Generic(const int, const int: 1)` selects 1. The flag is
    /// the association-style constness (`GenericAssociation::is_const`).
    TypeOperand(Box<TypeSpecifier>, bool, Span),
    /// GCC extension: &&label (address of label, for computed goto)
    LabelAddr(String, Span),
    /// GCC extension: __builtin_types_compatible_p(type1, type2).
    /// Qualifier vectors run from the innermost/base type out through each
    /// derived pointer/array level (bit 0 const, bit 1 volatile, bit 2
    /// restrict). CType intentionally erases CVR qualifiers, but this builtin
    /// must ignore only the *top-level* qualifier and preserve nested ones.
    BuiltinTypesCompatibleP(TypeSpecifier, TypeSpecifier, Vec<u8>, Vec<u8>, Span),
}

pub(crate) fn nested_type_qualifiers_compatible(
    a: &[u8],
    a_levels: usize,
    b: &[u8],
    b_levels: usize,
) -> bool {
    fn normalized_nested(input: &[u8], levels: usize) -> Vec<u8> {
        let mut normalized = input.to_vec();
        // Typedef expansion can reveal pointer/array layers after parsing. A
        // layer absent from the syntax has no separately-written qualifier.
        normalized.resize(levels.max(1), 0);
        normalized.truncate(levels.max(1).saturating_sub(1));
        normalized
    }
    normalized_nested(a, a_levels) == normalized_nested(b, b_levels)
}

/// A _Generic association: either a type-expression pair, or a default expression.
#[derive(Debug, Clone)]
pub struct GenericAssociation {
    pub type_spec: Option<TypeSpecifier>, // None for "default"
    pub expr: Expr,
    /// Whether the association type has a const qualifier on the top-level type
    /// (for non-pointer types) or on the pointee (for pointer types like `const int *`).
    /// Used by _Generic matching to distinguish e.g. `const int *` from `int *`,
    /// since CType does not track const/volatile qualifiers.
    pub is_const: bool,
}

/// Sizeof argument can be a type or expression.
#[derive(Debug, Clone)]
pub enum SizeofArg {
    Type(TypeSpecifier),
    Expr(Expr),
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LogicalAnd,
    LogicalOr,
}

impl BinOp {
    /// Returns true for comparison operators (==, !=, <, <=, >, >=, &&, ||).
    pub fn is_comparison(self) -> bool {
        matches!(
            self,
            BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::LogicalAnd
                | BinOp::LogicalOr
        )
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Neg,
    BitNot,
    LogicalNot,
    PreInc,
    PreDec,
    /// __real__ expr - extract real part of complex number (GCC extension)
    RealPart,
    /// __imag__ expr - extract imaginary part of complex number (GCC extension)
    ImagPart,
}

/// Postfix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostfixOp {
    PostInc,
    PostDec,
}

impl InitDeclarator {
    /// Calls `f` on each expression of this declarator: array bounds, then
    /// the initializer's values and designator indices, in source order.
    pub fn for_each_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        for derived in &self.derived {
            if let DerivedDeclarator::Array(Some(bound)) = derived {
                f(bound);
            }
        }
        if let Some(init) = &self.init {
            init.for_each_expr(f);
        }
    }
}

impl TypeSpecifier {
    /// Calls `f` on each expression written inside this type, recursively
    /// through the types it is built from: array bounds, `typeof` operands,
    /// member bit-widths and array bounds, enumerator values, and those of
    /// parameter types. Exhaustive on purpose, like `Expr::for_each_child`.
    pub fn for_each_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        match self {
            TypeSpecifier::Void
            | TypeSpecifier::Char
            | TypeSpecifier::SignedChar
            | TypeSpecifier::Short
            | TypeSpecifier::Int
            | TypeSpecifier::Long
            | TypeSpecifier::LongLong
            | TypeSpecifier::Float
            | TypeSpecifier::Double
            | TypeSpecifier::LongDouble
            | TypeSpecifier::Float128
            | TypeSpecifier::Decimal32
            | TypeSpecifier::Decimal64
            | TypeSpecifier::Decimal128
            | TypeSpecifier::Signed
            | TypeSpecifier::Unsigned
            | TypeSpecifier::UnsignedChar
            | TypeSpecifier::PlainUnsignedChar
            | TypeSpecifier::UnsignedShort
            | TypeSpecifier::UnsignedInt
            | TypeSpecifier::UnsignedLong
            | TypeSpecifier::UnsignedLongLong
            | TypeSpecifier::Int128
            | TypeSpecifier::UnsignedInt128
            | TypeSpecifier::Bool
            | TypeSpecifier::ComplexFloat
            | TypeSpecifier::ComplexDouble
            | TypeSpecifier::ComplexLongDouble
            | TypeSpecifier::TypedefName(_)
            | TypeSpecifier::AutoType
            | TypeSpecifier::NullPtrT
            | TypeSpecifier::Struct(_, None, ..)
            | TypeSpecifier::Union(_, None, ..)
            | TypeSpecifier::Enum(_, None, _, None) => {}
            TypeSpecifier::Enum(_, None, _, Some(underlying)) => underlying.for_each_expr(f),
            TypeSpecifier::Struct(_, Some(fields), ..)
            | TypeSpecifier::Union(_, Some(fields), ..) => {
                for field in fields {
                    field.type_spec.for_each_expr(f);
                    for derived in &field.derived {
                        derived.for_each_expr(f);
                    }
                    if let Some(width) = &field.bit_width {
                        f(width);
                    }
                }
            }
            TypeSpecifier::Enum(_, Some(variants), _, underlying) => {
                if let Some(underlying) = underlying {
                    underlying.for_each_expr(f);
                }
                for value in variants.iter().filter_map(|v| v.value.as_deref()) {
                    f(value);
                }
            }
            TypeSpecifier::Pointer(inner, _)
            | TypeSpecifier::TypeofType(inner)
            | TypeSpecifier::Vector(inner, _)
            | TypeSpecifier::ExtVector(inner, _) => inner.for_each_expr(f),
            TypeSpecifier::Array(element, bound) => {
                element.for_each_expr(f);
                if let Some(bound) = bound {
                    f(bound);
                }
            }
            TypeSpecifier::FunctionPointer(ret, params, _)
            | TypeSpecifier::BareFunction(ret, params, _) => {
                ret.for_each_expr(f);
                for param in params {
                    param.for_each_type_expr(f);
                }
            }
            TypeSpecifier::Typeof(operand, _) => f(operand),
        }
    }
}

impl DerivedDeclarator {
    /// Calls `f` on each expression in this declarator part: an array
    /// bound, or the expressions of its parameter types.
    pub fn for_each_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        match self {
            DerivedDeclarator::Pointer(_) | DerivedDeclarator::Array(None) => {}
            DerivedDeclarator::Array(Some(bound)) => f(bound),
            DerivedDeclarator::Function(params, _)
            | DerivedDeclarator::FunctionPointer(params, _) => {
                for param in params {
                    param.for_each_type_expr(f);
                }
            }
        }
    }
}

impl ParamDecl {
    /// Calls `f` on each expression in this parameter's type, including a
    /// `_Alignas(type-name)` and a function-pointer parameter's own
    /// parameter types.
    pub fn for_each_type_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        self.type_spec.for_each_expr(f);
        if let Some(ts) = &self.alignas_type {
            ts.for_each_expr(f);
        }
        for param in self.fptr_params.iter().flatten() {
            param.for_each_type_expr(f);
        }
    }
}

impl Initializer {
    /// Calls `f` on each expression of this initializer: element values and
    /// designator indices, in source order.
    pub fn for_each_expr<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        match self {
            Initializer::Expr(e) => f(e),
            Initializer::List(items) => {
                for item in items {
                    for designator in &item.designators {
                        match designator {
                            Designator::Index(e) => f(e),
                            Designator::Range(lo, hi) => {
                                f(lo);
                                f(hi);
                            }
                            Designator::Field(_) => {}
                        }
                    }
                    item.init.for_each_expr(f);
                }
            }
        }
    }
}

impl Expr {
    /// Calls `f` on each direct sub-expression, in source order: operands,
    /// the callee and arguments of a call, a compound literal's initializer
    /// expressions, a `sizeof` operand, `_Generic`'s controlling expression
    /// and association results. A statement expression's statements are not
    /// visited: they are statements of their own. The match is exhaustive on
    /// purpose -- a new variant must decide what its children are.
    pub fn for_each_child<'a>(&'a self, f: &mut dyn FnMut(&'a Expr)) {
        match self {
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
            | Expr::ImaginaryLiteral(..)
            | Expr::ImaginaryLiteralF32(..)
            | Expr::ImaginaryLiteralLongDouble(..)
            | Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..)
            | Expr::CharLiteral(..)
            | Expr::Identifier(..)
            | Expr::StmtExpr(..)
            | Expr::Alignof(..)
            | Expr::GnuAlignof(..)
            | Expr::TypeOperand(..)
            | Expr::LabelAddr(..)
            | Expr::BuiltinTypesCompatibleP(..) => {}
            Expr::BinaryOp(_, lhs, rhs, _)
            | Expr::Assign(lhs, rhs, _)
            | Expr::CompoundAssign(_, lhs, rhs, _)
            | Expr::ArraySubscript(lhs, rhs, _)
            | Expr::Comma(lhs, rhs, _) => {
                f(lhs);
                f(rhs);
            }
            Expr::UnaryOp(_, e, _)
            | Expr::PostfixOp(_, e, _)
            | Expr::MemberAccess(e, ..)
            | Expr::PointerMemberAccess(e, ..)
            | Expr::Cast(_, e, ..)
            | Expr::VaArg(e, ..)
            | Expr::ConvertVector(e, ..)
            | Expr::AlignofExpr(e, _)
            | Expr::GnuAlignofExpr(e, _)
            | Expr::AddressOf(e, _)
            | Expr::Deref(e, _) => f(e),
            Expr::Conditional(cond, then_e, else_e, _) => {
                f(cond);
                f(then_e);
                f(else_e);
            }
            Expr::GnuConditional(cond, else_e, _) => {
                f(cond);
                f(else_e);
            }
            Expr::FunctionCall(callee, args, _) => {
                f(callee);
                args.iter().for_each(&mut *f);
            }
            Expr::CompoundLiteral(_, init, _) => init.for_each_expr(f),
            Expr::Sizeof(arg, _) => {
                if let SizeofArg::Expr(e) = arg.as_ref() {
                    f(e);
                }
            }
            Expr::GenericSelection(controlling, associations, _) => {
                f(controlling);
                associations.iter().for_each(|a| f(&a.expr));
            }
        }
    }

    /// Calls `f` on each type-name this node itself is written with: a cast,
    /// compound literal, `va_arg`, `__builtin_convertvector`, `_Alignof` or
    /// `sizeof` type operand, both `__builtin_types_compatible_p` operands,
    /// and `_Generic` association types. Exhaustive on purpose.
    pub fn for_each_type_name<'a>(&'a self, f: &mut dyn FnMut(&'a TypeSpecifier)) {
        match self {
            Expr::Cast(ts, ..)
            | Expr::CompoundLiteral(ts, ..)
            | Expr::VaArg(_, ts, _)
            | Expr::ConvertVector(_, ts, _)
            | Expr::Alignof(ts, _)
            | Expr::GnuAlignof(ts, _) => f(ts),
            Expr::Sizeof(arg, _) => {
                if let SizeofArg::Type(ts) = arg.as_ref() {
                    f(ts);
                }
            }
            Expr::BuiltinTypesCompatibleP(first, second, ..) => {
                f(first);
                f(second);
            }
            Expr::TypeOperand(ts, _, _) => f(ts),
            Expr::GenericSelection(_, associations, _) => {
                associations
                    .iter()
                    .filter_map(|a| a.type_spec.as_ref())
                    .for_each(f);
            }
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
            | Expr::ImaginaryLiteral(..)
            | Expr::ImaginaryLiteralF32(..)
            | Expr::ImaginaryLiteralLongDouble(..)
            | Expr::StringLiteral(..)
            | Expr::WideStringLiteral(..)
            | Expr::Char16StringLiteral(..)
            | Expr::CharLiteral(..)
            | Expr::Identifier(..)
            | Expr::StmtExpr(..)
            | Expr::LabelAddr(..)
            | Expr::BinaryOp(..)
            | Expr::Assign(..)
            | Expr::CompoundAssign(..)
            | Expr::ArraySubscript(..)
            | Expr::Comma(..)
            | Expr::UnaryOp(..)
            | Expr::PostfixOp(..)
            | Expr::MemberAccess(..)
            | Expr::PointerMemberAccess(..)
            | Expr::AlignofExpr(..)
            | Expr::GnuAlignofExpr(..)
            | Expr::AddressOf(..)
            | Expr::Deref(..)
            | Expr::Conditional(..)
            | Expr::GnuConditional(..)
            | Expr::FunctionCall(..) => {}
        }
    }

    /// Whether evaluating this expression may have a side effect: a call,
    /// assignment, increment or decrement, `va_arg`, or statement
    /// expression anywhere in it. Syntactic and conservative (a call to a
    /// pure function counts); volatile reads are not considered.
    pub fn may_have_side_effects(&self) -> bool {
        let here = match self {
            Expr::FunctionCall(..)
            | Expr::Assign(..)
            | Expr::CompoundAssign(..)
            | Expr::PostfixOp(..)
            | Expr::VaArg(..)
            | Expr::StmtExpr(..) => true,
            Expr::UnaryOp(op, ..) => matches!(op, UnaryOp::PreInc | UnaryOp::PreDec),
            _ => false,
        };
        let mut found = here;
        if !found {
            self.for_each_child(&mut |child| found = found || child.may_have_side_effects());
        }
        found
    }

    /// Returns a type-safe identity key for this expression node.
    /// Used as a HashMap key for caching expression types and constant values.
    #[inline]
    pub fn id(&self) -> ExprId {
        ExprId::of(self)
    }

    pub fn span(&self) -> Span {
        match self {
            Expr::IntLiteral(_, s)
            | Expr::UIntLiteral(_, s)
            | Expr::LongLiteral(_, s)
            | Expr::ULongLiteral(_, s)
            | Expr::LongLongLiteral(_, s)
            | Expr::ULongLongLiteral(_, s)
            | Expr::FloatLiteral(_, s)
            | Expr::FloatLiteralF32(_, s)
            | Expr::FloatLiteralLongDouble(_, _, s)
            | Expr::FloatLiteralF128(_, _, s)
            | Expr::FloatLiteralDecimal(_, _, s)
            | Expr::ImaginaryLiteral(_, s)
            | Expr::ImaginaryLiteralF32(_, s)
            | Expr::ImaginaryLiteralLongDouble(_, _, s)
            | Expr::StringLiteral(_, s)
            | Expr::WideStringLiteral(_, s)
            | Expr::Char16StringLiteral(_, s)
            | Expr::CharLiteral(_, s)
            | Expr::Identifier(_, s)
            | Expr::BinaryOp(_, _, _, s)
            | Expr::UnaryOp(_, _, s)
            | Expr::PostfixOp(_, _, s)
            | Expr::Assign(_, _, s)
            | Expr::CompoundAssign(_, _, _, s)
            | Expr::Conditional(_, _, _, s)
            | Expr::GnuConditional(_, _, s)
            | Expr::FunctionCall(_, _, s)
            | Expr::ArraySubscript(_, _, s)
            | Expr::MemberAccess(_, _, s)
            | Expr::PointerMemberAccess(_, _, s)
            | Expr::Cast(_, _, s, _)
            | Expr::CompoundLiteral(_, _, s)
            | Expr::StmtExpr(_, s)
            | Expr::Sizeof(_, s)
            | Expr::VaArg(_, _, s)
            | Expr::ConvertVector(_, _, s)
            | Expr::Alignof(_, s)
            | Expr::AlignofExpr(_, s)
            | Expr::GnuAlignof(_, s)
            | Expr::GnuAlignofExpr(_, s)
            | Expr::Comma(_, _, s)
            | Expr::AddressOf(_, s)
            | Expr::Deref(_, s)
            | Expr::GenericSelection(_, _, s)
            | Expr::TypeOperand(_, _, s)
            | Expr::LabelAddr(_, s)
            | Expr::BuiltinTypesCompatibleP(_, _, _, _, s) => *s,
        }
    }
}

/// Whether `name` is a predefined identifier naming the enclosing function's
/// name string: C's `__func__` and GCC's `__FUNCTION__` and (in C, the same
/// string) `__PRETTY_FUNCTION__`.
pub fn is_function_name_builtin(name: &str) -> bool {
    matches!(name, "__func__" | "__FUNCTION__" | "__PRETTY_FUNCTION__")
}

/// The string a function-name identifier designates: the enclosing
/// function's name (C11 6.4.2.2p1; GCC's C `__PRETTY_FUNCTION__` is the bare
/// name too), or, at file scope, where C defines none, GCC 16.2's: `""` for
/// `__func__` (with a warning) and `__FUNCTION__`, `"top level"` for
/// `__PRETTY_FUNCTION__` (Clang 23.1.3 agrees on all three).
pub fn function_name_builtin_text<'a>(name: &str, function: Option<&'a str>) -> &'a str {
    match function {
        Some(f) => f,
        None if name == "__PRETTY_FUNCTION__" => "top level",
        None => "",
    }
}

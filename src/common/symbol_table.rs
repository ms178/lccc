use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{AddressSpace, CType};

/// Information about a declared symbol.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub ty: CType,
    /// Explicit alignment from _Alignas or __attribute__((aligned(N))).
    /// Used by _Alignof(var) to return the correct alignment per C11 6.2.8p3.
    pub explicit_alignment: Option<usize>,
    /// Alignment of the declared type when the declaration's type specifier
    /// is a typedef carrying `__attribute__((aligned(N)))` — it REPLACES the
    /// natural alignment (may lower it), so `_Alignof(var)` reports it (GCC
    /// `DECL_ALIGN`). Mirrors `VarInfo::type_align` in the lowering.
    pub type_align: Option<usize>,
    /// Named address space of the *object* this symbol designates
    /// (`__seg_gs`/`__seg_fs`, the kernel's `__percpu`).
    ///
    /// The space is a property of the storage, not of the declared type, so a
    /// non-pointer object has nowhere to carry it in `ty` (and for a pointer
    /// object it would be ambiguous with the pointee's space). GCC keeps it on
    /// the object: for `static __seg_gs T v;` the type of `&v` is
    /// `__seg_gs T *`, and for `static __seg_gs T *p;` the type of `&p` is
    /// `__seg_gs T **` (measured with gcc 14.2: `T **b = &p;` is rejected
    /// while `T __seg_gs **c = &p;` is accepted). `&v` must reproduce that, or
    /// a per-CPU object cannot be passed to the `__percpu`-qualified
    /// parameter it belongs to (kernel/softirq.c: `&tasklet_vec`). Lowering
    /// already records the same fact (`VarInfo::address_space`); this field
    /// is what lets sema agree with it.
    pub address_space: AddressSpace,
}

/// A scope in the symbol table.
#[derive(Debug)]
struct Scope {
    symbols: FxHashMap<String, Symbol>,
    /// Block-scope names declared `extern` in this scope: they denote an
    /// object with linkage (C11 6.2.2p4), like every file-scope object.
    linked: FxHashSet<String>,
    /// Block-scope or parameter objects without linkage and without `static`
    /// (C11 6.2.4p5): their address is not a link-time constant.
    automatic: FxHashSet<String>,
    /// Const-qualified objects whose initializer is a constant expression.
    /// GCC folds their value in constant contexts (`const int c = 5;
    /// static int v = c;` is accepted); their address is still not constant
    /// unless the object has static storage.
    folded: FxHashSet<String>,
    /// The value of a folded const object, converted to its type, so that
    /// `int a = cc;` can be evaluated for `const int cc = 5;`.
    folded_values: FxHashMap<String, FoldedValue>,
    /// Enumeration constants declared in this scope: a block-scope object of
    /// the same name hides one (`enum { A = 1 }; void f(void) { int A; }`).
    enumerators: FxHashSet<String>,
}

/// The value of a folded const object (see `Scope::folded_values`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FoldedValue {
    /// An integer object, converted to its type and sign-extended to 64 bits.
    Int(i64),
    /// A `float` or `double` object, rounded to its type (a `float` value is
    /// exactly representable as an `f64`).
    Float(f64),
}

impl Scope {
    fn new() -> Self {
        Self {
            symbols: FxHashMap::default(),
            linked: FxHashSet::default(),
            automatic: FxHashSet::default(),
            folded: FxHashSet::default(),
            folded_values: FxHashMap::default(),
            enumerators: FxHashSet::default(),
        }
    }
}

/// Scoped symbol table supporting nested lexical scopes.
#[derive(Debug)]
pub struct SymbolTable {
    scopes: Vec<Scope>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new()],
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(Scope::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn declare(&mut self, symbol: Symbol) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.symbols.insert(symbol.name.clone(), symbol);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.lookup_ordinary(name).map(|(symbol, _)| symbol)
    }

    /// The innermost visible declaration of `name` and whether it is an
    /// enumeration constant. Enumerators share C's ordinary-identifier
    /// namespace with objects, functions and typedefs (C11 6.2.3), so the
    /// innermost declaration decides which one a use denotes; sema keeps the
    /// enumerator's value in `TypeContext::enum_constants`, and this flag is
    /// what selects it. One scope walk, and no invariant tying this table's
    /// scope depth to another structure's.
    pub fn lookup_ordinary(&self, name: &str) -> Option<(&Symbol, bool)> {
        self.scopes.iter().rev().find_map(|scope| {
            scope
                .symbols
                .get(name)
                .map(|symbol| (symbol, scope.enumerators.contains(name)))
        })
    }

    /// Look up a declaration in this scope only. A nested function definition
    /// may refine its own forward declaration, not an outer function it hides.
    pub fn lookup_current(&self, name: &str) -> Option<&Symbol> {
        self.scopes.last()?.symbols.get(name)
    }

    /// Whether declarations are currently at file scope.
    pub fn at_file_scope(&self) -> bool {
        self.scopes.len() <= 1
    }

    /// Record that the innermost scope's `name` was declared `extern` and so
    /// denotes an object with linkage.
    pub fn mark_linked(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.linked.insert(name.to_string());
        }
    }

    /// Record that the innermost scope's `name` is an automatic object.
    pub fn mark_automatic(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.automatic.insert(name.to_string());
        }
    }

    /// Record that the innermost scope's const-qualified `name` has a
    /// constant initializer, so its value may be folded (see `Scope::folded`).
    /// `value` is the converted value of an integer object, when it is known.
    pub fn mark_folded(&mut self, name: &str, value: Option<FoldedValue>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.folded.insert(name.to_string());
            if let Some(v) = value {
                scope.folded_values.insert(name.to_string(), v);
            }
        }
    }

    /// Record that the innermost scope's `name` is an enumeration constant.
    pub fn mark_enumerator(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.enumerators.insert(name.to_string());
        }
    }

    /// Whether the innermost visible declaration of `name` is an enumeration
    /// constant, i.e. no object, function or typedef of that name hides it.
    pub fn is_enumerator(&self, name: &str) -> bool {
        self.lookup_ordinary(name)
            .is_some_and(|(_, enumerator)| enumerator)
    }

    /// Whether the innermost visible declaration of `name` is an automatic
    /// object. Enumerators, functions and typedefs are never recorded, so
    /// they answer `false`.
    pub fn is_automatic_object(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .find(|scope| scope.symbols.contains_key(name))
            .is_some_and(|scope| scope.automatic.contains(name))
    }

    /// Whether the innermost visible declaration of `name` is a const object
    /// whose value folds (see `mark_folded`).
    pub fn is_folded(&self, name: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .find(|scope| scope.symbols.contains_key(name))
            .is_some_and(|scope| scope.folded.contains(name))
    }

    /// The folded value of the innermost visible declaration of `name`, if it
    /// is a folded integer const object (see `mark_folded`).
    pub fn folded_value(&self, name: &str) -> Option<FoldedValue> {
        self.scopes
            .iter()
            .rev()
            .find(|scope| scope.symbols.contains_key(name))
            .and_then(|scope| scope.folded_values.get(name).copied())
    }

    /// The innermost visible declaration of `name`, but only if it denotes an
    /// object with linkage: a file-scope declaration, or a block-scope one
    /// marked by `mark_linked`.  A visible block-scope object without linkage
    /// (an `auto`/`static` local) yields `None`: a new `extern` declaration
    /// then refers to a different object (C11 6.2.2p4) and does not
    /// inherit its type (6.2.7p4).
    pub fn lookup_linked(&self, name: &str) -> Option<&Symbol> {
        for (depth, scope) in self.scopes.iter().enumerate().rev() {
            if let Some(sym) = scope.symbols.get(name) {
                return (depth == 0 || scope.linked.contains(name)).then_some(sym);
            }
        }
        None
    }

    /// The depth of the scope that binds `name`, or `None` when nothing does.
    /// Depth 0 is file scope.
    ///
    /// `lookup` reports *what* the name means but not *where* it was bound, and
    /// C11 6.2.1p4 makes those two different questions: a block-scope
    /// declaration of the same basename denotes a different entity, so a caller
    /// that must not answer a scoped query from the file-scope record needs the
    /// depth.  Keep it a distinct query rather than folding it into `lookup`:
    /// every other consumer wants the symbol, not its scope.
    pub fn lookup_depth(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(depth, scope)| scope.symbols.contains_key(name).then_some(depth))
    }

    /// Look up only file-scope storage. Constant-expression queries need to
    /// distinguish a global object (which cannot become a compile-time value
    /// through caller specialization) from a parameter/local that may become
    /// constant after inlining.
    pub fn lookup_global(&self, name: &str) -> Option<&Symbol> {
        self.scopes.first()?.symbols.get(name)
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod binding_depth_tests {
    use super::{Symbol, SymbolTable};
    use crate::common::types::{AddressSpace, CType, FunctionType};

    fn fn_sym(name: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            ty: CType::Function(Box::new(FunctionType {
                return_type: CType::Int,
                params: Vec::new(),
                variadic: false,
            })),
            explicit_alignment: None,
            type_align: None,
            // A test that builds a `Symbol` by literal has to spell every field, and that is
            // the point: this fixture is about *bindings*, so the address space
            // it names is `Default`, written rather than defaulted-with-`..`.
            address_space: AddressSpace::Default,
        }
    }

    /// `lookup_depth` is what lets a scoped query refuse the file-scope record.
    /// Depth 0 is file scope; a shadowed name reports a positive depth even
    /// though `lookup` returns the same *shape* of symbol either way.
    #[test]
    fn depth_distinguishes_the_file_binding_from_a_shadow() {
        let mut table = SymbolTable::new();
        assert_eq!(table.lookup_depth("f"), None, "nothing declared yet");
        table.declare(fn_sym("f"));
        assert_eq!(table.lookup_depth("f"), Some(0), "file scope is depth 0");
        table.push_scope();
        assert_eq!(
            table.lookup_depth("f"),
            Some(0),
            "an inner scope that binds nothing keeps the file binding"
        );
        table.declare(fn_sym("f"));
        assert_eq!(
            table.lookup_depth("f"),
            Some(1),
            "a block-scope declaration of the same basename is a distinct entity"
        );
        table.pop_scope();
        assert_eq!(
            table.lookup_depth("f"),
            Some(0),
            "and it stops at the scope end"
        );
    }
}

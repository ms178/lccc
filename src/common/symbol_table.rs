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
}

impl Scope {
    fn new() -> Self {
        Self {
            symbols: FxHashMap::default(),
            linked: FxHashSet::default(),
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
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.symbols.get(name) {
                return Some(sym);
            }
        }
        None
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

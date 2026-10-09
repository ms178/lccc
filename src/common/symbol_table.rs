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
    /// Segment space of the OBJECT itself (`__seg_gs struct s x`, the kernel's
    /// `DEFINE_PER_CPU` objects), so that `&x` is typed `T __seg_gs *`. The
    /// space a pointer points INTO lives on its `CType::Pointer`, not here.
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

//! Per-function build state for IR lowering.
//!
//! `FunctionBuildState` holds all state that is created fresh for each function
//! being lowered and discarded afterward. This makes the function-vs-module
//! lifecycle boundary explicit: the Lowerer wraps this in `Option<FunctionBuildState>`,
//! which is `None` between functions.
//!
//! Scope management uses an undo-log pattern (`FuncScopeFrame`) rather than
//! cloning entire HashMaps at scope boundaries. On scope exit, only the changes
//! made within that scope are undone, giving O(changes) cost instead of O(total).

use super::definitions::{LocalInfo, SwitchFrame};
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::{CType, IrType};
use crate::ir::reexports::{BasicBlock, BlockId, Instruction, Value};

/// Records undo operations for function-local scoped variables.
///
/// Pushed on scope entry, popped on scope exit to restore previous state.
/// Tracks both newly-added keys (removed on undo) and shadowed keys
/// (restored to previous value on undo).
#[derive(Debug)]
pub(super) struct FuncScopeFrame {
    /// Keys that were newly inserted into `locals` (not present before scope entry).
    pub locals_added: Vec<String>,
    /// Keys that were overwritten in `locals`: (key, previous_value).
    pub locals_shadowed: Vec<(String, LocalInfo)>,
    /// Keys newly inserted into `static_local_names`.
    pub statics_added: Vec<String>,
    /// Keys that were overwritten in `static_local_names`: (key, previous_value).
    pub statics_shadowed: Vec<(String, String)>,
    /// Keys newly inserted into `const_local_values`.
    pub consts_added: Vec<String>,
    /// Keys that were overwritten in `const_local_values`: (key, previous_value).
    pub consts_shadowed: Vec<(String, i64)>,
    /// Keys newly inserted into `var_ctypes`.
    pub var_ctypes_added: Vec<String>,
    /// Keys that were overwritten in `var_ctypes`: (key, previous_value).
    pub var_ctypes_shadowed: Vec<(String, CType)>,
    /// Keys newly inserted into `vla_typedef_sizes`.
    pub vla_typedef_sizes_added: Vec<String>,
    /// Keys that were overwritten in `vla_typedef_sizes`: (key, previous_value).
    pub vla_typedef_sizes_shadowed: Vec<(String, Value)>,
    /// Keys newly inserted into `vla_field_strides`.
    pub vla_field_strides_added: Vec<String>,
    /// Keys that were overwritten in `vla_field_strides`: (key, previous_value).
    pub vla_field_strides_shadowed: Vec<(String, Vec<Value>)>,
    /// Keys newly inserted into `vla_field_offsets`.
    pub vla_field_offsets_added: Vec<String>,
    /// Keys that were overwritten in `vla_field_offsets`: (key, previous_value).
    pub vla_field_offsets_shadowed: Vec<(String, Value)>,
    /// Nested-function names newly declared in this scope.
    pub nested_fns_added: Vec<String>,
    /// Nested-function names shadowed by this scope: (name, previous entry).
    pub nested_fns_shadowed: Vec<(String, NestedFnEntry)>,
    /// Saved stack pointer before the first VLA in this scope.
    /// When set, StackRestore is emitted at scope exit to reclaim VLA stack space.
    pub scope_stack_save: Option<Value>,
    /// Variables with __attribute__((cleanup(func))) in this scope.
    /// Stored in declaration order; cleanup calls are emitted in reverse order at scope exit.
    /// Each entry is (cleanup_function_name, alloca_value) where alloca_value is the
    /// address of the variable to pass as &var to the cleanup function.
    pub cleanup_vars: Vec<(String, Value)>,
}

impl FuncScopeFrame {
    fn new() -> Self {
        Self {
            locals_added: Vec::new(),
            locals_shadowed: Vec::new(),
            statics_added: Vec::new(),
            statics_shadowed: Vec::new(),
            consts_added: Vec::new(),
            consts_shadowed: Vec::new(),
            var_ctypes_added: Vec::new(),
            var_ctypes_shadowed: Vec::new(),
            vla_typedef_sizes_added: Vec::new(),
            vla_typedef_sizes_shadowed: Vec::new(),
            vla_field_strides_added: Vec::new(),
            vla_field_strides_shadowed: Vec::new(),
            vla_field_offsets_added: Vec::new(),
            vla_field_offsets_shadowed: Vec::new(),
            nested_fns_added: Vec::new(),
            nested_fns_shadowed: Vec::new(),
            scope_stack_save: None,
            cleanup_vars: Vec::new(),
        }
    }
}

/// Per-function build state, extracted from Lowerer to make the function-vs-module
/// state boundary explicit. Created fresh at the start of each function, replacing
/// the old pattern of clearing individual fields.
#[derive(Debug)]
pub(super) struct FunctionBuildState {
    /// Basic blocks accumulated for the current function
    pub blocks: Vec<BasicBlock>,
    /// Instructions for the current basic block being built
    pub instrs: Vec<Instruction>,
    /// Label of the current basic block
    pub current_label: BlockId,
    /// Name of the function currently being lowered
    pub name: String,
    /// Return type of the function currently being lowered
    pub return_type: IrType,
    /// Whether the current function returns _Bool
    pub return_is_bool: bool,
    /// C type of the current function's return value (for _Float128 routing).
    pub return_ctype: Option<CType>,
    /// sret pointer alloca for current function (struct returns > 16 bytes)
    pub sret_ptr: Option<Value>,
    /// Variable -> alloca mapping with metadata
    pub locals: FxHashMap<String, LocalInfo>,
    /// Loop context: (label, scope_depth) to jump to on `break`.
    /// scope_depth records the scope_stack length when the loop was entered,
    /// so break can emit cleanup calls for scopes being exited.
    pub break_labels: Vec<(BlockId, usize)>,
    /// Loop context: (label, scope_depth) to jump to on `continue`.
    /// scope_depth records the scope_stack length when the loop was entered,
    /// so continue can emit cleanup calls for scopes being exited.
    pub continue_labels: Vec<(BlockId, usize)>,
    /// Stack of switch statement contexts
    pub switch_stack: Vec<SwitchFrame>,
    /// User-defined goto labels -> unique IR labels
    pub user_labels: FxHashMap<String, BlockId>,
    /// Set of user-defined goto labels that have been defined (label statement lowered).
    /// Used to distinguish forward gotos (label not yet defined) from backward gotos
    /// (label already defined) for VLA stack restore decisions.
    pub defined_user_labels: FxHashSet<String>,
    /// User-defined goto labels -> scope depth at label definition site.
    /// Populated by a prescan of the function body before lowering, so that
    /// `goto` cleanup emission can determine which scopes are actually exited.
    pub user_label_depths: FxHashMap<String, usize>,
    /// Scope stack for function-local variable undo tracking
    pub scope_stack: Vec<FuncScopeFrame>,
    /// Static local variable name -> mangled global name
    pub static_local_names: FxHashMap<String, String>,
    /// Const-qualified local variable values
    pub const_local_values: FxHashMap<String, i64>,
    /// CType for each local variable
    pub var_ctypes: FxHashMap<String, CType>,
    /// Runtime sizeof Values for VLA typedef types (e.g., `typedef char buf[n][m]`).
    /// Keyed by typedef name, value is the IR Value holding the runtime byte size.
    pub vla_typedef_sizes: FxHashMap<String, Value>,
    /// Dynamic dimension strides for VLA fields in struct types defined inside functions.
    /// Keyed by struct type key (e.g. tag or type name), value is the vector of dimension stride values.
    pub vla_field_strides: FxHashMap<String, Vec<Value>>,
    /// Dynamic field byte offsets for fields following a VLA member in local structs.
    /// Keyed by "struct_key::field_name", value is the IR Value holding the byte offset.
    pub vla_field_offsets: FxHashMap<String, Value>,
    /// Per-function value counter (reset for each function)
    pub next_value: u32,
    /// Current statement root for FP expression tagging (OP-36): bumped by
    /// `lower_stmt` per statement; every FP Mul/Add/Sub result is tagged
    /// with it so the backend's OnExpr contraction gate can fuse only
    /// within one source expression.
    pub fp_expr_root: u64,
    /// Saved stack pointer Value for VLA deallocation.
    /// When a function contains VLA declarations and gotos, the stack pointer is saved
    /// at function entry so it can be restored before backward jumps that cross VLA scopes.
    pub vla_stack_save: Option<Value>,
    /// Whether the function has any VLA declarations.
    /// Used to decide whether to emit StackSave/StackRestore around gotos.
    pub has_vla: bool,
    /// Alloca instructions deferred to the entry block.
    /// Local variable allocas are collected here during lowering so that
    /// variables whose declarations are skipped by `goto` still have valid
    /// stack slots at runtime. Merged into blocks[0] in finalize_function.
    pub entry_allocas: Vec<Instruction>,
    /// Values corresponding to the allocas created for function parameters.
    /// Populated during allocate_function_params and transferred to IrFunction
    /// in finalize_function. Used by the backend to detect dead param allocas.
    pub param_alloca_values: Vec<Value>,
    /// Source location spans for the current basic block being built,
    /// parallel to `instrs`. Each entry corresponds to one instruction.
    pub instr_spans: Vec<Span>,
    /// The current source span, set before lowering each statement/expression.
    /// Emitted instructions inherit this span for debug info tracking.
    pub current_span: Span,
    /// Whether this function is a candidate for inlining (always_inline, inline, or static).
    /// Used by __builtin_constant_p lowering: in non-inline-candidate functions, non-constant
    /// expressions resolve immediately to 0. In inline candidates, they emit IsConstant
    /// instructions that can be resolved to 1 after inlining if the argument becomes constant.
    pub is_inline_candidate: bool,
    /// Block IDs referenced by static local variable initializers via &&label.
    /// Transferred to IrFunction in finalize_function so CFG simplify can keep
    /// these blocks reachable (their labels appear in global data like .quad .LBB3).
    pub global_init_label_blocks: Vec<BlockId>,
    /// ── GNU C nested-function support ────────────────────────────────────
    /// Nested functions visible in the current function (by plain name),
    /// with scope-restore bookkeeping (added to the current scope frame).
    pub nested_functions: FxHashMap<String, NestedFnEntry>,
    /// Static-nesting depth: 0 for file-scope functions, +1 per nested level.
    pub nesting_depth: usize,
    /// Frame struct state: the frame alloca value (an entry-block alloca
    /// whose size is patched at finalize), its current size/alignment, the
    /// member layout cursor, and the reserved header words. The header is
    /// [link (if this function is itself nested)][rbp][rsp (if non-local
    /// gotos target this function)].
    pub frame: Option<FrameState>,
    /// The GetStaticChain dest (this function's incoming chain value),
    /// present when the function is nested AND uses (or forwards) the chain.
    pub chain_value: Option<Value>,
    /// Frame-member memoization: captured-local name -> byte offset in this
    /// function's frame struct. The FIRST nested function that captures a
    /// local moves it into the frame; later nested functions capturing the
    /// same local MUST reuse the identical offset (their bodies were lowered
    /// with that offset baked in, and re-framing would allocate a second,
    /// never-written member).
    pub frame_members: FxHashMap<String, i64>,
    /// Captured-variable access bookkeeping for THIS function's visible
    /// locals that live in an ANCESTOR's frame: alloca-value-id ->
    /// (frame-link hops, byte offset in the owning frame). Used to compute
    /// the access path when a deeper nested function captures the same
    /// variable again.
    pub chain_locals: FxHashMap<u32, (usize, i64)>,
    /// Non-local goto targets visible in this function: resolved label name
    /// -> (hops to the owning frame, rbp offset, rsp offset, label block).
    pub nonlocal_labels: FxHashMap<String, NonlocalGotoTarget>,
    /// Named-label aliases to emit AT a label statement in THIS function,
    /// for nested functions (non-local goto targets and &&label refs):
    /// resolved label name -> aliases. Consumed by lower_label_stmt, which
    /// emits each alias as an InlineAsm label at the label's block — a real
    /// instruction that survives block merging and label renumbering.
    pub pending_label_aliases: FxHashMap<String, Vec<String>>,
    /// &&label references to ENCLOSING function labels, resolved to named
    /// aliases: label name (as written) -> alias. Consulted by the
    /// LabelAddr lowering hook; empty in non-nested contexts.
    pub addr_label_aliases: FxHashMap<String, String>,
}

/// A nested function visible for call resolution.
#[derive(Debug, Clone)]
pub struct NestedFnEntry {
    /// IR symbol name (parent-mangled: "parent.name", possibly nested further).
    pub mangled: String,
    /// Static-nesting depth of the nested function (parent depth + 1).
    pub depth: usize,
    /// Whether the function reads its static chain (or forwards it for
    /// descendants). When false, callers skip SetStaticChain and
    /// address-taking needs no trampoline.
    pub uses_chain: bool,
}

/// Frame struct layout state for a function containing nested functions.
#[derive(Debug, Clone)]
pub struct FrameState {
    /// The frame alloca value (pointer to the frame struct).
    pub alloca: Value,
    /// Current total size in bytes (patched into the alloca at finalize).
    pub size: usize,
    /// Alignment of the frame (>= 8; 16 when any member needs it).
    pub align: usize,
    /// Whether the link word (offset 0) is present (function is nested).
    pub has_link: bool,
    /// Whether the non-local-goto save area (2 words after the link) is
    /// present, and at which offsets.
    pub goto_save: Option<(i64, i64)>,
    /// Member cursor: next free byte offset (after the header).
    pub cursor: i64,
}

/// A non-local goto target reachable from the current function.
#[derive(Debug, Clone)]
pub struct NonlocalGotoTarget {
    /// Frame-link hops from THIS function's static chain to the frame that
    /// owns the save area.
    pub up: usize,
    pub rbp_off: i64,
    pub rsp_off: i64,
    /// Named assembly label emitted by the ENCLOSING function at the target
    /// block (block IDs are per-function; cross-function references need a
    /// stable name).
    pub label: String,
}

impl FunctionBuildState {
    /// Create a new function build state for the given function.
    pub fn new(name: String, return_type: IrType, return_is_bool: bool) -> Self {
        Self {
            blocks: Vec::new(),
            instrs: Vec::new(),
            current_label: BlockId(0),
            name,
            return_type,
            return_is_bool,
            return_ctype: None,
            sret_ptr: None,
            locals: FxHashMap::default(),
            break_labels: Vec::new(),
            continue_labels: Vec::new(),
            switch_stack: Vec::new(),
            user_labels: FxHashMap::default(),
            defined_user_labels: FxHashSet::default(),
            user_label_depths: FxHashMap::default(),
            scope_stack: Vec::new(),
            static_local_names: FxHashMap::default(),
            const_local_values: FxHashMap::default(),
            var_ctypes: FxHashMap::default(),
            vla_typedef_sizes: FxHashMap::default(),
            vla_field_strides: FxHashMap::default(),
            vla_field_offsets: FxHashMap::default(),
            next_value: 0,
            fp_expr_root: 0,
            vla_stack_save: None,
            has_vla: false,
            entry_allocas: Vec::new(),
            param_alloca_values: Vec::new(),
            instr_spans: Vec::new(),
            current_span: Span::dummy(),
            is_inline_candidate: false,
            global_init_label_blocks: Vec::new(),
            nested_functions: FxHashMap::default(),
            nesting_depth: 0,
            frame: None,
            chain_value: None,
            frame_members: FxHashMap::default(),
            chain_locals: FxHashMap::default(),
            nonlocal_labels: FxHashMap::default(),
            pending_label_aliases: FxHashMap::default(),
            addr_label_aliases: FxHashMap::default(),
        }
    }

    /// Push a new function-local scope frame.
    pub fn push_scope(&mut self) {
        self.scope_stack.push(FuncScopeFrame::new());
    }

    /// Declare a nested function in the CURRENT scope (with undo bookkeeping
    /// so pop_scope restores the previous binding).
    pub fn declare_nested_fn(&mut self, name: &str, entry: NestedFnEntry) {
        match self.scope_stack.last_mut() {
            Some(frame) => {
                if let Some(prev) = self.nested_functions.insert(name.to_string(), entry) {
                    frame.nested_fns_shadowed.push((name.to_string(), prev));
                } else {
                    frame.nested_fns_added.push(name.to_string());
                }
            }
            None => {
                // No scope frame (shouldn't happen: blocks with nested defs
                // always push one) — declare without undo tracking.
                self.nested_functions.insert(name.to_string(), entry);
            }
        }
    }

    /// Pop the top function-local scope frame and undo changes to locals,
    /// static_local_names, const_local_values, and var_ctypes.
    /// Returns (scope_stack_save, cleanup_vars) - the VLA save value and cleanup variables.
    pub fn pop_scope(&mut self) -> (Option<Value>, Vec<(String, Value)>) {
        if let Some(frame) = self.scope_stack.pop() {
            let scope_stack_save = frame.scope_stack_save;
            let cleanup_vars = frame.cleanup_vars;
            for key in frame.locals_added {
                self.locals.remove(&key);
            }
            for (key, val) in frame.locals_shadowed {
                self.locals.insert(key, val);
            }
            for key in frame.statics_added {
                self.static_local_names.remove(&key);
            }
            for (key, val) in frame.statics_shadowed {
                self.static_local_names.insert(key, val);
            }
            for key in frame.consts_added {
                self.const_local_values.remove(&key);
            }
            for (key, val) in frame.consts_shadowed {
                self.const_local_values.insert(key, val);
            }
            for key in frame.var_ctypes_added {
                self.var_ctypes.remove(&key);
            }
            for (key, val) in frame.var_ctypes_shadowed {
                self.var_ctypes.insert(key, val);
            }
            for key in frame.vla_typedef_sizes_added {
                self.vla_typedef_sizes.remove(&key);
            }
            for (key, val) in frame.vla_typedef_sizes_shadowed {
                self.vla_typedef_sizes.insert(key, val);
            }
            for key in frame.vla_field_strides_added {
                self.vla_field_strides.remove(&key);
            }
            for (key, val) in frame.vla_field_strides_shadowed {
                self.vla_field_strides.insert(key, val);
            }
            for key in frame.vla_field_offsets_added {
                self.vla_field_offsets.remove(&key);
            }
            for (key, val) in frame.vla_field_offsets_shadowed {
                self.vla_field_offsets.insert(key, val);
            }
            for key in frame.nested_fns_added {
                self.nested_functions.remove(&key);
            }
            for (key, val) in frame.nested_fns_shadowed {
                self.nested_functions.insert(key, val);
            }
            (scope_stack_save, cleanup_vars)
        } else {
            (None, Vec::new())
        }
    }

    /// Insert a VLA typedef runtime size, tracking for scope management.
    pub fn insert_vla_typedef_size_scoped(&mut self, name: String, size: Value) {
        if let Some(frame) = self.scope_stack.last_mut() {
            if let Some(prev) = self.vla_typedef_sizes.remove(&name) {
                frame.vla_typedef_sizes_shadowed.push((name.clone(), prev));
            } else {
                frame.vla_typedef_sizes_added.push(name.clone());
            }
        }
        self.vla_typedef_sizes.insert(name, size);
    }

    /// Insert VLA field strides for a struct type, tracking for scope management.
    pub fn insert_vla_field_strides_scoped(&mut self, key: String, strides: Vec<Value>) {
        if let Some(frame) = self.scope_stack.last_mut() {
            if let Some(prev) = self.vla_field_strides.remove(&key) {
                frame.vla_field_strides_shadowed.push((key.clone(), prev));
            } else {
                frame.vla_field_strides_added.push(key.clone());
            }
        }
        self.vla_field_strides.insert(key, strides);
    }

    /// Insert a VLA dynamic field offset for a struct type, tracking for scope management.
    pub fn insert_vla_field_offset_scoped(&mut self, key: String, offset: Value) {
        if let Some(frame) = self.scope_stack.last_mut() {
            if let Some(prev) = self.vla_field_offsets.remove(&key) {
                frame.vla_field_offsets_shadowed.push((key.clone(), prev));
            } else {
                frame.vla_field_offsets_added.push(key.clone());
            }
        }
        self.vla_field_offsets.insert(key, offset);
    }

    /// Insert a local variable, tracking the change in the current scope frame.
    pub fn insert_local_scoped(&mut self, name: String, info: LocalInfo) {
        if let Some(frame) = self.scope_stack.last_mut() {
            if let Some(prev) = self.locals.remove(&name) {
                frame.locals_shadowed.push((name.clone(), prev));
            } else {
                frame.locals_added.push(name.clone());
            }
        }
        self.locals.insert(name, info);
    }

    /// Insert a static local name, tracking the change in the current scope frame.
    pub fn insert_static_local_scoped(&mut self, name: String, mangled: String) {
        if let Some(frame) = self.scope_stack.last_mut() {
            if let Some(prev) = self.static_local_names.remove(&name) {
                frame.statics_shadowed.push((name.clone(), prev));
            } else {
                frame.statics_added.push(name.clone());
            }
        }
        self.static_local_names.insert(name, mangled);
    }

    /// Insert a const local value, tracking the change in the current scope frame.
    pub fn insert_const_local_scoped(&mut self, name: String, value: i64) {
        if let Some(frame) = self.scope_stack.last_mut() {
            if let Some(prev) = self.const_local_values.remove(&name) {
                frame.consts_shadowed.push((name.clone(), prev));
            } else {
                frame.consts_added.push(name.clone());
            }
        }
        self.const_local_values.insert(name, value);
    }

    /// Remove a local variable from `locals`, tracking the removal in the
    /// current scope frame so `pop_scope()` restores it.
    pub fn shadow_local_for_scope(&mut self, name: &str) {
        if let Some(prev_local) = self.locals.remove(name) {
            if let Some(frame) = self.scope_stack.last_mut() {
                frame.locals_shadowed.push((name.to_string(), prev_local));
            }
        }
    }

    /// Remove a static local name, tracking the removal in the current scope frame.
    pub fn shadow_static_for_scope(&mut self, name: &str) {
        if let Some(prev_static) = self.static_local_names.remove(name) {
            if let Some(frame) = self.scope_stack.last_mut() {
                frame.statics_shadowed.push((name.to_string(), prev_static));
            }
        }
    }
}

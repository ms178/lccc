//! Budgeted, flow-insensitive interprocedural points-to and mod/ref analysis.
//!
//! The solver is an inclusion-based (Andersen) constraint graph.  SSA values,
//! formal parameters, function returns and abstract-object contents are nodes;
//! copy edges propagate sorted points-to sets through a work queue.  Loads,
//! stores, memcpy and indirect calls install edges lazily as their pointer
//! sets grow.  This avoids both a whole-program constraint rebuild at every
//! iteration and the quadratic "union every set with every other set" shape.
//!
//! Precision is deliberately object-based rather than field-based: distinct
//! allocas, allocation sites, globals and functions are separate objects,
//! while GEP offsets are ignored.  That is a sound may-alias approximation.
//! Context-insensitive call edges bind actuals to formals and returns to call
//! results, including a dedicated variadic-argument node.  Weak/interposable
//! bodies are never used as the definition of a direct call.
//!
//! Escape and mod/ref are coupled.  Unknown code can modify globals and every
//! object whose address escaped, but not a caller's unescaped stack/heap
//! object.  Escaped objects also acquire unknown pointer contents, and pointer
//! values stored into escaped memory escape themselves.  The resulting call
//! summaries include both modification and reference sets.  A special `all`
//! effect is reserved for unmodelled IR (asm, fences, target pseudo-ops), which
//! may touch even otherwise unescaped objects.
//!
//! Every resource budget fails closed: a partial solution disables all
//! consumers rather than being presented as a proof of non-aliasing.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::module::GlobalInit;
use crate::ir::reexports::{
    CallInfo, Instruction, IrFunction, IrModule, Operand, Terminator, Value,
};
use std::collections::VecDeque;

const UNKNOWN: u32 = 0;
type Node = u32;
type Object = u32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ObjectSet(Vec<Object>);

impl ObjectSet {
    fn insert(&mut self, object: Object) -> bool {
        match self.0.binary_search(&object) {
            Ok(_) => false,
            Err(position) => {
                self.0.insert(position, object);
                true
            }
        }
    }

    fn union_with(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for &object in &other.0 {
            changed |= self.insert(object);
        }
        changed
    }

    fn contains(&self, object: Object) -> bool {
        self.0.binary_search(&object).is_ok()
    }
}

#[derive(Clone, Copy, Debug)]
enum ObjectKind {
    Unknown,
    Global,
    ExternalData,
    ConstantData,
    Alloca,
    Heap,
    Function(usize),
    ExternalFunction,
}

#[derive(Clone, Debug)]
struct AbstractObject {
    kind: ObjectKind,
    content: Node,
    escaped: bool,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum Watch {
    Load { dest: Node },
    Store { value: Node },
    MemcpyDestination { source_pointer: Node },
    MemcpySource { destination_pointer: Node },
    IndirectCall { site: usize },
}

#[derive(Debug)]
struct FunctionInfo {
    safe_body: bool,
    externally_visible: bool,
    variadic: bool,
    parameters: Vec<Node>,
    return_node: Node,
    varargs_node: Node,
}

#[derive(Debug)]
struct CallSite {
    caller: usize,
    arguments: Vec<Option<Node>>,
    destination: Option<Node>,
    targets: FxHashSet<usize>,
    external: bool,
    is_pure: bool,
    is_const: bool,
    is_sret: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AnalysisBudget {
    pub max_work: usize,
    pub max_edges: usize,
}

impl Default for AnalysisBudget {
    fn default() -> Self {
        Self {
            max_work: 4_000_000,
            max_edges: 1_000_000,
        }
    }
}

#[derive(Debug)]
struct Solver {
    points: Vec<ObjectSet>,
    edges: Vec<Vec<Node>>,
    edge_set: FxHashSet<(Node, Node)>,
    watchers: Vec<Vec<Watch>>,
    queue: VecDeque<(Node, Object)>,
    objects: Vec<AbstractObject>,
    escape_dependents: Vec<Vec<Node>>,
    escape_sources: FxHashSet<Node>,
    functions: Vec<FunctionInfo>,
    value_nodes: Vec<FxHashMap<u32, Node>>,
    function_indices: FxHashMap<String, usize>,
    named_objects: FxHashMap<String, Object>,
    call_sites: Vec<CallSite>,
    site_for_instruction: FxHashMap<(usize, usize, usize), usize>,
    unmodeled_instructions: FxHashSet<(usize, usize, usize)>,
    memory_pointer_nodes: Vec<Node>,
    work: usize,
    complete: bool,
    budget: AnalysisBudget,
}

impl Solver {
    fn new(function_count: usize, budget: AnalysisBudget) -> Self {
        let mut solver = Self {
            points: Vec::new(),
            edges: Vec::new(),
            edge_set: FxHashSet::default(),
            watchers: Vec::new(),
            queue: VecDeque::new(),
            objects: Vec::new(),
            escape_dependents: Vec::new(),
            escape_sources: FxHashSet::default(),
            functions: Vec::with_capacity(function_count),
            value_nodes: vec![FxHashMap::default(); function_count],
            function_indices: FxHashMap::default(),
            named_objects: FxHashMap::default(),
            call_sites: Vec::new(),
            site_for_instruction: FxHashMap::default(),
            unmodeled_instructions: FxHashSet::default(),
            memory_pointer_nodes: Vec::new(),
            work: 0,
            complete: true,
            budget,
        };
        let unknown = solver.add_object(ObjectKind::Unknown);
        debug_assert_eq!(unknown, UNKNOWN);
        solver.objects[UNKNOWN as usize].escaped = true;
        let content = solver.objects[UNKNOWN as usize].content;
        solver.insert(content, UNKNOWN);
        solver.escape_sources.insert(content);
        solver
    }

    fn charge(&mut self, amount: usize) {
        self.work = self.work.saturating_add(amount);
        if self.work > self.budget.max_work {
            self.complete = false;
            self.queue.clear();
        }
    }

    fn new_node(&mut self) -> Node {
        let node = self.points.len() as Node;
        self.points.push(ObjectSet::default());
        self.edges.push(Vec::new());
        self.watchers.push(Vec::new());
        node
    }

    fn add_object(&mut self, kind: ObjectKind) -> Object {
        let content = self.new_node();
        let object = self.objects.len() as Object;
        self.objects.push(AbstractObject {
            kind,
            content,
            escaped: false,
        });
        self.escape_dependents.push(Vec::new());
        object
    }

    fn insert(&mut self, node: Node, object: Object) {
        if !self.complete {
            return;
        }
        self.charge(1);
        if self.complete && self.points[node as usize].insert(object) {
            self.queue.push_back((node, object));
        }
    }

    fn add_edge(&mut self, source: Node, destination: Node) {
        if source == destination || !self.complete {
            return;
        }
        self.charge(1);
        if !self.edge_set.insert((source, destination)) {
            return;
        }
        if self.edge_set.len() > self.budget.max_edges {
            self.complete = false;
            self.queue.clear();
            return;
        }
        self.edges[source as usize].push(destination);
        for object in self.points[source as usize].0.clone() {
            self.insert(destination, object);
        }
    }

    fn add_watch(&mut self, pointer: Node, watch: Watch) {
        let watches = &mut self.watchers[pointer as usize];
        if !watches.contains(&watch) {
            watches.push(watch);
        }
    }

    fn value_node(&mut self, function: usize, value: u32) -> Node {
        if let Some(&node) = self.value_nodes[function].get(&value) {
            return node;
        }
        let node = self.new_node();
        self.value_nodes[function].insert(value, node);
        node
    }

    fn operand_node(&mut self, function: usize, operand: &Operand) -> Option<Node> {
        match operand {
            Operand::Value(value) => Some(self.value_node(function, value.0)),
            Operand::Const(_) => None,
        }
    }

    fn memory_pointer(&mut self, function: usize, value: Value) -> Node {
        let node = self.value_node(function, value.0);
        self.memory_pointer_nodes.push(node);
        node
    }

    fn activate_escape(&mut self, node: Node) {
        if self.escape_sources.insert(node) {
            for object in self.points[node as usize].0.clone() {
                self.mark_escaped(object);
            }
        }
    }

    fn mark_escaped(&mut self, object: Object) {
        if object == UNKNOWN
            || matches!(
                self.objects[object as usize].kind,
                ObjectKind::Function(_) | ObjectKind::ExternalFunction
            )
        {
            return;
        }
        if self.objects[object as usize].escaped {
            return;
        }
        self.charge(1);
        self.objects[object as usize].escaped = true;
        let content = self.objects[object as usize].content;
        // Unknown code may both read pointers from and write pointers into
        // escaped storage.  Both directions are required for soundness.
        self.insert(content, UNKNOWN);
        self.activate_escape(content);
        let unknown_content = self.objects[UNKNOWN as usize].content;
        // Collapse escaped *contents* with unknown memory in both
        // directions.  Merely putting the escaped object in unknown's
        // points-to set would lose the pointers stored inside it (and could
        // even mistake a data object for a function pointer on reload).
        self.add_edge(content, unknown_content);
        self.add_edge(unknown_content, content);
        for node in self.escape_dependents[object as usize].clone() {
            self.activate_escape(node);
        }
    }

    fn object_for_name(&mut self, name: &str) -> Object {
        if let Some(&object) = self.named_objects.get(name) {
            return object;
        }
        let object = self.add_object(ObjectKind::ExternalData);
        self.named_objects.insert(name.to_string(), object);
        self.mark_escaped(object);
        object
    }

    fn mark_external(&mut self, site: usize, unknown_target: bool) {
        self.mark_external_with_return(site, unknown_target, true);
    }

    fn mark_external_with_return(
        &mut self,
        site: usize,
        unknown_target: bool,
        unknown_return: bool,
    ) {
        self.call_sites[site].external = true;
        if unknown_target {
            self.call_sites[site].is_pure = false;
            self.call_sites[site].is_const = false;
        }
        let may_escape = !self.call_sites[site].is_pure && !self.call_sites[site].is_const;
        if may_escape {
            for argument in self.call_sites[site]
                .arguments
                .clone()
                .into_iter()
                .flatten()
            {
                self.activate_escape(argument);
            }
        }
        if unknown_return && let Some(destination) = self.call_sites[site].destination {
            self.insert(destination, UNKNOWN);
        }
    }

    fn bind_target(&mut self, site: usize, function: usize) {
        if !self.call_sites[site].targets.insert(function) {
            return;
        }
        let info = &self.functions[function];
        let safe = info.safe_body;
        let variadic = info.variadic;
        let parameters = info.parameters.clone();
        let return_node = info.return_node;
        let varargs_node = info.varargs_node;
        if !safe {
            self.mark_external(site, true);
            return;
        }
        let destination = self.call_sites[site].destination;
        let arguments = self.call_sites[site].arguments.clone();
        let argument_count = arguments.len();
        for (index, argument) in arguments.into_iter().enumerate() {
            let Some(argument) = argument else { continue };
            if index < parameters.len() {
                self.add_edge(argument, parameters[index]);
            } else if variadic {
                self.add_edge(argument, varargs_node);
            } else {
                self.mark_external(site, true);
            }
        }
        if argument_count < parameters.len() {
            for parameter in parameters.into_iter().skip(argument_count) {
                self.insert(parameter, UNKNOWN);
            }
        }
        if let Some(destination) = destination {
            self.add_edge(return_node, destination);
        }
    }

    fn new_call_site(
        &mut self,
        caller: usize,
        block: usize,
        instruction: usize,
        info: &CallInfo,
        destination: Option<Node>,
    ) -> usize {
        let mut arguments = Vec::with_capacity(info.args.len());
        for argument in &info.args {
            arguments.push(self.operand_node(caller, argument));
        }
        let site = self.call_sites.len();
        self.call_sites.push(CallSite {
            caller,
            arguments,
            destination,
            targets: FxHashSet::default(),
            external: false,
            is_pure: info.is_pure,
            is_const: info.is_const,
            is_sret: info.is_sret,
        });
        self.site_for_instruction
            .insert((caller, block, instruction), site);
        site
    }

    fn process_watch(&mut self, watch: Watch, object: Object) {
        match watch {
            Watch::Load { dest } => {
                let content = self.objects[object as usize].content;
                self.add_edge(content, dest);
            }
            Watch::Store { value } => {
                let content = self.objects[object as usize].content;
                self.add_edge(value, content);
                let dependents = &mut self.escape_dependents[object as usize];
                if !dependents.contains(&value) {
                    dependents.push(value);
                }
                if self.objects[object as usize].escaped {
                    self.activate_escape(value);
                }
            }
            Watch::MemcpyDestination { source_pointer } => {
                let destination_content = self.objects[object as usize].content;
                for source in self.points[source_pointer as usize].0.clone() {
                    let source_content = self.objects[source as usize].content;
                    self.add_edge(source_content, destination_content);
                }
            }
            Watch::MemcpySource {
                destination_pointer,
            } => {
                let source_content = self.objects[object as usize].content;
                for destination in self.points[destination_pointer as usize].0.clone() {
                    let destination_content = self.objects[destination as usize].content;
                    self.add_edge(source_content, destination_content);
                }
            }
            Watch::IndirectCall { site } => match self.objects[object as usize].kind {
                ObjectKind::Function(function) => self.bind_target(site, function),
                ObjectKind::ExternalFunction | ObjectKind::Unknown => {
                    self.mark_external(site, true);
                }
                _ => {
                    self.call_sites[site].external = true;
                    self.complete = false;
                }
            },
        }
    }

    fn solve(&mut self) {
        while self.complete && self.work <= self.budget.max_work {
            let Some((node, object)) = self.queue.pop_front() else {
                break;
            };
            self.charge(1);
            if !self.complete {
                break;
            }
            for destination in self.edges[node as usize].clone() {
                self.insert(destination, object);
            }
            for watch in self.watchers[node as usize].clone() {
                self.process_watch(watch, object);
            }
            if self.escape_sources.contains(&node) {
                self.mark_escaped(object);
            }
        }
        if self.work > self.budget.max_work || self.edge_set.len() > self.budget.max_edges {
            self.complete = false;
        }
    }

    fn finish(&mut self) {
        self.solve();
        if !self.complete {
            return;
        }
        // Empty memory-pointer sets carry no proof.  They normally denote an
        // unmodelled producer or a null-only path; widening just those nodes
        // is local, unlike poisoning every pointer in the translation unit.
        for node in self.memory_pointer_nodes.clone() {
            if self.points[node as usize].0.is_empty() {
                self.insert(node, UNKNOWN);
            }
        }
        self.solve();
    }
}

fn arguments_too_few(site: &CallSite, named: usize) -> bool {
    site.arguments.len() < named
}

#[derive(Clone, Debug, Default)]
struct ModRef {
    objects: ObjectSet,
    unknown: bool,
    all: bool,
}

impl ModRef {
    fn union_with(&mut self, other: &Self) -> bool {
        let mut changed = self.objects.union_with(&other.objects);
        if other.unknown && !self.unknown {
            self.unknown = true;
            changed = true;
        }
        if other.all && !self.all {
            self.all = true;
            changed = true;
        }
        changed
    }
}

#[derive(Debug)]
pub(crate) struct IpaPta {
    complete: bool,
    functions: FxHashMap<String, usize>,
    value_nodes: Vec<FxHashMap<u32, Node>>,
    points: Vec<ObjectSet>,
    objects: Vec<AbstractObject>,
    site_for_instruction: FxHashMap<(usize, usize, usize), usize>,
    site_effects: Vec<ModRef>,
    pub nodes: usize,
    pub edges: usize,
    pub work: usize,
    pub escaped_objects: usize,
}

impl IpaPta {
    pub(crate) fn may_call_modify(
        &self,
        function: &IrFunction,
        block: usize,
        instruction: usize,
        pointer: Value,
    ) -> bool {
        if !self.complete {
            return true;
        }
        let Some(&fi) = self.functions.get(&function.name) else {
            return true;
        };
        let Some(&site) = self.site_for_instruction.get(&(fi, block, instruction)) else {
            return true;
        };
        let Some(&node) = self.value_nodes[fi].get(&pointer.0) else {
            return true;
        };
        let points = &self.points[node as usize].0;
        if points.is_empty() {
            return true;
        }
        let effects = &self.site_effects[site];
        if effects.all {
            return true;
        }
        for &object in points {
            if object == UNKNOWN {
                // Unknown provenance is not a proof that an unescaped object
                // is unreachable: it can arise from an integer-to-pointer
                // cast or an external entry point.  Any concrete write is
                // therefore a may-alias with this address as well.
                if effects.unknown || !effects.objects.0.is_empty() {
                    return true;
                }
                continue;
            }
            if effects.objects.contains(object) {
                return true;
            }
            if effects.unknown
                && (self.objects[object as usize].escaped
                    || is_global(&self.objects[object as usize]))
            {
                return true;
            }
        }
        false
    }

    #[cfg(test)]
    fn points_to(&self, function: &str, value: u32) -> Option<Vec<usize>> {
        let &fi = self.functions.get(function)?;
        let &node = self.value_nodes[fi].get(&value)?;
        Some(
            self.points[node as usize]
                .0
                .iter()
                .map(|&o| o as usize)
                .collect(),
        )
    }
}

fn is_global(object: &AbstractObject) -> bool {
    matches!(
        object.kind,
        ObjectKind::Global | ObjectKind::ExternalData | ObjectKind::ConstantData
    )
}

fn external_modifications(site: &CallSite) -> ModRef {
    ModRef {
        unknown: site.external && !site.is_const && !site.is_pure,
        ..ModRef::default()
    }
}

fn external_references(site: &CallSite) -> ModRef {
    ModRef {
        unknown: site.external && !site.is_const,
        ..ModRef::default()
    }
}

fn pointer_effects(solver: &Solver, node: Node) -> ModRef {
    let mut effects = ModRef::default();
    for &object in &solver.points[node as usize].0 {
        if object == UNKNOWN {
            effects.unknown = true;
        } else {
            effects.objects.insert(object);
        }
    }
    if solver.points[node as usize].0.is_empty() {
        effects.unknown = true;
    }
    effects
}

/// Analyze a module and eliminate redundant loads using the resulting call
/// mod/ref summaries.  `semantic_interposition` is true for -fPIC/-shared.
pub(crate) fn run(module: &mut IrModule, semantic_interposition: bool) -> usize {
    run_with_budget(module, semantic_interposition, AnalysisBudget::default())
}

fn run_with_budget(
    module: &mut IrModule,
    semantic_interposition: bool,
    budget: AnalysisBudget,
) -> usize {
    let mut solver = build_solver(module, semantic_interposition, budget);
    solver.finish();
    if !solver.complete {
        if std::env::var_os("CCC_IPA_PTA_STATS").is_some() {
            eprintln!("[IPA-PTA] analysis budget exhausted; all consumers disabled");
        }
        return 0;
    }
    let site_effects = compute_effects(&solver, module);
    let functions: FxHashMap<String, usize> = module
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.clone(), i))
        .collect();
    let escaped_objects = solver.objects.iter().filter(|o| o.escaped).count();
    let analysis = IpaPta {
        complete: solver.complete,
        functions,
        value_nodes: solver.value_nodes.clone(),
        points: solver.points.clone(),
        objects: solver.objects.clone(),
        site_for_instruction: solver.site_for_instruction.clone(),
        site_effects,
        nodes: solver.points.len(),
        edges: solver.edge_set.len(),
        work: solver.work,
        escaped_objects,
    };
    if std::env::var_os("CCC_IPA_PTA_STATS").is_some() {
        eprintln!(
            "[IPA-PTA] nodes={} objects={} edges={} calls={} escaped={} work={}",
            analysis.nodes,
            analysis.objects.len(),
            analysis.edges,
            analysis.site_effects.len(),
            analysis.escaped_objects,
            analysis.work
        );
    }
    let mut eliminated = 0;
    for function in &mut module.functions {
        if !function.is_declaration {
            eliminated += super::redundant_loads::run_with_ipa_pta(function, &analysis);
        }
    }
    if eliminated != 0 {
        module.for_each_function(super::dce::eliminate_dead_code);
    }
    if std::env::var_os("CCC_IPA_PTA_STATS").is_some() {
        eprintln!("[IPA-PTA] redundant-loads-forwarded={eliminated}");
    }
    eliminated
}

fn compute_effects(solver: &Solver, module: &IrModule) -> Vec<ModRef> {
    let function_count = solver.functions.len();
    let mut local_mods = vec![ModRef::default(); function_count];
    let mut local_refs = vec![ModRef::default(); function_count];
    let mut call_extra_mods = vec![ModRef::default(); solver.call_sites.len()];

    for (fi, function) in module.functions.iter().enumerate() {
        if function.is_declaration {
            continue;
        }
        for (bi, block) in function.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                let key = (fi, bi, ii);
                let node = |solver: &Solver, value: Value| -> Option<Node> {
                    solver.value_nodes[fi].get(&value.0).copied()
                };
                let operand_node = |solver: &Solver, operand: &Operand| -> Option<Node> {
                    match operand {
                        Operand::Value(value) => node(solver, *value),
                        Operand::Const(_) => None,
                    }
                };
                match inst {
                    Instruction::Load { ptr, .. } => {
                        if let Some(node) = node(solver, *ptr) {
                            local_refs[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::Store { ptr, .. } => {
                        if let Some(node) = node(solver, *ptr) {
                            local_mods[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::AtomicLoad { ptr, .. } => {
                        if let Some(node) = operand_node(solver, ptr) {
                            local_refs[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::AtomicStore { ptr, .. } => {
                        if let Some(node) = operand_node(solver, ptr) {
                            local_mods[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::AtomicRmw { ptr, .. }
                    | Instruction::AtomicCmpxchg { ptr, .. }
                    | Instruction::AtomicInc { ptr, .. } => {
                        if let Some(node) = operand_node(solver, ptr) {
                            let effects = pointer_effects(solver, node);
                            local_mods[fi].union_with(&effects);
                            local_refs[fi].union_with(&effects);
                        }
                    }
                    Instruction::Memcpy { dest, src, .. } => {
                        if let Some(node) = node(solver, *dest) {
                            local_mods[fi].union_with(&pointer_effects(solver, node));
                        }
                        if let Some(node) = node(solver, *src) {
                            local_refs[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::VaArg { va_list_ptr, .. } => {
                        if let Some(node) = node(solver, *va_list_ptr) {
                            let effects = pointer_effects(solver, node);
                            local_mods[fi].union_with(&effects);
                            local_refs[fi].union_with(&effects);
                        }
                    }
                    Instruction::VaStart { va_list_ptr } | Instruction::VaEnd { va_list_ptr } => {
                        if let Some(node) = node(solver, *va_list_ptr) {
                            local_mods[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::VaCopy { dest_ptr, src_ptr } => {
                        if let Some(node) = node(solver, *dest_ptr) {
                            local_mods[fi].union_with(&pointer_effects(solver, node));
                        }
                        if let Some(node) = node(solver, *src_ptr) {
                            local_refs[fi].union_with(&pointer_effects(solver, node));
                        }
                    }
                    Instruction::VaArgStruct {
                        dest_ptr,
                        va_list_ptr,
                        ..
                    }
                    | Instruction::VaArgVector {
                        dest_ptr,
                        va_list_ptr,
                        ..
                    } => {
                        if let Some(node) = node(solver, *dest_ptr) {
                            local_mods[fi].union_with(&pointer_effects(solver, node));
                        }
                        if let Some(node) = node(solver, *va_list_ptr) {
                            let effects = pointer_effects(solver, node);
                            local_mods[fi].union_with(&effects);
                            local_refs[fi].union_with(&effects);
                        }
                    }
                    Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                        if info.is_sret
                            && let Some(site) = solver.site_for_instruction.get(&key)
                            && let Some(Some(pointer)) = solver.call_sites[*site].arguments.first()
                        {
                            call_extra_mods[*site].union_with(&pointer_effects(solver, *pointer));
                        }
                    }
                    _ => {}
                }
                if solver.unmodeled_instructions.contains(&key) {
                    local_mods[fi].all = true;
                    local_refs[fi].all = true;
                }
            }
        }
    }

    for (site_id, site) in solver.call_sites.iter().enumerate() {
        local_mods[site.caller].union_with(&external_modifications(site));
        local_mods[site.caller].union_with(&call_extra_mods[site_id]);
        local_refs[site.caller].union_with(&external_references(site));
    }

    let mut callers = vec![Vec::<usize>::new(); function_count];
    for site in &solver.call_sites {
        for &callee in &site.targets {
            if solver.functions[callee].safe_body && !callers[callee].contains(&site.caller) {
                callers[callee].push(site.caller);
            }
        }
    }

    // Propagate both components over recursive call graphs.  Cloning the
    // predecessor summary avoids aliasing a mutable caller with an immutable
    // callee when a recursive edge is a self-edge.
    let mut queue: VecDeque<usize> = (0..function_count).collect();
    let mut summaries = local_mods;
    let mut references = local_refs;
    while let Some(function) = queue.pop_front() {
        let callee_mods = summaries[function].clone();
        let callee_refs = references[function].clone();
        for &caller in &callers[function] {
            let changed = summaries[caller].union_with(&callee_mods);
            let ref_changed = references[caller].union_with(&callee_refs);
            if changed || ref_changed {
                queue.push_back(caller);
            }
        }
    }
    // Keep references in the same fixpoint as modifications so a future
    // load-consumer cannot observe a different call-graph approximation.
    let _ = references;
    solver
        .call_sites
        .iter()
        .enumerate()
        .map(|(site_id, site)| {
            let mut effects = external_modifications(site);
            effects.union_with(&call_extra_mods[site_id]);
            for &target in &site.targets {
                if solver.functions[target].safe_body {
                    effects.union_with(&summaries[target]);
                }
            }
            effects
        })
        .collect()
}

fn canonical_name<'a>(aliases: &'a FxHashMap<String, String>, mut name: &'a str) -> &'a str {
    // Alias cycles are malformed IR.  Bounding the walk keeps analysis
    // total; the resulting unknown object is conservatively escaped.
    for _ in 0..32 {
        let Some(next) = aliases.get(name) else {
            return name;
        };
        if next == name {
            break;
        }
        name = next;
    }
    name
}

fn add_initializer_constraints(
    solver: &mut Solver,
    aliases: &FxHashMap<String, String>,
    object: Object,
    init: &GlobalInit,
) {
    match init {
        GlobalInit::GlobalAddr(name) | GlobalInit::GlobalAddrOffset(name, _) => {
            let target = solver.object_for_name(canonical_name(aliases, name));
            let content = solver.objects[object as usize].content;
            solver.insert(content, target);
        }
        GlobalInit::Compound(fields) => {
            for field in fields {
                add_initializer_constraints(solver, aliases, object, field);
            }
        }
        _ => {}
    }
}

fn mark_unmodeled(
    solver: &mut Solver,
    function: usize,
    block: usize,
    instruction: usize,
    inst: &Instruction,
) {
    solver
        .unmodeled_instructions
        .insert((function, block, instruction));
    if let Some(dest) = inst.dest() {
        let node = solver.value_node(function, dest.0);
        solver.insert(node, UNKNOWN);
    }
    let mut values = Vec::new();
    inst.for_each_used_value(|value| values.push(value));
    for value in values {
        let node = solver.value_node(function, value);
        solver.activate_escape(node);
    }
}

fn operand_memory_pointer(solver: &mut Solver, function: usize, operand: &Operand) -> Option<Node> {
    match operand {
        Operand::Value(value) => Some(solver.memory_pointer(function, *value)),
        Operand::Const(_) => None,
    }
}

fn is_allocation_name(name: &str) -> bool {
    matches!(
        name,
        "malloc"
            | "calloc"
            | "realloc"
            | "strdup"
            | "strndup"
            | "aligned_alloc"
            | "valloc"
            | "pvalloc"
    )
}

fn build_instruction(
    solver: &mut Solver,
    aliases: &FxHashMap<String, String>,
    function: usize,
    block: usize,
    instruction: usize,
    inst: &Instruction,
) {
    match inst {
        Instruction::Alloca { dest, .. } | Instruction::DynAlloca { dest, .. } => {
            let object = solver.add_object(ObjectKind::Alloca);
            let node = solver.value_node(function, dest.0);
            solver.insert(node, object);
        }
        Instruction::GlobalAddr { dest, name } => {
            let object = solver.object_for_name(canonical_name(aliases, name));
            let node = solver.value_node(function, dest.0);
            solver.insert(node, object);
        }
        Instruction::Copy { dest, src } => {
            let destination = solver.value_node(function, dest.0);
            if let Some(source) = solver.operand_node(function, src) {
                solver.add_edge(source, destination);
            }
        }
        Instruction::Cast {
            dest,
            src,
            from_ty,
            to_ty,
        } => {
            let destination = solver.value_node(function, dest.0);
            if let Some(source) = solver.operand_node(function, src) {
                solver.add_edge(source, destination);
            }
            if *to_ty == IrType::Ptr && *from_ty != IrType::Ptr {
                // Integer-to-pointer provenance is not recoverable with a
                // field-insensitive SSA graph.  Keep any tracked incoming
                // solution and add the unknown provenance as well.
                solver.insert(destination, UNKNOWN);
            }
        }
        Instruction::GetElementPtr { dest, base, .. } => {
            let destination = solver.value_node(function, dest.0);
            let source = solver.value_node(function, base.0);
            solver.add_edge(source, destination);
        }
        Instruction::BinOp { dest, lhs, rhs, .. } => {
            let destination = solver.value_node(function, dest.0);
            if let Some(source) = solver.operand_node(function, lhs) {
                solver.add_edge(source, destination);
            }
            if let Some(source) = solver.operand_node(function, rhs) {
                solver.add_edge(source, destination);
            }
        }
        Instruction::UnaryOp { dest, src, .. } => {
            let destination = solver.value_node(function, dest.0);
            if let Some(source) = solver.operand_node(function, src) {
                solver.add_edge(source, destination);
            }
        }
        Instruction::Select {
            dest,
            true_val,
            false_val,
            ..
        } => {
            let destination = solver.value_node(function, dest.0);
            if let Some(source) = solver.operand_node(function, true_val) {
                solver.add_edge(source, destination);
            }
            if let Some(source) = solver.operand_node(function, false_val) {
                solver.add_edge(source, destination);
            }
        }
        Instruction::Phi { dest, incoming, .. } => {
            let destination = solver.value_node(function, dest.0);
            for (operand, _) in incoming {
                if let Some(source) = solver.operand_node(function, operand) {
                    solver.add_edge(source, destination);
                }
            }
        }
        Instruction::Load { dest, ptr, .. } => {
            let pointer = solver.memory_pointer(function, *ptr);
            let destination = solver.value_node(function, dest.0);
            solver.add_watch(pointer, Watch::Load { dest: destination });
        }
        Instruction::Store { val, ptr, .. } => {
            let pointer = solver.memory_pointer(function, *ptr);
            if let Some(value) = solver.operand_node(function, val) {
                solver.add_watch(pointer, Watch::Store { value });
            }
        }
        Instruction::Memcpy { dest, src, size } => {
            if *size != 0 {
                let destination = solver.memory_pointer(function, *dest);
                let source = solver.memory_pointer(function, *src);
                solver.add_watch(
                    destination,
                    Watch::MemcpyDestination {
                        source_pointer: source,
                    },
                );
                solver.add_watch(
                    source,
                    Watch::MemcpySource {
                        destination_pointer: destination,
                    },
                );
            }
        }
        Instruction::AtomicLoad { dest, ptr, .. } => {
            if let Some(pointer) = operand_memory_pointer(solver, function, ptr) {
                let destination = solver.value_node(function, dest.0);
                solver.add_watch(pointer, Watch::Load { dest: destination });
            } else {
                mark_unmodeled(solver, function, block, instruction, inst);
            }
        }
        Instruction::AtomicStore { ptr, val, .. } => {
            if let Some(pointer) = operand_memory_pointer(solver, function, ptr) {
                if let Some(value) = solver.operand_node(function, val) {
                    solver.add_watch(pointer, Watch::Store { value });
                }
            } else {
                mark_unmodeled(solver, function, block, instruction, inst);
            }
        }
        Instruction::AtomicRmw { dest, ptr, val, .. } => {
            if let Some(pointer) = operand_memory_pointer(solver, function, ptr) {
                let destination = solver.value_node(function, dest.0);
                solver.add_watch(pointer, Watch::Load { dest: destination });
                if let Some(value) = solver.operand_node(function, val) {
                    solver.add_watch(pointer, Watch::Store { value });
                }
            } else {
                mark_unmodeled(solver, function, block, instruction, inst);
            }
        }
        Instruction::AtomicCmpxchg {
            dest,
            ptr,
            desired,
            returns_bool,
            ..
        } => {
            if let Some(pointer) = operand_memory_pointer(solver, function, ptr) {
                if !returns_bool {
                    let destination = solver.value_node(function, dest.0);
                    solver.add_watch(pointer, Watch::Load { dest: destination });
                }
                if let Some(value) = solver.operand_node(function, desired) {
                    solver.add_watch(pointer, Watch::Store { value });
                }
            } else {
                mark_unmodeled(solver, function, block, instruction, inst);
            }
        }
        Instruction::AtomicInc { ptr, .. } => {
            if operand_memory_pointer(solver, function, ptr).is_none() {
                mark_unmodeled(solver, function, block, instruction, inst);
            }
        }
        Instruction::Call { func, info } => {
            let destination = info.dest.map(|dest| solver.value_node(function, dest.0));
            let site = solver.new_call_site(function, block, instruction, info, destination);
            let name = canonical_name(aliases, func);
            let target = solver.function_indices.get(name).copied();
            let allocation = destination.is_some()
                && is_allocation_name(name)
                && target.is_none_or(|target| !solver.functions[target].safe_body);
            if allocation {
                let destination = destination.expect("allocation destination");
                let object = solver.add_object(ObjectKind::Heap);
                solver.insert(destination, object);
                if name == "realloc"
                    && let Some(Operand::Value(source)) = info.args.first()
                {
                    let source = solver.value_node(function, source.0);
                    solver.add_edge(source, destination);
                }
                solver.mark_external_with_return(site, false, false);
            } else if let Some(target) = target {
                solver.bind_target(site, target);
            } else {
                solver.mark_external(site, false);
            }
        }
        Instruction::CallIndirect { func_ptr, info } => {
            let destination = info.dest.map(|dest| solver.value_node(function, dest.0));
            let site = solver.new_call_site(function, block, instruction, info, destination);
            if let Some(pointer) = operand_memory_pointer(solver, function, func_ptr) {
                solver.add_watch(pointer, Watch::IndirectCall { site });
            } else {
                solver.mark_external(site, true);
            }
        }
        Instruction::VaArg {
            dest, va_list_ptr, ..
        } => {
            solver.memory_pointer(function, *va_list_ptr);
            let source = solver.functions[function].varargs_node;
            let destination = solver.value_node(function, dest.0);
            solver.add_edge(source, destination);
        }
        Instruction::ParamRef {
            dest, param_idx, ..
        } => {
            let destination = solver.value_node(function, dest.0);
            if let Some(&source) = solver.functions[function].parameters.get(*param_idx) {
                solver.add_edge(source, destination);
            } else {
                solver.insert(destination, UNKNOWN);
            }
        }
        Instruction::VaStart { va_list_ptr } | Instruction::VaEnd { va_list_ptr } => {
            solver.memory_pointer(function, *va_list_ptr);
        }
        Instruction::VaCopy { dest_ptr, src_ptr } => {
            solver.memory_pointer(function, *dest_ptr);
            solver.memory_pointer(function, *src_ptr);
        }
        Instruction::VaArgStruct {
            dest_ptr,
            va_list_ptr,
            ..
        }
        | Instruction::VaArgVector {
            dest_ptr,
            va_list_ptr,
            ..
        } => {
            solver.memory_pointer(function, *dest_ptr);
            solver.memory_pointer(function, *va_list_ptr);
        }
        Instruction::PgoCounterInc { .. }
        | Instruction::GetReturnF64Second { .. }
        | Instruction::GetReturnF32Second { .. }
        | Instruction::GetReturnF128Second { .. }
        | Instruction::SetReturnF64Second { .. }
        | Instruction::SetReturnF32Second { .. }
        | Instruction::SetReturnF128Second { .. }
        | Instruction::LabelAddr { .. } => {
            // These operations have no user-memory pointer-producing
            // semantics that need an inclusion edge.  Their memory effects,
            // if any, are recorded by compute_effects.
        }
        Instruction::GetStaticChain { dest } | Instruction::StackSave { dest } => {
            let node = solver.value_node(function, dest.0);
            solver.insert(node, UNKNOWN);
        }
        Instruction::SetStaticChain { src } => {
            if let Some(node) = solver.operand_node(function, src) {
                solver.activate_escape(node);
            }
        }
        Instruction::Intrinsic { dest, .. } if !inst.may_write_memory() => {
            // Pure/readonly target intrinsics still need a conservative
            // pointer result, but they do not make arbitrary caller memory
            // modifiable.  This is important for vectorized call-heavy code:
            // an unrelated SIMD operation must not become an IPA barrier.
            if let Some(dest) = dest {
                let node = solver.value_node(function, dest.0);
                solver.insert(node, UNKNOWN);
            }
        }
        _ => mark_unmodeled(solver, function, block, instruction, inst),
    }
}

fn build_solver(module: &IrModule, semantic_interposition: bool, budget: AnalysisBudget) -> Solver {
    let mut solver = Solver::new(module.functions.len(), budget);
    let aliases: FxHashMap<String, String> = module
        .aliases
        .iter()
        .map(|(alias, target, _)| (alias.clone(), target.clone()))
        .collect();

    for (index, function) in module.functions.iter().enumerate() {
        solver.function_indices.insert(function.name.clone(), index);
        let safe_body = !function.is_declaration
            && !function.is_weak
            && !(semantic_interposition
                && !function.is_static
                && function.visibility.as_deref() != Some("hidden"));
        let parameters = (0..function.params.len())
            .map(|_| solver.new_node())
            .collect();
        let return_node = solver.new_node();
        let varargs_node = solver.new_node();
        solver.functions.push(FunctionInfo {
            safe_body,
            externally_visible: !function.is_static,
            variadic: function.is_variadic,
            parameters,
            return_node,
            varargs_node,
        });
    }

    // Functions and data have distinct abstract objects even when a backend
    // gives them similar machine addresses.  Indirect calls accept only the
    // former; treating data as a possible callee fails the whole analysis.
    for (index, function) in module.functions.iter().enumerate() {
        let object = solver.add_object(ObjectKind::Function(index));
        solver.named_objects.insert(function.name.clone(), object);
    }
    for global in &module.globals {
        let object = solver.add_object(ObjectKind::Global);
        solver.named_objects.insert(global.name.clone(), object);
    }
    for (label, _) in &module.string_literals {
        let object = solver.add_object(ObjectKind::ConstantData);
        solver.named_objects.insert(label.clone(), object);
    }
    for (label, _) in &module.wide_string_literals {
        let object = solver.add_object(ObjectKind::ConstantData);
        solver.named_objects.insert(label.clone(), object);
    }
    for (label, _) in &module.char16_string_literals {
        let object = solver.add_object(ObjectKind::ConstantData);
        solver.named_objects.insert(label.clone(), object);
    }
    for name in &module.extern_function_symbols {
        if !solver.named_objects.contains_key(name) {
            let object = solver.add_object(ObjectKind::ExternalFunction);
            solver.named_objects.insert(name.clone(), object);
            solver.mark_escaped(object);
        }
    }
    for (alias, target, _) in &module.aliases {
        let target = canonical_name(&aliases, target);
        let object = solver.object_for_name(target);
        solver.named_objects.insert(alias.clone(), object);
    }
    for global in &module.globals {
        let object = solver.named_objects[&global.name];
        add_initializer_constraints(&mut solver, &aliases, object, &global.init);
        if !global.is_static {
            solver.mark_escaped(object);
        }
    }
    for (index, function) in module.functions.iter().enumerate() {
        let info = &solver.functions[index];
        let parameters = info.parameters.clone();
        let return_node = info.return_node;
        let varargs_node = info.varargs_node;
        if info.externally_visible {
            solver.activate_escape(return_node);
            if function.is_variadic {
                solver.insert(varargs_node, UNKNOWN);
                solver.activate_escape(varargs_node);
            }
            for (parameter, ty) in parameters
                .into_iter()
                .zip(function.params.iter().map(|p| p.ty))
            {
                if ty == IrType::Ptr {
                    solver.insert(parameter, UNKNOWN);
                }
            }
        }
    }

    for (fi, function) in module.functions.iter().enumerate() {
        if function.is_declaration {
            continue;
        }
        for (bi, block) in function.blocks.iter().enumerate() {
            for (ii, inst) in block.instructions.iter().enumerate() {
                build_instruction(&mut solver, &aliases, fi, bi, ii, inst);
            }
            if let Terminator::Return(Some(Operand::Value(value))) = &block.terminator {
                let source = solver.value_node(fi, value.0);
                let destination = solver.functions[fi].return_node;
                solver.add_edge(source, destination);
            }
        }
    }
    solver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::module::IrParam;
    use crate::ir::reexports::{BasicBlock, BlockId};

    fn function(name: &str, return_type: IrType) -> IrFunction {
        let mut function = IrFunction::new(name.into(), return_type, Vec::new(), false);
        function.is_static = true;
        function.next_value_id = 128;
        function.next_label = 1;
        function
    }

    fn finish(function: &mut IrFunction, instructions: Vec<Instruction>, result: Option<Value>) {
        function.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions,
            terminator: Terminator::Return(result.map(Operand::Value)),
            source_spans: Vec::new(),
        });
    }

    fn alloca(value: u32) -> Instruction {
        Instruction::Alloca {
            dest: Value(value),
            ty: IrType::I32,
            size: 4,
            align: 4,
            volatile: false,
            semantic_volatile: false,
        }
    }

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn store(value: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(value)),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: AddressSpace::Default,
            volatile: false,
        }
    }

    fn copy_const(dest: u32, value: i32) -> Instruction {
        Instruction::Copy {
            dest: Value(dest),
            src: Operand::Const(crate::ir::constants::IrConst::I32(value)),
        }
    }

    fn global_addr(dest: u32, name: &str) -> Instruction {
        Instruction::GlobalAddr {
            dest: Value(dest),
            name: name.into(),
        }
    }

    fn call(name: &str, args: Vec<Operand>) -> Instruction {
        let count = args.len();
        Instruction::Call {
            func: name.into(),
            info: CallInfo {
                args,
                arg_types: vec![IrType::Ptr; count],
                return_type: IrType::Void,
                num_fixed_args: count,
                ..CallInfo::default()
            },
        }
    }

    fn indirect(pointer: u32) -> Instruction {
        Instruction::CallIndirect {
            func_ptr: Operand::Value(Value(pointer)),
            info: CallInfo {
                return_type: IrType::Void,
                ..CallInfo::default()
            },
        }
    }

    fn pointer_param() -> IrParam {
        IrParam {
            ty: IrType::Ptr,
            noalias: false,
            struct_size: None,
            struct_align: None,
            param_align: None,
            struct_eightbyte_classes: Vec::new(),
            is_f128_sse: false,
            riscv_float_class: None,
        }
    }

    fn module(functions: Vec<IrFunction>) -> IrModule {
        let mut module = IrModule::new();
        module.functions = functions;
        module
    }

    fn load_count(function: &IrFunction) -> usize {
        function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|inst| matches!(inst, Instruction::Load { .. }))
            .count()
    }

    #[test]
    fn direct_read_only_call_preserves_unescaped_load() {
        let mut reader = function("reader", IrType::I32);
        finish(
            &mut reader,
            vec![global_addr(0, "external_global"), load(1, 0)],
            Some(Value(1)),
        );
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![alloca(0), load(1, 0), call("reader", vec![]), load(2, 0)],
            Some(Value(2)),
        );
        let mut baseline = module(vec![reader.clone(), caller.clone()]);
        assert_eq!(
            super::super::redundant_loads::run(&mut baseline.functions[1]),
            0
        );
        let mut module = module(vec![reader, caller]);
        assert_eq!(run(&mut module, false), 1);
        assert_eq!(load_count(&module.functions[1]), 1);
    }

    #[test]
    fn direct_store_summary_reaches_pointer_argument() {
        let mut writer = function("writer", IrType::Void);
        writer.params.push(pointer_param());
        finish(
            &mut writer,
            vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                copy_const(1, 9),
                store(1, 0),
            ],
            None,
        );
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                alloca(0),
                load(1, 0),
                call("writer", vec![Operand::Value(Value(0))]),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![writer, caller]);
        assert_eq!(run(&mut module, false), 0);
        assert_eq!(load_count(&module.functions[1]), 2);
    }

    #[test]
    fn unknown_call_escapes_argument_and_keeps_load() {
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                alloca(0),
                load(1, 0),
                call("unknown_external", vec![Operand::Value(Value(0))]),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![caller]);
        assert_eq!(run(&mut module, false), 0);
        assert_eq!(load_count(&module.functions[0]), 2);
    }

    #[test]
    fn direct_read_through_parameter_does_not_invent_a_write() {
        let mut reader = function("reader", IrType::Void);
        reader.params.push(pointer_param());
        finish(
            &mut reader,
            vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                load(1, 0),
            ],
            None,
        );
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                alloca(0),
                load(1, 0),
                call("reader", vec![Operand::Value(Value(0))]),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![reader, caller]);
        assert_eq!(run(&mut module, false), 1);
        assert_eq!(load_count(&module.functions[1]), 1);
    }

    #[test]
    fn indirect_function_address_gets_direct_summary() {
        let mut reader = function("reader", IrType::Void);
        finish(&mut reader, vec![], None);
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                alloca(0),
                global_addr(3, "reader"),
                load(1, 0),
                indirect(3),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![reader, caller]);
        assert_eq!(run(&mut module, false), 1);
        assert_eq!(load_count(&module.functions[1]), 1);
    }

    #[test]
    fn recursive_summary_propagates_through_an_intermediate_caller() {
        let mut recursive = function("recursive", IrType::Void);
        recursive.params.push(pointer_param());
        finish(
            &mut recursive,
            vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                copy_const(1, 4),
                store(1, 0),
                call("recursive", vec![Operand::Value(Value(0))]),
            ],
            None,
        );
        let mut outer = function("outer", IrType::Void);
        outer.params.push(pointer_param());
        finish(
            &mut outer,
            vec![
                Instruction::ParamRef {
                    dest: Value(0),
                    param_idx: 0,
                    ty: IrType::Ptr,
                },
                call("recursive", vec![Operand::Value(Value(0))]),
            ],
            None,
        );
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                alloca(0),
                load(1, 0),
                call("outer", vec![Operand::Value(Value(0))]),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![recursive, outer, caller]);
        assert_eq!(run(&mut module, false), 0);
        assert_eq!(load_count(&module.functions[2]), 2);
    }

    #[test]
    fn pointer_stored_in_global_escapes_before_an_unknown_call() {
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                alloca(0),
                load(1, 0),
                global_addr(3, "pointer_slot"),
                Instruction::Store {
                    val: Operand::Value(Value(0)),
                    ptr: Value(3),
                    ty: IrType::Ptr,
                    seg_override: AddressSpace::Default,
                    volatile: false,
                },
                call("unknown_external", vec![]),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![caller]);
        assert_eq!(run(&mut module, false), 0);
        assert_eq!(load_count(&module.functions[0]), 2);
    }

    #[test]
    fn variadic_argument_edges_feed_va_arg_memory_effects() {
        let mut variadic = function("variadic", IrType::Void);
        variadic.is_variadic = true;
        variadic.params.push(pointer_param());
        finish(
            &mut variadic,
            vec![
                alloca(1),
                Instruction::VaArg {
                    dest: Value(0),
                    va_list_ptr: Value(1),
                    result_ty: IrType::Ptr,
                },
                copy_const(2, 8),
                store(2, 0),
            ],
            None,
        );
        let mut caller = function("caller", IrType::I32);
        let mut invoke = call(
            "variadic",
            vec![Operand::Value(Value(4)), Operand::Value(Value(0))],
        );
        if let Instruction::Call { info, .. } = &mut invoke {
            info.is_variadic = true;
            info.num_fixed_args = 1;
        }
        finish(
            &mut caller,
            vec![alloca(0), alloca(4), load(1, 0), invoke, load(2, 0)],
            Some(Value(2)),
        );
        let mut module = module(vec![variadic, caller]);
        assert_eq!(run(&mut module, false), 0);
        assert_eq!(load_count(&module.functions[1]), 2);
    }

    #[test]
    fn semantic_interposition_disables_body_based_precision() {
        let mut reader = function("reader", IrType::Void);
        reader.is_static = false;
        finish(&mut reader, vec![], None);
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![
                global_addr(0, "local_global"),
                load(1, 0),
                call("reader", vec![]),
                load(2, 0),
            ],
            Some(Value(2)),
        );
        let mut module = module(vec![reader.clone(), caller.clone()]);
        assert_eq!(run(&mut module, true), 0);
        assert_eq!(load_count(&module.functions[1]), 2);
        let mut native = module(vec![reader, caller]);
        assert_eq!(run(&mut native, false), 1);
    }

    #[test]
    fn exhausted_budget_disables_all_forwarding() {
        let mut caller = function("caller", IrType::I32);
        finish(
            &mut caller,
            vec![alloca(0), load(1, 0), call("reader", vec![]), load(2, 0)],
            Some(Value(2)),
        );
        let mut module = module(vec![caller]);
        let budget = AnalysisBudget {
            max_work: 1,
            max_edges: 1,
        };
        assert_eq!(run_with_budget(&mut module, false, budget), 0);
        assert_eq!(load_count(&module.functions[0]), 2);
    }
}

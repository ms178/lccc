//! Loop rotation: `while (c) body` becomes `if (c) do body while (c)`.
//!
//! ```text
//!   before                              after
//!
//!   P ──► H: phis                       P ──► H: phis          (the guard: runs once)
//!         test                                test
//!         br c, B, X                          br c, PH, X
//!   B:    ...                           PH:   br N             (runs iff the body will)
//!   L:    ...                           N:    merges           (the loop's header)
//!         br H                          B:    ...
//!   X:    ...                           L:    ...
//!                                             test'            (a copy, on this trip's values)
//!                                             br c', N, X
//!                                       X:    merges
//! ```
//!
//! # What it is for
//!
//! Rarely for the branch. The default pipeline already bottom-tests a loop
//! after phi elimination ([`crate::passes::loop_invert`]) wherever the
//! header's values stay in the header, so the hot path of such a loop has
//! one taken branch per trip either way. (Where they do not, see
//! [`rotate_fed_loops`] below.) What only the
//! SSA form can give is `PH`: **a block that runs exactly when the body is
//! going to run at least once**. A top-tested loop has no such place: its
//! preheader also runs on the zero-trip path, so nothing that can fault or
//! trap (a load through a pointer the loop test guards, a division by a value
//! the loop test guards, a call that reads memory) may move out of the body,
//! and nothing may be written behind the loop that the loop did not write.
//! Behind the guard both are allowed: LICM hoists into `PH` once what the
//! body paid for every trip; the memory promoter keeps `*p += x` in a
//! register from `PH` to the loop's exit edge, and loop-carried forwarding
//! keeps `a[i-1]` there for `a[i] = f(a[i-1])`, so that
//! `for (i = 0; i < n; i++) s->count += f(i)` and a prefix sum through a
//! pointer run at the speed of their addition instead of the store-to-load
//! forwarding's.
//!
//! # Which loops are rotated
//!
//! * [`RotatePolicy::Profit`], the default at `-O2` and above: a loop is
//!   rotated when, and only when, the rotation is what takes enough out of
//!   it. That is decided by doing it, not by predicting it:
//!   [`rotate_for_hoisting`] rotates the candidates in a copy of the
//!   function, runs the real LICM, memory promotion and loop-carried
//!   forwarding on the copy and keeps the copy only if every rotated loop
//!   lost instructions that had to wait for the guard, worth a measurable
//!   part of a trip and no register; a loop that did not pay is dropped from the next attempt,
//!   and a function in which no loop pays is left exactly as it was, value
//!   numbers included. A loop that rotation cannot help is therefore never
//!   touched, which is what makes the default safe for the passes that
//!   match the top-tested shape (the vectorizer, the unroller,
//!   induction-variable strength reduction, the affine folds, the
//!   machine-level inversion and its down-counter): rotating every loop at
//!   the same place costs 3.4% of the benchmark suite's instructions.
//!
//!   The same policy takes a second kind of loop, late: the one whose
//!   header computes a value the body reads (`while (n--)`). Machine-level
//!   inversion cannot bottom-test it, because it cannot give the body the
//!   latch's copy of that value; this pass can, and
//!   [`rotate_fed_loops`] does so behind every pass that matches a loop's
//!   shape, for the loops whose bottom test comes out tight.
//! * [`RotatePolicy::Always`] (`CCC_LOOP_ROTATE=1`): every loop the
//!   transformation can take, early in the pipeline. This is the stress
//!   mode: it puts the SSA reconstruction below in front of every loop of a
//!   test corpus, and it is the A/B that measures what blanket rotation
//!   would cost.
//! * [`RotatePolicy::Off`] (`CCC_NO_LOOP_ROTATE`, or `CCC_LOOP_ROTATE=0`).
//!
//! # How it stays correct
//!
//! The header's instructions exist twice afterwards: in `H`, where they run
//! once on the values the loop is entered with, and in the latch, where they
//! run on the values of the trip that just finished. Every value the header
//! defines therefore has two definitions, and each reader outside the header
//! has to see the one that reaches it. That is ordinary SSA reconstruction,
//! and it is done for *every* reader the same way ([`Merges`]): the value at
//! the end of `H` is the original, the value at the end of the latch is the
//! copy (for a header phi: what the latch passes over the back edge), and a
//! block reached from both gets a phi. Nothing about the loop's shape is
//! assumed beyond what [`plan_rotation`] checks, so a body of any number of
//! blocks, several latches, exits out of the body, values of the header read
//! in the body or behind the loop, loops inside loops, and headers entered
//! over several edges all take the same path through the same code.
//!
//! The division of labour is in the types: [`plan_rotation`] reads the
//! function through a shared reference and is the only place that refuses;
//! [`apply_rotation`] changes the function and cannot fail.
//!
//! # Cost
//!
//! The guard and the bottom test are the same test, so a rotated loop has
//! one copy of the header more than a top-tested one before inversion, and
//! exactly as many as the default pipeline emits after it. The merges at
//! `N` and `X` name the same values the header's phis did; phi-web
//! coalescing gives each pair one register again
//! (`ir::mem2reg::phi_webs`, "exit twins").

use std::cell::Cell;

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::source::Span;
use crate::common::types::IrType;
use crate::ir::analysis::{self, CfgAnalysis};
use crate::ir::reexports::{
    BasicBlock, BlockId, Instruction, IrConst, IrFunction, Operand, Terminator, Value,
    replace_operand_value, replace_terminator_value, replace_values_in_inst_map,
    replace_values_in_terminator_map,
};
use crate::passes::loop_analysis::{NaturalLoop, find_natural_loops, merge_loops_by_header};
use crate::passes::loop_unroll::rename_inst_dest;

mod affine_fold;

pub(crate) use affine_fold::{
    affine_fold_debug, affine_fold_enabled, fold_affine_exit_compares, integer_constant,
    integer_type_bounds, set_affine_fold_knobs,
};

/// Upper bound on the rotations of one function in one run. Every rotation
/// rebuilds the CFG analysis, so this bounds the quadratic worst case of a
/// pathological loop nest.
const MAX_ROTATIONS_PER_FUNC: usize = 256;

/// Most header instructions a rotation duplicates. A loop test is a compare
/// and what feeds it: one to four instructions in practice. LLVM's
/// `-rotation-max-header-size` is 16 and GCC's `max-loop-header-insns` 20; a
/// larger header is a body that happens to end in the exit test, and copying
/// it would trade code size for nothing.
const MAX_HEADER_INSTS: usize = 16;

/// Attempts [`rotate_for_hoisting`] makes before it gives a function up. The
/// first rotates every candidate; each later one rotates the loops that paid
/// in the one before. Two is the common case (try all, confirm the ones that
/// paid); a third covers loops whose hoist depended on a neighbour that did
/// not pay.
const MAX_PROFIT_ATTEMPTS: usize = 3;

/// Which loops are rotated. Resolved once per translation unit in
/// `run_passes` (the pass pipeline does not read the environment per
/// function, see `tests/regression/check_env_test_hygiene.sh`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RotatePolicy {
    /// No loop.
    Off,
    /// The loops whose rotation lets LICM hoist something
    /// ([`rotate_for_hoisting`]). The default.
    Profit,
    /// Every loop [`plan_rotation`] accepts ([`rotate_every_loop`]).
    Always,
}

impl RotatePolicy {
    /// `kill_switch` is `CCC_NO_LOOP_ROTATE` being set (to anything) and wins
    /// over `choice`, the value of `CCC_LOOP_ROTATE`: `0`/`false`/`no`/`off`
    /// turn the pass off, `1`/`true`/`yes`/`on`/`always` rotate every loop,
    /// and anything else (unset and empty included) is the default.
    pub(crate) fn resolve(kill_switch: bool, choice: Option<&str>) -> Self {
        if kill_switch {
            return Self::Off;
        }
        let is = |v: &str, words: &[&str]| words.iter().any(|w| v.eq_ignore_ascii_case(w));
        match choice.map(str::trim) {
            Some(v) if is(v, &["0", "false", "no", "off"]) => Self::Off,
            Some(v) if is(v, &["1", "true", "yes", "on", "always"]) => Self::Always,
            _ => Self::Profit,
        }
    }
}

thread_local! {
    /// `CCC_DEBUG_LOOP_ROTATE`: one `[ROT]` line per decision.
    static ROTATE_DEBUG: Cell<bool> = const { Cell::new(false) };
}

/// Hand over `CCC_DEBUG_LOOP_ROTATE` (see [`RotatePolicy`] for why it is not
/// read here).
pub(crate) fn set_rotate_debug(debug: bool) {
    ROTATE_DEBUG.with(|cell| cell.set(debug));
}

fn rotate_debug() -> bool {
    ROTATE_DEBUG.with(Cell::get)
}

/// A `[ROT]` line under `CCC_DEBUG_LOOP_ROTATE`.
macro_rules! trace {
    ($on:expr, $($arg:tt)*) => {
        if $on {
            lccc::cli::stderr_line(format_args!("[ROT] {}", format_args!($($arg)*)));
        }
    };
}

/// What one pass over a function's loops shares.
struct Shape<'a> {
    cfg: &'a CfgAnalysis,
    index_of: FxHashMap<BlockId, usize>,
    /// A block no path reaches branches into one that is reached. Such an
    /// edge needs a value in every merge it enters and has none to give.
    dead_code_feeds_live: bool,
    debug: bool,
}

impl<'a> Shape<'a> {
    fn of(func: &IrFunction, cfg: &'a CfgAnalysis, debug: bool) -> Self {
        let index_of = analysis::build_label_map(func);
        let reached =
            |block: usize| block == 0 || cfg.idom.get(block).is_some_and(|&d| d != usize::MAX);
        let dead_code_feeds_live = (0..func.blocks.len())
            .filter(|&b| !reached(b))
            .any(|b| cfg.succs.row(b).iter().any(|&s| reached(s as usize)));
        Self {
            cfg,
            index_of,
            dead_code_feeds_live,
            debug,
        }
    }

    fn reached(&self, block: usize) -> bool {
        block == 0 || self.cfg.idom.get(block).is_some_and(|&d| d != usize::MAX)
    }
}

/// A rotation that has been decided: everything [`apply_rotation`] needs.
/// Blocks are named by label; applying one rotation inserts blocks, and an
/// index would not survive that.
struct Plan {
    header: BlockId,
    /// The first block of the body: the header's successor inside the loop.
    entry: BlockId,
    /// The header's successor outside the loop.
    exit: BlockId,
    /// The header goes on into the loop when its condition is true.
    continue_on_true: bool,
    cond: Operand,
    /// The blocks of the loop that branch back to the header.
    latches: Vec<BlockId>,
    /// The loop's blocks other than the header.
    body: FxHashSet<BlockId>,
    /// The header's test is proven to stay in its type's range
    /// (`affine_fold::header_test_is_proven_affine`).
    affine_guard_proven: bool,
}

/// What a rotation left behind.
#[derive(Debug)]
pub(crate) struct Rotated {
    /// The old header: the guard, run once per entry.
    pub(crate) guard: BlockId,
    /// Between the guard and the loop: runs iff the body runs at least once.
    pub(crate) preheader: BlockId,
    /// The loop's header after the rotation: where every trip starts.
    pub(crate) header: BlockId,
}

/// Whether the guard may run `inst` once more than the loop used to: a copy
/// in the latch takes over every execution after the first, so the number
/// and the order of executions are those of the original program. The list
/// is what a loop test is made of. A volatile load stays where it is (its
/// accesses are counted in the generated code by the volatile gates, and one
/// source access must stay one instruction), and nothing that writes memory,
/// calls, allocates or transfers control is duplicated.
fn is_duplicable(inst: &Instruction) -> bool {
    match inst {
        Instruction::BinOp { .. }
        | Instruction::UnaryOp { .. }
        | Instruction::Cmp { .. }
        | Instruction::Cast { .. }
        | Instruction::Copy { .. }
        | Instruction::Select { .. }
        | Instruction::GetElementPtr { .. }
        | Instruction::GlobalAddr { .. } => true,
        Instruction::Load { volatile, .. } => !*volatile,
        _ => false,
    }
}

/// The type a merge of `inst`'s value and of its copy has, when the
/// instruction states one. A comparison's result and a copy's do not: the IR
/// gives them the type of whatever reads them, and a phi has to declare its
/// own. Such a value is merged nowhere ([`plan_rotation`] refuses the loop
/// that would need it).
fn merge_type(inst: &Instruction) -> Option<IrType> {
    match inst {
        Instruction::Phi { ty, .. }
        | Instruction::BinOp { ty, .. }
        | Instruction::UnaryOp { ty, .. }
        | Instruction::Load { ty, .. }
        | Instruction::Select { ty, .. } => Some(*ty),
        Instruction::Cast { to_ty, .. } => Some(*to_ty),
        Instruction::GetElementPtr { .. } | Instruction::GlobalAddr { .. } => Some(IrType::Ptr),
        _ => None,
    }
}

/// Whether `inst` names `value` in a place that takes a value and nothing
/// else (the address of a load, the base of an address computation): a
/// constant cannot stand there.
fn names_value_directly(inst: &Instruction, value: u32) -> bool {
    let mut probe = inst.clone();
    replace_operand_value(&mut probe, Value(value), Operand::Const(IrConst::I64(0)));
    let mut still = false;
    probe.for_each_used_value(|v| still |= v == value);
    still
}

/// Every question that can refuse a rotation, and what the rotation needs
/// once none did. Reads the function and changes nothing.
fn plan_rotation(func: &IrFunction, lp: &NaturalLoop, shape: &Shape<'_>) -> Option<Plan> {
    let h = lp.header;
    let Some(header) = func.blocks.get(h) else {
        return None;
    };
    macro_rules! refuse {
        ($($arg:tt)*) => {{
            trace!(shape.debug, "header .LBB{}: not rotated: {}", header.label.0, format_args!($($arg)*));
            return None;
        }};
    }
    let in_loop = |label: BlockId| {
        shape
            .index_of
            .get(&label)
            .is_some_and(|i| lp.body.contains(i))
    };

    // 1. The header decides whether the loop runs.
    if h == 0 {
        refuse!("the header is the function's entry block");
    }
    if shape.dead_code_feeds_live {
        refuse!("a block no path reaches branches into reachable code");
    }
    let Terminator::CondBranch {
        cond,
        true_label,
        false_label,
    } = &header.terminator
    else {
        refuse!("the header does not end in a conditional branch");
    };
    let (entry, exit, continue_on_true) = match (in_loop(*true_label), in_loop(*false_label)) {
        (true, false) => (*true_label, *false_label, true),
        (false, true) => (*false_label, *true_label, false),
        _ => refuse!("the header's branch does not choose between the loop and its exit"),
    };
    if entry == header.label {
        refuse!("the header is its own body: bottom-tested already");
    }
    if !shape
        .cfg
        .preds
        .row(h)
        .iter()
        .any(|&p| !lp.body.contains(&(p as usize)))
    {
        refuse!("nothing enters the header from outside the loop");
    }

    // 2. The back edges. A latch that also leaves the loop is the bottom
    //    test of a loop that is rotated already: rotating it again would
    //    copy its first block's test once per run of the pass and never
    //    reach a fixed point.
    let mut latches = lp.latches(&shape.cfg.preds);
    latches.sort_unstable();
    latches.dedup();
    if latches.is_empty() {
        refuse!("no back edge");
    }
    for &latch in &latches {
        let block = &func.blocks[latch];
        let mut leaves = false;
        block.terminator.for_each_target(|t| leaves |= !in_loop(t));
        if leaves {
            refuse!(
                "latch .LBB{} leaves the loop: bottom-tested already",
                block.label.0
            );
        }
        if matches!(block.terminator, Terminator::IndirectBranch { .. })
            || analysis::asm_goto_successors(block).any(|t| t == header.label)
        {
            refuse!(
                "latch .LBB{} reaches the header over an edge that cannot be redirected",
                block.label.0
            );
        }
    }
    let latch_labels: Vec<BlockId> = latches.iter().map(|&l| func.blocks[l].label).collect();

    // 3. The header: phis, then what the guard and the bottom test both run.
    let mut copied = 0usize;
    let mut defined: FxHashSet<u32> = FxHashSet::default();
    let mut typeless: FxHashSet<u32> = FxHashSet::default();
    for inst in &header.instructions {
        match inst {
            Instruction::Phi { incoming, .. } => {
                if copied > 0 {
                    refuse!("a phi follows an instruction in the header");
                }
                for latch in &latch_labels {
                    if incoming.iter().filter(|(_, from)| from == latch).count() != 1 {
                        refuse!(
                            "a header phi does not name latch .LBB{} exactly once",
                            latch.0
                        );
                    }
                }
            }
            other => {
                if !is_duplicable(other) {
                    refuse!("the header holds an instruction that is not duplicated: {other:?}");
                }
                copied += 1;
            }
        }
        if let Some(dest) = inst.dest() {
            defined.insert(dest.0);
            if merge_type(inst).is_none() {
                typeless.insert(dest.0);
            }
        }
    }
    if copied > MAX_HEADER_INSTS {
        refuse!("the header has {copied} instructions to duplicate, limit {MAX_HEADER_INSTS}");
    }

    // 4. What the back edges hand a header phi. A value of the header itself
    //    is merged at the loop's new header and needs a type for that; a
    //    constant has to fit wherever the bottom test reads the phi.
    for inst in &header.instructions {
        let Instruction::Phi { dest, incoming, .. } = inst else {
            continue;
        };
        let over_back_edges: Vec<Operand> = incoming
            .iter()
            .filter(|(_, from)| latch_labels.contains(from))
            .map(|(op, _)| *op)
            .collect();
        for op in &over_back_edges {
            if let Operand::Value(v) = op
                && typeless.contains(&v.0)
            {
                refuse!(
                    "header phi v{} takes v{}, a header value without a type to merge it with",
                    dest.0,
                    v.0
                );
            }
        }
        // One constant over every back edge is what the bottom test reads
        // in the phi's place (several different values get a phi of their
        // own in the latch).
        let one_constant = matches!(over_back_edges.first(), Some(Operand::Const(_)))
            && over_back_edges.iter().all(|op| *op == over_back_edges[0]);
        if one_constant
            && header
                .instructions
                .iter()
                .any(|i| !matches!(i, Instruction::Phi { .. }) && names_value_directly(i, dest.0))
        {
            refuse!(
                "header phi v{} takes a constant where the test names it directly",
                dest.0
            );
        }
    }

    // 5. Who reads the header's values outside it. Each such reader gets the
    //    definition that reaches it, through a merge where two do.
    for (index, block) in func.blocks.iter().enumerate() {
        if index == h {
            continue;
        }
        let mut reads = false;
        let mut reads_typeless = None;
        let mut see = |v: u32| {
            if defined.contains(&v) {
                reads = true;
                if typeless.contains(&v) {
                    reads_typeless = Some(v);
                }
            }
        };
        for inst in &block.instructions {
            inst.for_each_used_value(&mut see);
        }
        block.terminator.for_each_used_value(&mut see);
        if let Some(v) = reads_typeless {
            refuse!(
                "v{v} is read in .LBB{} and has no type to merge it with",
                block.label.0
            );
        }
        if reads && !shape.reached(index) {
            refuse!(
                ".LBB{} reads a header value and no path reaches it",
                block.label.0
            );
        }
    }

    // 6. The phis of the two blocks the header branches to name it once.
    for target in [entry, exit] {
        let Some(&index) = shape.index_of.get(&target) else {
            refuse!("the header branches to a block that does not exist");
        };
        for inst in &func.blocks[index].instructions {
            if let Instruction::Phi { incoming, .. } = inst
                && incoming
                    .iter()
                    .filter(|(_, from)| *from == header.label)
                    .count()
                    != 1
            {
                refuse!(
                    "a phi of .LBB{} does not name the header exactly once",
                    target.0
                );
            }
        }
    }

    Some(Plan {
        header: header.label,
        entry,
        exit,
        continue_on_true,
        cond: *cond,
        latches: latch_labels,
        body: lp
            .body
            .iter()
            .filter(|&&b| b != h)
            .map(|&b| func.blocks[b].label)
            .collect(),
        affine_guard_proven: affine_fold::header_test_is_proven_affine(func, lp, shape.cfg),
    })
}

/// Point every edge of `term` that names `from` at `to`.
fn redirect(term: &mut Terminator, from: BlockId, to: BlockId) {
    let swap = |label: &mut BlockId| {
        if *label == from {
            *label = to;
        }
    };
    match term {
        Terminator::Branch(target) => swap(target),
        Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } => {
            swap(true_label);
            swap(false_label);
        }
        Terminator::Switch { cases, default, .. } => {
            cases.iter_mut().for_each(|(_, target)| swap(target));
            swap(default);
        }
        Terminator::IndirectBranch { .. } | Terminator::Return(_) | Terminator::Unreachable => {}
    }
}

fn empty_block(label: BlockId, to: BlockId) -> BasicBlock {
    BasicBlock {
        label,
        instructions: Vec::new(),
        terminator: Terminator::Branch(to),
        source_spans: Vec::new(),
        source_inline_ids: Vec::new(),
    }
}

/// The index of the block labelled `label`. Every label [`apply_rotation`]
/// asks for was found in the function by [`plan_rotation`] or put there by
/// `apply_rotation` itself.
fn index_of(func: &IrFunction, label: BlockId) -> usize {
    func.blocks
        .iter()
        .position(|b| b.label == label)
        .expect("loop rotation: a block of the plan is in the function")
}

/// Fresh value and label numbers for one rotation.
struct Fresh {
    value: u32,
    label: u32,
}

impl Fresh {
    fn of(func: &IrFunction) -> Self {
        let past_labels = func
            .blocks
            .iter()
            .map(|b| b.label.0)
            .max()
            .map_or(1, |m| m.saturating_add(1));
        Self {
            // The scan, not the cached counter alone: an inline-asm output
            // is a definition `dest()` does not show, and a pass in front
            // that forgot the counter must not make this one reuse a number.
            value: func.sound_next_value_id(),
            label: func.next_label.max(past_labels),
        }
    }

    fn value(&mut self) -> u32 {
        let v = self.value;
        self.value += 1;
        v
    }

    fn label(&mut self) -> BlockId {
        let l = BlockId(self.label);
        self.label = self.label.saturating_add(1);
        l
    }
}

/// Give the loop one latch that ends in `Branch(header)`: the place the
/// bottom test goes. A loop with a single such latch has it already; one with
/// several back edges, or with a back edge that is one arm of a conditional
/// branch, gets a new block that all of them reach, with a phi for every
/// header phi the back edges disagree on.
fn single_latch(func: &mut IrFunction, plan: &Plan, fresh: &mut Fresh) -> (BlockId, bool) {
    if let [only] = plan.latches.as_slice() {
        let block = &func.blocks[index_of(func, *only)];
        if matches!(block.terminator, Terminator::Branch(to) if to == plan.header) {
            return (*only, false);
        }
    }
    let latch = fresh.label();
    let mut merges = Vec::new();
    let h = index_of(func, plan.header);
    for inst in &mut func.blocks[h].instructions {
        let Instruction::Phi { ty, incoming, .. } = inst else {
            continue;
        };
        let (over_back_edges, mut rest): (Vec<_>, Vec<_>) = incoming
            .drain(..)
            .partition(|(_, from)| plan.latches.contains(from));
        let first = over_back_edges[0].0;
        let next = if over_back_edges.iter().all(|(op, _)| *op == first) {
            first
        } else {
            let dest = Value(fresh.value());
            merges.push(Instruction::Phi {
                dest,
                ty: *ty,
                incoming: over_back_edges,
            });
            Operand::Value(dest)
        };
        rest.push((next, latch));
        *incoming = rest;
    }
    for from in &plan.latches {
        let at = index_of(func, *from);
        redirect(&mut func.blocks[at].terminator, plan.header, latch);
    }
    // Behind the loop's last block, so the back edge stays a backward jump.
    let after = func
        .blocks
        .iter()
        .rposition(|b| b.label == plan.header || plan.body.contains(&b.label))
        .map_or(func.blocks.len(), |last| last + 1);
    let mut block = empty_block(latch, plan.header);
    block.instructions = merges;
    func.blocks.insert(after, block);
    (latch, true)
}

/// A merge [`Merges`] has decided on: `dest = phi` of `of` at the top of
/// `block`.
struct Merge {
    block: BlockId,
    dest: u32,
    /// The header value this is a merge of.
    of: u32,
    ty: IrType,
    incoming: Vec<(Operand, BlockId)>,
}

/// The definitions of the header's values after the rotation, and the merges
/// their readers need.
///
/// Each value `v` the header defines has two definitions once the header's
/// instructions are copied into the latch: `v` itself, at the end of the
/// guard, and its counterpart at the end of the latch (the copy of the
/// instruction, or for a header phi what the latch passed over the back
/// edge). No other block defines it, so the value a reader sees is settled
/// by where the reader is:
///
/// * in the preheader: `v`, the guard is its only predecessor;
/// * in the loop: the merge at the loop's header, which every block of the
///   loop is dominated by and which both definitions enter;
/// * behind the loop: found by walking up the predecessors, with a merge at
///   each block several of them reach (Braun et al., "Simple and Efficient
///   Construction of Static Single Assignment Form"; the merges that turn
///   out to join one value with itself are removed by [`Merges::settle`]).
///
/// Merges are made on demand, so a value nobody reads behind the loop gets
/// none there.
struct Merges {
    guard: BlockId,
    preheader: BlockId,
    header: BlockId,
    latch: BlockId,
    /// The loop's blocks after the rotation.
    in_loop: FxHashSet<BlockId>,
    /// The values the old header defines.
    header_values: FxHashSet<u32>,
    /// The merge type of each header value that has one.
    ty_of: FxHashMap<u32, IrType>,
    /// What the latch passed each header phi over the back edge.
    next: FxHashMap<u32, Operand>,
    /// The copy of each header instruction in the latch.
    copy_of: FxHashMap<u32, u32>,
    /// The predecessors of every block, after the rotation's edges changed.
    preds: FxHashMap<BlockId, Vec<BlockId>>,
    blocks: usize,
    at_entry: FxHashMap<(u32, BlockId), Operand>,
    merges: Vec<Merge>,
    unfilled: Vec<usize>,
}

impl Merges {
    fn merge(&mut self, v: u32, block: BlockId, fresh: &mut Fresh) -> Operand {
        let dest = fresh.value();
        // `plan_rotation` refused every loop in which a value without a
        // merge type would be merged.
        debug_assert!(
            self.ty_of.contains_key(&v),
            "merge of an untyped header value"
        );
        let ty = self.ty_of.get(&v).copied().unwrap_or(IrType::I64);
        self.merges.push(Merge {
            block,
            dest,
            of: v,
            ty,
            incoming: Vec::new(),
        });
        self.unfilled.push(self.merges.len() - 1);
        let merged = Operand::Value(Value(dest));
        self.at_entry.insert((v, block), merged);
        merged
    }

    /// `v` as the loop's blocks see it.
    fn in_the_loop(&mut self, v: u32, fresh: &mut Fresh) -> Operand {
        match self.at_entry.get(&(v, self.header)) {
            Some(&known) => known,
            None => self.merge(v, self.header, fresh),
        }
    }

    /// `v` at the end of the latch, behind the copy of the header.
    fn at_latch_end(&mut self, v: u32, fresh: &mut Fresh) -> Operand {
        if let Some(&copy) = self.copy_of.get(&v) {
            return Operand::Value(Value(copy));
        }
        match self.next.get(&v).copied() {
            // The latch hands the phi a value of the header: the one this
            // trip saw, which is the merge at the loop's header.
            Some(Operand::Value(u)) if self.header_values.contains(&u.0) => {
                self.in_the_loop(u.0, fresh)
            }
            Some(passed) => passed,
            None => Operand::Value(Value(v)),
        }
    }

    /// `v` at the end of `block`.
    fn at_end(&mut self, v: u32, block: BlockId, fresh: &mut Fresh) -> Operand {
        if block == self.guard || block == self.preheader {
            Operand::Value(Value(v))
        } else if block == self.latch {
            self.at_latch_end(v, fresh)
        } else {
            // No other block defines a header value.
            self.at_start(v, block, fresh)
        }
    }

    /// `v` at the start of `block` (which is not the guard).
    fn at_start(&mut self, v: u32, block: BlockId, fresh: &mut Fresh) -> Operand {
        if block == self.preheader {
            return Operand::Value(Value(v));
        }
        if self.in_loop.contains(&block) {
            return self.in_the_loop(v, fresh);
        }
        // Behind the loop: up the single-predecessor chain to a block whose
        // answer is known, or to one several blocks reach.
        let mut walked = Vec::new();
        let mut at = block;
        let found = loop {
            if let Some(&known) = self.at_entry.get(&(v, at)) {
                break known;
            }
            let (count, first) = self
                .preds
                .get(&at)
                .map_or((0, None), |p| (p.len(), p.first().copied()));
            match first {
                Some(only) if count == 1 && walked.len() < self.blocks => {
                    walked.push(at);
                    if only == self.guard || only == self.preheader {
                        break Operand::Value(Value(v));
                    }
                    if only == self.latch {
                        break self.at_latch_end(v, fresh);
                    }
                    if self.in_loop.contains(&only) {
                        break self.in_the_loop(v, fresh);
                    }
                    at = only;
                }
                Some(_) if count > 1 => break self.merge(v, at, fresh),
                // A block nothing reaches: `plan_rotation` found no reader
                // of a header value in one, and no edge out of one.
                _ => break Operand::Value(Value(v)),
            }
        };
        for passed in walked {
            self.at_entry.insert((v, passed), found);
        }
        found
    }

    /// Give every merge its incoming values. Filling one can ask for more.
    fn fill(&mut self, fresh: &mut Fresh) {
        while let Some(i) = self.unfilled.pop() {
            let (v, block) = (self.merges[i].of, self.merges[i].block);
            let incoming = if block == self.header {
                vec![
                    (Operand::Value(Value(v)), self.preheader),
                    (self.at_latch_end(v, fresh), self.latch),
                ]
            } else {
                let preds = self.preds.get(&block).cloned().unwrap_or_default();
                preds
                    .into_iter()
                    .map(|pred| (self.at_end(v, pred, fresh), pred))
                    .collect()
            };
            self.merges[i].incoming = incoming;
        }
    }

    /// Drop the merges that join one value with itself, and return what each
    /// stood for. A merge made at a block that two paths from the same
    /// definition reach is one of them; so is the merge of a header phi the
    /// loop never changes.
    fn settle(&mut self) -> FxHashMap<u32, u32> {
        let mut stands_for: FxHashMap<u32, u32> = FxHashMap::default();
        let resolve = |mut op: Operand, stands_for: &FxHashMap<u32, u32>| {
            while let Operand::Value(v) = op
                && let Some(&to) = stands_for.get(&v.0)
            {
                op = Operand::Value(Value(to));
            }
            op
        };
        loop {
            let mut changed = false;
            for merge in &self.merges {
                if stands_for.contains_key(&merge.dest) {
                    continue;
                }
                let mut only = None;
                let mut one_value = true;
                for &(op, _) in &merge.incoming {
                    let op = resolve(op, &stands_for);
                    if matches!(op, Operand::Value(v) if v.0 == merge.dest) {
                        continue;
                    }
                    match only {
                        None => only = Some(op),
                        Some(seen) if seen == op => {}
                        Some(_) => {
                            one_value = false;
                            break;
                        }
                    }
                }
                // A constant is left merged: it could not replace the merge
                // where an instruction names it directly.
                if one_value && let Some(Operand::Value(to)) = only {
                    stands_for.insert(merge.dest, to.0);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        // Chains end: a merge never stands for itself.
        let flat: FxHashMap<u32, u32> = stands_for
            .keys()
            .map(|&from| {
                let Operand::Value(to) = resolve(Operand::Value(Value(from)), &stands_for) else {
                    return (from, from);
                };
                (from, to.0)
            })
            .collect();
        self.merges.retain(|m| !flat.contains_key(&m.dest));
        for merge in &mut self.merges {
            for (op, _) in &mut merge.incoming {
                if let Operand::Value(v) = op
                    && let Some(&to) = flat.get(&v.0)
                {
                    *op = Operand::Value(Value(to));
                }
            }
        }
        flat
    }
}

/// The predecessors of every block, each named once.
fn predecessors(func: &IrFunction) -> FxHashMap<BlockId, Vec<BlockId>> {
    let mut preds: FxHashMap<BlockId, Vec<BlockId>> = FxHashMap::default();
    for block in &func.blocks {
        let mut add = |to: BlockId| {
            let list = preds.entry(to).or_default();
            if !list.contains(&block.label) {
                list.push(block.label);
            }
        };
        block.terminator.for_each_target(&mut add);
        analysis::asm_goto_successors(block).for_each(&mut add);
    }
    preds
}

/// Rotate the loop `plan` describes. Cannot fail: everything that could was
/// asked by [`plan_rotation`].
fn apply_rotation(func: &mut IrFunction, plan: &Plan, debug: bool) -> Rotated {
    let mut fresh = Fresh::of(func);

    // 1. One latch, ending in `Branch(header)`.
    let (latch, latch_is_new) = single_latch(func, plan, &mut fresh);

    // 2. The loop's header. The body's first block serves when the old
    //    header is all that reaches it; a block other loop blocks branch to
    //    (the header of an inner loop, the target of a `goto`) or one that
    //    has phis keeps its edges and gets an empty block in front.
    let entry_index = index_of(func, plan.entry);
    let reaches_entry = func
        .blocks
        .iter()
        .filter(|b| {
            let mut reaches = false;
            b.terminator.for_each_target(|t| reaches |= t == plan.entry);
            reaches || analysis::asm_goto_successors(b).any(|t| t == plan.entry)
        })
        .count();
    let entry_has_phis = func.blocks[entry_index]
        .instructions
        .iter()
        .any(|i| matches!(i, Instruction::Phi { .. }));
    let header = if reaches_entry == 1 && !entry_has_phis {
        plan.entry
    } else {
        let header = fresh.label();
        for inst in &mut func.blocks[entry_index].instructions {
            if let Instruction::Phi { incoming, .. } = inst {
                for (_, from) in incoming.iter_mut().filter(|(_, from)| *from == plan.header) {
                    *from = header;
                }
            }
        }
        func.blocks
            .insert(entry_index, empty_block(header, plan.entry));
        header
    };

    // 3. The preheader, on the guard's edge into the loop, placed so that
    //    the loop is entered by falling through.
    let preheader = fresh.label();
    let header_index = index_of(func, header);
    func.blocks
        .insert(header_index, empty_block(preheader, header));
    let guard_index = index_of(func, plan.header);
    redirect(
        &mut func.blocks[guard_index].terminator,
        plan.entry,
        preheader,
    );

    // 4. The header's values: what the latch hands each phi (its incoming
    //    over the back edge, which the guard no longer has) and a number for
    //    the copy of each instruction.
    let mut merges = Merges {
        guard: plan.header,
        preheader,
        header,
        latch,
        in_loop: plan.body.clone(),
        header_values: FxHashSet::default(),
        ty_of: FxHashMap::default(),
        next: FxHashMap::default(),
        copy_of: FxHashMap::default(),
        preds: FxHashMap::default(),
        blocks: 0,
        at_entry: FxHashMap::default(),
        merges: Vec::new(),
        unfilled: Vec::new(),
    };
    merges.in_loop.insert(header);
    merges.in_loop.insert(latch);
    let mut originals = Vec::new();
    for inst in &mut func.blocks[guard_index].instructions {
        if let Some(dest) = inst.dest() {
            merges.header_values.insert(dest.0);
            if let Some(ty) = merge_type(inst) {
                merges.ty_of.insert(dest.0, ty);
            }
        }
        if let Instruction::Phi { dest, incoming, .. } = inst {
            if let Some(at) = incoming.iter().position(|(_, from)| *from == latch) {
                merges.next.insert(dest.0, incoming.remove(at).0);
            }
        } else {
            if let Some(dest) = inst.dest() {
                merges.copy_of.insert(dest.0, fresh.value());
            }
            originals.push(inst.clone());
        }
    }

    // 5. The copy of the header's instructions, reading this trip's values.
    let mut copies = Vec::with_capacity(originals.len());
    for original in &originals {
        let mut copy = original.clone();
        let mut reads = Vec::new();
        copy.for_each_used_value(|v| {
            if merges.header_values.contains(&v) && !reads.contains(&v) {
                reads.push(v);
            }
        });
        for v in reads {
            let now = merges.at_latch_end(v, &mut fresh);
            replace_operand_value(&mut copy, Value(v), now);
        }
        rename_inst_dest(&mut copy, &merges.copy_of);
        copies.push(copy);
    }
    let cond = match plan.cond {
        Operand::Value(c) if merges.header_values.contains(&c.0) => {
            merges.at_latch_end(c.0, &mut fresh)
        }
        other => other,
    };
    if plan.affine_guard_proven {
        let proved = match plan.cond {
            Operand::Value(c) => merges.copy_of.get(&c.0).copied().map(Value),
            Operand::Const(_) => None,
        };
        let folded = affine_fold::canonicalise_affine_exit_cmps(&mut copies, proved);
        trace!(debug && folded > 0, "affine exit-compare folds: {folded}");
    }

    // 6. The bottom test, with the header's polarity: an exit-on-true header
    //    needs an exit-on-true latch.
    let latch_index = index_of(func, latch);
    let latch_block = &mut func.blocks[latch_index];
    let latch_own = latch_block.instructions.len();
    latch_block.extend_insts(copies, Span::dummy());
    let (true_label, false_label) = if plan.continue_on_true {
        (header, plan.exit)
    } else {
        (plan.exit, header)
    };
    latch_block.terminator = Terminator::CondBranch {
        cond,
        true_label,
        false_label,
    };

    // 7. The exit block is reached from the latch now as well: each of its
    //    phis takes over that edge what it took from the header (step 8
    //    turns that into the latch's value).
    let exit_index = index_of(func, plan.exit);
    for inst in &mut func.blocks[exit_index].instructions {
        if let Instruction::Phi { incoming, .. } = inst
            && let Some(&(from_header, _)) = incoming.iter().find(|(_, from)| *from == plan.header)
        {
            incoming.push((from_header, latch));
        }
    }

    // 8. Every reader of a header value outside the guard: the definition
    //    that reaches it. A phi reads at the end of the block its incoming
    //    names; anything else at the start of its own block (no block but
    //    the guard and the latch's copy defines a header value).
    merges.preds = predecessors(func);
    merges.blocks = func.blocks.len();
    for block in &mut func.blocks {
        let label = block.label;
        if label == plan.header {
            continue;
        }
        let own = if label == latch {
            latch_own
        } else {
            block.instructions.len()
        };
        for inst in &mut block.instructions[..own] {
            if let Instruction::Phi { incoming, .. } = inst {
                for (op, from) in incoming.iter_mut() {
                    if let Operand::Value(v) = *op
                        && merges.header_values.contains(&v.0)
                    {
                        *op = merges.at_end(v.0, *from, &mut fresh);
                    }
                }
                continue;
            }
            let mut reads = Vec::new();
            inst.for_each_used_value(|v| {
                if merges.header_values.contains(&v) && !reads.contains(&v) {
                    reads.push(v);
                }
            });
            for v in reads {
                let now = merges.at_start(v, label, &mut fresh);
                debug_assert!(
                    matches!(now, Operand::Value(_)),
                    "a reader's value is a value"
                );
                replace_operand_value(inst, Value(v), now);
            }
        }
        if label != latch {
            let mut reads = Vec::new();
            block.terminator.for_each_used_value(|v| {
                if merges.header_values.contains(&v) && !reads.contains(&v) {
                    reads.push(v);
                }
            });
            for v in reads {
                let now = merges.at_start(v, label, &mut fresh);
                replace_terminator_value(&mut block.terminator, Value(v), now);
            }
        }
    }

    // 9. The merges themselves: filled, the ones that join a value with
    //    itself removed, the rest put at the top of their blocks.
    merges.fill(&mut fresh);
    let stands_for = merges.settle();
    if !stands_for.is_empty() {
        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                replace_values_in_inst_map(inst, &stands_for);
            }
            replace_values_in_terminator_map(&mut block.terminator, &stands_for);
        }
    }
    let mut by_block: FxHashMap<BlockId, Vec<Instruction>> = FxHashMap::default();
    for merge in merges.merges.drain(..) {
        by_block
            .entry(merge.block)
            .or_default()
            .push(Instruction::Phi {
                dest: Value(merge.dest),
                ty: merge.ty,
                incoming: merge.incoming,
            });
    }
    for block in &mut func.blocks {
        if let Some(phis) = by_block.remove(&block.label) {
            block.insert_insts(0, phis, Span::dummy());
        }
    }

    // 10. The guard's phis lost their incoming over the back edge. Where
    //     that leaves one way in they are copies, and are written as copies:
    //     a phi with one incoming in a block that heads no loop is a shape
    //     the address analyses do not look through (LICM's and the
    //     forwarder's linear forms stop at it), and the hoisting this
    //     rotation is for runs before any cleanup would rewrite it. All of
    //     the guard's phis name the same predecessors, so it is all or none;
    //     an incoming from the guard's one predecessor is defined in front
    //     of the guard, never by another of its phis, so the order of the
    //     copies does not matter.
    let guard = &mut func.blocks[guard_index];
    let phis = guard
        .instructions
        .iter()
        .take_while(|inst| matches!(inst, Instruction::Phi { .. }))
        .count();
    let one_way_in = guard.instructions[..phis]
        .iter()
        .all(|inst| matches!(inst, Instruction::Phi { incoming, .. } if incoming.len() == 1));
    if one_way_in {
        for inst in &mut guard.instructions[..phis] {
            if let Instruction::Phi { dest, incoming, .. } = inst {
                *inst = Instruction::Copy {
                    dest: *dest,
                    src: incoming[0].0,
                };
            }
        }
    }

    func.next_value_id = fresh.value;
    func.next_label = fresh.label;
    trace!(
        debug,
        "SUCCESS: rotated header=.LBB{} preheader=.LBB{} loop=.LBB{} latch=.LBB{}{}",
        plan.header.0,
        preheader.0,
        header.0,
        latch.0,
        if latch_is_new { " (new)" } else { "" }
    );
    Rotated {
        guard: plan.header,
        preheader,
        header,
    }
}

/// The loops of `func`, innermost first (ties by header, so two runs over the
/// same function agree), with the analysis they were found in.
fn loops_innermost_first(func: &IrFunction) -> (CfgAnalysis, Vec<NaturalLoop>) {
    let cfg = CfgAnalysis::build(func);
    let mut loops = merge_loops_by_header(find_natural_loops(
        cfg.num_blocks,
        &cfg.preds,
        &cfg.succs,
        &cfg.idom,
    ));
    loops.sort_by_key(|lp| (lp.body.len(), lp.header));
    (cfg, loops)
}

/// Rotate the loops of `func` that `wanted` picks, innermost first, each
/// against an analysis of the function as the rotations before it left it.
/// Returns what each rotation left behind, in the order they were made.
fn rotate_loops_where(
    func: &mut IrFunction,
    mut wanted: impl FnMut(&IrFunction, &NaturalLoop) -> bool,
) -> Vec<Rotated> {
    let debug = rotate_debug();
    let mut done = Vec::new();
    if func.blocks.len() < 2 {
        return done;
    }
    // A header that was looked at is not looked at again: a refusal does not
    // change with the rotation of another loop in ways worth a second
    // analysis, and a rotated loop is bottom-tested.
    let mut seen: FxHashSet<BlockId> = FxHashSet::default();
    while done.len() < MAX_ROTATIONS_PER_FUNC {
        let (cfg, loops) = loops_innermost_first(func);
        let shape = Shape::of(func, &cfg, debug);
        let plan = loops.iter().find_map(|lp| {
            let label = func.blocks[lp.header].label;
            if !seen.insert(label) || !wanted(func, lp) {
                return None;
            }
            plan_rotation(func, lp, &shape)
        });
        let Some(plan) = plan else {
            break;
        };
        done.push(apply_rotation(func, &plan, debug));
    }
    done
}

/// [`RotatePolicy::Always`]: rotate every loop of `func` that can be.
/// Returns the number of loops rotated.
pub(crate) fn rotate_every_loop(func: &mut IrFunction) -> usize {
    rotate_loops_where(func, |_, _| true).len()
}

/// Most pure instructions in the header of a loop [`rotate_fed_loops`]
/// takes: the bound of the pass it stands in for
/// (`loop_invert::MAX_DUP_INSTS`).
const MAX_FED_HEADER_INSTS: usize = 8;

/// Whether `lp` is a loop that machine-level inversion will leave
/// top-tested for one reason only, that its header computes a value the
/// rest of the function reads, and that is worth rotating here instead:
///
/// 1. The header is phis and then a handful of pure instructions:
///    inversion's own list and bound. No load: a header that reads memory
///    is the one case inversion refuses on measurement (its rule 2), and
///    that measurement is not revisited here.
/// 2. A latch jumps back to it unconditionally: the jump a bottom test
///    saves. (A latch that branches is a loop bottom-tested already.)
/// 3. Something outside the header reads a value the header computes: the
///    body, the code behind the loop, or a header phi over a back edge.
///    Without that, inversion takes the loop and is the better tool: its
///    copy of the test starts a live range of its own, and its two folds
///    (the value bound, the down-counter) apply to what it rotated.
/// 4. With `settled`: a header phi whose next value the header computes is
///    read by nothing behind that computation: not later in the header,
///    not in the body, not behind the loop, not by another phi. The
///    rotated loop goes round with the *next* value, and each such reader
///    keeps the old one alive beside it: a second register through the
///    loop and a copy a trip between the two. Where the reader stands at
///    the bottom next to the new value, or behind the exit, the copy
///    cannot go into the latch either; the back edge then gets a block of
///    its own, and the loop has two branches a trip again plus that
///    block's copies of *every* carried value: worse than the top-tested
///    loop it was. (A loop copying eight bytes a trip that steps its
///    pointer at the top and loads through the old one cost
///    `expat_siphash24` 5% of its instructions that way.) The test of
///    `while (n--)` is such a read. `retarget_loop_tests` moves it onto
///    the stepped value where it can (an equality test), which is why the
///    question is asked again after it ran; `while (n-- > 0)` on a signed
///    counter, where it cannot, stays top-tested.
/// Clause 1 of [`header_feeds_the_loop`], which a block answers alone: it
/// starts with a phi, goes on with at most [`MAX_FED_HEADER_INSTS`] pure
/// instructions that are no load, at least one of which defines a value,
/// and ends in a conditional branch (the test: a rotation needs one). Most
/// functions have no such block, and for them [`rotate_fed_loops`] is over
/// before it has looked for a loop.
fn may_be_fed_header(block: &BasicBlock) -> bool {
    if !matches!(block.instructions.first(), Some(Instruction::Phi { .. }))
        || !matches!(block.terminator, Terminator::CondBranch { .. })
    {
        return false;
    }
    let mut pure = 0usize;
    let mut defines = false;
    for inst in &block.instructions {
        match inst {
            Instruction::Phi { .. } => {}
            Instruction::Load { .. } => return false,
            other if is_duplicable(other) => {
                pure += 1;
                defines |= other.dest().is_some();
            }
            _ => return false,
        }
    }
    defines && pure <= MAX_FED_HEADER_INSTS
}

fn header_feeds_the_loop(func: &IrFunction, lp: &NaturalLoop, settled: bool) -> bool {
    let Some(header) = func.blocks.get(lp.header) else {
        return false;
    };
    // 1. Each value the header computes, and where.
    if !may_be_fed_header(header) {
        return false;
    }
    let computed: FxHashMap<u32, usize> = header
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, inst)| !matches!(inst, Instruction::Phi { .. }))
        .filter_map(|(at, inst)| inst.dest().map(|dest| (dest.0, at)))
        .collect();
    // 2.
    let jumps_back = lp.body.iter().any(|&b| {
        b != lp.header
            && func.blocks.get(b).is_some_and(
                |block| matches!(block.terminator, Terminator::Branch(to) if to == header.label),
            )
    });
    if !jumps_back {
        return false;
    }
    // 3.
    let read_outside = func.blocks.iter().enumerate().any(|(index, block)| {
        let mut reads = false;
        if index == lp.header {
            for inst in &block.instructions {
                if let Instruction::Phi { incoming, .. } = inst {
                    reads |= incoming.iter().any(
                        |(operand, _)| matches!(operand, Operand::Value(v) if computed.contains_key(&v.0)),
                    );
                }
            }
        } else {
            let mut see = |v: u32| reads |= computed.contains_key(&v);
            for inst in &block.instructions {
                inst.for_each_used_value(&mut see);
            }
            block.terminator.for_each_used_value(&mut see);
        }
        reads
    });
    if !read_outside {
        return false;
    }
    // 4. The phis the header steps, each with the place of its step (the
    // first, where it is handed more than one).
    if settled {
        let mut stepped: FxHashMap<u32, usize> = FxHashMap::default();
        for inst in &header.instructions {
            let Instruction::Phi { dest, incoming, .. } = inst else {
                continue;
            };
            let next_at = incoming
                .iter()
                .filter_map(|(operand, _)| match operand {
                    Operand::Value(v) => computed.get(&v.0).copied(),
                    Operand::Const(_) => None,
                })
                .min();
            if let Some(next_at) = next_at {
                stepped.insert(dest.0, next_at);
            }
        }
        if !stepped.is_empty() {
            for (index, block) in func.blocks.iter().enumerate() {
                let mut read_behind = false;
                for (at, inst) in block.instructions.iter().enumerate() {
                    // Up to its step, in the header, the old value is the
                    // only one there is. A phi reads on an edge, behind
                    // whatever the block it comes from computed.
                    let ahead = index == lp.header && !matches!(inst, Instruction::Phi { .. });
                    inst.for_each_used_value(|v| {
                        if let Some(&next_at) = stepped.get(&v) {
                            read_behind |= !(ahead && at <= next_at);
                        }
                    });
                }
                block
                    .terminator
                    .for_each_used_value(|v| read_behind |= stepped.contains_key(&v));
                if read_behind {
                    return false;
                }
            }
        }
    }
    true
}

/// Rotate the loops that would keep their test at the top for good. Returns
/// the number of loops rotated. Run whenever rotation is not off.
///
/// A loop is bottom-tested in the end by `loop_invert`, after phi
/// elimination, which copies the header's handful of pure instructions into
/// the latch. It can do so only while nothing outside the header reads what
/// the header computes: out of single assignment it has no way to give such
/// a reader the latch's copy. `while (n--)`, `while (--i >= 0)`, `while
/// (i++ != n)` and `for (;;) { x = step(x); if (!x) break; … }` all compute
/// at the top what the body reads, the stepped counter, and stayed
/// top-tested: a branch not taken and a jump back on every trip. On
/// `while (n--) *d++ = *s++;` (819 million trips, this host, best of 9)
/// that loop took 0.405 s as it was, 0.377 s once its test read the
/// stepped counter, and takes 0.348 s rotated.
///
/// Here every reader can still be given its definition. The pass runs
/// behind everything that looks at a loop's shape, so none of those sees a
/// rotated loop (the reason the default policy does not rotate early), and
/// it takes exactly the loops of [`header_feeds_the_loop`]: the ones
/// inversion will refuse for that one reason and whose bottom test will be
/// tight. A loop's test is moved onto the value the loop goes round with
/// first (`retarget_loop_tests`), which is what makes `while (n--)` one of
/// them.
pub(crate) fn rotate_fed_loops(func: &mut IrFunction) -> usize {
    if func.blocks.len() < 2 || !func.blocks.iter().any(may_be_fed_header) {
        return 0;
    }
    let (_, loops) = loops_innermost_first(func);
    if !loops
        .iter()
        .any(|lp| header_feeds_the_loop(func, lp, false))
    {
        return 0;
    }
    crate::ir::mem2reg::phi_eliminate::retarget_loop_tests(func);
    let done = rotate_loops_where(func, |func, lp| header_feeds_the_loop(func, lp, true));
    // Said of the rotations made, not of the loops picked: `plan_rotation`
    // may still refuse one, and says so itself.
    let debug = rotate_debug();
    for rotated in &done {
        trace!(
            debug,
            "{}: header .LBB{}: computes what the loop reads: rotated so that it is bottom-tested",
            func.name,
            rotated.guard.0
        );
    }
    done.len()
}

/// Whether the body of `lp` holds something that has to stay in a
/// top-tested loop because it must not run unless the body does:
///
/// * a load, a division or remainder, or a call that only reads, whose
///   operands the loop does not compute. (Anything else that is
///   loop-invariant LICM hoists without a guard; this runs after it has. A
///   memory recurrence the promoter could take starts with such a load.)
/// * a load through an address the loop computes, when the loop also stores
///   a value of that type through an address it computes: the shape of
///   `a[i] = f(a[i-1])`, whose first load loop-carried forwarding may only
///   put in front of a loop that is known to run.
///
/// Two things rule a loop out whatever it holds, because each of the three
/// passes that move such an instruction refuses them:
///
/// * The instruction is in a block some trip skips (one that does not
///   dominate every latch). What is not certain to run on a trip is not
///   certain to run on the first, and none of the passes speculates it.
/// * For a load: the loop calls a function that may write memory, or holds
///   anything else that writes it other than a plain store. LICM, the
///   memory promoter and loop-carried forwarding all take such a loop's
///   memory for unknown.
///
/// The test is a filter for [`rotate_for_hoisting`], not its decision: it
/// keeps the trial rotation, which copies the function, away from loops in
/// which there is nothing to try. Measured when it was written, over the
/// 1,493 files the regression corpus then had: the two exclusions left
/// 1,161 of the 2,232 trials the clauses above ask for, and no loop that
/// pays was among the ones they dropped.
fn has_guarded_hoist_candidate(func: &IrFunction, cfg: &CfgAnalysis, lp: &NaturalLoop) -> bool {
    let latches: Vec<usize> = cfg
        .preds
        .row(lp.header)
        .iter()
        .map(|&p| p as usize)
        .filter(|p| lp.body.contains(p))
        .collect();
    let runs_every_trip = |block: usize| {
        latches
            .iter()
            .all(|&latch| analysis::dominates_idom(&cfg.idom, block, latch))
    };

    let mut defined_in_loop: FxHashSet<u32> = FxHashSet::default();
    // The types stored through an address the loop computes.
    let mut stored: Vec<IrType> = Vec::new();
    // Whether the loop's memory is what its loads and plain stores say.
    let mut memory_is_known = true;
    for &b in &lp.body {
        for inst in &func.blocks[b].instructions {
            if let Some(dest) = inst.dest() {
                defined_in_loop.insert(dest.0);
            }
            memory_is_known &= match inst {
                Instruction::Store { .. } => true,
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    (info.is_pure || info.is_const) && !info.is_sret
                }
                other => !other.may_write_memory(),
            };
        }
    }
    if memory_is_known {
        for &b in &lp.body {
            for inst in &func.blocks[b].instructions {
                if let Instruction::Store {
                    ptr,
                    ty,
                    volatile: false,
                    ..
                } = inst
                    && defined_in_loop.contains(&ptr.0)
                    && !stored.contains(ty)
                {
                    stored.push(*ty);
                }
            }
        }
    }
    lp.body
        .iter()
        .filter(|&&b| b != lp.header && runs_every_trip(b))
        .any(|&b| {
            func.blocks[b].instructions.iter().any(|inst| {
                let mut invariant = true;
                inst.for_each_used_value(|v| invariant &= !defined_in_loop.contains(&v));
                match inst {
                    Instruction::Load {
                        volatile: false,
                        ty,
                        ..
                    } => memory_is_known && (invariant || stored.contains(ty)),
                    Instruction::BinOp { op, .. } => invariant && op.can_trap(),
                    Instruction::Call { info, .. } => invariant && (info.is_pure || info.is_const),
                    _ => false,
                }
            })
        })
}

/// What a load hoisted out of a loop is worth per trip, in ordinary
/// instructions: itself.
const LOAD_WORTH: u64 = 1;

/// What a load is worth that left a loop because its value is carried in a
/// register from one trip to the next (the memory promoter's `*p += x`,
/// loop-carried forwarding's `a[i] = f(a[i-1])`). Such a load read what the
/// trip before stored, so the loop ran at one trip per store-to-load
/// forwarding: four to five cycles on every current x86-64 and AArch64
/// core, where the register recurrence takes the one cycle of its
/// operation. Counted as eight instructions.
const CARRIED_WORTH: u64 = 8;

/// What a division, a remainder or a call that leaves a loop is worth. A
/// 64-bit `idiv` has a latency of 14 to 18 cycles on the newest cores and 40
/// to 90 before Ice Lake, `divsd` 13 to 15; a call costs its own body.
const HEAVY_WORTH: u64 = 24;

/// A rotation pays when what left the loop is worth at least one part in
/// this many of a trip. Below that the gain is under the three percent a
/// measurement resolves, while the rotation still changes the function for
/// every pass behind it; such a loop is better left as the rest of the
/// pipeline knows it.
const TRIP_SHARE: u64 = 32;

/// Deepest nesting [`trip_size`] distinguishes: a block that deep inside the
/// rotated loop counts ten to this power.
const MAX_WEIGHTED_DEPTH: u32 = 6;

/// The instructions of a set of blocks that must not run unless their loop's
/// body does: what a guarded preheader exists to take out. Each is known by
/// the value it defines. A pass that moves an instruction keeps its number
/// and one that makes an instruction gives it a number nothing had, so a
/// number says whether *this* instruction is still there, which a count
/// could not: the promoter takes a load out of a loop and puts another one
/// in front of it, and for a loop inside the one asked about both are in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Guarded {
    /// Non-volatile loads.
    loads: FxHashSet<u32>,
    /// Divisions and remainders.
    divisions: FxHashSet<u32>,
    /// Calls that only read (`pure`, `const`) and have a result.
    calls: FxHashSet<u32>,
}

impl Guarded {
    fn of(func: &IrFunction, in_set: impl Fn(BlockId) -> bool) -> Self {
        let mut found = Self::default();
        for block in func.blocks.iter().filter(|b| in_set(b.label)) {
            for inst in &block.instructions {
                match inst {
                    Instruction::Load {
                        dest,
                        volatile: false,
                        ..
                    } => {
                        found.loads.insert(dest.0);
                    }
                    Instruction::BinOp { dest, op, .. } if op.can_trap() => {
                        found.divisions.insert(dest.0);
                    }
                    Instruction::Call { info, .. } if info.is_pure || info.is_const => {
                        if let Some(dest) = info.dest {
                            found.calls.insert(dest.0);
                        }
                    }
                    _ => {}
                }
            }
        }
        found
    }

    fn is_empty(&self) -> bool {
        self.loads.is_empty() && self.divisions.is_empty() && self.calls.is_empty()
    }

    /// The instructions of `self` for which `keep` holds, by kind: `keep`
    /// is asked about the corresponding set of `other`.
    fn filtered(&self, other: &Self, keep: impl Fn(&FxHashSet<u32>, u32) -> bool) -> Self {
        let pick = |mine: &FxHashSet<u32>, theirs: &FxHashSet<u32>| -> FxHashSet<u32> {
            mine.iter().copied().filter(|&v| keep(theirs, v)).collect()
        };
        Self {
            loads: pick(&self.loads, &other.loads),
            divisions: pick(&self.divisions, &other.divisions),
            calls: pick(&self.calls, &other.calls),
        }
    }

    /// What of `self` is not in `other`.
    fn not_in(&self, other: &Self) -> Self {
        self.filtered(other, |theirs, v| !theirs.contains(&v))
    }

    /// What of `self` is in `other` as well.
    fn also_in(&self, other: &Self) -> Self {
        self.filtered(other, |theirs, v| theirs.contains(&v))
    }
}

/// The non-volatile loads of `func`, by the value they define.
fn loads_of(func: &IrFunction) -> FxHashSet<u32> {
    Guarded::of(func, |_| true).loads
}

/// What left a loop: the hoisted loads by value, the rest by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Left {
    /// Loads that run in front of the loop now.
    hoisted: Vec<u32>,
    /// Loads whose value a register carries from trip to trip now.
    carried: usize,
    divisions: usize,
    calls: usize,
}

impl Left {
    /// `gone` is what is in the loop no more. `loads_anywhere` are the
    /// loads of the whole function afterwards: a load that left the loop
    /// and is one of them was hoisted, one that is nowhere was replaced by
    /// a value carried in a register.
    fn of(gone: &Guarded, loads_anywhere: &FxHashSet<u32>) -> Self {
        let mut hoisted: Vec<u32> = gone
            .loads
            .iter()
            .copied()
            .filter(|load| loads_anywhere.contains(load))
            .collect();
        hoisted.sort_unstable();
        Self {
            carried: gone.loads.len() - hoisted.len(),
            hoisted,
            divisions: gone.divisions.len(),
            calls: gone.calls.len(),
        }
    }

    /// Hoisted loads and nothing else: the one gain small enough for a
    /// register to outweigh it (see [`loads_hold_no_more`]).
    fn is_hoisted_loads_only(&self) -> bool {
        self.carried == 0 && self.divisions == 0 && self.calls == 0
    }

    /// Per trip, in ordinary instructions.
    fn worth(&self) -> u64 {
        let n = |count: usize| u64::try_from(count).unwrap_or(u64::MAX);
        [
            (self.hoisted.len(), LOAD_WORTH),
            (self.carried, CARRIED_WORTH),
            (self.divisions.saturating_add(self.calls), HEAVY_WORTH),
        ]
        .into_iter()
        .map(|(count, worth)| n(count).saturating_mul(worth))
        .fold(0, u64::saturating_add)
    }
}

impl std::fmt::Display for Left {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut sep = "";
        for (count, what) in [
            (self.hoisted.len(), "load"),
            (self.carried, "carried load"),
            (self.divisions, "division"),
            (self.calls, "call"),
        ] {
            if count > 0 {
                write!(
                    f,
                    "{sep}{count} {what}{}",
                    if count == 1 { "" } else { "s" }
                )?;
                sep = ", ";
            }
        }
        Ok(())
    }
}

/// The instructions one trip of the loop made of `blocks` runs, by the
/// static estimate the block layout uses: ten trips of every loop inside
/// it. `depth` is the number of loops each block of `func` is in.
fn trip_size(func: &IrFunction, blocks: &FxHashSet<BlockId>, depth: &[u32]) -> u64 {
    let inside = || {
        func.blocks
            .iter()
            .zip(depth)
            .filter(|(block, _)| blocks.contains(&block.label))
    };
    let own = inside().map(|(_, &d)| d).min().unwrap_or(0);
    inside()
        .map(|(block, &d)| {
            let insts = block
                .instructions
                .iter()
                .filter(|inst| !matches!(inst, Instruction::Phi { .. }))
                .count();
            let weight = 10u64.pow((d - own).min(MAX_WEIGHTED_DEPTH));
            u64::try_from(insts)
                .unwrap_or(u64::MAX)
                .saturating_mul(weight)
        })
        .fold(0, u64::saturating_add)
}

/// The number of `loops` each of `blocks` blocks is in, by block index.
fn loop_depths(blocks: usize, loops: &[NaturalLoop]) -> Vec<u32> {
    let mut depth = vec![0u32; blocks];
    for lp in loops {
        for &b in &lp.body {
            if let Some(d) = depth.get_mut(b) {
                *d += 1;
            }
        }
    }
    depth
}

/// The registers a value of type `ty` takes, by class: general purpose,
/// floating point. An integer wider than a pointer takes several.
fn registers_of(ty: IrType) -> [u32; 2] {
    if ty.is_float() || matches!(ty, IrType::D32 | IrType::D64) {
        return [0, 1];
    }
    let pointer = crate::common::types::target_ptr_size().max(1);
    let width = ty.size().max(1).div_ceil(pointer).min(8);
    [u32::try_from(width).unwrap_or(8), 0]
}

/// Whether `value` is read on a path that starts at the top of block
/// `start` and does not pass the definition of `value`, which is in block
/// `defined_in`: whether it is live into `start`. (A block reached again
/// behind the definition, around an enclosing loop, reads the next value of
/// that number, not this one.)
fn is_live_into(
    func: &IrFunction,
    index_of: &FxHashMap<BlockId, usize>,
    start: usize,
    value: u32,
    defined_in: usize,
) -> bool {
    let mut seen = vec![false; func.blocks.len()];
    let mut work = vec![start];
    seen[start] = true;
    while let Some(b) = work.pop() {
        if b == defined_in {
            // Entered from the top: everything in it that reads `value`
            // comes behind the new definition.
            continue;
        }
        let block = &func.blocks[b];
        let mut read = false;
        for inst in &block.instructions {
            if !matches!(inst, Instruction::Phi { .. }) {
                inst.for_each_used_value(|v| read |= v == value);
            }
        }
        block.terminator.for_each_used_value(|v| read |= v == value);
        if read {
            return true;
        }
        let mut successors = Vec::new();
        block.terminator.for_each_target(|to| successors.push(to));
        successors.extend(analysis::asm_goto_successors(block));
        for to in successors {
            let Some(&next) = index_of.get(&to) else {
                continue;
            };
            // A phi reads at the end of the block its incoming names.
            let handed_over = func.blocks[next].instructions.iter().any(|inst| {
                matches!(inst, Instruction::Phi { incoming, .. }
                    if incoming.iter().any(|(op, from)| *from == block.label && *op == Operand::Value(Value(value))))
            });
            if handed_over {
                return true;
            }
            if !seen[next] {
                seen[next] = true;
                work.push(next);
            }
        }
    }
    false
}

/// Whether the loop that starts at `header` holds no more registers across
/// its trips for `loads` having been hoisted out of it.
///
/// A hoisted value is live through the whole loop, where the instruction it
/// came from needed only its address. That is no loss when the address is
/// needed nowhere from the loop on, so the value takes over the register the
/// address had; it is one when the address stays live or the value is of
/// another class (a `double` behind a pointer), because the allocator may
/// answer with a spill inside the loop, and one saved load does not pay for
/// a spill. Measured on `nbody`: four `double` fields hoisted out of an
/// inner loop put two values of the enclosing function on the stack. So a
/// rotation whose whole gain is hoisted loads is kept only when it is free
/// in every register class. (A division, a call or a value kept out of
/// memory is worth more than a spill costs and is not asked this.)
fn loads_hold_no_more(func: &IrFunction, header: BlockId, loads: &[u32]) -> bool {
    let index_of = analysis::build_label_map(func);
    let Some(&start) = index_of.get(&header) else {
        return false;
    };
    // The loads, wherever they went, and where every value is defined.
    let mut hoisted: Vec<(IrType, u32)> = Vec::with_capacity(loads.len());
    let mut defined_in: FxHashMap<u32, usize> = FxHashMap::default();
    for (b, block) in func.blocks.iter().enumerate() {
        for inst in &block.instructions {
            if let Some(dest) = inst.dest() {
                defined_in.insert(dest.0, b);
            }
            if let Instruction::Load { dest, ptr, ty, .. } = inst
                && loads.contains(&dest.0)
            {
                hoisted.push((*ty, ptr.0));
            }
        }
    }
    if hoisted.len() != loads.len() {
        // A load that is nowhere any more was not hoisted; nothing is known
        // about what replaced it.
        return false;
    }

    let mut held = [0i64; 2];
    let mut asked: FxHashSet<u32> = FxHashSet::default();
    for &(ty, ptr) in &hoisted {
        for (class, n) in registers_of(ty).into_iter().enumerate() {
            held[class] += i64::from(n);
        }
        if !asked.insert(ptr) {
            continue;
        }
        // An address without a definition is nothing this can reason about.
        let freed = defined_in
            .get(&ptr)
            .is_some_and(|&at| !is_live_into(func, &index_of, start, ptr, at));
        if freed {
            for (class, n) in registers_of(IrType::Ptr).into_iter().enumerate() {
                held[class] -= i64::from(n);
            }
        }
    }
    held.iter().all(|&more| more <= 0)
}

/// What [`rotate_for_hoisting`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hoisted {
    /// Loops rotated; when zero the function is exactly as it was.
    pub(crate) loops: usize,
    /// What `hoist` reported for the function that was kept.
    pub(crate) moved: usize,
}

/// The blocks of each loop of `func`, by the label of its header.
fn loop_bodies(func: &IrFunction, loops: &[NaturalLoop]) -> FxHashMap<BlockId, FxHashSet<BlockId>> {
    loops
        .iter()
        .map(|lp| {
            let body = lp.body.iter().map(|&b| func.blocks[b].label).collect();
            (func.blocks[lp.header].label, body)
        })
        .collect()
}

/// `func` with the hoisting applied and nothing rotated: what a rotation
/// has to do better than.
struct Unrotated {
    func: IrFunction,
    /// Its loops as the hoisting left them.
    bodies: FxHashMap<BlockId, FxHashSet<BlockId>>,
}

impl Unrotated {
    fn of(func: &IrFunction, hoist: &mut impl FnMut(&mut IrFunction) -> usize) -> Self {
        let mut func = func.clone();
        hoist(&mut func);
        let (_, loops) = loops_innermost_first(&func);
        let bodies = loop_bodies(&func, &loops);
        Self { func, bodies }
    }

    /// What the loop with header `header` still holds outside that header.
    /// Nothing when there is no such loop any more: then nothing can be
    /// said to have stayed, and nothing is credited.
    fn still_in(&self, header: BlockId) -> Guarded {
        self.bodies
            .get(&header)
            .map_or_else(Guarded::default, |body| {
                Guarded::of(&self.func, |label| label != header && body.contains(&label))
            })
    }
}

/// [`RotatePolicy::Profit`]: rotate the loops of `func` whose rotation lets
/// `hoist` (LICM, memory promotion, loop-carried forwarding) take enough
/// out of them, and take it out. When no loop is rotated `func` is exactly
/// as it was.
///
/// `hoist` has run on `func` before, so what is left in a loop is what could
/// not be moved out of the top-tested form. The decision is made on a copy:
/// rotate the candidates there, run `hoist`, and look at what each rotated
/// loop held before that must not run unless the body does (a load, a
/// division, a call that only reads) and holds no longer. A loop pays when
/// that is
///
/// * its own: in a block of the loop that is in no loop inside it. What
///   leaves an inner loop is that loop's gain and is asked of that loop;
/// * something `hoist` leaves in the loop when the loop is not rotated
///   (asked of a second copy that is only hoisted, and only once a loop
///   looks like paying: the passes between the last hoist and this one
///   leave instructions behind that move for reasons of their own);
/// * worth at least one part in [`TRIP_SHARE`] of a trip of the loop
///   ([`Left::worth`] against [`trip_size`]), so that an instruction saved
///   once per trip of an outer loop does not rotate a loop nest;
/// * and, when it is hoisted loads alone, free in registers
///   ([`loads_hold_no_more`]).
///
/// Whether an instruction is in a loop is asked of the loops as `hoist`
/// left them, not of the blocks the loop had when it was rotated: `hoist`
/// makes blocks (a writeback on an exit edge of an inner loop is inside the
/// outer one).
///
/// If every rotated loop pays, the copy becomes the function. Otherwise the
/// loops that did not are set aside and the rest tried again, since a hoist
/// can depend on a neighbouring loop having been rotated.
pub(crate) fn rotate_for_hoisting(
    func: &mut IrFunction,
    mut hoist: impl FnMut(&mut IrFunction) -> usize,
) -> Hoisted {
    let debug = rotate_debug();
    let mut set_aside: FxHashSet<BlockId> = FxHashSet::default();
    // Made when the first loop looks like paying.
    let mut unrotated: Option<Unrotated> = None;
    for _ in 0..MAX_PROFIT_ATTEMPTS {
        // What each candidate holds itself, by the header's label.
        let mut candidates: FxHashMap<BlockId, Guarded> = FxHashMap::default();
        {
            let (cfg, loops) = loops_innermost_first(func);
            let shape = Shape::of(func, &cfg, false);
            let depth = loop_depths(func.blocks.len(), &loops);
            for lp in &loops {
                let label = func.blocks[lp.header].label;
                if set_aside.contains(&label)
                    || !has_guarded_hoist_candidate(func, &cfg, lp)
                    || plan_rotation(func, lp, &shape).is_none()
                {
                    continue;
                }
                // The loop's own blocks are as deep as its header.
                let own: FxHashSet<BlockId> = lp
                    .body
                    .iter()
                    .filter(|&&b| b != lp.header && depth[b] == depth[lp.header])
                    .map(|&b| func.blocks[b].label)
                    .collect();
                candidates.insert(label, Guarded::of(func, |label| own.contains(&label)));
            }
        }
        if candidates.is_empty() {
            return Hoisted::default();
        }

        let mut trial = func.clone();
        let rotated = rotate_loops_where(&mut trial, |f, lp| {
            candidates.contains_key(&f.blocks[lp.header].label)
        });
        if rotated.is_empty() {
            return Hoisted::default();
        }
        let moved = hoist(&mut trial);
        let (_, loops) = loops_innermost_first(&trial);
        let bodies = loop_bodies(&trial, &loops);
        let depth = loop_depths(trial.blocks.len(), &loops);
        let loads_anywhere = loads_of(&trial);

        let mut all_paid = true;
        for loop_ in &rotated {
            let (Some(before), Some(body)) =
                (candidates.get(&loop_.guard), bodies.get(&loop_.header))
            else {
                // `rotate_loops_where` rotates candidates only, and `hoist`
                // removes no loop.
                debug_assert!(false, "a rotated loop is not a candidate, or is gone");
                all_paid = false;
                set_aside.insert(loop_.guard);
                continue;
            };
            let gone = before.not_in(&Guarded::of(&trial, |label| body.contains(&label)));
            let refusal = if gone.is_empty() {
                Some("nothing left the loop".to_string())
            } else {
                // What `hoist` moves without the rotation is not the
                // rotation's doing.
                let stayed = unrotated
                    .get_or_insert_with(|| Unrotated::of(func, &mut hoist))
                    .still_in(loop_.guard);
                let left = Left::of(&gone.also_in(&stayed), &loads_anywhere);
                let worth = left.worth();
                let size = trip_size(&trial, body, &depth);
                if worth == 0 {
                    Some("nothing left the loop that had to wait for the guard".to_string())
                } else if worth.saturating_mul(TRIP_SHARE) < size {
                    Some(format!(
                        "{left} left the loop, worth {worth} of a trip of {size}"
                    ))
                } else if left.is_hoisted_loads_only()
                    && !loads_hold_no_more(&trial, loop_.header, &left.hoisted)
                {
                    Some(format!(
                        "{left} left the loop and the loop holds more registers for it"
                    ))
                } else {
                    trace!(
                        debug,
                        "{}: header .LBB{}: rotation pays: {left} left the loop, worth {worth} of a trip of {size} \
                         (preheader .LBB{})",
                        func.name,
                        loop_.guard.0,
                        loop_.preheader.0
                    );
                    None
                }
            };
            if let Some(why) = refusal {
                trace!(
                    debug,
                    "{}: header .LBB{}: rotation does not pay: {why}: undone",
                    func.name,
                    loop_.guard.0
                );
                all_paid = false;
                set_aside.insert(loop_.guard);
            }
        }
        // A candidate that was planned on `func` and not rotated in the
        // copy (a rotation before it changed what it is) is not retried.
        for label in candidates.keys() {
            if !rotated.iter().any(|r| r.guard == *label) {
                set_aside.insert(*label);
            }
        }
        if all_paid {
            let loops = rotated.len();
            *func = trial;
            return Hoisted { loops, moved };
        }
    }
    Hoisted::default()
}

#[cfg(test)]
mod tests;

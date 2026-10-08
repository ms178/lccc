//! Machine-level loop inversion (bottom-testing), run after phi elimination.
//!
//! lccc emits counted loops in top-test form: the header tests, the body runs,
//! and the latch jumps unconditionally back to the header.
//!
//! ```text
//! .LBB5:  cmpq $8192, %r12 ; jae .LBB9    # not taken
//! .LBB6:  movzbl (%rbx,%r12), %r8d
//!         testb %r8b, %r8b ; je .LBB8     # not taken
//! .LBB7:  leaq 1(%r12), %r12 ; jmp .LBB5  # TAKEN, every iteration
//! ```
//!
//! GCC and Clang emit the bottom-test form, where the single backward branch
//! *is* the loop test. Measured on `tests/bench/k_memchr.c` by hand-editing the
//! assembly and timing all four variants (best-of-13, this host):
//!
//! | variant | time |
//! |---|---|
//! | as emitted, 6 insn/iter | 17.459 ms |
//! | **bottom-tested, guard kept, 5 insn/iter** | **8.769 ms** |
//! | bottom-tested + load folded into the compare, 4 insn/iter | 8.771 ms |
//! | GCC 14.2 reference | ~8.80 ms |
//!
//! Nearly a **2x** difference, and note the third row: folding the load into
//! the compare on top of the rotation buys *nothing*. The cost was never the
//! instruction count, it was the unconditional `jmp` — the rotated loop has a
//! single taken branch per iteration instead of a taken conditional plus a
//! taken jump.
//!
//! # Why here and not in `loop_rotate`
//!
//! [`crate::passes::loop_rotate`] does this on SSA, where duplicating the
//! header means rewriting header phis; that is the part which produced a
//! string of miscompiles, and the pass is still opt-in and restricted to
//! single-block bodies (it declines `k_memchr` with
//! `not single-block body (body_len=3)`).
//!
//! This pass runs after `eliminate_phis`, so **there are no phis left**.
//! Inversion degenerates to copying a handful of side-effect-free
//! instructions and retargeting one edge — no phi surgery, nothing to get
//! wrong. Phi elimination has already placed the induction variable's copy at
//! the end of the latch, so the duplicated test naturally reads the *updated*
//! value, which is precisely what a bottom test must do.
//!
//! # When it is legal
//!
//! For a natural loop with header `H`, a single latch `T` (`T != H`) whose
//! terminator is `Branch(H)`:
//!
//! 1. `H` ends in a `CondBranch` with exactly one target inside the loop (the
//!    body entry `B`) and one outside (the exit `X`). A loop whose header does
//!    not decide the exit is not a top-test loop.
//! 2. Every instruction in `H` is **pure and duplicable** — no calls, no
//!    side effects, no stack allocation — or belongs to the *memory licence*
//!    below, which admits non-volatile loads and (when the loop's only exit is
//!    the header's own test) non-volatile stores. Volatile traffic, atomics,
//!    calls, inline asm, `alloca`, `memcpy` and `PgoCounterInc` are refused
//!    under both licences.
//!
//! # The memory licence: implemented, measurable, **not shipped**
//!
//! Duplicating a non-volatile access is *sound*. The header executes `n + 1`
//! times at the same program positions in both forms, the copy is renamed
//! (`set_dest` knows `Load` for exactly this reason) so the latch defines its
//! own values, and rule 3 keeps a header value from reaching the body. Two
//! store-specific hazards are handled: a store is licensed **only** when the
//! loop has no side exit (see [`loop_exits_only_through_header`]), and volatile
//! traffic, atomics, calls, inline asm, `alloca`, `memcpy` and `PgoCounterInc`
//! are refused under both licences.
//!
//! It is nevertheless off by default, and that is a measurement. Licensed, the
//! pass reaches **five** loops in the 90-TU corpus, all of them a 3-4
//! instruction `while (*p) p++;` walk, and rotating them is neutral:
//! 61-sample interleaved A/B on the same binary with the licence on and off,
//! `k_strlen_scan` +0.57% on min / 41-61 wins, `strlen_bench` the same. An
//! earlier record rejected the licence on a 15.9% regression for exactly that
//! kernel; that number does not reproduce here — it was a `min`-of-31 sample on
//! a host whose sample distribution has a heavy fast tail, and the same
//! statistic produces phantom 5-25% effects on four other kernels of this
//! corpus when the two arms are the *same* code. The re-measurement, the
//! instrument that produced it (`scripts/loop_invert_census.py`) and the
//! statistics are in `docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md` §6.
//!
//! So the switch stays — `CCC_LOOP_INVERT_MEMORY=1` turns the licence on so the
//! experiment is reproducible on a built binary — and the default is pure-only,
//! which keeps this pass's codegen byte-identical to what it produced before
//! the licence existed.
//! 3. Every value defined in `H` is used **only inside `H`**. Otherwise the
//!    body, once reached from `T`, would read the value `H` computed on its
//!    single guard execution instead of this iteration's.
//! 4. `H` has at least one predecessor outside the loop, so the guard it
//!    becomes is actually reachable.
//! 5. `H` is small (`MAX_DUP_INSTS`), because its instructions are duplicated.
//!
//! The transform then appends a renamed copy of `H`'s instructions to `T` and
//! gives `T` a copy of `H`'s terminator. `H` keeps its own terminator and
//! becomes the loop guard, executed once. The back edge becomes `T -> B`.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::instruction::{Instruction, Terminator};
use crate::ir::reexports::{BlockId, IrFunction, Operand, Value};

/// Duplicating more than a handful of instructions trades I-cache for branches.
/// A counted loop's test is one or two instructions; anything much larger is
/// not the shape this pass is for.
const MAX_DUP_INSTS: usize = 8;

/// True when `inst` may be duplicated into the latch: pure, no memory traffic,
/// no control flow, no side effects.
fn is_duplicable(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::Cmp { .. }
            | Instruction::BinOp { .. }
            | Instruction::UnaryOp { .. }
            | Instruction::Copy { .. }
            | Instruction::Cast { .. }
            | Instruction::Select { .. }
            | Instruction::GetElementPtr { .. }
            | Instruction::GlobalAddr { .. }
    )
}

/// Which licence a header qualifies for. Determines nothing about *how* the
/// copy is made (both licences rename every destination) — it exists so the
/// pass, its diagnostics and its tests can distinguish a header that computes
/// from one that walks memory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Licence {
    Pure,
    Memory,
}

impl Licence {
    fn label(self) -> &'static str {
        match self {
            Licence::Pure => "pure",
            Licence::Memory => "memory",
        }
    }
}

/// Classify a header under the two licences, or `None` when it can never be
/// duplicated.
///
/// `stores_ok` is [`loop_exits_only_through_header`] for this loop; a store
/// that fails it is not licensed (see that function for why a store and a load
/// differ).
fn header_licence(insts: &[Instruction], stores_ok: bool) -> Option<Licence> {
    let mut licence = Licence::Pure;
    for inst in insts {
        match inst {
            Instruction::Load { volatile, .. } if !volatile => licence = Licence::Memory,
            Instruction::Store { volatile, .. } if !volatile && stores_ok => {
                licence = Licence::Memory
            }
            _ if is_duplicable(inst) => {}
            _ => return None,
        }
    }
    Some(licence)
}

/// The shape summary the pass prints for every rotation.
///
/// Deliberately a small, pre-computed summary rather than the IR itself: the
/// census (`scripts/loop_invert_census.py`) correlates a loop's decision with
/// these numbers, and a diagnostic whose inputs are cheap to print is one whose
/// decisions can be audited from a build log.
#[derive(Clone, Copy, Debug)]
struct LoopShape {
    /// Instructions in the header, including the licensed accesses.
    header_insts: usize,
    header_loads: usize,
    header_stores: usize,
    /// Instructions in the whole loop: header + body blocks + latch.
    loop_insts: usize,
    /// Memory instructions outside the header (the per-iteration work the
    /// rotation's removed taken branch is amortised against).
    loop_memory: usize,
}

impl LoopShape {
    fn of(
        func: &IrFunction,
        lp: &crate::passes::loop_analysis::NaturalLoop,
        header: usize,
    ) -> Self {
        let mut shape = LoopShape {
            header_insts: func.blocks[header].instructions.len(),
            header_loads: 0,
            header_stores: 0,
            loop_insts: 0,
            loop_memory: 0,
        };
        for &b in &lp.body {
            for inst in &func.blocks[b].instructions {
                shape.loop_insts += 1;
                let (load, store) = match inst {
                    Instruction::Load { .. } => (1, 0),
                    Instruction::Store { .. } => (0, 1),
                    _ => (0, 0),
                };
                if b == header {
                    shape.header_loads += load;
                    shape.header_stores += store;
                } else {
                    shape.loop_memory += load + store;
                }
            }
        }
        shape
    }
}

/// True when every exit from the loop leaves through the header's own test:
/// no block of the loop other than the header and the latch branches out of it.
///
/// This is what makes duplicating a header *store* exact. The header executes
/// `n + 1` times in both forms only while every iteration runs to the latch.
/// With a `break` in the body the top-tested form executes it once more than
/// the rotated form does (at the top of the iteration that breaks), so at the
/// loop exit the two forms have written different bytes. A dropped *load* is
/// unobservable — rule 3 keeps its value inside `H` — which is why only stores
/// are gated on this.
fn loop_exits_only_through_header(
    lp: &crate::passes::loop_analysis::NaturalLoop,
    cfg: &crate::ir::analysis::CfgAnalysis,
    header: usize,
    latch: usize,
) -> bool {
    lp.body
        .iter()
        .filter(|&&b| b != header && b != latch)
        .all(|&b| {
            cfg.succs
                .row(b)
                .iter()
                .all(|&s| lp.body.contains(&(s as usize)))
        })
}

/// How the pass is configured for one pipeline run.
///
/// The three switches are resolved **once**, by the driver's pipeline, and then
/// passed down explicitly (`src/driver/pipeline.rs`).  They used to be read out
/// of the process environment deep inside the pass -- once per function, and,
/// because `std::env::set_var` is process-global, in a way that raced with
/// every test running in parallel on the other threads of the test binary,
/// where those tests observed the wrong value (the symptom was a ~1-in-10 flake
/// in this module).  Configuration a caller can supply belongs in a parameter,
/// not in ambient global state, and the env-read ratchet in
/// `tests/regression/check_env_test_hygiene.sh` counts reads here for exactly
/// that reason.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LoopInvertConfig {
    pub enabled: bool,
    /// Whether the memory licence is available (opt-in, **off by default**).
    ///
    /// Off is a measurement, not a precaution: licensed, the pass reaches five
    /// loops in the 90-TU corpus and every one of them is neutral (61-sample
    /// interleaved A/B, +0.57% on min, 41/61 wins), so the licence is not worth
    /// shipping -- but it stays a switch because `CCC_LOOP_INVERT_MEMORY=1`
    /// makes the experiment reproducible on a built binary instead of living in
    /// a document. See `docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md` §6.
    pub memory_licence: bool,
    pub debug: bool,
}

impl LoopInvertConfig {
    /// The driver's configuration, built from the documented
    /// `CCC_NO_LOOP_INVERT` / `CCC_LOOP_INVERT_MEMORY` /
    /// `CCC_DEBUG_LOOP_INVERT` escape hatches at the pipeline's start.
    ///
    /// Takes already-resolved values rather than reading the environment: the
    /// reads belong to the driver, once per run, not to the pass.
    pub const fn new(enabled: bool, memory_licence: bool, debug: bool) -> Self {
        Self {
            enabled,
            memory_licence,
            debug,
        }
    }

    /// Configuration used by the unit tests: no environment access at all.
    /// The shipped defaults: the pass runs, the memory licence does not.
    #[cfg(test)]
    const fn testing() -> Self {
        Self {
            enabled: true,
            memory_licence: false,
            debug: false,
        }
    }

    /// The opt-in memory licence (`CCC_LOOP_INVERT_MEMORY=1`).
    #[cfg(test)]
    const fn testing_memory() -> Self {
        Self {
            enabled: true,
            memory_licence: true,
            debug: false,
        }
    }

    #[cfg(test)]
    const fn disabled() -> Self {
        Self {
            enabled: false,
            memory_licence: true,
            debug: false,
        }
    }
}

/// The pass itself, parameterised by [`LoopInvertConfig`].
pub(crate) fn invert_loops_with(func: &mut IrFunction, cfg: LoopInvertConfig) -> usize {
    if func.blocks.len() < 3 {
        return 0;
    }
    if !cfg.enabled {
        return 0;
    }

    // Phis must already be gone; this pass's whole safety argument rests on it.
    debug_assert!(
        !func
            .blocks
            .iter()
            .flat_map(|b| b.instructions.iter())
            .any(|i| matches!(i, Instruction::Phi { .. })),
        "loop_invert runs after eliminate_phis; a surviving phi means the \
         pipeline order changed and the duplication below is no longer safe"
    );

    let mut rotated = 0;
    // One rotation per call per loop: the CFG changes, so re-derive it.
    loop {
        let Some(plan) = find_one(func, &cfg) else {
            break;
        };
        apply(func, &plan);
        rotated += 1;
        if rotated > 64 {
            break; // pathological input guard
        }
    }
    rotated
}

struct Plan {
    header: usize,
    latch: usize,
}

fn find_one(func: &IrFunction, lic: &LoopInvertConfig) -> Option<Plan> {
    let debug = lic.debug;
    let cfg = crate::ir::analysis::CfgAnalysis::build(func);
    // MERGE BY HEADER. `find_natural_loops` returns one loop per BACK EDGE,
    // so a loop with two latches arrives as two single-latch loops and the
    // single-latch guard below would wave both through, duplicating the test
    // once per latch. Merging first makes the guard mean what it says.
    let loops = crate::passes::loop_analysis::merge_loops_by_header(
        crate::passes::loop_analysis::find_natural_loops(
            cfg.num_blocks,
            &cfg.preds,
            &cfg.succs,
            &cfg.idom,
        ),
    );

    let label_to_idx: FxHashMap<u32, usize> = func
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.label.0, i))
        .collect();

    'next_loop: for lp in &loops {
        let h = lp.header;
        if h >= func.blocks.len() {
            continue;
        }

        // A single latch, branching unconditionally to the header.
        let mut latch = None;
        let mut latches = 0usize;
        for &p32 in cfg.preds.row(h) {
            let p = p32 as usize;
            if lp.body.contains(&p) {
                latches += 1;
                if latch.is_some() {
                    continue 'next_loop; // more than one latch
                }
                latch = Some(p);
            }
        }
        if latches > 1 && debug {
            eprintln!("[INV] header {} has {} latches", h, latches);
        }
        let Some(t) = latch else { continue };
        if t == h {
            continue; // self-loop: already bottom-tested
        }
        if !matches!(&func.blocks[t].terminator, Terminator::Branch(l) if *l == func.blocks[h].label)
        {
            continue;
        }

        // Guard 1: the header decides the exit.
        let Terminator::CondBranch {
            true_label,
            false_label,
            ..
        } = &func.blocks[h].terminator
        else {
            continue;
        };
        let (Some(&tb), Some(&fb)) = (
            label_to_idx.get(&true_label.0),
            label_to_idx.get(&false_label.0),
        ) else {
            continue;
        };
        let t_in = lp.body.contains(&tb);
        let f_in = lp.body.contains(&fb);
        if t_in == f_in {
            if debug {
                eprintln!(
                    "[INV] header {} does not decide the exit ({} successor(s) {})",
                    h,
                    if t_in { "both" } else { "neither" },
                    if t_in { "inside" } else { "outside" }
                );
            }
            continue; // both inside (not an exit test) or both outside
        }

        // Guard 5 + 2: small, and every instruction pure -- or memory under
        // the opt-in licence (`CCC_LOOP_INVERT_MEMORY=1`).
        let hb = &func.blocks[h];
        if hb.instructions.len() > MAX_DUP_INSTS {
            if debug {
                eprintln!(
                    "[INV] header {} is oversized ({} insns > {})",
                    h,
                    hb.instructions.len(),
                    MAX_DUP_INSTS
                );
            }
            continue;
        }
        let stores_ok = lic.memory_licence && loop_exits_only_through_header(lp, &cfg, h, t);
        let Some(licence) = header_licence(&hb.instructions, stores_ok) else {
            if debug {
                eprintln!("[INV] header {} has a non-duplicable instruction", h);
            }
            continue;
        };
        let shape = LoopShape::of(func, lp, h);
        if licence == Licence::Memory && !lic.memory_licence {
            if debug {
                eprintln!(
                    "[INV] header {} walks memory, licence not taken (enabled={}, \
                     header={} insn/{} ld/{} st, loop={} insn/{} mem)",
                    h,
                    lic.memory_licence,
                    shape.header_insts,
                    shape.header_loads,
                    shape.header_stores,
                    shape.loop_insts,
                    shape.loop_memory,
                );
            }
            continue;
        }

        // Guard 3: nothing defined in the header escapes it.
        let defined: FxHashSet<u32> = hb
            .instructions
            .iter()
            .filter_map(|i| i.dest())
            .map(|d| d.0)
            .collect();
        if !defined.is_empty() {
            for (bi, b) in func.blocks.iter().enumerate() {
                if bi == h {
                    continue;
                }
                let mut escaped = None;
                for inst in &b.instructions {
                    inst.for_each_used_value(|v| {
                        if defined.contains(&v) && escaped.is_none() {
                            escaped = Some(v);
                        }
                    });
                }
                terminator_uses(&b.terminator, &mut |v| {
                    if defined.contains(&v) && escaped.is_none() {
                        escaped = Some(v);
                    }
                });
                if let Some(v) = escaped {
                    if debug {
                        let used_in_loop = lp.body.contains(&bi);
                        eprintln!(
                            "[INV] header {} defines v{}, used in block {} ({})",
                            h,
                            v,
                            bi,
                            if used_in_loop {
                                "in loop"
                            } else {
                                "outside loop"
                            }
                        );
                    }
                    continue 'next_loop;
                }
            }
        }

        // Guard 4: an entry from outside the loop, so the guard is reachable.
        if !cfg
            .preds
            .row(h)
            .iter()
            .any(|&p| !lp.body.contains(&(p as usize)))
        {
            continue;
        }

        if debug {
            eprintln!(
                "[INV] rotating header={} latch={} ({}: header={} insn/{} ld/{} st, \
                 loop={} insn/{} mem)",
                h,
                t,
                licence.label(),
                shape.header_insts,
                shape.header_loads,
                shape.header_stores,
                shape.loop_insts,
                shape.loop_memory
            );
        }
        return Some(Plan {
            header: h,
            latch: t,
        });
    }
    None
}

fn apply(func: &mut IrFunction, plan: &Plan) {
    // Seed from the SOUND high-water mark: InlineAsm outputs are definitions
    // that `dest()` does not report, so a dest-only max() scan silently
    // re-issues IDs already taken by an inline-asm output and corrupts SSA
    // numbering (pr84524: the cloned latch test collided with `"+r"(v)` at
    // id 55, demoted the asm output to an indirect slot, and stored through
    // an uninitialized pointer). Write the new bound back so every later
    // pass seeds above it.
    let mut next_id = func.sound_next_value_id();

    let header_insts = func.blocks[plan.header].instructions.clone();
    let header_term = func.blocks[plan.header].terminator.clone();

    // Fresh names for everything the header defines, so the copy in the latch
    // is a separate live range from the guard's.
    let mut remap: FxHashMap<u32, u32> = FxHashMap::default();
    let mut cloned: Vec<Instruction> = Vec::with_capacity(header_insts.len());
    for inst in &header_insts {
        let mut c = inst.clone();
        // Rewrite USES first: an earlier header instruction may feed this one.
        c.for_each_operand_mut(|op| {
            if let Operand::Value(v) = op {
                if let Some(&n) = remap.get(&v.0) {
                    *v = Value(n);
                }
            }
        });
        // Bare-Value positions (Load/Store ptr, GEP base, ...) are not
        // Operands: without this walk a clone keeps naming the guard's
        // definitions instead of its own fresh ones (separate-live-range
        // violation). Same two-walk substitution contract as loop_memset.
        c.for_each_value_use_mut(|v| {
            if let Some(&n) = remap.get(&v.0) {
                *v = Value(n);
            }
        });
        if let Some(d) = c.dest() {
            let fresh = next_id;
            next_id += 1;
            remap.insert(d.0, fresh);
            set_dest(&mut c, Value(fresh));
        }
        cloned.push(c);
    }

    let mut term = header_term;
    if let Terminator::CondBranch { cond, .. } = &mut term {
        if let Operand::Value(v) = cond {
            if let Some(&n) = remap.get(&v.0) {
                *v = Value(n);
            }
        }
    }

    let latch = &mut func.blocks[plan.latch];
    // Keep source_spans parallel to instructions, or -g emits .loc against the
    // wrong instruction. The duplicated test inherits the latch's last span
    // when one is being tracked.
    if latch.source_spans.len() == latch.instructions.len() {
        let fill = latch.source_spans.last().copied();
        if let Some(sp) = fill {
            latch
                .source_spans
                .extend(std::iter::repeat_n(sp, cloned.len()));
        } else {
            latch.source_spans.clear();
        }
    }
    latch.instructions.extend(cloned);
    latch.terminator = term;
    func.next_value_id = func.next_value_id.max(next_id);
}

fn set_dest(inst: &mut Instruction, new: Value) {
    macro_rules! d {
        ($($v:ident),+ $(,)?) => {
            match inst {
                $(Instruction::$v { dest, .. } => *dest = new,)+
                _ => {}
            }
        };
    }
    d!(
        Cmp,
        BinOp,
        UnaryOp,
        Copy,
        Cast,
        Select,
        GetElementPtr,
        GlobalAddr,
        // A licensed header load is a definition like any other: without this
        // the clone re-defines the guard's value, which is what verify.rs
        // check 7 rejects ("defined more than once" -- consumers index def
        // sites by value id) and what makes the body read the guard's byte
        // forever.
        Load
    );
}

fn terminator_uses(term: &Terminator, f: &mut impl FnMut(u32)) {
    let mut on = |op: &Operand| {
        if let Operand::Value(v) = op {
            f(v.0);
        }
    };
    match term {
        Terminator::Return(Some(op)) => on(op),
        Terminator::CondBranch { cond, .. } => on(cond),
        Terminator::Switch { val, .. } => on(val),
        Terminator::IndirectBranch { target, .. } => on(target),
        _ => {}
    }
}

/// Unused import guard: `BlockId` keeps the signature readable above.
const _: Option<BlockId> = None;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types::IrType;
    use crate::ir::instruction::BasicBlock;
    use crate::ir::reexports::IrConst;
    use crate::ir::reexports::{IrBinOp, IrCmpOp};

    fn blk(label: u32, instructions: Vec<Instruction>, terminator: Terminator) -> BasicBlock {
        BasicBlock {
            label: BlockId(label),
            instructions,
            terminator,
            source_spans: Vec::new(),
        }
    }
    fn br(t: u32) -> Terminator {
        Terminator::Branch(BlockId(t))
    }
    fn cond(c: u32, t: u32, f: u32) -> Terminator {
        Terminator::CondBranch {
            cond: Operand::Value(Value(c)),
            true_label: BlockId(t),
            false_label: BlockId(f),
        }
    }
    fn ret() -> Terminator {
        Terminator::Return(None)
    }
    fn cmp(dest: u32, lhs: u32) -> Instruction {
        Instruction::Cmp {
            dest: Value(dest),
            op: IrCmpOp::Slt,
            lhs: Operand::Value(Value(lhs)),
            rhs: Operand::Const(IrConst::I32(100)),
            ty: IrType::I32,
        }
    }
    fn add1(dest: u32, src: u32) -> Instruction {
        Instruction::BinOp {
            dest: Value(dest),
            op: IrBinOp::Add,
            lhs: Operand::Value(Value(src)),
            rhs: Operand::Const(IrConst::I32(1)),
            ty: IrType::I32,
        }
    }
    fn func_of(blocks: Vec<BasicBlock>) -> IrFunction {
        let mut f = IrFunction::new("t".into(), IrType::Void, Vec::new(), false);
        f.blocks = blocks;
        f
    }

    /// The canonical counted loop, exactly the `k_memchr` shape:
    ///   0 preheader -> 1
    ///   1 header : cmp ; i < n ? 2 : 4
    ///   2 body   : -> 3
    ///   3 latch  : i++ ; -> 1
    ///   4 exit
    fn counted_loop() -> IrFunction {
        func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20)], cond(10, 2, 4)),
            blk(2, vec![], br(3)),
            blk(3, vec![add1(20, 20)], br(1)),
            blk(4, vec![], ret()),
        ])
    }

    #[test]
    fn a_top_test_counted_loop_is_bottom_tested() {
        let mut f = counted_loop();
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);

        // The header keeps its test and becomes the guard, executed once.
        assert!(matches!(
            f.blocks[1].terminator,
            Terminator::CondBranch { .. }
        ));
        assert_eq!(f.blocks[1].instructions.len(), 1);

        // The latch now carries a COPY of the test and branches like the
        // header: the back edge goes to the body, not the header.
        let latch = &f.blocks[3];
        assert_eq!(latch.instructions.len(), 2, "increment + duplicated cmp");
        match &latch.terminator {
            Terminator::CondBranch {
                true_label,
                false_label,
                cond,
            } => {
                assert_eq!(true_label.0, 2, "back edge must target the body");
                assert_eq!(false_label.0, 4, "exit unchanged");
                // ...and it must test the LATCH's copy, not the guard's value.
                assert!(
                    matches!(cond, Operand::Value(v) if v.0 != 10),
                    "the duplicated test must define a fresh value"
                );
            }
            other => panic!("latch must end in a CondBranch, got {:?}", other),
        }
    }

    #[test]
    fn the_duplicated_test_reads_the_updated_induction_variable() {
        // Phi elimination puts `i = i + 1` at the end of the latch, so the
        // duplicated compare that follows it must read the NEW value -- that
        // is the whole point of a bottom test.
        let mut f = counted_loop();
        invert_loops_with(&mut f, LoopInvertConfig::testing());
        let latch = &f.blocks[3];
        let Instruction::Cmp { lhs, .. } = &latch.instructions[1] else {
            panic!("expected the duplicated Cmp second");
        };
        assert!(
            matches!(lhs, Operand::Value(v) if v.0 == 20),
            "must compare the incremented IV"
        );
    }

    #[test]
    fn value_ids_stay_unique_after_inversion() {
        let mut f = counted_loop();
        invert_loops_with(&mut f, LoopInvertConfig::testing());
        let mut seen = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    assert!(seen.insert(d.0), "duplicate definition of v{}", d.0);
                }
            }
        }
    }

    /// pr84524 end-to-end shape: an InlineAsm output lives at an ID above
    /// every `dest()` ID. The latch clone's fresh-ID allocator must seed from
    /// the sound high-water mark (InlineAsm outputs are definitions that
    /// `dest()` does not report) or it re-issues the asm output's ID, which
    /// demotes the asm output slot to indirect storage and miscompiles the
    /// asm-emit path (store through an uninitialized pointer).
    #[test]
    fn inversion_never_reuses_inline_asm_output_ids() {
        let asm_out = Value(55);
        let asm = Instruction::InlineAsm {
            template: String::new(),
            outputs: vec![("+r".to_string(), asm_out, None)],
            inputs: vec![("r".to_string(), Operand::Value(Value(99)), None)],
            clobbers: Vec::new(),
            operand_types: vec![IrType::U16, IrType::U16],
            goto_labels: Vec::new(),
            input_symbols: vec![None],
            seg_overrides: vec![
                crate::common::types::AddressSpace::Default,
                crate::common::types::AddressSpace::Default,
            ],
        };
        // The asm lives OUTSIDE the rotated loop (pr84524: the colliding asm
        // sat in a sibling inner loop while the outer loop was inverted);
        // the collision is purely an ID-allocation hazard across the function.
        let mut f = counted_loop();
        f.blocks[4].instructions.insert(0, asm);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 1);

        let mut seen = FxHashSet::default();
        for b in &f.blocks {
            for i in &b.instructions {
                if let Some(d) = i.dest() {
                    assert!(seen.insert(d.0), "duplicate definition of v{}", d.0);
                }
                if let Instruction::InlineAsm { outputs, .. } = i {
                    for (_, v, _) in outputs {
                        assert!(
                            seen.insert(v.0),
                            "inline-asm output v{} re-defined elsewhere",
                            v.0
                        );
                    }
                }
            }
        }
        assert!(seen.contains(&55), "asm output id must survive");
        // The allocator high-water mark must be published for later passes.
        assert!(
            f.next_value_id > 55,
            "next_value_id must be bumped above the allocated range, got {}",
            f.next_value_id
        );
    }

    // ── negative controls ───────────────────────────────────────────────────

    fn load(dest: u32, ptr: u32) -> Instruction {
        Instruction::Load {
            dest: Value(dest),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: Default::default(),
            volatile: false,
        }
    }

    fn store(val: u32, ptr: u32) -> Instruction {
        Instruction::Store {
            val: Operand::Value(Value(val)),
            ptr: Value(ptr),
            ty: IrType::I32,
            seg_override: Default::default(),
            volatile: false,
        }
    }

    /// The `while (*p) p++;` shape: a header that walks memory, decides the
    /// exit from what it loaded, and defines both the loaded value and the test.
    ///   1 header : inst ; v10 = cmp v`cmp_lhs`, 100 -> 2 : 4
    fn walk_loop(inst: Instruction, cmp_lhs: u32) -> IrFunction {
        let mut f = counted_loop();
        f.blocks[1].instructions = vec![inst, cmp(10, cmp_lhs)];
        f
    }

    /// `counted_loop` whose body can leave the loop:
    ///   2 body : if v50 -> 3 (latch) else 4 (exit)
    /// The header is still the loop's own exit test, but the body has one too.
    fn loop_with_side_exit() -> IrFunction {
        let mut f = counted_loop();
        f.blocks[2].terminator = cond(50, 3, 4);
        f
    }

    #[test]
    fn a_header_load_is_licensed_and_renamed() {
        // The byte-walk shape under the memory licence. Sound because the copy
        // is renamed: `set_dest` knows `Load`, so the latch defines its own
        // value instead of re-defining the guard's (which verify.rs check 7
        // refuses and which would make the body read the guard's byte forever).
        let mut f = walk_loop(load(30, 20), 30);
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::testing_memory()),
            1
        );

        assert_eq!(f.blocks[1].instructions.len(), 2, "guard keeps its load");
        let Instruction::Load { dest, .. } = &f.blocks[1].instructions[0] else {
            panic!("the guard's first instruction is its load");
        };
        assert_eq!(dest.0, 30, "the guard keeps the original value id");

        let latch = &f.blocks[3];
        assert_eq!(
            latch.instructions.len(),
            3,
            "increment + duplicated load + duplicated compare"
        );
        let Instruction::Load { dest: ldest, .. } = &latch.instructions[1] else {
            panic!("the duplicated load must land after the increment");
        };
        assert_ne!(ldest.0, 30, "the copy must not re-define the guard's value");
        let Instruction::Cmp { lhs, .. } = &latch.instructions[2] else {
            panic!("the duplicated compare follows the duplicated load");
        };
        assert!(
            matches!(lhs, Operand::Value(v) if v.0 == ldest.0),
            "the duplicated test must read the copy, not the guard's byte"
        );
    }

    #[test]
    fn a_header_load_is_refused_by_default() {
        // The shipped configuration. The licence is off because it does not
        // pay, not because it is unsound: licensed, the pass reaches five
        // loops in the 90-TU corpus, all `while (*p) p++;` walks, and rotating
        // them is neutral (61 interleaved samples on one binary with the
        // licence on and off: +0.57%, 41-61 wins). The 15.9% regression an
        // earlier session recorded for this kernel does not reproduce; it was
        // a min-of-31 statistic, and min on this host invents 5-25% effects on
        // four other kernels when both arms are the same code.
        // docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md §5-§6.
        let mut f = walk_loop(load(30, 20), 30);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_volatile_header_load_is_refused_under_both_licences() {
        let mut f = walk_loop(
            Instruction::Load {
                dest: Value(30),
                ptr: Value(20),
                ty: IrType::I32,
                seg_override: Default::default(),
                volatile: true,
            },
            30,
        );
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::testing_memory()),
            0
        );
    }

    #[test]
    fn a_header_store_is_rotated_when_the_loop_exits_only_through_the_header() {
        // With no side exit the header executes `n + 1` times in both forms, at
        // the same positions, so the store is duplicated verbatim: same value,
        // same pointer.
        let mut f = walk_loop(store(20, 21), 20);
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::testing_memory()),
            1
        );

        let latch = &f.blocks[3];
        assert_eq!(latch.instructions.len(), 3, "increment + store + compare");
        let Instruction::Store { val, ptr, .. } = &latch.instructions[1] else {
            panic!("the duplicated store must land after the increment");
        };
        assert!(matches!(val, Operand::Value(v) if v.0 == 20));
        assert_eq!(ptr.0, 21);
    }

    #[test]
    fn a_header_store_is_refused_while_the_loop_has_a_side_exit() {
        // `for (i = 0; (a[i] = b[i]) != 0; i++) { if (x) break; }`: the
        // top-tested form stores once more than the rotated form (at the top of
        // the iteration that breaks) and leaves different bytes behind, so the
        // store is not licensed. The side exit does not affect a *load*.
        let mut f = loop_with_side_exit();
        f.blocks[1].instructions.insert(0, store(20, 21));
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::testing_memory()),
            0
        );

        let mut g = loop_with_side_exit();
        g.blocks[1].instructions.insert(0, load(30, 20));
        assert_eq!(
            invert_loops_with(&mut g, LoopInvertConfig::testing_memory()),
            1,
            "a dropped load is unobservable"
        );
    }

    #[test]
    fn a_licensed_header_value_escaping_into_the_body_is_still_refused() {
        // What makes the renaming sound: the body may not read a value the
        // header defined, because the latch copy defines a *fresh* id and the
        // body would otherwise see the guard's value on every later iteration.
        let mut f = walk_loop(load(30, 20), 30);
        f.blocks[2].instructions.push(cmp(40, 30));
        assert_eq!(
            invert_loops_with(&mut f, LoopInvertConfig::testing_memory()),
            0
        );
    }

    #[test]
    fn a_header_whose_value_escapes_into_the_body_is_not_inverted() {
        // The body would read the value the guard computed on its single
        // execution instead of this iteration's.
        let mut f = counted_loop();
        f.blocks[2].instructions.push(add1(31, 10));
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_self_loop_is_not_inverted() {
        // Already bottom-tested; there is nothing to move.
        let mut f = func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20), add1(20, 20)], cond(10, 1, 2)),
            blk(2, vec![], ret()),
        ]);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_loop_with_two_latches_is_not_inverted() {
        //   1 header : ? 2 : 5 ; 2 : ? 3 : 4 ; 3 -> 1 ; 4 -> 1
        let mut f = func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20)], cond(10, 2, 5)),
            blk(2, vec![cmp(11, 20)], cond(11, 3, 4)),
            blk(3, vec![add1(20, 20)], br(1)),
            blk(4, vec![add1(21, 20)], br(1)),
            blk(5, vec![], ret()),
        ]);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn a_header_that_does_not_decide_the_exit_is_not_inverted() {
        // Both header successors are inside the loop: this is not a top-test
        // loop, so there is no test to sink.
        let mut f = func_of(vec![
            blk(0, vec![], br(1)),
            blk(1, vec![cmp(10, 20)], cond(10, 2, 3)),
            blk(2, vec![], br(4)),
            blk(3, vec![], br(4)),
            blk(4, vec![cmp(11, 20), add1(20, 20)], cond(11, 1, 5)),
            blk(5, vec![], ret()),
        ]);
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn an_oversized_header_is_not_duplicated() {
        let mut f = counted_loop();
        for k in 0..(MAX_DUP_INSTS as u32 + 1) {
            f.blocks[1].instructions.insert(0, add1(40 + k, 20));
        }
        // Those values are used nowhere, so only the size guard can stop it.
        assert_eq!(invert_loops_with(&mut f, LoopInvertConfig::testing()), 0);
    }

    #[test]
    fn the_pass_can_be_disabled() {
        // Escape hatch for bisecting a codegen regression. Exercised through
        // the config parameter: setting the environment variable here would be
        // process-global and would race with the tests running in parallel.
        let mut f = counted_loop();
        let n = invert_loops_with(&mut f, LoopInvertConfig::disabled());
        assert_eq!(n, 0);
    }
}

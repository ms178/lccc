//! Causal stack-slot census (backlog item SPILL-01).
//!
//! `ra_quality_census.py` counts *how many* stack references a function emits.
//! It cannot say *why* any of them exists, and "why" is the whole question when
//! the goal is to remove them: a slot that holds a spilled candidate is a
//! register-allocator problem, while a slot that holds an `alloca`, an
//! address-taken local, an i128/vector value or a call-argument area is
//! structurally required memory that no allocator change can remove.
//!
//! Only the allocator knows which is which — it is the component that decided
//! a value would not get a register — so the cause is published here, next to
//! the frame layout, instead of being re-derived (and eventually drifting) in a
//! script.
//!
//! The census is opt-in (`CCC_SLOT_CENSUS=1`) and purely observational: it is
//! emitted on stderr, it never changes codegen, and it is consumed by
//! `scripts/stack_census.py`, which joins it with the post-peephole assembly to
//! attribute every *emitted* stack reference to a cause.  Measuring the
//! assembly matters: peepholes delete accesses (dead stores, callee-save
//! elimination) and add others (address materialization), so a census taken
//! before emission would report traffic that never reaches the object file.

use super::super::state::CodegenState;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::IrType;
use crate::ir::reexports::IrFunction;

/// Why a stack slot exists.
///
/// Ordered by increasing "allocator's fault": `Alloca`/`Address`/`Wide` are
/// structural, `Spill` is the allocator losing, `Temp` is unclassified (the
/// census reports its share so it can be driven to zero).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum SlotCause {
    /// An explicit `alloca` (or a parameter homed in an alloca): addressable
    /// memory by construction, never removable by the allocator.
    Alloca,
    /// A value that also has a register home but needs a memory home because
    /// its address is observed (address-taken, or read through a pointer the
    /// allocator cannot track).
    Address,
    /// i128 / vector / over-aligned value: wider than any single register home
    /// the backend models.
    Wide,
    /// Register pressure: the value was a register-allocation candidate and
    /// lost.  This is the only cause a better allocator can remove.
    Spill,
    /// The allocator cannot put this value in a GPR at all — a float /
    /// long-double / 128-bit value in a configuration without vector/FP
    /// register homes.  Structural for the allocator, but a policy choice for
    /// the backend, which is why it is reported apart from `Wide`.
    NonGpr,
    /// Everything else (conservatively classified).  Reported explicitly so a
    /// growing `Temp` share is visible instead of being silently folded into
    /// `Spill`.
    Temp,
}

impl SlotCause {
    pub fn as_str(self) -> &'static str {
        match self {
            SlotCause::Alloca => "alloca",
            SlotCause::Address => "address",
            SlotCause::Wide => "wide",
            SlotCause::Spill => "spill",
            SlotCause::NonGpr => "nongpr",
            SlotCause::Temp => "temp",
        }
    }
}

/// One frame slot: offset from the frame base, cause, and how many values share
/// it (slot coalescing overlaps non-interfering values, so a slot can carry
/// several causes — the census keeps the strongest (max) one and reports the
/// value count so a surprising overlap is visible).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlotCensusEntry {
    /// Negative offset from the frame base (%rbp, or the virtual rbp in
    /// frame-pointer-omitted mode), matching the assembly's displacement.
    pub offset: i64,
    pub cause: SlotCause,
    pub values: u32,
}

/// Snapshot the frame layout's causal structure.
///
/// `eligible` is the allocator's own candidate set (see
/// [`crate::backend::regalloc::RegAllocResult::eligible`]); `reg_assigned` is
/// the final value → register map handed to the emitter.  `value_locations` is
/// authoritative for who lives in memory.
///
/// `is_32bit` is the pointer width of the target being code-generated.  It is
/// a parameter rather than a `crate::common::types::target_is_32bit()` read
/// for two reasons: the `NonGpr`/`Temp` split below is decided by
/// `is_non_gpr_type`, which takes the width explicitly, so taking it here too
/// keeps the one place that classifies types on one width; and the caller can
/// assert it against the process-wide target, which turns a silent drift into
/// a debug abort instead of a census that disagrees with the allocator that
/// produced the slots it is explaining.
pub fn build_slot_census(
    func: &IrFunction,
    state: &CodegenState,
    eligible: &FxHashSet<u32>,
    reg_assigned: &FxHashMap<u32, crate::backend::regalloc::PhysReg>,
    is_32bit: bool,
) -> Vec<SlotCensusEntry> {
    use crate::ir::reexports::Instruction;
    // Result type of every value with a typed definition, so a non-eligible
    // value can be split into "no GPR can hold it" (`NonGpr`) and "unclassified"
    // (`Temp`) using the allocator's own predicate.
    let mut types: FxHashMap<u32, IrType> = FxHashMap::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { dest, ty, .. }
                | Instruction::BinOp { dest, ty, .. }
                | Instruction::UnaryOp { dest, ty, .. }
                | Instruction::AtomicLoad { dest, ty, .. } => {
                    types.insert(dest.0, ty.clone());
                }
                Instruction::Cast { dest, to_ty, .. } => {
                    types.insert(dest.0, to_ty.clone());
                }
                Instruction::Call { info, .. } | Instruction::CallIndirect { info, .. } => {
                    if let Some(dest) = info.dest {
                        types.insert(dest.0, info.return_type.clone());
                    }
                }
                // `Intrinsic` carries no result type; intrinsics that produce a
                // GPR-sized result are eligible anyway, so leaving them out of
                // this map only affects the `Temp`/`NonGpr` split of the rare
                // FP-producing intrinsic that keeps a slot.
                _ => {}
            }
        }
    }
    let mut by_offset: FxHashMap<i64, SlotCensusEntry> = FxHashMap::default();
    for (&value_id, slot) in &state.value_locations {
        let offset = slot.0;
        let cause = if state.alloca_values.contains(&value_id) {
            SlotCause::Alloca
        } else if eligible.contains(&value_id) {
            if reg_assigned.contains_key(&value_id) {
                // Eligible and homed in a register, yet still materialized to
                // memory: something observes its address.
                SlotCause::Address
            } else {
                SlotCause::Spill
            }
        } else if state.i128_values.contains(&value_id)
            || state.vector_values.contains(&value_id)
            || state.vector128_values.contains(&value_id)
            || state.wide_values.contains(&value_id)
        {
            SlotCause::Wide
        } else if types
            .get(&value_id)
            .is_some_and(|ty| crate::backend::regalloc::is_non_gpr_type(ty, is_32bit))
        {
            SlotCause::NonGpr
        } else {
            SlotCause::Temp
        };
        let entry = by_offset.entry(offset).or_insert(SlotCensusEntry {
            offset,
            cause,
            values: 0,
        });
        entry.cause = entry.cause.max(cause);
        entry.values = entry.values.saturating_add(1);
    }
    let mut entries: Vec<SlotCensusEntry> = by_offset.into_values().collect();
    // Deterministic output: a census that shuffles between runs cannot be
    // diffed, and diffing is the point.
    entries.sort_by_key(|e| e.offset);
    entries
}

/// True when the census diagnostic is enabled.
pub fn slot_census_enabled() -> bool {
    std::env::var("CCC_SLOT_CENSUS").is_ok()
}

/// Emit one machine-readable line per function.
///
/// `off:cause:count` triples joined by `|`; offsets are sorted so two runs of
/// the same compiler on the same input produce byte-identical output.
///
/// The x86-64 prologue lays a frame out more than once for the same function
/// (the second and later passes refine the first), so identical consecutive
/// lines are suppressed: one function, one line, and the LAST distinct layout
/// wins when they genuinely differ.
pub fn emit_slot_census(
    func: &IrFunction,
    frame_size: i64,
    base: &str,
    callee_saves: usize,
    entries: &[SlotCensusEntry],
) {
    let mut line = String::from("[SLOT-MAP] fn=");
    line.push_str(&func.name);
    line.push_str(" frame=");
    line.push_str(&frame_size.to_string());
    line.push_str(" base=");
    line.push_str(base);
    line.push_str(" csave=");
    line.push_str(&callee_saves.to_string());
    line.push_str(" slots=");
    let mut first = true;
    for entry in entries {
        if !first {
            line.push('|');
        }
        first = false;
        line.push_str(&entry.offset.to_string());
        line.push(':');
        line.push_str(entry.cause.as_str());
        line.push(':');
        line.push_str(&entry.values.to_string());
    }
    thread_local! {
        /// Last line emitted per function, for the dedupe described above.
        static LAST: std::cell::RefCell<std::collections::HashMap<String, String>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }
    let changed = LAST.with(|cell| {
        let mut map = cell.borrow_mut();
        match map.get(&func.name) {
            Some(prev) if *prev == line => false,
            _ => {
                map.insert(func.name.clone(), line.clone());
                true
            }
        }
    });
    if changed {
        eprintln!("{}", line);
    }
}

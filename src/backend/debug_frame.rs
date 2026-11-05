//! Where the code generator put a function's parameters, as plain data.
//!
//! The DWARF writer can describe a parameter's name and type from the analysis,
//! but not its *location*: only the instruction selector knows which slot the
//! prologue wrote the incoming argument register into, and only it knows how far
//! that slot sits from the caller's stack pointer.  Those two facts are the whole
//! content of this module, and they travel as data for the same reason
//! [`crate::backend::elf::dwarf_spec`] does: the writer stays byte-oriented, the
//! code generator stays frame-oriented, and the driver is the one place that
//! knows both.
//!
//! # What a displacement means
//!
//! Each entry is the parameter's distance, in bytes, from the function's
//! **canonical frame address** -- the value the caller's stack pointer had at the
//! `call`, which is what `.eh_frame` already describes at every PC and what the
//! subprogram DIE names as its frame base (`DW_OP_call_frame_cfa`).  A
//! displacement is therefore a constant of the frame layout, not of an
//! instruction: the slot and the frame address both stay put for the whole body,
//! which is what lets the DIE state one expression instead of a location list.
//!
//! A backend reports `None` where it cannot make that promise.  The alternatives
//! are not equivalent: `None` costs a consumer one `print` (it prints
//! `<optimized out>`, as GCC does for a value it cannot locate), while a wrong
//! displacement costs it the truth.

use crate::common::fx_hash::FxHashMap;

/// Frame facts for one translation unit, keyed by the emitted symbol name.
///
/// Only functions whose frame gives a parameter a trustworthy home appear, and
/// only those parameters are listed: the vector is positional, so a function
/// with three parameters has three entries, each `Some(delta)` (a proven home)
/// or `None` (no home this writer can name).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DebugFrameFacts {
    /// Per function symbol, one displacement per parameter in IR parameter
    /// order, or `None` for a parameter the layout did not place.
    pub parameters: FxHashMap<String, Vec<Option<i32>>>,
}

impl DebugFrameFacts {
    /// Record one function's parameter homes.  An empty list is not recorded:
    /// a function without parameters has nothing to say, and an entry would only
    /// make the writer look up a name it cannot use.
    pub(crate) fn record(&mut self, function: &str, offsets: Vec<Option<i32>>) {
        if offsets.is_empty() {
            return;
        }
        self.parameters.insert(function.to_string(), offsets);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }
}

/// Turn per-parameter frame facts into the displacements a `DW_AT_location`
/// states, keeping only the homes the layout can promise.
///
/// The inputs are the prologue's own decisions, in IR parameter order:
///
/// * `bias` -- the distance from the register a slot displacement is measured
///   against to the canonical frame address (`%rbp + 16` on x86-64 and AArch64,
///   `%ebp + 8` on i686).  `None` when the function has no stable anchor for slot
///   displacements (an omitted frame pointer, a frame the prologue moves at
///   points no expression can name), in which case nothing is described.
/// * `slots` -- each parameter's alloca displacement from that register, `None`
///   when the parameter has no alloca slot at all.
/// * `over_aligned` -- the alloca's address is aligned *at run time*, so the raw
///   slot is a bound, not the object's address.
/// * `slot_dead` -- the prologue never wrote the slot (the value lives in a
///   register or vector home the expression cannot name).
///
/// Every refusal is silent on purpose: the DIE then carries no
/// `DW_AT_location`, which a consumer renders as `<optimized out>`.  The
/// alternative -- describing a slot that does not hold the value -- is a
/// confident wrong answer, and no consumer can detect one.
pub(crate) fn parameter_homes(
    bias: Option<i32>,
    skip: usize,
    slots: &[Option<i64>],
    over_aligned: &[bool],
    slot_dead: &[bool],
) -> Vec<Option<i32>> {
    // A hidden return pointer occupies IR parameter 0 while the source list --
    // and therefore the DWARF parameter list -- starts at the first visible one.
    // Applying the hidden slot to the first source parameter would describe `n`
    // through the return buffer, which is the confident-lie failure this module
    // exists to prevent; the length check in `apply_frame_facts` would then
    // reject the whole function, so the skip is also what keeps a struct-returning
    // function's parameters described at all.
    let slots = slots.get(skip..).unwrap_or_default();
    let over_aligned = over_aligned.get(skip..).unwrap_or_default();
    let slot_dead = slot_dead.get(skip..).unwrap_or_default();
    let Some(bias) = bias else {
        return vec![None; slots.len()];
    };
    slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let slot = (*slot)?;
            if over_aligned.get(index).copied().unwrap_or(false)
                || slot_dead.get(index).copied().unwrap_or(false)
            {
                return None;
            }
            // The displacement has to be an `i32`: a slot beyond two gigabytes of
            // frame is not a displacement any consumer should be handed, and
            // saturating would state a wrong address. `i32::try_from` on the sum
            // keeps the arithmetic checked rather than wrapping in release.
            i32::try_from(slot).ok().and_then(|slot| slot.checked_add(bias))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_keeps_one_positional_entry_per_parameter() {
        let mut facts = DebugFrameFacts::default();
        facts.record("f", vec![Some(-24), None, Some(-40)]);
        assert_eq!(facts.parameters["f"], vec![Some(-24), None, Some(-40)]);
        assert!(!facts.is_empty());
    }

    #[test]
    fn record_ignores_a_function_without_parameters() {
        let mut facts = DebugFrameFacts::default();
        facts.record("f", Vec::new());
        assert!(
            facts.is_empty(),
            "a parameterless function must not create an entry: the writer would \
             look up a list it cannot index"
        );
    }

    #[test]
    fn a_proven_home_is_slot_plus_bias() {
        // `-8(%rbp)` on x86-64 is 24 bytes below the caller's stack pointer:
        // -8 + 16. This is the number GCC 16.2 states for the same slot.
        assert_eq!(
            parameter_homes(Some(16), 0, &[Some(-8), Some(-16)], &[false, false], &[false, false]),
            vec![Some(8), Some(0)]
        );
        // i686: `%ebp + 8`, so `-8(%ebp)` is 16 below the caller's stack pointer.
        assert_eq!(
            parameter_homes(Some(8), 0, &[Some(-8)], &[false], &[false]),
            vec![Some(0)]
        );
    }

    #[test]
    fn no_anchor_describes_nothing() {
        // A frame with no stable anchor must not fall back to the raw slot: the
        // slot displacement is relative to a register the expression does not
        // name, so every number it could print would be wrong.
        assert_eq!(
            parameter_homes(None, 0, &[Some(-8), Some(-16)], &[false, false], &[false, false]),
            vec![None, None]
        );
    }

    #[test]
    fn a_missing_slot_is_not_described() {
        assert_eq!(
            parameter_homes(Some(16), 0, &[None, Some(-24)], &[false, false], &[false, false]),
            vec![None, Some(-8)]
        );
    }

    #[test]
    fn the_run_time_aligned_case_is_not_described() {
        // `_Alignas(32) int a` has a slot, but the object sits at a run-time
        // aligned address above it, so the slot is a bound and not the object.
        assert_eq!(
            parameter_homes(Some(16), 0, &[Some(-64), Some(-80)], &[true, false], &[false, false]),
            vec![None, Some(-64)]
        );
    }

    #[test]
    fn a_slot_the_prologue_never_wrote_is_not_described() {
        // A parameter whose value the prologue left in a vector register: the
        // slot is allocated but stale, and reading it would print the padding.
        assert_eq!(
            parameter_homes(Some(16), 0, &[Some(-48)], &[false], &[true]),
            vec![None]
        );
    }

    #[test]
    fn every_input_shorter_than_the_list_refuses_the_overflowing_entry() {
        // Defensive: the flag vectors are indexed per parameter, and a caller
        // that passes a short one must get refusals, never a panic and never an
        // "aligned" verdict it did not measure.
        assert_eq!(
            parameter_homes(Some(16), 0, &[Some(-8), Some(-16)], &[], &[]),
            vec![Some(8), Some(0)],
            "absent flags mean 'not over-aligned, not dead'; they are indexed by \
             presence, not by length"
        );
        assert_eq!(
            parameter_homes(Some(16), 0, &[Some(-8)], &[true], &[]),
            vec![None]
        );
    }

    #[test]
    fn a_hidden_return_pointer_does_not_shift_the_list() {
        // IR parameter 0 is the return buffer of a struct-returning function; the
        // source list starts at the parameter after it. The first reported entry
        // must therefore be `n`, not the buffer.
        assert_eq!(
            parameter_homes(
                Some(16),
                1,
                &[Some(-8), Some(-24), Some(-40)],
                &[false, false, false],
                &[false, false, false]
            ),
            vec![Some(-8), Some(-24)]
        );
        // A skip past the end is an empty list, never a panic.
        assert!(parameter_homes(Some(16), 9, &[Some(-8)], &[false], &[false]).is_empty());
    }

    #[test]
    fn a_displacement_that_does_not_fit_is_refused() {
        // A frame beyond two gigabytes has slots no `i32` displacement names;
        // refusing keeps the arithmetic checked instead of wrapping.
        assert_eq!(
            parameter_homes(Some(16), 0, &[Some(i64::from(i32::MAX))], &[false], &[false]),
            vec![None]
        );
    }

    #[test]
    fn record_replaces_a_duplicate_name() {
        // A name can be emitted twice (an alias body, a `.set` symbol); the last
        // word wins, which keeps the map's value the one the last emission used.
        let mut facts = DebugFrameFacts::default();
        facts.record("f", vec![Some(-16)]);
        facts.record("f", vec![Some(-32)]);
        assert_eq!(facts.parameters["f"], vec![Some(-32)]);
    }
}

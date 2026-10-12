//! The bounded tail copy: the one ladder both x86-family backends walk.
//!
//! An aggregate argument is classified per eightbyte (x86-64) or per word
//! (i386), and the psABI rounds it up to whole classes.  The bytes of the last
//! class beyond the end of the C object are *unspecified*, which licenses
//! padding in the destination -- not a wider access.  Reading past the object
//! is measurable, not theoretical: `tests/regression/large_aggregate_call_copy.c`
//! places its aggregate flush against a PROT_NONE page and sizes 1, 7, 9 and 15
//! faulted with a `movq`-per-eightbyte caller, while 8 and 16 did not -- the
//! exact profile of rounding.  So every partial class is copied with a
//! widest-first {4,2,1} sequence that stays inside the object.
//!
//! That rule was written three times (the x86-64 register path, the x86-64 stack
//! path and the i386 regparm path), each with its own loop condition and its own
//! width table.  Only the coverage law is shared, so it lives here once, with the
//! tests; the mnemonic and register naming stay per-site because the ferries
//! differ (`%eax` staging vs a live `%ecx`).

/// One step of a tail copy: `width` bytes at `offset`, relative to the object's
/// start within the class being loaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NarrowCopy {
    pub(crate) offset: usize,
    pub(crate) width: usize,
}

/// Greedy widest-first cover of `len` bytes using widths in `1..=max_width`,
/// where `max_width` is one of the x86 move widths (1, 2 or 4).
///
/// A `len` of zero yields an empty plan, which means *nothing was copied*: a
/// caller must not read an empty plan as a successful copy, so the sites that
/// promise `1..=len` say so with a `debug_assert!` instead.
pub(crate) fn narrow_copy_plan(len: usize, max_width: usize) -> Vec<NarrowCopy> {
    debug_assert!(matches!(max_width, 1 | 2 | 4), "not an x86 move width");
    let mut plan = Vec::new();
    let mut done = 0usize;
    while done < len {
        // Widest move that still lands inside the object: `min` caps it at the
        // caller's class width, `ilog2` floors it to a power of two.
        let width = 1 << (len - done).min(max_width).ilog2();
        plan.push(NarrowCopy {
            offset: done,
            width,
        });
        done += width;
    }
    plan
}

/// The move mnemonic for one width of the ladder.  Both backends spell the
/// three widths identically; keeping the table here is what stops a site from
/// quietly offering a narrower cover than the object needs.
pub(crate) const fn narrow_move_mnemonic(width: usize) -> &'static str {
    match width {
        4 => "movl",
        2 => "movw",
        _ => "movb",
    }
}

/// The `%eax`-family ferry register for one width.  The partial class is staged
/// through the accumulator because it is the only register both tail-copy sites
/// can assume is free at that point.
pub(crate) const fn eax_ferry(width: usize) -> &'static str {
    match width {
        4 => "%eax",
        2 => "%ax",
        _ => "%al",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widths(len: usize, max_width: usize) -> Vec<usize> {
        narrow_copy_plan(len, max_width)
            .into_iter()
            .map(|s| s.width)
            .collect()
    }

    #[test]
    fn plan_covers_exactly_the_object_length() {
        for len in 0..=64 {
            for max_width in [1, 2, 4] {
                let plan = narrow_copy_plan(len, max_width);
                let covered: usize = plan.iter().map(|s| s.width).sum();
                assert_eq!(covered, len, "len {len} max {max_width}: {plan:?}");
                let mut expect = 0usize;
                for step in &plan {
                    assert!(step.width <= max_width, "wider than the class: {step:?}");
                    assert_eq!(step.offset, expect, "gap or overlap at {step:?}");
                    expect += step.width;
                }
            }
        }
    }

    #[test]
    fn eightbyte_plan_matches_the_loop_it_replaces() {
        // These are the sequences the x86-64 register and stack paths emitted
        // line by line; the unification may not change one instruction.
        assert_eq!(widths(1, 4), vec![1]);
        assert_eq!(widths(2, 4), vec![2]);
        assert_eq!(widths(3, 4), vec![2, 1]);
        assert_eq!(widths(4, 4), vec![4]);
        assert_eq!(widths(5, 4), vec![4, 1]);
        assert_eq!(widths(6, 4), vec![4, 2]);
        assert_eq!(widths(7, 4), vec![4, 2, 1]);
        assert_eq!(widths(8, 4), vec![4, 4]);
        assert_eq!(widths(15, 4), vec![4, 4, 4, 2, 1]);
    }

    #[test]
    fn word_plan_matches_the_regparm_loop_it_replaces() {
        // i386 handles a full word with one `movl` before reaching the ladder,
        // so it only ever asks for a 1..=3 byte cover with 2-byte moves.
        assert_eq!(widths(1, 2), vec![1]);
        assert_eq!(widths(2, 2), vec![2]);
        assert_eq!(widths(3, 2), vec![2, 1]);
        assert_eq!(widths(4, 2), vec![2, 2]);
    }

    #[test]
    fn empty_length_copies_nothing_and_says_so() {
        // The fail-open the audit asked about: a zero-length tail must produce no
        // move at all (so the destination keeps whatever the transient held,
        // which the psABI allows) rather than one bogus byte-wide access.
        assert!(narrow_copy_plan(0, 4).is_empty());
        assert!(narrow_copy_plan(0, 1).is_empty());
    }

    #[test]
    fn width_tables_are_total_over_the_ladder() {
        for width in [1, 2, 4] {
            assert_ne!(narrow_move_mnemonic(width), "");
            assert!(eax_ferry(width).starts_with('%'));
        }
        assert_eq!(narrow_move_mnemonic(4), "movl");
        assert_eq!(narrow_move_mnemonic(2), "movw");
        assert_eq!(narrow_move_mnemonic(1), "movb");
        assert_eq!(eax_ferry(4), "%eax");
        assert_eq!(eax_ferry(2), "%ax");
        assert_eq!(eax_ferry(1), "%al");
    }
}

//! Branch relaxation of one section, computed on its resizable items alone.
//!
//! Relaxation is a fixed point: a branch is short while its target is in
//! rel8 reach, alignment padding depends on where the branches before it
//! end, and a branch that grows in one pass pushes others out of reach in
//! the next. A section of a few thousand functions needs hundreds of
//! passes, each of which lengthens three or four branches.
//!
//! The assembler used to edit the section for every pass: rebuild its bytes
//! twice (the branches, then the padding) and move every label, relocation
//! and line-table row behind the first change. That is passes x section
//! size, and it was half the time spent on a large file (3000 functions,
//! 60,000 branches, 234 passes: 6.3 s, 3.2 s of it here; GNU as, which
//! walks its frags once per pass, takes 1.0 s for the whole file).
//!
//! Nothing in a pass needs the bytes. Its decisions read only
//!
//! * where each relaxable branch and each padding run starts, and how long
//!   it is;
//! * where the label a branch goes to stands.
//!
//! So this module keeps exactly that: the *items* -- relaxable branches and
//! alignment markers -- in layout order, each with the size it has in the
//! section's bytes (`phys`) and the size the layout gives it (`len`). The
//! position of anything is its offset in the bytes plus the size change of
//! the items in front of it, `d[k]`, a prefix sum. A pass updates sizes and
//! prefix sums; the caller edits the section once, when the sizes have
//! settled.
//!
//! # The passes are GNU as's
//!
//! The decisions are the ones the in-place implementation makes
//! (`ElfWriterCore::plan_jump_growth`, `fixup_alignment_markers_behind`),
//! which are GNU as's `relax_segment`: one sweep in address order that
//! carries the growth so far (`stretch`), so that a backward target and the
//! padding in between already reflect this pass, while a forward target is
//! its old address plus the stretch -- or its old address alone when the
//! stretch is positive and an alignment lies in between, which may absorb
//! it. Then the padding is fitted again, front to back. Both sweeps are
//! transcribed here item for item, and `elf_writer_common`'s differential
//! tests assemble the same sources through both and compare the objects.
//!
//! # A pass looks only at what can have changed
//!
//! GNU as walks every frag in every pass. Most of a section is settled long
//! before the last pass, and a sweep over it decides nothing:
//!
//! * A short branch whose span (itself to its target, at most 130 bytes)
//!   holds no item that changed size stands where it stood relative to its
//!   target. If it reached before, it reaches now.
//! * An alignment that moved by a multiple of its boundary keeps its
//!   padding.
//!
//! So a growth sweep visits *windows*: `SHORT_REACH` either way of every
//! item the pass before resized. It extends a window for as long as it
//! changes sizes itself, and between windows it goes straight to the next
//! alignment whose boundary does not divide the stretch (`boundaries`).
//! What it skips, the whole sweep would have left alone; the argument is
//! with [`Layout::plan`]. The padding sweep is cut the same way.
//!
//! # Postponed branches
//!
//! The stretch rule has a consequence that makes large sections slow for
//! GNU as: a forward branch across an alignment that is out of reach by
//! fewer bytes than the section has grown so far *in this pass* is kept
//! short, to be lengthened in a later pass. Behind the first functions that
//! grow, that is nearly every branch still to be lengthened, pass after
//! pass (3000 functions: 234 passes). Looking at each of them in every pass
//! is what the windows are meant to avoid.
//!
//! Such a branch grows in the pass in which the stretch in front of it is
//! smaller than the bytes it misses its target by (`Layout::plan`), and
//! that number changes only when something inside its span is resized. So
//! the postponed branches are kept in a tree by that number (`postponed`),
//! and a sweep asks it for the next one the current stretch lets through.
//! A second tree holds where each of them ends (`postponed_end`); after a
//! pass it names the ones whose span holds a resized item, and those the
//! next sweep looks at again, which brings their numbers up to date.
//!
//! One whole sweep at the end confirms that no branch is out of reach: a
//! layout that passes it is a fixed point of the whole-sweep algorithm by
//! definition, and one that does not is finished with whole sweeps.

use crate::backend::elf::MAX_DIRECTIVE_FILL;
use crate::passes::loop_align::tight_bucket_log2;

/// More than a rel8 displacement spans, either way, with the longest short
/// form in front of it.
const SHORT_REACH: i64 = 256;

#[cfg(test)]
thread_local! {
    /// Test-only: confirming sweeps that found a branch the windows missed.
    /// The differential tests require zero.
    pub(super) static WINDOW_MISSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test-only: growth sweeps taken, whole and windowed, and the steps
    /// they took (an item looked at, or a jump ahead to the next one that
    /// matters) -- what the windows are for.
    pub(super) static SWEEP_ITEMS: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

/// A place in the section's bytes: an offset and, for a zero-width record
/// (a label), its source position. `None` marks a byte-bearing record (an
/// instruction), which always stands behind padding that ends at its offset.
#[derive(Clone, Copy, Debug)]
pub(super) struct Point {
    pub off: u64,
    pub seq: Option<u64>,
}

/// A branch relaxation may resize.
#[derive(Clone, Debug)]
pub(super) struct JumpSpec {
    pub off: u64,
    /// Length of the short form: opcode, rel8 and a dead 0x66 if any.
    pub short_len: u64,
    pub long_len: u64,
    /// Whether it has its short form now; else its near form.
    pub short: bool,
    /// The label it goes to, and the constant added to it.
    pub target: Point,
    pub addend: i64,
}

/// The branch that closes a tight loop: where it starts and how long it is.
#[derive(Clone, Copy, Debug)]
pub(super) struct Latch {
    pub at: u64,
    pub len: LatchLen,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum LatchLen {
    /// A branch relaxation leaves alone.
    Fixed(u64),
    /// Index into the `JumpSpec` list the layout was built from.
    Jump(usize),
}

#[derive(Clone, Debug)]
pub(super) enum MarkerSpec {
    /// `.p2align` / `.balign`: pad to the next multiple of `align`, unless
    /// that takes more than `max_skip` bytes.
    Align { align: u64, max_skip: Option<u64> },
    /// `.lccc_tight_loop`: pad the header to the bucket of the loop's
    /// encoded span, header label to the end of the latch. `header` says
    /// why the label is no header of this section, `latch` is `None` when
    /// no branch behind the header goes back to it; either way the marker
    /// asks for nothing.
    Tight {
        header: Result<Point, &'static str>,
        latch: Option<Latch>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct MarkerSpecAt {
    pub off: u64,
    /// Bytes of padding the section holds at `off` now.
    pub padding: u64,
    /// Source position of the directive.
    pub seq: u64,
    pub spec: MarkerSpec,
}

/// What the layout made of a branch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct JumpOutcome {
    /// The first pass gave it its short form.
    pub shrunk: bool,
    /// The growth pass (counted from 1) that gave it its near form.
    pub grown: Option<u32>,
}

/// What the layout made of a marker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MarkerOutcome {
    pub padding: u64,
    /// A padding sweep changed its size at least once, so its bytes are to
    /// be written again even if the size came back to what it was.
    pub repadded: bool,
    /// For a tight-loop marker a padding sweep decided: the bucket it was
    /// given last, `None` when it was refused.
    pub tight: Option<Option<u64>>,
}

/// What a tight-loop marker was found to span (bytes, log2 of the bucket,
/// the bucket), or why it has none. Reported for `CCC_DEBUG_TIGHT`.
pub(super) type TightVerdict = Result<(u64, u32, u64), &'static str>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Jump(u32),
    Marker(u32),
}

#[derive(Clone, Copy, Debug)]
struct Item {
    /// Offset in the section's bytes.
    off: u64,
    /// Size in the section's bytes.
    phys: u64,
    /// Size in the layout.
    len: u64,
    kind: Kind,
}

#[derive(Clone, Debug)]
struct Jump {
    item: u32,
    short_len: u64,
    long_len: u64,
    short: bool,
    target_off: u64,
    /// Number of items in front of the target label.
    target_k: u32,
    addend: i64,
    outcome: JumpOutcome,
}

#[derive(Clone, Copy, Debug)]
enum Fit {
    Align {
        align: u64,
        max_skip: Option<u64>,
    },
    Tight {
        /// The header label and the number of items in front of it.
        header: Result<(u64, u32), &'static str>,
        /// The latch, the number of items in front of it, and its length.
        latch: Option<(u64, u32, LatchLen)>,
    },
}

#[derive(Clone, Debug)]
struct Marker {
    item: u32,
    fit: Fit,
    repadded: bool,
    tight: Option<Option<u64>>,
}

/// Sizes changed by one sweep: item index, ascending, and the running total
/// of the changes up to and including it.
type Changes = Vec<(u32, i64)>;

/// One number per item, and the first item at or behind a given one whose
/// number exceeds a bound, in logarithmic time (a maximum tree).
struct MaxTree {
    /// Leaves in the upper half, `leaves` of them; node `k` holds the
    /// maximum of nodes `2k` and `2k + 1`.
    nodes: Vec<i64>,
    leaves: usize,
}

impl MaxTree {
    /// The number of an item that is never found.
    const NONE: i64 = i64::MIN;

    fn new(values: &[i64]) -> Self {
        let leaves = values.len().next_power_of_two().max(1);
        let mut nodes = vec![Self::NONE; 2 * leaves];
        nodes[leaves..leaves + values.len()].copy_from_slice(values);
        for k in (1..leaves).rev() {
            nodes[k] = nodes[2 * k].max(nodes[2 * k + 1]);
        }
        MaxTree { nodes, leaves }
    }

    fn set(&mut self, i: usize, value: i64) {
        let mut k = self.leaves + i;
        if self.nodes[k] == value {
            return;
        }
        self.nodes[k] = value;
        while k > 1 {
            k /= 2;
            self.nodes[k] = self.nodes[2 * k].max(self.nodes[2 * k + 1]);
        }
    }

    /// The first item at or behind `from` whose number is above `bound`.
    fn first_above(&self, from: usize, bound: i64) -> Option<usize> {
        if from >= self.leaves || self.nodes[1] <= bound {
            return None;
        }
        // Climb from the leaf `from` to the first subtree to its right (or
        // its own) that holds such a number, then descend to its leftmost.
        let mut k = self.leaves + from;
        loop {
            if self.nodes[k] > bound {
                break;
            }
            // Step to the next subtree on the right: up while a right
            // child, then across.
            while k % 2 == 1 {
                k /= 2;
                if k <= 1 {
                    return None;
                }
            }
            k += 1;
        }
        while k < self.leaves {
            k = if self.nodes[2 * k] > bound {
                2 * k
            } else {
                2 * k + 1
            };
        }
        Some(k - self.leaves)
    }
}

/// The resizable items of one section, see the module documentation.
pub(super) struct Layout {
    items: Vec<Item>,
    /// `d[k]`: size change of `items[..k]` against the section's bytes, as
    /// of the start of the pass being computed.
    d: Vec<i64>,
    jumps: Vec<Jump>,
    markers: Vec<Marker>,
    /// For every item, the index of the first marker behind it (or the
    /// number of items).
    next_marker: Vec<u32>,
    /// Item indices of the tight-loop markers, ascending.
    tight: Vec<u32>,
    /// Item indices of the branches with an addend, ascending: their label
    /// can be anywhere, so every growth sweep looks at them.
    far: Vec<u32>,
    /// The boundary of every alignment, by item: a shift moves the padding
    /// of those whose boundary does not divide it, and the tree finds the
    /// next of them. A boundary that is no power of two (`.balign 12`) is
    /// kept as `i64::MAX`: any shift may move it.
    boundaries: MaxTree,
    /// The postponed branches (see the module documentation), by item: the
    /// bytes each missed its target by when a sweep last looked at it.
    postponed: MaxTree,
    /// For each of them, the number of items in front of its target: a
    /// resized item below that number and behind the branch is in its span.
    postponed_end: MaxTree,
    /// Sizes changed so far by the growth sweep under way.
    visited: Changes,
    passes: u32,
    /// Take every sweep over the whole section (the tests' reference, and
    /// the way a layout is finished once a confirming sweep found the
    /// windows wanting).
    whole_sweeps: bool,
}

/// Padding an alignment asks for at `at`.
#[inline]
fn align_padding(at: i64, align: u64, max_skip: Option<u64>) -> u64 {
    let at = at as u64;
    let pad = if align.is_power_of_two() {
        // The distance up to the next multiple: `-at` modulo the boundary
        // (no division, and nothing to overflow for a boundary of 2^63).
        at.wrapping_neg() & (align - 1)
    } else {
        at.div_ceil(align) * align - at
    };
    match max_skip {
        Some(skip) if pad > skip => 0,
        _ => pad,
    }
}

/// Total of the entries of `changes` whose index is below `k`.
fn total_before(changes: &[(u32, i64)], k: u32) -> i64 {
    match changes.partition_point(|&(idx, _)| idx < k) {
        0 => 0,
        n => changes[n - 1].1,
    }
}

impl Layout {
    /// Lay out `jumps` and `markers` as they stand in the section's bytes.
    ///
    /// `None` when the records do not have the shape the layout relies on,
    /// and the caller is to relax the section in place instead:
    ///
    /// * items whose order by offset and source position is not the order
    ///   of their ends (an input would need padding runs out of source
    ///   order; none is known);
    /// * an alignment whose padding is not what its own offset asks for.
    ///   The sweeps skip markers that have not moved, which is right only
    ///   if they were fitted where they stand -- as they are when emitted,
    ///   and after every padding sweep.
    pub(super) fn build(jumps: &[JumpSpec], markers: &[MarkerSpecAt]) -> Option<Self> {
        let mut keyed: Vec<(u64, u8, u64, Item)> = Vec::with_capacity(jumps.len() + markers.len());
        for (j, jump) in jumps.iter().enumerate() {
            let phys = if jump.short {
                jump.short_len
            } else {
                jump.long_len
            };
            keyed.push((
                jump.off,
                1,
                0,
                Item {
                    off: jump.off,
                    phys,
                    len: phys,
                    kind: Kind::Jump(j as u32),
                },
            ));
        }
        for (m, marker) in markers.iter().enumerate() {
            if let MarkerSpec::Align { align, max_skip } = marker.spec {
                if align > 1 && align_padding(marker.off as i64, align, max_skip) != marker.padding
                {
                    return None;
                }
            }
            keyed.push((
                marker.off,
                0,
                marker.seq,
                Item {
                    off: marker.off,
                    phys: marker.padding,
                    len: marker.padding,
                    kind: Kind::Marker(m as u32),
                },
            ));
        }
        // A marker at a jump's offset is its (empty) leading padding.
        keyed.sort_by_key(|&(off, rank, seq, _)| (off, rank, seq));
        let items: Vec<Item> = keyed.into_iter().map(|(.., item)| item).collect();

        // The order above must also be the order of the items' ends, which
        // is what decides on which side of an item a record stands.
        let (mut covered, mut last_end) = (0u64, (0u64, 0u64));
        for item in &items {
            if item.off < covered {
                return None;
            }
            covered = item.off + item.phys;
            if let Kind::Marker(m) = item.kind {
                let end = (covered, markers[m as usize].seq);
                if end < last_end {
                    return None;
                }
                last_end = end;
            }
        }

        let n = items.len();
        let mut next_marker = vec![n as u32; n];
        let mut next = n as u32;
        for i in (0..n).rev() {
            next_marker[i] = next;
            if matches!(items[i].kind, Kind::Marker(_)) {
                next = i as u32;
            }
        }

        // Number of items that stand in front of `p`, and so move it when
        // they change size. Growth inside a jump moves what follows its
        // first byte: a label at the jump's own offset stays. Padding moves
        // what stands behind its END: every byte-bearing record there, and
        // a label there only if it was written after the directive --
        // `ElfWriterCore::shift_after`'s tie rule, and `PadQueue::map`'s.
        let before = |p: Point| -> u32 {
            items.partition_point(|item| match item.kind {
                Kind::Jump(_) => item.off < p.off,
                Kind::Marker(m) => {
                    let end = item.off + item.phys;
                    match p.seq {
                        Some(seq) => (end, markers[m as usize].seq) < (p.off, seq),
                        None => end <= p.off,
                    }
                }
            }) as u32
        };

        let mut item_of_jump = vec![0u32; jumps.len()];
        let mut item_of_marker = vec![0u32; markers.len()];
        for (i, item) in items.iter().enumerate() {
            match item.kind {
                Kind::Jump(j) => item_of_jump[j as usize] = i as u32,
                Kind::Marker(m) => item_of_marker[m as usize] = i as u32,
            }
        }
        let jump_states: Vec<Jump> = jumps
            .iter()
            .enumerate()
            .map(|(j, jump)| Jump {
                item: item_of_jump[j],
                short_len: jump.short_len,
                long_len: jump.long_len,
                short: jump.short,
                target_off: jump.target.off,
                target_k: before(jump.target),
                addend: jump.addend,
                outcome: JumpOutcome::default(),
            })
            .collect();
        let marker_states: Vec<Marker> = markers
            .iter()
            .enumerate()
            .map(|(m, marker)| {
                let fit = match &marker.spec {
                    MarkerSpec::Align { align, max_skip } => Fit::Align {
                        align: *align,
                        max_skip: *max_skip,
                    },
                    MarkerSpec::Tight { header, latch } => Fit::Tight {
                        header: header.map(|h| (h.off, before(h))),
                        latch: latch.map(|l| {
                            let at = Point {
                                off: l.at,
                                seq: None,
                            };
                            (l.at, before(at), l.len)
                        }),
                    },
                };
                Marker {
                    item: item_of_marker[m],
                    fit,
                    repadded: false,
                    tight: None,
                }
            })
            .collect();
        let boundaries: Vec<i64> = items
            .iter()
            .map(|item| match item.kind {
                Kind::Marker(m) => match marker_states[m as usize].fit {
                    Fit::Align { align, .. } if align > 1 => {
                        if align.is_power_of_two() {
                            i64::try_from(align).unwrap_or(i64::MAX)
                        } else {
                            i64::MAX
                        }
                    }
                    _ => MaxTree::NONE,
                },
                Kind::Jump(_) => MaxTree::NONE,
            })
            .collect();
        let (mut tight, mut far) = (Vec::new(), Vec::new());
        for (i, item) in items.iter().enumerate() {
            match item.kind {
                Kind::Marker(m) => {
                    if matches!(marker_states[m as usize].fit, Fit::Tight { .. }) {
                        tight.push(i as u32);
                    }
                }
                Kind::Jump(j) => {
                    if jump_states[j as usize].addend != 0 {
                        far.push(i as u32);
                    }
                }
            }
        }
        Some(Layout {
            items,
            d: vec![0; n + 1],
            jumps: jump_states,
            markers: marker_states,
            next_marker,
            tight,
            far,
            boundaries: MaxTree::new(&boundaries),
            postponed: MaxTree::new(&vec![MaxTree::NONE; n]),
            postponed_end: MaxTree::new(&vec![MaxTree::NONE; n]),
            visited: Vec::new(),
            passes: 0,
            whole_sweeps: false,
        })
    }

    /// Take every sweep over the whole section.
    #[cfg(test)]
    pub(super) fn with_whole_sweeps(mut self) -> Self {
        self.whole_sweeps = true;
        self
    }

    pub(super) fn jump_outcome(&self, j: usize) -> JumpOutcome {
        self.jumps[j].outcome
    }

    pub(super) fn marker_outcome(&self, m: usize) -> MarkerOutcome {
        let marker = &self.markers[m];
        MarkerOutcome {
            padding: self.items[marker.item as usize].len,
            repadded: marker.repadded,
            tight: marker.tight,
        }
    }

    /// Growth passes that changed something.
    pub(super) fn passes(&self) -> u32 {
        self.passes
    }

    /// Where item `i` starts, as of the start of the pass.
    #[inline]
    fn base(&self, i: usize) -> i64 {
        self.items[i].off as i64 + self.d[i]
    }

    /// The first alignment at or behind item `i` whose padding a shift by
    /// `s` bytes can change: one whose boundary does not divide `s`. (A
    /// power of two divides `s` exactly when it is no larger than the
    /// lowest set bit of `s`.)
    #[inline]
    fn next_moved(&self, i: usize, s: i64) -> Option<usize> {
        if s == 0 {
            return None;
        }
        self.boundaries.first_above(i, s.isolate_lowest_one())
    }

    /// Run the passes: with `from_shortest`, give every branch its short
    /// form first; then lengthen the ones that do not reach, pass after
    /// pass, until none is left.
    ///
    /// `charge` is asked for the bytes each padding run grows by (the
    /// directive-fill budget); `tight` is told what each tight-loop marker
    /// spans, every time one is decided.
    pub(super) fn settle(
        &mut self,
        from_shortest: bool,
        charge: &mut dyn FnMut(u64) -> Result<(), String>,
        tight: &mut dyn FnMut(usize, TightVerdict),
    ) -> Result<(), String> {
        let n = self.items.len() as u32;
        if from_shortest {
            // The first pass shortens every relaxable jump the layout
            // resolves, even one that does not currently fit: the growth
            // passes restore exactly those that still do not fit once the
            // layout has settled.
            let mut shrunk = Changes::new();
            let mut total = 0i64;
            for i in 0..self.items.len() {
                let Kind::Jump(j) = self.items[i].kind else {
                    continue;
                };
                let jump = &mut self.jumps[j as usize];
                if jump.short {
                    continue;
                }
                jump.short = true;
                jump.outcome.shrunk = true;
                total += jump.short_len as i64 - jump.long_len as i64;
                self.items[i].len = jump.short_len;
                shrunk.push((i as u32, total));
            }
            if !shrunk.is_empty() {
                let repads = self.fit_padding(&shrunk, charge, tight)?;
                self.fold(&shrunk, &repads);
            }
        }

        // The first growth sweep is over the whole section: whatever
        // happened before it (the shortening above, a deferred `.skip`
        // resized, padding fitted again) can have moved anything.
        let whole = [(0u32, n)];
        let mut windows = whole.to_vec();
        // The sweep under way is the whole one that confirms a result the
        // windows arrived at.
        let mut confirming = false;
        loop {
            let is_whole = windows == whole;
            let mut recheck = Vec::new();
            let grow = self.plan(&windows, &mut recheck);
            if grow.is_empty() {
                if is_whole {
                    return Ok(());
                }
                windows = whole.to_vec();
                confirming = true;
                continue;
            }
            if confirming {
                // The windows missed a branch. What follows is still the
                // fixed point of whole sweeps, so the object is sound; the
                // tests count on never getting here.
                self.whole_sweeps = true;
                #[cfg(test)]
                WINDOW_MISSES.set(WINDOW_MISSES.get() + 1);
                debug_assert!(false, "windowed relaxation missed a branch");
            }
            confirming = false;
            self.passes += 1;
            let mut changed = Changes::with_capacity(grow.len());
            let mut total = 0i64;
            for &i in &grow {
                // `plan` names jumps only.
                let Kind::Jump(j) = self.items[i as usize].kind else {
                    continue;
                };
                let jump = &mut self.jumps[j as usize];
                jump.short = false;
                jump.outcome.grown = Some(self.passes);
                total += jump.long_len as i64 - jump.short_len as i64;
                self.items[i as usize].len = jump.long_len;
                changed.push((i, total));
            }
            let repads = self.fit_padding(&changed, charge, tight)?;
            windows = if self.whole_sweeps {
                whole.to_vec()
            } else {
                self.windows_after(&changed, &repads, &recheck)
            };
            self.fold(&changed, &repads);
        }
    }

    /// Decide which short jumps must grow, in one sweep in layout order
    /// that carries the growth so far -- GNU as's `relax_segment` order,
    /// see `ElfWriterCore::plan_jump_growth`, whose decisions these are.
    ///
    /// Only the items in `windows` (ascending, disjoint index ranges) are
    /// looked at, and what the sweep itself makes necessary:
    ///
    /// * every alignment whose boundary does not divide the stretch (its
    ///   padding may change, which changes the stretch);
    /// * every jump within `SHORT_REACH` behind a size the sweep changed
    ///   (a backward span that holds the change);
    /// * every postponed jump the stretch lets through.
    ///
    /// The whole sweep decides the same, given what holds at the start of
    /// every pass but the first (which is whole): every alignment is padded
    /// for where it stands, and every short jump that is neither in a
    /// window nor postponed reaches its target.
    ///
    /// * A marker that is skipped sees a stretch that is a multiple of its
    ///   boundary, so it asks for the padding it has, and changes nothing.
    /// * A jump in reach that is skipped has no size the sweep changed in
    ///   its backward span, and with a forward target it is judged, like
    ///   any jump, before the sweep reaches what lies in between. So the
    ///   sweep would see it where it stood when the pass began, shifted as
    ///   a whole -- in reach -- or, across an alignment and under a
    ///   positive stretch, with its target held back, which the rule below
    ///   never takes for a reason to grow.
    /// * A postponed jump is a short forward jump across an alignment that
    ///   misses its target by `e > 0` bytes. Under a stretch `s` the rule
    ///   below lengthens it when `s < 0` (it is judged where it stood), or
    ///   when `s >= 0` and it still misses a target held back by `s`, that
    ///   is `e > s`: in all, when `e > max(s, 0)`, which is what the tree
    ///   is asked for. `e` is the number the jump was filed under when a
    ///   sweep last looked at it, and it is still true: `e` changes only
    ///   when a size in the jump's span does, and then the jump is in the
    ///   windows (`windows_after` asks `postponed_end` for it).
    ///
    /// And the windows keep that true for the next pass. A jump that
    /// reached before the pass and has no resized item in its span still
    /// reaches; one that has lies within `SHORT_REACH` of that item, which
    /// `windows_after` covers; one that did not reach and was kept short
    /// all the same is filed as postponed, or -- a backward jump that
    /// padding shrinking in front of it brought back for the sweep but
    /// perhaps not for the layout, or one whose addend puts its target in
    /// front of it -- put on `recheck`; and one with an addend (whose
    /// label can be anywhere) is in every pass's windows.
    fn plan(&mut self, windows: &[(u32, u32)], recheck: &mut Vec<u32>) -> Vec<u32> {
        let n = self.items.len();
        let mut grow = Vec::new();
        let mut stretch = 0i64;
        // Jumps that start at or in front of this are within reach of a
        // size the sweep changed.
        let mut reach_end = i64::MIN;
        self.visited.clear();
        let (mut i, mut w) = (0usize, 0usize);
        #[cfg(test)]
        let mut steps = 0usize;
        while i < n {
            while w < windows.len() && windows[w].1 as usize <= i {
                w += 1;
            }
            let in_window = windows.get(w).is_some_and(|&(lo, _)| lo as usize <= i);
            if !in_window && self.base(i) > reach_end {
                // The next item that can matter: the next window, the next
                // alignment the stretch moves, the next postponed jump it
                // lets through.
                let mut next = windows.get(w).map_or(n, |&(lo, _)| lo as usize);
                if let Some(marker) = self.next_moved(i, stretch) {
                    next = next.min(marker);
                }
                if let Some(jump) = self.postponed.first_above(i, stretch.max(0)) {
                    next = next.min(jump);
                }
                #[cfg(test)]
                {
                    steps += 1;
                }
                if next > i {
                    i = next;
                    continue;
                }
            }
            let item = self.items[i];
            #[cfg(test)]
            {
                steps += 1;
            }
            match item.kind {
                Kind::Jump(j) => {
                    let jump = &self.jumps[j as usize];
                    if jump.short {
                        let here = self.base(i) + stretch;
                        let short = jump.short_len as i64;
                        let old =
                            jump.target_off as i64 + self.d[jump.target_k as usize] + jump.addend;
                        // Its displacement where it stood when the pass
                        // began.
                        let at_start = old - (self.base(i) + short);
                        let forward = jump.target_k as usize > i;
                        let crosses = forward && self.next_marker[i] < jump.target_k;
                        let target = if forward {
                            // Not reached yet: GAS `relax_frag` assumes it
                            // moves by `stretch` too -- unless the stretch
                            // is positive and an alignment (a new relax
                            // region) lies in between, which may absorb
                            // it; then it keeps its old position, and a
                            // jump whose target would fall behind its
                            // displacement byte does not grow.
                            if stretch < 0 || !crosses {
                                Some(old + stretch)
                            } else if old < here + short - 1 {
                                None
                            } else {
                                Some(old)
                            }
                        } else {
                            Some(old + total_before(&self.visited, jump.target_k))
                        };
                        let long = jump.long_len as i64;
                        match target.map(|t| t - (here + short)) {
                            Some(disp) if !(-128..=127).contains(&disp) => {
                                stretch += long - short;
                                self.visited.push((i as u32, stretch));
                                reach_end = reach_end.max(self.base(i) + SHORT_REACH);
                                grow.push(i as u32);
                                self.postponed.set(i, MaxTree::NONE);
                                self.postponed_end.set(i, MaxTree::NONE);
                            }
                            // Kept short although out of reach where it
                            // stood when the pass began: postponed.
                            _ if crosses && at_start > 127 => {
                                self.postponed.set(i, at_start - 127);
                                self.postponed_end.set(i, i64::from(jump.target_k));
                            }
                            // The same, for a reason the tree does not
                            // model.
                            _ if !(-128..=127).contains(&at_start) => {
                                self.postponed.set(i, MaxTree::NONE);
                                self.postponed_end.set(i, MaxTree::NONE);
                                recheck.push(i as u32);
                            }
                            // In reach: it stays so until a size in its
                            // span changes.
                            _ => {
                                self.postponed.set(i, MaxTree::NONE);
                                self.postponed_end.set(i, MaxTree::NONE);
                            }
                        }
                    }
                }
                Kind::Marker(m) => {
                    if let Fit::Align { align, max_skip } = self.markers[m as usize].fit {
                        if align > 1 {
                            let here = self.base(i) + stretch;
                            let needed = align_padding(here, align, max_skip);
                            let delta = needed as i64 - item.len as i64;
                            if delta != 0 {
                                stretch += delta;
                                self.visited.push((i as u32, stretch));
                                reach_end = reach_end.max(self.base(i) + SHORT_REACH);
                            }
                        }
                    }
                    // A tight-loop marker (an lccc policy with no GAS
                    // counterpart, sized from the very spans being decided)
                    // keeps its padding here and is decided by the padding
                    // sweep.
                }
            }
            i += 1;
        }
        #[cfg(test)]
        SWEEP_ITEMS.set({
            let (sweeps, items) = SWEEP_ITEMS.get();
            (sweeps + 1, items + steps)
        });
        grow
    }

    /// Fit the padding of every marker behind the jumps a pass resized
    /// (`changed`), front to back: each marker's decision depends on its
    /// offset AFTER the markers before it were fitted. Returns the markers
    /// whose padding changed.
    ///
    /// These are `ElfWriterCore::fixup_alignment_markers_behind`'s
    /// decisions. An alignment is passed over while the shift in front of
    /// it -- the jumps of this pass and the padding changed so far -- is a
    /// multiple of its boundary: it asks for what it has. A tight-loop
    /// marker is always decided: its span holds jumps and padding that can
    /// be anywhere behind it.
    fn fit_padding(
        &mut self,
        changed: &[(u32, i64)],
        charge: &mut dyn FnMut(u64) -> Result<(), String>,
        tight: &mut dyn FnMut(usize, TightVerdict),
    ) -> Result<Changes, String> {
        let n = self.items.len();
        let mut repads = Changes::new();
        let mut repad_total = 0i64;
        // Cursors: the first resized jump at or behind `i`, the first
        // tight-loop marker behind `i`.
        let (mut g, mut t) = (0usize, 0usize);
        let first_behind_jump = changed
            .first()
            .map_or(n, |&(c, _)| self.next_marker[c as usize] as usize);
        let first_tight = self.tight.first().map_or(n, |&i| i as usize);
        let mut i = first_behind_jump.min(first_tight);
        while i < n {
            while g < changed.len() && (changed[g].0 as usize) < i {
                g += 1;
            }
            let jumps_before = if g == 0 { 0 } else { changed[g - 1].1 };
            // The sweep stops at markers only.
            let Kind::Marker(m) = self.items[i].kind else {
                i = self.next_marker[i] as usize;
                continue;
            };
            let current = self.base(i) + jumps_before + repad_total;
            let existing = self.items[i].len;
            let decided = match self.markers[m as usize].fit {
                Fit::Align { align, max_skip } => {
                    (align > 1).then(|| (align_padding(current, align, max_skip), ".align"))
                }
                Fit::Tight { header, latch } => {
                    // The decision is re-derived on EVERY sweep, so a
                    // rejection collapses any padding an earlier sweep
                    // gave the marker.
                    let verdict = self.tight_span(header, latch, changed, &repads);
                    let bucket = verdict.ok().map(|(.., align)| align);
                    tight(m as usize, verdict);
                    self.markers[m as usize].tight = Some(bucket);
                    Some((
                        bucket.map_or(0, |align| align_padding(current, align, None)),
                        ".lccc_tight_loop",
                    ))
                }
            };
            if let Some((needed, what)) = decided {
                // Relaxation shrinks code, so a run can grow past the check
                // made when the directive was read.
                if needed > MAX_DIRECTIVE_FILL as u64 {
                    return Err(format!(
                        "{what}: regenerating {needed} bytes of padding exceeds the {MAX_DIRECTIVE_FILL}-byte directive limit"
                    ));
                }
                // Budget: charge only the NET growth.
                if needed > existing {
                    charge(needed - existing)?;
                }
                if needed != existing {
                    self.items[i].len = needed;
                    self.markers[m as usize].repadded = true;
                    repad_total += needed as i64 - existing as i64;
                    repads.push((i as u32, repad_total));
                }
            }
            while t < self.tight.len() && self.tight[t] as usize <= i {
                t += 1;
            }
            let next_tight = self.tight.get(t).map_or(n, |&x| x as usize);
            // The next marker with something to decide: a tight loop, the
            // first one behind the next resized jump (where the shift
            // changes), or an alignment the shift so far moves off its
            // boundary.
            let behind_next_jump = changed
                .get(g)
                .map_or(n, |&(c, _)| self.next_marker[c as usize] as usize);
            let moved = self
                .next_moved(i + 1, jumps_before + repad_total)
                .unwrap_or(n);
            i = next_tight.min(behind_next_jump).min(moved);
        }
        Ok(repads)
    }

    /// The span of a tight loop in the layout the padding sweep has reached:
    /// header label to the end of the first branch jumping back to it,
    /// which is precisely the body GCC's `align_tight_loops` measures with
    /// `ix86_min_insn_size`.
    ///
    /// Positions are the ones `ElfWriterCore::tight_loop_bucket` reads: the
    /// jumps of this pass at their new sizes, and of the padding only what
    /// the sweep has fitted so far -- the markers behind this one still
    /// hold what the sweep before gave them.
    fn tight_span(
        &self,
        header: Result<(u64, u32), &'static str>,
        latch: Option<(u64, u32, LatchLen)>,
        changed: &[(u32, i64)],
        repads: &[(u32, i64)],
    ) -> TightVerdict {
        let now = |off: u64, k: u32| {
            off as i64 + self.d[k as usize] + total_before(changed, k) + total_before(repads, k)
        };
        let (h_off, h_k) = header?;
        let (l_off, l_k, l_len) = latch.ok_or("no-backedge")?;
        let len = match l_len {
            LatchLen::Fixed(len) => len,
            LatchLen::Jump(j) => self.items[self.jumps[j].item as usize].len,
        };
        let size = (now(l_off, l_k) + len as i64 - now(h_off, h_k)).max(0) as u64;
        let log2 = tight_bucket_log2(size).ok_or(if size == 0 { "empty" } else { "span>64" })?;
        Ok((size, log2, 1u64 << log2))
    }

    /// The items the next growth sweep has to look at after a pass that
    /// resized `changed` (jumps) and `repads` (markers) and left `recheck`
    /// open: everything within `SHORT_REACH` of a resized item, as things
    /// stood when the pass began -- a jump that reached then has its whole
    /// span within that distance of anything inside it -- and every
    /// postponed jump with a resized item in its span.
    fn windows_after(
        &self,
        changed: &[(u32, i64)],
        repads: &[(u32, i64)],
        recheck: &[u32],
    ) -> Vec<(u32, u32)> {
        let n = self.items.len();
        // The first item at or behind `from` that starts at `pos` or
        // further on (`strict`), or beyond `pos`. The resized items come in
        // layout order, so each search starts where the last one ended and
        // doubles its step: a run of resized items next to each other costs
        // a step apiece.
        let first_at = |from: usize, pos: i64, strict: bool| -> usize {
            let in_front = |i: usize| {
                let base = self.base(i);
                base < pos || (!strict && base == pos)
            };
            if from >= n || !in_front(from) {
                return from.min(n);
            }
            // `lo` is in front; the answer is in `(lo, hi]`.
            let (mut lo, mut step) = (from, 1usize);
            while lo + step < n && in_front(lo + step) {
                lo += step;
                step *= 2;
            }
            let mut hi = (lo + step).min(n);
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if in_front(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            hi
        };
        let mut windows: Vec<(u32, u32)> = Vec::with_capacity(recheck.len() + self.far.len() + 8);
        let (mut jumps, mut pads) = (changed.iter().peekable(), repads.iter().peekable());
        // Where the last window began and ended, and the item behind the
        // last resized one.
        let (mut low, mut high, mut from) = (0usize, 0usize, 0usize);
        loop {
            let resized = match (jumps.peek(), pads.peek()) {
                (Some(&&(jump, _)), Some(&&(pad, _))) if jump <= pad => {
                    jumps.next();
                    jump
                }
                (_, Some(&&(pad, _))) => {
                    pads.next();
                    pad
                }
                (Some(&&(jump, _)), None) => {
                    jumps.next();
                    jump
                }
                (None, None) => break,
            } as usize;
            let at = self.base(resized);
            low = first_at(low, at - SHORT_REACH, true);
            high = first_at(high.max(low), at + SHORT_REACH, false);
            match windows.last_mut() {
                Some(last) if low as u32 <= last.1 => last.1 = last.1.max(high as u32),
                _ => windows.push((low as u32, high as u32)),
            }
            // A postponed jump has a resized item in its span exactly when
            // the first one behind it stands in front of its target. So:
            // between this resized item and the one before it, the
            // postponed jumps that end behind it.
            while let Some(jump) = self.postponed_end.first_above(from, resized as i64) {
                if jump >= resized {
                    break;
                }
                windows.push((jump as u32, jump as u32 + 1));
                from = jump + 1;
            }
            from = from.max(resized + 1);
        }
        windows.extend(recheck.iter().chain(&self.far).map(|&i| (i, i + 1)));
        windows.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(windows.len());
        for (lo, hi) in windows {
            match merged.last_mut() {
                Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
                _ => merged.push((lo, hi)),
            }
        }
        merged
    }

    /// Take the size changes of a pass into the prefix sums.
    fn fold(&mut self, changed: &[(u32, i64)], repads: &[(u32, i64)]) {
        let (mut a, mut b) = (0usize, 0usize);
        let (mut total_a, mut total_b) = (0i64, 0i64);
        // `d[k]` counts the items in front of `k`: a change at `c` moves
        // `d[c + 1..]`.
        let mut from = match (changed.first(), repads.first()) {
            (Some(&(x, _)), Some(&(y, _))) => x.min(y) as usize + 1,
            (Some(&(x, _)), None) | (None, Some(&(x, _))) => x as usize + 1,
            (None, None) => return,
        };
        loop {
            let next = match (changed.get(a), repads.get(b)) {
                (Some(&(x, _)), Some(&(y, _))) => x.min(y),
                (Some(&(x, _)), None) | (None, Some(&(x, _))) => x,
                (None, None) => break,
            } as usize;
            let total = total_a + total_b;
            for d in &mut self.d[from..=next] {
                *d += total;
            }
            from = next + 1;
            if changed.get(a).is_some_and(|&(x, _)| x as usize == next) {
                total_a = changed[a].1;
                a += 1;
            } else {
                total_b = repads[b].1;
                b += 1;
            }
        }
        let total = total_a + total_b;
        for d in &mut self.d[from..] {
            *d += total;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// A section of `functions` functions as the layout sees it: fixed
    /// bytes, branches (all in their near form, as the encoder leaves
    /// them), alignment in front of every function and inside some, and a
    /// tight loop here and there.
    fn section(rng: &mut Rng, functions: usize) -> (Vec<JumpSpec>, Vec<MarkerSpecAt>) {
        let (mut jumps, mut markers) = (Vec::new(), Vec::new());
        let mut off = 0u64;
        let mut seq = 0u64;
        // Labels: (offset, seq). Jumps name them by index and are resolved
        // once every label is placed.
        let mut labels: Vec<(u64, u64)> = Vec::new();
        let mut pending: Vec<(usize, usize, i64)> = Vec::new();
        let mut tights: Vec<(usize, usize)> = Vec::new();
        for _ in 0..functions {
            seq += 1;
            let (align, max_skip) = match rng.below(4) {
                0 => (16, Some(10)),
                1 => (8, None),
                2 => (32, None),
                _ => (16, None),
            };
            let padding = align_padding(off as i64, align, max_skip);
            markers.push(MarkerSpecAt {
                off,
                padding,
                seq,
                spec: MarkerSpec::Align { align, max_skip },
            });
            off += padding;
            let first_label = labels.len();
            for _ in 0..2 + rng.below(8) {
                seq += 1;
                match rng.below(8) {
                    0..=2 => off += rng.below(70),
                    3 => {
                        labels.push((off, seq));
                    }
                    4 | 5 => {
                        // A branch to a label of this function (placed or
                        // yet to come) or of one nearby.
                        let conditional = rng.below(2) == 0;
                        let long_len = if conditional { 6 } else { 5 };
                        // ... or, now and then, of one far ahead: a span
                        // of many alignments that stays out of reach while
                        // everything inside it settles.
                        let label = if rng.below(16) == 0 {
                            first_label + rng.below(300) as usize
                        } else {
                            first_label.saturating_sub(rng.below(3) as usize)
                                + rng.below(6) as usize
                        };
                        let addend = if rng.below(12) == 0 {
                            rng.below(4) as i64
                        } else {
                            0
                        };
                        pending.push((jumps.len(), label, addend));
                        jumps.push(JumpSpec {
                            off,
                            short_len: 2,
                            long_len,
                            short: false,
                            target: Point { off: 0, seq: None },
                            addend,
                        });
                        off += long_len;
                    }
                    6 => {
                        let align = 1u64 << (1 + rng.below(5));
                        let max_skip = (rng.below(3) == 0).then(|| rng.below(12));
                        let padding = align_padding(off as i64, align, max_skip);
                        markers.push(MarkerSpecAt {
                            off,
                            padding,
                            seq,
                            spec: MarkerSpec::Align { align, max_skip },
                        });
                        off += padding;
                    }
                    _ => {
                        // A tight loop: marker, header, body, latch.
                        let marker = markers.len();
                        markers.push(MarkerSpecAt {
                            off,
                            padding: 0,
                            seq,
                            spec: MarkerSpec::Tight {
                                header: Err("unresolved-header"),
                                latch: None,
                            },
                        });
                        seq += 1;
                        let header = labels.len();
                        labels.push((off, seq));
                        off += 4 + rng.below(70);
                        tights.push((marker, jumps.len()));
                        pending.push((jumps.len(), header, 0));
                        jumps.push(JumpSpec {
                            off,
                            short_len: 2,
                            long_len: 6,
                            short: false,
                            target: Point { off: 0, seq: None },
                            addend: 0,
                        });
                        off += 6;
                    }
                }
            }
            seq += 1;
            labels.push((off, seq));
            off += 1 + rng.below(40);
        }
        for _ in 0..8 {
            seq += 1;
            labels.push((off, seq));
        }
        for (j, label, _) in pending {
            let (l_off, l_seq) = labels[label.min(labels.len() - 1)];
            jumps[j].target = Point {
                off: l_off,
                seq: Some(l_seq),
            };
        }
        for (marker, latch) in tights {
            let header = jumps[latch].target;
            markers[marker].spec = MarkerSpec::Tight {
                header: Ok(header),
                latch: Some(Latch {
                    at: jumps[latch].off,
                    len: LatchLen::Jump(latch),
                }),
            };
        }
        (jumps, markers)
    }

    type Outcome = (Vec<JumpOutcome>, Vec<MarkerOutcome>, u32);

    fn settle(jumps: &[JumpSpec], markers: &[MarkerSpecAt], whole: bool) -> Outcome {
        let mut layout = Layout::build(jumps, markers).expect("a well-formed section");
        if whole {
            layout = layout.with_whole_sweeps();
        }
        layout
            .settle(true, &mut |_| Ok(()), &mut |_, _| {})
            .expect("settles");
        (
            (0..jumps.len()).map(|j| layout.jump_outcome(j)).collect(),
            (0..markers.len())
                .map(|m| layout.marker_outcome(m))
                .collect(),
            layout.passes(),
        )
    }

    /// Where every item ends up, computed from the outcome alone: the
    /// independent check that every short jump reaches and every alignment
    /// is padded for where it stands.
    fn check_fixed_point(jumps: &[JumpSpec], markers: &[MarkerSpecAt], outcome: &Outcome) {
        let layout = Layout::build(jumps, markers).unwrap();
        let mut d = vec![0i64; layout.items.len() + 1];
        for (i, item) in layout.items.iter().enumerate() {
            let len = match item.kind {
                Kind::Jump(j) => {
                    let o = outcome.0[j as usize];
                    if o.shrunk && o.grown.is_none() {
                        jumps[j as usize].short_len
                    } else {
                        jumps[j as usize].long_len
                    }
                }
                Kind::Marker(m) => outcome.1[m as usize].padding,
            };
            d[i + 1] = d[i] + len as i64 - item.phys as i64;
        }
        for (i, item) in layout.items.iter().enumerate() {
            let at = item.off as i64 + d[i];
            match item.kind {
                Kind::Jump(j) => {
                    let (spec, state) = (&jumps[j as usize], &layout.jumps[j as usize]);
                    let o = outcome.0[j as usize];
                    if o.shrunk && o.grown.is_none() {
                        let target =
                            spec.target.off as i64 + d[state.target_k as usize] + spec.addend;
                        let disp = target - (at + spec.short_len as i64);
                        assert!(
                            (-128..=127).contains(&disp),
                            "short jump {j} at {at} does not reach {target}"
                        );
                    }
                }
                Kind::Marker(m) => {
                    if let MarkerSpec::Align { align, max_skip } = markers[m as usize].spec {
                        assert_eq!(
                            outcome.1[m as usize].padding,
                            align_padding(at, align, max_skip),
                            "marker {m} at {at}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn windowed_sweeps_decide_what_whole_sweeps_decide() {
        WINDOW_MISSES.set(0);
        let mut multi_pass = 0usize;
        for seed in 1..=600u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let functions = 1 + rng.below(60) as usize;
            let (jumps, markers) = section(&mut rng, functions);
            let whole = settle(&jumps, &markers, true);
            let windowed = settle(&jumps, &markers, false);
            assert_eq!(windowed, whole, "seed {seed}");
            check_fixed_point(&jumps, &markers, &whole);
            multi_pass += usize::from(whole.2 >= 3);
        }
        assert_eq!(WINDOW_MISSES.get(), 0, "a confirming sweep found growth");
        // The comparison has to be over layouts that need several passes.
        assert!(multi_pass >= 100, "only {multi_pass} multi-pass layouts");
    }

    #[test]
    fn windows_spare_the_settled_part_of_a_large_section() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let (jumps, markers) = section(&mut rng, 4000);
        SWEEP_ITEMS.set((0, 0));
        let whole = settle(&jumps, &markers, true);
        let (whole_sweeps, whole_items) = SWEEP_ITEMS.get();
        SWEEP_ITEMS.set((0, 0));
        WINDOW_MISSES.set(0);
        let windowed = settle(&jumps, &markers, false);
        let (_, window_items) = SWEEP_ITEMS.get();
        assert_eq!(windowed, whole);
        assert_eq!(WINDOW_MISSES.get(), 0);
        assert!(whole_sweeps >= 4, "{whole_sweeps} sweeps: nothing to spare");
        // Alignments of every size with random maximum skips let a shift
        // travel far, and the sweep has to follow it; even so most of the
        // section is passed over.
        assert!(
            window_items * 4 < whole_items,
            "windowed sweeps looked at {window_items} items, whole sweeps at {whole_items}"
        );
    }

    /// A section shaped like compiled code -- `.p2align 4` in front of
    /// every function, `.p2align 4,,10` + `.p2align 3` in front of a loop
    /// -- whose first branches sit at the edge of rel8 reach, so that
    /// growth cascades backwards pass after pass.
    fn compiled(rng: &mut Rng, functions: usize) -> (Vec<JumpSpec>, Vec<MarkerSpecAt>) {
        let (mut jumps, mut markers) = (Vec::new(), Vec::new());
        let (mut off, mut seq) = (0u64, 0u64);
        let mut align = |off: &mut u64, seq: &mut u64, align: u64, max_skip: Option<u64>| {
            *seq += 1;
            let padding = align_padding(*off as i64, align, max_skip);
            markers.push(MarkerSpecAt {
                off: *off,
                padding,
                seq: *seq,
                spec: MarkerSpec::Align { align, max_skip },
            });
            *off += padding;
        };
        // Branches out of a function to the end of one further on (a
        // shared exit, a cold path): (jump, functions still to pass).
        let mut leaving: Vec<(usize, u64)> = Vec::new();
        for _ in 0..functions {
            align(&mut off, &mut seq, 16, None);
            // A chain of forward branches, each to just behind the next:
            // the last one growing pushes the one before out of reach.
            let links = 1 + rng.below(4);
            let mut open: Vec<usize> = Vec::new();
            if rng.below(5) == 0 {
                leaving.push((jumps.len(), rng.below(12)));
                jumps.push(JumpSpec {
                    off,
                    short_len: 2,
                    long_len: 6,
                    short: false,
                    target: Point { off: 0, seq: None },
                    addend: 0,
                });
                off += 6 + rng.below(20);
            }
            for _ in 0..links {
                open.push(jumps.len());
                jumps.push(JumpSpec {
                    off,
                    short_len: 2,
                    long_len: 6,
                    short: false,
                    target: Point { off: 0, seq: None },
                    addend: 0,
                });
                off += 6 + 100 + rng.below(40);
                if rng.below(3) == 0 {
                    align(&mut off, &mut seq, 16, Some(10));
                    align(&mut off, &mut seq, 8, None);
                }
            }
            seq += 1;
            let here = Point {
                off,
                seq: Some(seq),
            };
            for j in open {
                jumps[j].target = here;
            }
            leaving.retain_mut(|(j, functions)| {
                if *functions == 0 {
                    jumps[*j].target = here;
                    return false;
                }
                *functions -= 1;
                true
            });
            off += 1 + rng.below(30);
        }
        seq += 1;
        for (j, _) in leaving {
            jumps[j].target = Point {
                off,
                seq: Some(seq),
            };
        }
        (jumps, markers)
    }

    /// Steps per item the windowed sweeps may take on `compiled`: 7.6
    /// measured, two of them the whole sweeps that start and confirm.
    const COMPILED_STEPS_PER_ITEM: usize = 10;

    #[test]
    fn a_compiled_section_costs_a_few_sweeps_worth_of_work() {
        let mut rng = Rng(0x0dd_ba11_5eed_0001);
        let (jumps, markers) = compiled(&mut rng, 6000);
        let items = jumps.len() + markers.len();
        SWEEP_ITEMS.set((0, 0));
        let whole = settle(&jumps, &markers, true);
        let (whole_sweeps, whole_items) = SWEEP_ITEMS.get();
        SWEEP_ITEMS.set((0, 0));
        WINDOW_MISSES.set(0);
        let windowed = settle(&jumps, &markers, false);
        let (_, window_items) = SWEEP_ITEMS.get();
        assert_eq!(windowed, whole);
        assert_eq!(WINDOW_MISSES.get(), 0);
        check_fixed_point(&jumps, &markers, &whole);
        assert!(
            whole_sweeps >= 50,
            "{whole_sweeps} sweeps: nothing to spare"
        );
        // The whole sweep that starts and the one that confirms, and the
        // windows in between: a small multiple of the section, however
        // many passes it takes (well over a hundred here).
        assert!(
            window_items < COMPILED_STEPS_PER_ITEM * items && window_items * 16 < whole_items,
            "windowed sweeps took {window_items} steps for {items} items ({whole_sweeps} whole sweeps: {whole_items})"
        );
    }

    /// The tree against a scan, under random updates.
    #[test]
    fn the_maximum_tree_finds_what_a_scan_finds() {
        let mut rng = Rng(0x5eed_0f7e_e5a1);
        for len in [0usize, 1, 2, 3, 5, 8, 13, 64, 100] {
            let mut values: Vec<i64> = (0..len)
                .map(|_| match rng.below(3) {
                    0 => MaxTree::NONE,
                    _ => rng.below(40) as i64 - 8,
                })
                .collect();
            let mut tree = MaxTree::new(&values);
            for round in 0..200 {
                if len > 0 && round % 2 == 1 {
                    let i = rng.below(len as u64) as usize;
                    values[i] = match rng.below(4) {
                        0 => MaxTree::NONE,
                        1 => i64::MAX,
                        _ => rng.below(40) as i64 - 8,
                    };
                    tree.set(i, values[i]);
                }
                let from = rng.below(len as u64 + 3) as usize;
                let bound = match rng.below(8) {
                    0 => i64::MIN,
                    1 => i64::MAX,
                    _ => rng.below(44) as i64 - 10,
                };
                let scan = (from..len).find(|&i| values[i] > bound);
                assert_eq!(
                    tree.first_above(from, bound),
                    scan,
                    "len {len} from {from} bound {bound}: {values:?}"
                );
            }
        }
    }

    #[test]
    fn an_alignment_that_is_not_fitted_is_left_to_the_caller() {
        let markers = [MarkerSpecAt {
            off: 3,
            padding: 0,
            seq: 1,
            spec: MarkerSpec::Align {
                align: 16,
                max_skip: None,
            },
        }];
        assert!(Layout::build(&[], &markers).is_none());
    }

    #[test]
    fn a_label_stands_on_the_side_of_padding_it_was_written_on() {
        // `before: .p2align 4; after:` and a jump back to each. Only the
        // label behind the directive moves with its padding, whether the
        // run is empty (both labels at the marker's offset: source order
        // tells them apart) or not (the label behind stands at its end).
        let jump = |off, target| JumpSpec {
            off,
            short_len: 2,
            long_len: 5,
            short: false,
            target,
            addend: 0,
        };
        let marker = |off, padding| {
            [MarkerSpecAt {
                off,
                padding,
                seq: 2,
                spec: MarkerSpec::Align {
                    align: 16,
                    max_skip: None,
                },
            }]
        };
        let at = |off, seq| Point {
            off,
            seq: Some(seq),
        };
        for (start, padding) in [(16, 0), (5, 11)] {
            let end = start + padding;
            let jumps = [jump(end + 3, at(start, 1)), jump(end + 8, at(end, 3))];
            let layout = Layout::build(&jumps, &marker(start, padding)).unwrap();
            // Items: the marker, then the two jumps.
            assert_eq!(layout.jumps[0].target_k, 0, "padding {padding}: before");
            assert_eq!(layout.jumps[1].target_k, 1, "padding {padding}: after");
        }
        // An instruction at the end of the run always stands behind it.
        let layout = Layout::build(&[jump(16, at(16, 1))], &marker(16, 0)).unwrap();
        assert_eq!(layout.jumps[0].item, 1);
    }
}

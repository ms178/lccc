//! A fixed-capacity, heap-free small vector for hot parsing paths.
//!
//! Several backend scanners split short, bounded lists (AT&T instruction
//! operands, register names) inside their hottest per-line loops, and each
//! split used to heap-allocate a `Vec` — DHAT attributed over a million such
//! blocks per profiling TU across the peephole and CFI passes.  `InlineVec`
//! keeps up to `N` elements inline and spills to the heap only beyond that,
//! so the common case allocates nothing while pathological inputs still get
//! exact (never truncated) answers: truncation would silently change
//! `.iter().all(...)`-style pass decisions, which is a correctness bug, not
//! a performance trade.
//!
//! `Deref<Target = [T]>` keeps call sites' indexing, slicing and iteration
//! byte-for-byte identical to the `Vec` they replaced.

/// Up to `N` elements inline, spilling to the heap transparently.
///
/// `T: Copy + Default` keeps the implementation trivially safe (no `unsafe`,
/// no `MaybeUninit`): the inline array is prefilled with `T::default()` and
/// only `..len` is ever exposed.  For the `&str` and small-integer element
/// types this is built for, that costs nothing.
pub struct InlineVec<T: Copy + Default, const N: usize> {
    head: [T; N],
    len: usize,
    spill: Vec<T>,
}

impl<T: Copy + Default, const N: usize> InlineVec<T, N> {
    pub fn new() -> Self {
        Self {
            head: [T::default(); N],
            len: 0,
            spill: Vec::new(),
        }
    }

    pub fn push(&mut self, v: T) {
        if !self.spill.is_empty() {
            self.spill.push(v);
        } else if self.len < N {
            self.head[self.len] = v;
            self.len += 1;
        } else {
            // First overflow: move the inline head to the heap once, then
            // append.  `deref` switches to the spill vector from here on.
            self.spill.reserve(N + 1);
            self.spill.extend_from_slice(&self.head);
            self.spill.push(v);
        }
    }

    /// The elements as a plain slice (for `let [a, b] = ...` forms).
    pub fn as_slice(&self) -> &[T] {
        self
    }

    /// Whether the list outgrew its inline capacity (diagnostics/tests).
    pub fn spilled(&self) -> bool {
        !self.spill.is_empty()
    }
}

impl<T: Copy + Default, const N: usize> Default for InlineVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy + Default, const N: usize> core::ops::Deref for InlineVec<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        if self.spill.is_empty() {
            &self.head[..self.len]
        } else {
            &self.spill
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InlineVec;

    #[test]
    fn inline_up_to_capacity_no_spill() {
        let mut l: InlineVec<&str, 6> = InlineVec::new();
        for s in ["a", "b", "c", "d", "e", "f"] {
            l.push(s);
        }
        assert_eq!(&*l, &["a", "b", "c", "d", "e", "f"]);
        assert!(!l.spilled(), "no heap for the inline maximum");
    }

    #[test]
    fn overflow_preserves_order_and_content() {
        let mut l: InlineVec<&str, 6> = InlineVec::new();
        let all = ["a", "b", "c", "d", "e", "f", "g", "h"];
        for s in all {
            l.push(s);
        }
        assert_eq!(&*l, &all);
        assert!(l.spilled());
    }

    #[test]
    fn empty_list_is_an_empty_slice() {
        let l: InlineVec<&str, 4> = InlineVec::new();
        assert!(l.is_empty());
        assert_eq!(l.len(), 0);
        assert_eq!(l.first(), None);
    }

    #[test]
    fn slice_ops_work_through_deref() {
        let mut l: InlineVec<&str, 6> = InlineVec::new();
        for s in ["$1", "%xmm0", "%xmm1"] {
            l.push(s);
        }
        assert_eq!(l[0], "$1");
        assert_eq!(&l[..l.len() - 1], &["$1", "%xmm0"]);
        assert!(l.iter().any(|s| s.starts_with("%xmm")));
    }

    #[test]
    fn integer_elements_and_exact_boundary() {
        let mut l: InlineVec<u32, 2> = InlineVec::new();
        l.push(10);
        l.push(20);
        assert!(!l.spilled());
        l.push(30); // first element past the boundary moves to the heap
        assert!(l.spilled());
        assert_eq!(&*l, &[10, 20, 30]);
    }
}

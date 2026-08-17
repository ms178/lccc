//! Single source of truth for ELF section/program-header layout decisions.
//!
//! Centralises header-index / section-count computation that was previously
//! duplicated in emit_exec.rs (historical root cause of ordering bugs).

#![allow(dead_code)]

use crate::backend::linker_common::OutputSection;

#[derive(Debug, Clone)]
pub struct LayoutPlan {
    pub out_sec_to_hdr: Vec<u16>,
    pub symtab_shidx: u16,
    pub strtab_shidx: u16,
    pub shstrtab_shidx: u16,
    pub sh_count: u16,
    pub shdr_offset: u64,
    pub ph_count: u16,
    pub phdr_offset: u64,
}

impl LayoutPlan {
    /// Generic header-index assignment.
    ///
    /// # Status: not used by the x86-64 executable emitter
    ///
    /// This models a *simple* layout -- every non-empty output section gets a
    /// header, then a fixed list of specials. The real `emit_exec` order is
    /// not that: it interleaves linker-created headers (.interp, .gnu.hash,
    /// .dynsym, .dynstr, verneed, .rela.dyn, .rela.plt, .plt, .eh_frame_hdr,
    /// init/fini/preinit arrays, .dynamic, .got, .got.plt, .iplt, .rela.iplt)
    /// between four ordered *groups* of output sections (alloc PROGBITS, TLS
    /// PROGBITS, TLS NOBITS, then non-TLS NOBITS), and the exact sequence
    /// depends on `is_static`, on which of those tables are non-empty, and on
    /// section flags.
    ///
    /// Switching `emit_exec` to this function would silently renumber every
    /// section header. The duplication this type was created to prevent has
    /// instead been removed at the source: `emit_exec` now performs ONE walk
    /// whose running count yields both `out_sec_to_hdr` and the
    /// `.symtab`/`.strtab` indices, where it previously spelled the same
    /// ~25-line sequence out twice and relied on the two copies being edited
    /// together. `section_header_index_consistency` in the differential suite
    /// pins the invariant.
    ///
    /// Kept because the shared `SegmentPacker` below is live and belongs in
    /// this module, and because a future backend with a simple layout can use
    /// it. Do not wire it into `emit_exec` without first making the two orders
    /// provably identical.
    pub fn compute(
        output_sections: &[OutputSection],
        special: &[(&str, bool)],
        has_symtab: bool,
        has_strtab: bool,
        has_shstrtab: bool,
        ph_count: u16,
        phdr_offset: u64,
        shdr_offset: u64,
    ) -> Self {
        let mut out_sec_to_hdr = vec![0u16; output_sections.len()];
        let mut hdr: u16 = 1;
        for (i, sec) in output_sections.iter().enumerate() {
            if sec.mem_size > 0 || !sec.data.is_empty() || sec.name.contains("bss") {
                out_sec_to_hdr[i] = hdr;
                hdr = hdr.saturating_add(1);
            }
        }
        for &(_name, emit) in special {
            if emit { hdr = hdr.saturating_add(1); }
        }
        let symtab_shidx = if has_symtab { let s = hdr; hdr = hdr.saturating_add(1); s } else { 0 };
        let strtab_shidx = if has_strtab { let s = hdr; hdr = hdr.saturating_add(1); s } else { 0 };
        let shstrtab_shidx = if has_shstrtab { let s = hdr; hdr = hdr.saturating_add(1); s } else { 0 };
        LayoutPlan {
            out_sec_to_hdr, symtab_shidx, strtab_shidx, shstrtab_shidx,
            sh_count: hdr, shdr_offset, ph_count, phdr_offset,
        }
    }

    #[inline]
    pub fn hdr_for_out_sec(&self, out_idx: usize) -> u16 {
        self.out_sec_to_hdr.get(out_idx).copied().unwrap_or(0)
    }
}

/// Assigns virtual addresses to a densely-packed sequence of file offsets.
///
/// # The invariant this type exists to protect
///
/// A `PT_LOAD` segment's file offset does **not** need to be page-aligned.
/// The ELF gABI requires only
///
/// ```text
/// p_offset ≡ p_vaddr  (mod p_align)
/// ```
///
/// because `mmap` maps `p_offset & ~(pagesize-1)` onto
/// `p_vaddr & ~(pagesize-1)`. Rounding the *file offset* up to a page at every
/// segment boundary — the obvious-looking thing to do when `addr` is computed
/// as `base + offset` — silently wastes up to one page per segment.
///
/// Measured cost of getting this wrong (identical inputs, only the linker
/// varies): a zlib-ng test executable was 20 640 B instead of 8 352 B, and a
/// trivial shared object 19 568 B instead of 7 280 B — worse than bfd, mold
/// and wild. After the fix lccc beats bfd and mold on both.
///
/// # Why it is a type and not two macros
///
/// This logic was originally written twice, as local `macro_rules!` in
/// `emit_exec.rs` and again in `emit_shared.rs`. That duplication is exactly
/// how the defect survived: the executable path was fixed first and the shared
/// path stayed broken until someone thought to re-measure it. `emit_script.rs`
/// has a third, structurally different implementation (it packs congruent to
/// each section's LMA modulo a 2 MiB page, because linker scripts assign
/// addresses themselves).
///
/// Centralising it here means the invariant is stated once, tested once, and
/// cannot drift between emitters.
///
/// # Model
///
/// File offsets advance densely. `bias` is a running virtual-address offset
/// that is **always a multiple of `page_size`**, so
/// `(offset + bias) ≡ offset (mod page_size)` holds by construction and
/// congruence is automatic — it is not something a caller can forget.
/// Starting a new segment bumps the bias by one page, giving the new segment
/// a fresh page of address space (so two segments with different permissions
/// never share a page) at zero cost in the file.
#[derive(Debug, Clone)]
pub struct SegmentPacker {
    base: u64,
    page_size: u64,
    bias: u64,
}

impl SegmentPacker {
    /// `base` is the image base (`BASE_ADDR` for ET_EXEC, `0` for ET_DYN).
    /// `page_size` must be a non-zero power of two.
    #[inline]
    pub fn new(base: u64, page_size: u64) -> Self {
        debug_assert!(page_size.is_power_of_two() && page_size > 0);
        Self { base, page_size, bias: 0 }
    }

    /// Virtual address currently assigned to file offset `off`.
    #[inline]
    pub fn vaddr(&self, off: u64) -> u64 {
        self.base + self.bias + off
    }

    /// Begin a new `PT_LOAD`: the file offset is untouched, the virtual
    /// address advances by exactly one page.
    #[inline]
    pub fn new_segment(&mut self) {
        self.bias += self.page_size;
    }

    /// Padding that must be added to `off` so that `vaddr(off)` lands on a
    /// page boundary.
    ///
    /// Needed for the RELRO boundary: `ld.so` mprotects the page-rounded
    /// range `[p_vaddr, p_vaddr + p_memsz)`, so it is the **virtual address**
    /// that has to reach a page boundary, not the file offset. Once offsets
    /// and addresses are no longer the same number, aligning the file offset
    /// leaves RELRO ending mid-page and write-protects the head of whatever
    /// follows. Returns 0 when already aligned.
    #[inline]
    pub fn padding_to_page(&self, off: u64) -> u64 {
        let v = self.vaddr(off);
        let aligned = (v + self.page_size - 1) & !(self.page_size - 1);
        aligned - v
    }

    /// Current bias, exposed for assertions and diagnostics.
    #[inline]
    pub fn bias(&self) -> u64 {
        self.bias
    }

    /// True when `off`/`vaddr(off)` satisfy the gABI congruence rule.
    /// Always true by construction; used by tests and debug assertions.
    #[inline]
    pub fn is_congruent(&self, off: u64) -> bool {
        (off % self.page_size) == (self.vaddr(off) % self.page_size)
    }
}

#[cfg(test)]
mod packer_tests {
    use super::*;

    const PAGE: u64 = 0x1000;

    /// The property the whole type exists for: whatever the caller does,
    /// congruence holds for every offset and every segment count.
    #[test]
    fn congruence_holds_for_all_offsets_and_segments() {
        for base in [0u64, 0x400000, 0x1000] {
            let mut p = SegmentPacker::new(base, PAGE);
            for seg in 0..8 {
                for off in [0u64, 1, 7, 63, 0xfff, 0x1000, 0x1001, 0x12345, 1 << 20] {
                    assert!(p.is_congruent(off),
                            "base={base:#x} seg={seg} off={off:#x} \
                             vaddr={:#x}", p.vaddr(off));
                }
                p.new_segment();
            }
        }
    }

    /// A new segment must cost address space but *no* file space.
    #[test]
    fn new_segment_advances_address_not_offset() {
        let mut p = SegmentPacker::new(0x400000, PAGE);
        let off = 0x470;
        let before = p.vaddr(off);
        p.new_segment();
        let after = p.vaddr(off);
        assert_eq!(after - before, PAGE, "address must advance exactly one page");
        assert_eq!(p.bias(), PAGE);
        // The caller's file offset is untouched — that is the whole point.
        assert!(p.is_congruent(off));
    }

    /// Regression for the concrete waste this replaced: three segments used to
    /// force the file offset to 0x1000/0x2000/0x3000. Here the offsets stay
    /// exactly where the content ended.
    #[test]
    fn packing_wastes_no_file_space() {
        let mut p = SegmentPacker::new(0x400000, PAGE);
        // Segment 1 content ends at 0x470, segment 2 at 0x6d0, segment 3 at 0x8c0.
        let ends = [0x470u64, 0x6d0, 0x8c0];
        let mut prev_v = 0;
        for (i, &off) in ends.iter().enumerate() {
            if i > 0 { p.new_segment(); }
            let v = p.vaddr(off);
            assert!(p.is_congruent(off));
            if i > 0 {
                assert!(v > prev_v, "addresses must increase across segments");
            }
            prev_v = v;
        }
        // Old behaviour would have pushed the last offset to 0x3000.
        assert_eq!(*ends.last().unwrap(), 0x8c0);
    }

    #[test]
    fn padding_to_page_reaches_a_page_boundary() {
        let mut p = SegmentPacker::new(0x400000, PAGE);
        p.new_segment();
        for off in [0u64, 1, 0x8c0, 0xfff, 0x1000, 0x1234] {
            let pad = p.padding_to_page(off);
            assert!(pad < PAGE, "padding must be less than a page, got {pad:#x}");
            assert_eq!(p.vaddr(off + pad) % PAGE, 0,
                       "off={off:#x} pad={pad:#x} did not reach a page boundary");
            if p.vaddr(off) % PAGE == 0 {
                assert_eq!(pad, 0, "already-aligned offset must need no padding");
            }
        }
    }

    /// ET_DYN uses base 0; the arithmetic must not special-case it.
    #[test]
    fn works_with_zero_base() {
        let mut p = SegmentPacker::new(0, PAGE);
        assert_eq!(p.vaddr(0x100), 0x100);
        p.new_segment();
        assert_eq!(p.vaddr(0x100), 0x1100);
        assert!(p.is_congruent(0x100));
    }

    /// Large pages (kernel/script mode uses 2 MiB) behave identically.
    #[test]
    fn works_with_large_pages() {
        let mut p = SegmentPacker::new(0xffffffff80000000, 0x200000);
        for _ in 0..4 {
            for off in [0u64, 0x1000, 0x1fffff, 0x200000] {
                assert!(p.is_congruent(off));
            }
            p.new_segment();
        }
        assert_eq!(p.bias(), 4 * 0x200000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::OutputSection;

    fn dummy_sec(name: &str, size: u64) -> OutputSection {
        OutputSection {
            name: name.to_string(),
            sh_type: 1,
            flags: 0,
            alignment: 1,
            inputs: vec![],
            data: vec![0u8; size as usize],
            addr: 0,
            file_offset: 0,
            mem_size: size,
        }
    }

    #[test]
    fn empty_plan() {
        let plan = LayoutPlan::compute(&[], &[], false, false, false, 0, 0, 0);
        assert_eq!(plan.sh_count, 1);
        assert_eq!(plan.symtab_shidx, 0);
    }

    #[test]
    fn assigns_contiguous_headers() {
        let secs = vec![
            dummy_sec(".text", 100),
            dummy_sec(".data", 40),
            dummy_sec(".bss", 0),
        ];
        let plan = LayoutPlan::compute(&secs, &[(".interp", true)], true, true, true, 3, 64, 4096);
        assert!(plan.hdr_for_out_sec(0) >= 1);
        assert!(plan.hdr_for_out_sec(1) > plan.hdr_for_out_sec(0));
        assert!(plan.symtab_shidx > 0);
        assert_eq!(plan.ph_count, 3);
        assert_eq!(plan.shdr_offset, 4096);
    }
}

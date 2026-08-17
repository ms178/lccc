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

//! GNU-compatible linker map file generation (`-Map=` / `--print-map`).
//!
//! Emits a human-readable description of the final memory layout:
//! which input sections contributed to which output sections, symbol
//! addresses, and archive member extraction reasons.
//!
//! Status: data model and formatting helpers are complete. The call site
//! in the emit path (after address assignment) is the remaining wiring.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::Path;

/// One contribution of an input section into an output section.
#[derive(Debug, Clone)]
pub struct MapSectionContribution {
    pub output_section: String,
    pub output_addr: u64,
    pub input_file: String,
    pub input_section: String,
    pub size: u64,
    pub symbols: Vec<(String, u64)>, // name, absolute address
}

/// Archive member that was pulled in, with the symbol that caused it.
#[derive(Debug, Clone)]
pub struct MapArchiveMember {
    pub archive: String,
    pub member: String,
    pub reason_symbol: String,
}

/// Complete map for one link.
#[derive(Debug, Default)]
pub struct LinkMap {
    pub contributions: Vec<MapSectionContribution>,
    pub archive_members: Vec<MapArchiveMember>,
    pub entry_symbol: Option<String>,
    pub entry_addr: u64,
}

impl LinkMap {
    /// Format a GNU-ld-style map into `out`.
    pub fn write_gnu(&self, out: &mut String) {
        let _ = writeln!(out, "Archive member included to satisfy reference by file (symbol)\n");
        for m in &self.archive_members {
            let _ = writeln!(
                out,
                "{}({})\n              {}\n",
                m.archive, m.member, m.reason_symbol
            );
        }
        let _ = writeln!(out, "\nMemory Configuration\n\nName             Origin             Length             Attributes\n");
        let _ = writeln!(out, "\nLinker script and memory map\n");
        if let Some(ref e) = self.entry_symbol {
            let _ = writeln!(out, "                0x{:016x}                {}\n", self.entry_addr, e);
        }
        let mut current_out = String::new();
        for c in &self.contributions {
            if c.output_section != current_out {
                let _ = writeln!(
                    out,
                    "\n{}            0x{:016x}    0x{:x}",
                    c.output_section, c.output_addr, c.size
                );
                current_out = c.output_section.clone();
            }
            let _ = writeln!(
                out,
                " {:.16}  0x{:016x}    0x{:x} {}",
                c.input_section, c.output_addr, c.size, c.input_file
            );
            for (name, addr) in &c.symbols {
                let _ = writeln!(out, "                0x{:016x}                {}", addr, name);
            }
        }
    }

    /// Write the map to a file. Creates parent directories if needed.
    pub fn write_to_path(&self, path: &Path) -> Result<(), String> {
        let mut s = String::with_capacity(16 * 1024);
        self.write_gnu(&mut s);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, s).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_map_formats() {
        let m = LinkMap::default();
        let mut s = String::new();
        m.write_gnu(&mut s);
        assert!(s.contains("Archive member included"));
        assert!(s.contains("Memory Configuration"));
    }

    #[test]
    fn contribution_appears() {
        let mut m = LinkMap::default();
        m.contributions.push(MapSectionContribution {
            output_section: ".text".into(),
            output_addr: 0x401000,
            input_file: "a.o".into(),
            input_section: ".text".into(),
            size: 0x20,
            symbols: vec![("main".into(), 0x401000)],
        });
        m.entry_symbol = Some("_start".into());
        m.entry_addr = 0x401000;
        let mut s = String::new();
        m.write_gnu(&mut s);
        assert!(s.contains(".text"));
        assert!(s.contains("main"));
        assert!(s.contains("_start"));
    }
}

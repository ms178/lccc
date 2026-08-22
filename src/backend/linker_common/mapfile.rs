//! GNU-compatible linker map file generation (`-Map=` / `--print-map`).
//!
//! Emits a human-readable description of the final memory layout:
//! which input sections contributed to which output sections, symbol
//! addresses, and archive member extraction reasons.
//!
//! The output follows GNU ld's column layout so that existing tooling which
//! scrapes map files (kernel `scripts/`, embedded size-accounting scripts,
//! bloaty-style analysers) keeps working:
//!
//! ```text
//! .text           0x0000000000401000       0x3a
//!  .text          0x0000000000401000       0x2d bmain.o
//!                 0x0000000000401000                _start
//! ```
//!
//! Output-section lines start in column 0, input-section contributions are
//! indented by one space, and symbol lines are indented to the address column.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::Path;

/// One contribution of an input section into an output section.
#[derive(Debug, Clone)]
pub struct MapSectionContribution {
    pub output_section: String,
    /// Address of the *output* section this contribution belongs to.
    pub out_section_addr: u64,
    /// Total size of the *output* section this contribution belongs to.
    pub out_section_size: u64,
    /// Address at which this particular input section was placed.
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
        if self.archive_members.is_empty() {
            let _ = writeln!(out, "\nThere are no discarded input sections\n");
        } else {
            let _ = writeln!(
                out,
                "Archive member included to satisfy reference by file (symbol)\n"
            );
            for m in &self.archive_members {
                let _ = writeln!(out, "{}({})", m.archive, m.member);
                let _ = writeln!(out, "                              {}", m.reason_symbol);
            }
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "Memory Configuration\n");
        let _ = writeln!(
            out,
            "Name             Origin             Length             Attributes"
        );
        let _ = writeln!(
            out,
            "*default*        0x0000000000000000 0xffffffffffffffff\n"
        );

        let _ = writeln!(out, "Linker script and memory map\n");

        let mut current_out: Option<&str> = None;
        for c in &self.contributions {
            if current_out != Some(c.output_section.as_str()) {
                // Output-section header line: name in column 0, then the
                // section's own address and total size.
                let _ = writeln!(out);
                let _ = writeln!(
                    out,
                    "{:<15} 0x{:016x} {:>8x}",
                    c.output_section, c.out_section_addr, c.out_section_size
                );
                current_out = Some(c.output_section.as_str());
            }
            // Input-section contribution: one leading space, then the address
            // and size this input occupies, then the file it came from.
            let _ = writeln!(
                out,
                " {:<14} 0x{:016x} {:>8x} {}",
                c.input_section, c.output_addr, c.size, c.input_file
            );
            for (name, addr) in &c.symbols {
                let _ = writeln!(
                    out,
                    "                0x{:016x}                {}",
                    addr, name
                );
            }
        }

        if let Some(ref e) = self.entry_symbol {
            let _ = writeln!(
                out,
                "\n                0x{:016x}                {}",
                self.entry_addr, e
            );
        }
    }

    /// Write the map to a file. Creates parent directories if needed.
    /// Write the map to `path`.
    ///
    /// The single character `-` means standard output, which is how GNU ld
    /// implements `--print-map` / `-M`. Routing it through the same call keeps
    /// one code path for both spellings, and means a build system can pipe the
    /// map without needing a temporary file.
    pub fn write_to_path(&self, path: &Path) -> Result<(), String> {
        let mut s = String::with_capacity(16 * 1024);
        self.write_gnu(&mut s);
        if path.as_os_str() == "-" {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            return lock
                .write_all(s.as_bytes())
                .and_then(|_| lock.flush())
                .map_err(|e| e.to_string());
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
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
        assert!(s.contains("Memory Configuration"));
        assert!(s.contains("Linker script and memory map"));
    }

    #[test]
    fn contribution_appears() {
        let mut m = LinkMap::default();
        m.contributions.push(MapSectionContribution {
            output_section: ".text".into(),
            out_section_addr: 0x401000,
            out_section_size: 0x20,
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

/// Build a `LinkMap` from the linker's final layout state.
///
/// Called after address assignment, when `output_sections[*].addr` and each
/// input's `output_offset` are final. Symbols are attached to the input
/// section that contains them so the map shows, for every address, which
/// object file put it there — the question a map file exists to answer.
pub fn build_link_map(
    output_sections: &[crate::backend::linker_common::OutputSection],
    object_names: &[String],
    // (name, object_idx, section_idx, value-within-section)
    symbols: &[(String, usize, usize, u64)],
    entry_symbol: Option<&str>,
    entry_addr: u64,
) -> LinkMap {
    // Index symbols by the input section they live in, so the contribution
    // loop below is O(symbols) overall rather than O(inputs x symbols).
    let mut by_input: crate::common::fx_hash::FxHashMap<(usize, usize), Vec<(&str, u64)>> =
        crate::common::fx_hash::FxHashMap::default();
    for (name, oi, si, val) in symbols {
        by_input
            .entry((*oi, *si))
            .or_default()
            .push((name.as_str(), *val));
    }

    let mut map = LinkMap {
        entry_symbol: entry_symbol.map(str::to_string),
        entry_addr,
        ..LinkMap::default()
    };

    for sec in output_sections {
        if sec.inputs.is_empty() && sec.mem_size == 0 {
            continue;
        }
        for input in &sec.inputs {
            let addr = sec.addr + input.output_offset;
            let mut syms: Vec<(String, u64)> = by_input
                .get(&(input.object_idx, input.section_idx))
                .map(|v| {
                    v.iter()
                        .map(|(n, off)| ((*n).to_string(), addr + off))
                        .collect()
                })
                .unwrap_or_default();
            // Ascending address, like GNU ld.
            syms.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

            map.contributions.push(MapSectionContribution {
                output_section: sec.name.clone(),
                out_section_addr: sec.addr,
                out_section_size: sec.mem_size,
                input_file: object_names
                    .get(input.object_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("<object {}>", input.object_idx)),
                input_section: sec.name.clone(),
                output_addr: addr,
                size: input.size,
                symbols: syms,
            });
        }
    }
    map
}

#[cfg(test)]
mod build_tests {
    use super::*;
    use crate::backend::linker_common::{InputSection, OutputSection};

    fn sec(name: &str, addr: u64, size: u64, inputs: Vec<InputSection>) -> OutputSection {
        OutputSection {
            name: name.into(),
            sh_type: 1,
            flags: 0,
            alignment: 1,
            inputs,
            data: Vec::new(),
            addr,
            file_offset: 0,
            mem_size: size,
        }
    }

    #[test]
    fn map_reports_addresses_per_input_and_symbol() {
        let secs = vec![sec(
            ".text",
            0x401000,
            0x40,
            vec![
                InputSection {
                    object_idx: 0,
                    section_idx: 1,
                    output_offset: 0,
                    size: 0x2d,
                },
                InputSection {
                    object_idx: 1,
                    section_idx: 1,
                    output_offset: 0x2d,
                    size: 0xd,
                },
            ],
        )];
        let names = vec!["bmain.o".to_string(), "syms.o".to_string()];
        let syms = vec![
            ("_start".to_string(), 0usize, 1usize, 0u64),
            ("touch".to_string(), 1, 1, 0),
        ];
        let m = build_link_map(&secs, &names, &syms, Some("_start"), 0x401000);
        let mut s = String::new();
        m.write_gnu(&mut s);

        // Output-section header carries the section's own address and size.
        assert!(
            s.contains(".text           0x0000000000401000       40"),
            "{s}"
        );
        // Each input is attributed to its file at its real address.
        assert!(s.contains("0x0000000000401000       2d bmain.o"), "{s}");
        assert!(s.contains("0x000000000040102d        d syms.o"), "{s}");
        // Symbol addresses are absolute, not section-relative.
        assert!(s.contains("0x000000000040102d                touch"), "{s}");
        assert!(s.contains("_start"));
    }

    #[test]
    fn symbols_are_sorted_by_address() {
        let secs = vec![sec(
            ".data",
            0x403000,
            0x10,
            vec![InputSection {
                object_idx: 0,
                section_idx: 2,
                output_offset: 0,
                size: 0x10,
            }],
        )];
        let names = vec!["a.o".to_string()];
        // Deliberately supplied out of order.
        let syms = vec![
            ("z".to_string(), 0usize, 2usize, 8u64),
            ("a".to_string(), 0, 2, 0),
            ("m".to_string(), 0, 2, 4),
        ];
        let m = build_link_map(&secs, &names, &syms, None, 0);
        let mut s = String::new();
        m.write_gnu(&mut s);
        let ia = s.find("                a").unwrap();
        let im = s.find("                m").unwrap();
        let iz = s.find("                z").unwrap();
        assert!(ia < im && im < iz, "symbols not address-ordered:\n{s}");
    }

    #[test]
    fn empty_layout_still_produces_valid_map() {
        let m = build_link_map(&[], &[], &[], None, 0);
        let mut s = String::new();
        m.write_gnu(&mut s);
        assert!(s.contains("Memory Configuration"));
        assert!(s.contains("Linker script and memory map"));
    }
}

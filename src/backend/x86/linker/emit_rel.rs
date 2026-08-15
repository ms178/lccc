//! Relocatable linking (`ld -r`): merge ET_REL objects into one ET_REL object.
//!
//! This is the incremental-link mode used by the Linux kernel to produce
//! `vmlinux.o` from `vmlinux.a` (mandatory when CONFIG_X86_KERNEL_IBT or
//! CONFIG_LTO is enabled, since objtool runs on the merged object), and by
//! module builds (`.ko` files are `ld -r` outputs).
//!
//! Semantics implemented (matching GNU ld):
//! * Input sections with identical names are concatenated (no `.text.foo` →
//!   `.text` folding — that is script-driven and does not happen under -r).
//! * Relocations are NOT applied; they are carried over with `r_offset`
//!   rebased to the merged section and symbol indices remapped.
//! * References through STT_SECTION symbols are redirected to the merged
//!   output section's section symbol with the addend rebased.
//! * COMDAT (SHT_GROUP/GRP_COMDAT) groups are deduplicated by signature
//!   symbol: the first instance wins, later instances' member sections are
//!   dropped along with their relocations and local symbols.
//! * Global symbols are deduplicated: strong beats weak, defined beats
//!   undefined, largest COMMON wins, duplicate strong definitions error.
//! * Local symbols (including STT_FILE) are all preserved, values rebased.

use crate::common::fx_hash::{FxHashMap, FxHashSet};

use crate::backend::elf::{
    ELF_MAGIC, ELFCLASS64, ELFDATA2LSB, ET_REL, EM_X86_64,
    SHT_NULL, SHT_PROGBITS, SHT_SYMTAB, SHT_STRTAB, SHT_RELA, SHT_REL,
    SHT_NOBITS, SHT_GROUP,
    STB_LOCAL, STB_GLOBAL, STB_WEAK,
    STT_SECTION, STT_FILE,
    SHN_UNDEF, SHN_ABS, SHN_COMMON,
    read_u32, w16, w32, w64,
};
use crate::backend::linker_common::{Elf64Object, write_elf64_shdr};

/// One merged output section under construction.
struct RelOutSec {
    name: String,
    sh_type: u32,
    flags: u64,
    align: u64,
    entsize: u64,
    size: u64,
    /// (obj_idx, sec_idx, offset_in_output)
    inputs: Vec<(usize, usize, u64)>,
}

pub fn link_relocatable(
    objects: &[Elf64Object],
    output_path: &str,
) -> Result<(), String> {
    // ── 1. COMDAT group deduplication ──────────────────────────────────
    // dead: input sections dropped because their group lost.
    let mut dead: FxHashSet<(usize, usize)> = FxHashSet::default();
    let mut group_signatures: FxHashSet<String> = FxHashSet::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if sec.sh_type != SHT_GROUP { continue; }
            let data = &obj.section_data[si];
            if data.len() < 4 { continue; }
            let flags = read_u32(data, 0);
            if flags & 1 == 0 { continue; } // not GRP_COMDAT
            // Signature symbol: symtab entry sec.info
            let sig_idx = sec.info as usize;
            let sig = obj.symbols.get(sig_idx)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            if sig.is_empty() { continue; }
            let is_dup = !group_signatures.insert(sig);
            if is_dup {
                for k in (4..data.len()).step_by(4) {
                    if k + 4 > data.len() { break; }
                    let member = read_u32(data, k) as usize;
                    if member < obj.sections.len() {
                        dead.insert((oi, member));
                    }
                }
            }
        }
    }

    // ── 2. Merge input sections by exact name ──────────────────────────
    let mut out_secs: Vec<RelOutSec> = Vec::new();
    let mut out_by_name: FxHashMap<String, usize> = FxHashMap::default();
    // (obj, sec) -> (out_sec_idx, offset)
    let mut sec_map: FxHashMap<(usize, usize), (usize, u64)> = FxHashMap::default();

    for (oi, obj) in objects.iter().enumerate() {
        for (si, sec) in obj.sections.iter().enumerate() {
            if matches!(sec.sh_type,
                SHT_NULL | SHT_SYMTAB | SHT_STRTAB | SHT_RELA | SHT_REL | SHT_GROUP) {
                continue;
            }
            if sec.name.is_empty() { continue; }
            if dead.contains(&(oi, si)) { continue; }

            let idx = match out_by_name.get(&sec.name) {
                Some(&i) => i,
                None => {
                    let i = out_secs.len();
                    out_by_name.insert(sec.name.clone(), i);
                    out_secs.push(RelOutSec {
                        name: sec.name.clone(),
                        sh_type: sec.sh_type,
                        flags: sec.flags,
                        align: sec.addralign.max(1),
                        entsize: sec.entsize,
                        size: 0,
                        inputs: Vec::new(),
                    });
                    i
                }
            };
            let os = &mut out_secs[idx];
            // Flags are OR'd; PROGBITS wins over NOBITS if mixed (GNU rule).
            os.flags |= sec.flags;
            if sec.sh_type == SHT_PROGBITS { os.sh_type = SHT_PROGBITS; }
            if sec.entsize != os.entsize { os.entsize = 0; } // mixed entsize: clear
            let a = sec.addralign.max(1);
            if a > os.align { os.align = a; }
            let off = (os.size + a - 1) & !(a - 1);
            os.inputs.push((oi, si, off));
            os.size = off + sec.size;
            sec_map.insert((oi, si), (idx, off));
        }
    }

    // ── 3. Build the merged symbol table ───────────────────────────────
    // Layout: [0]=NULL, section symbols (one per output section), locals,
    // then globals (ELF requires all locals before the first global).
    struct OutSym {
        name: String,
        info: u8,
        other: u8,
        shndx_kind: SymShndx,
        value: u64,
        size: u64,
    }
    enum SymShndx {
        OutSec(usize),
        Abs,
        Common,
        Undef,
    }

    let mut out_syms: Vec<OutSym> = Vec::new();
    // section symbol index for each output section (into final symtab; the
    // final index is out_sym index + 1 because of the NULL entry).
    let n_out = out_secs.len();
    for i in 0..n_out {
        out_syms.push(OutSym {
            name: String::new(),
            info: STT_SECTION, // binding LOCAL(0) | type SECTION
            other: 0,
            shndx_kind: SymShndx::OutSec(i),
            value: 0,
            size: 0,
        });
    }

    // Per-object symbol remap: old sym idx -> (new symtab idx, extra addend)
    let mut remaps: Vec<FxHashMap<u32, (u32, i64)>> = Vec::with_capacity(objects.len());

    // First pass: locals.
    for (oi, obj) in objects.iter().enumerate() {
        let mut remap: FxHashMap<u32, (u32, i64)> = FxHashMap::default();
        for (yi, sym) in obj.symbols.iter().enumerate() {
            if !sym.is_local() { continue; }
            if sym.sym_type() == STT_SECTION {
                // Redirect to the output section symbol with addend rebase.
                if let Some(&(out_i, off)) = sec_map.get(&(oi, sym.shndx as usize)) {
                    remap.insert(yi as u32, ((out_i + 1) as u32, off as i64));
                }
                continue;
            }
            if sym.sym_type() == STT_FILE {
                let new_idx = (out_syms.len() + 1) as u32;
                remap.insert(yi as u32, (new_idx, 0));
                out_syms.push(OutSym {
                    name: sym.name.clone(), info: sym.info, other: sym.other,
                    shndx_kind: SymShndx::Abs, value: 0, size: 0,
                });
                continue;
            }
            // Skip locals defined in dead (COMDAT-loser) sections; any
            // reloc that still points at them is a malformed input.
            if sym.shndx != SHN_UNDEF && sym.shndx != SHN_ABS
                && dead.contains(&(oi, sym.shndx as usize)) {
                continue;
            }
            let (shndx_kind, value) = if sym.shndx == SHN_ABS {
                (SymShndx::Abs, sym.value)
            } else if sym.shndx == SHN_UNDEF {
                (SymShndx::Undef, 0)
            } else if let Some(&(out_i, off)) = sec_map.get(&(oi, sym.shndx as usize)) {
                (SymShndx::OutSec(out_i), sym.value + off)
            } else {
                continue; // symbol in a skipped section (e.g. .rela.*)
            };
            let new_idx = (out_syms.len() + 1) as u32;
            remap.insert(yi as u32, (new_idx, 0));
            out_syms.push(OutSym {
                name: sym.name.clone(), info: sym.info, other: sym.other,
                shndx_kind, value, size: sym.size,
            });
        }
        remaps.push(remap);
    }
    let n_local = out_syms.len() + 1; // + NULL entry

    // Second pass: globals with deduplication.
    // name -> index into out_syms
    let mut global_idx: FxHashMap<String, usize> = FxHashMap::default();
    for (oi, obj) in objects.iter().enumerate() {
        for (yi, sym) in obj.symbols.iter().enumerate() {
            if sym.is_local() || sym.name.is_empty() { continue; }
            if sym.sym_type() == STT_SECTION || sym.sym_type() == STT_FILE { continue; }

            // Definitions inside dead COMDAT sections become non-definitions
            // (their surviving twin provides the symbol).
            let in_dead = sym.shndx != SHN_UNDEF && sym.shndx != SHN_ABS
                && sym.shndx != SHN_COMMON
                && dead.contains(&(oi, sym.shndx as usize));

            let is_defined = !sym.is_undefined() && sym.shndx != SHN_COMMON && !in_dead;
            let is_common = sym.shndx == SHN_COMMON;

            let (shndx_kind, value): (SymShndx, u64) = if in_dead || sym.is_undefined() {
                (SymShndx::Undef, 0)
            } else if sym.shndx == SHN_ABS {
                (SymShndx::Abs, sym.value)
            } else if is_common {
                (SymShndx::Common, sym.value) // value = alignment for COMMON
            } else if let Some(&(out_i, off)) = sec_map.get(&(oi, sym.shndx as usize)) {
                (SymShndx::OutSec(out_i), sym.value + off)
            } else {
                (SymShndx::Undef, 0)
            };

            match global_idx.get(&sym.name) {
                None => {
                    let idx = out_syms.len();
                    global_idx.insert(sym.name.clone(), idx);
                    remaps[oi].insert(yi as u32, ((idx + 1) as u32, 0));
                    out_syms.push(OutSym {
                        name: sym.name.clone(), info: sym.info, other: sym.other,
                        shndx_kind, value, size: sym.size,
                    });
                }
                Some(&idx) => {
                    remaps[oi].insert(yi as u32, ((idx + 1) as u32, 0));
                    let existing = &mut out_syms[idx];
                    let e_defined = !matches!(existing.shndx_kind,
                        SymShndx::Undef | SymShndx::Common);
                    let e_weak = existing.info >> 4 == STB_WEAK;
                    let e_common = matches!(existing.shndx_kind, SymShndx::Common);
                    if is_defined {
                        if !e_defined || (e_weak && sym.is_global()) {
                            *existing = OutSym {
                                name: sym.name.clone(), info: sym.info, other: sym.other,
                                shndx_kind, value, size: sym.size,
                            };
                        } else if e_defined && !e_weak && sym.is_global() {
                            return Err(format!(
                                "-r: multiple definition of '{}' (duplicate in {})",
                                sym.name, obj.source_name));
                        }
                    } else if is_common {
                        if e_common && sym.size > existing.size {
                            existing.size = sym.size;
                            if sym.value > existing.value { existing.value = sym.value; }
                        } else if !e_defined && !e_common {
                            *existing = OutSym {
                                name: sym.name.clone(), info: sym.info, other: sym.other,
                                shndx_kind: SymShndx::Common, value, size: sym.size,
                            };
                        }
                    }
                    // undefined ref against anything existing: no change
                }
            }
        }
    }

    // ── 4. Merge section data & rebase relocations ─────────────────────
    const SHF_EXECINSTR: u64 = 0x4;
    let mut sec_datas: Vec<Vec<u8>> = Vec::with_capacity(n_out);
    for os in &out_secs {
        if os.sh_type == SHT_NOBITS {
            sec_datas.push(Vec::new());
            continue;
        }
        // Executable sections: fill alignment gaps with NOP (0x90), matching
        // GNU ld. Zero bytes between functions break instruction-stream
        // consumers (objtool: "can't find starting instruction").
        let fill: u8 = if os.flags & SHF_EXECINSTR != 0 { 0x90 } else { 0x00 };
        let mut data = vec![fill; os.size as usize];
        for &(oi, si, off) in &os.inputs {
            let sd = &objects[oi].section_data[si];
            let s = off as usize;
            let e = s + sd.len();
            if e <= data.len() && !sd.is_empty() {
                data[s..e].copy_from_slice(sd);
            }
        }
        sec_datas.push(data);
    }

    // .rela.<name> payloads, indexed by output section.
    let mut rela_datas: Vec<Vec<u8>> = vec![Vec::new(); n_out];
    for (oi, obj) in objects.iter().enumerate() {
        for (si, relas) in obj.relocations.iter().enumerate() {
            if relas.is_empty() { continue; }
            let Some(&(out_i, base_off)) = sec_map.get(&(oi, si)) else { continue };
            let rd = &mut rela_datas[out_i];
            rd.reserve(relas.len() * 24);
            for rela in relas {
                let (new_sym, extra_addend) = match remaps[oi].get(&rela.sym_idx) {
                    Some(&v) => v,
                    None => {
                        // Symbol vanished (local in a dead COMDAT section):
                        // GNU ld resolves such relocs against the surviving
                        // group. Only STT_SECTION references can hit this
                        // path legally; look up the surviving section by name.
                        let sym = obj.symbols.get(rela.sym_idx as usize);
                        let target = sym.and_then(|s| {
                            let ssec = obj.sections.get(s.shndx as usize)?;
                            out_by_name.get(&ssec.name).copied()
                        });
                        match target {
                            Some(t) => ((t + 1) as u32, 0),
                            None => {
                                return Err(format!(
                                    "-r: relocation against dropped symbol in {}",
                                    obj.source_name));
                            }
                        }
                    }
                };
                let mut e = [0u8; 24];
                e[0..8].copy_from_slice(&(rela.offset + base_off).to_le_bytes());
                let r_info = ((new_sym as u64) << 32) | (rela.rela_type as u64);
                e[8..16].copy_from_slice(&r_info.to_le_bytes());
                e[16..24].copy_from_slice(&(rela.addend + extra_addend).to_le_bytes());
                rd.extend_from_slice(&e);
            }
        }
    }

    // ── 5. Serialize the ET_REL file ────────────────────────────────────
    // Section header order:
    //   [0] NULL
    //   [1..=n_out]           merged sections
    //   [rela...]             one .rela.X per section with relocations
    //   [symtab] [strtab] [shstrtab]
    let mut rela_shidx: Vec<Option<usize>> = vec![None; n_out];
    let mut next_idx = n_out + 1;
    for i in 0..n_out {
        if !rela_datas[i].is_empty() {
            rela_shidx[i] = Some(next_idx);
            next_idx += 1;
        }
    }
    let symtab_idx = next_idx;
    let strtab_idx = symtab_idx + 1;
    let shstrtab_idx = strtab_idx + 1;
    let sh_count = shstrtab_idx + 1;

    // symtab/strtab payloads
    let mut symtab_data: Vec<u8> = Vec::with_capacity((out_syms.len() + 1) * 24);
    symtab_data.extend_from_slice(&[0u8; 24]);
    let mut strtab: Vec<u8> = vec![0];
    let mut str_off: FxHashMap<String, u32> = FxHashMap::default();
    for s in &out_syms {
        let noff: u32 = if s.name.is_empty() { 0 } else {
            *str_off.entry(s.name.clone()).or_insert_with(|| {
                let o = strtab.len() as u32;
                strtab.extend_from_slice(s.name.as_bytes());
                strtab.push(0);
                o
            })
        };
        let shndx: u16 = match s.shndx_kind {
            SymShndx::OutSec(i) => (i + 1) as u16,
            SymShndx::Abs => SHN_ABS,
            SymShndx::Common => SHN_COMMON,
            SymShndx::Undef => SHN_UNDEF,
        };
        let mut e = [0u8; 24];
        e[0..4].copy_from_slice(&noff.to_le_bytes());
        e[4] = s.info;
        e[5] = s.other;
        e[6..8].copy_from_slice(&shndx.to_le_bytes());
        e[8..16].copy_from_slice(&s.value.to_le_bytes());
        e[16..24].copy_from_slice(&s.size.to_le_bytes());
        symtab_data.extend_from_slice(&e);
    }

    // shstrtab
    let mut shstrtab: Vec<u8> = vec![0];
    let mut shstr = |t: &mut Vec<u8>, n: &str| -> u32 {
        let o = t.len() as u32;
        t.extend_from_slice(n.as_bytes());
        t.push(0);
        o
    };
    let sec_name_offs: Vec<u32> = out_secs.iter()
        .map(|os| shstr(&mut shstrtab, &os.name)).collect();
    let rela_name_offs: Vec<u32> = out_secs.iter().enumerate()
        .map(|(i, os)| if rela_datas[i].is_empty() { 0 }
             else { shstr(&mut shstrtab, &format!(".rela{}", os.name)) })
        .collect();
    let symtab_name = shstr(&mut shstrtab, ".symtab");
    let strtab_name = shstr(&mut shstrtab, ".strtab");
    let shstrtab_name = shstr(&mut shstrtab, ".shstrtab");

    // Data layout after the 64-byte ELF header.
    let mut out: Vec<u8> = Vec::new();
    out.resize(64, 0);
    let mut sec_offsets: Vec<u64> = vec![0; n_out];
    for i in 0..n_out {
        let a = out_secs[i].align.max(1) as usize;
        while out.len() % a != 0 { out.push(0); }
        sec_offsets[i] = out.len() as u64;
        if out_secs[i].sh_type != SHT_NOBITS {
            out.extend_from_slice(&sec_datas[i]);
        }
    }
    let mut rela_offsets: Vec<u64> = vec![0; n_out];
    for i in 0..n_out {
        if rela_datas[i].is_empty() { continue; }
        while out.len() % 8 != 0 { out.push(0); }
        rela_offsets[i] = out.len() as u64;
        out.extend_from_slice(&rela_datas[i]);
    }
    while out.len() % 8 != 0 { out.push(0); }
    let symtab_off = out.len() as u64;
    out.extend_from_slice(&symtab_data);
    let strtab_off = out.len() as u64;
    out.extend_from_slice(&strtab);
    let shstrtab_off = out.len() as u64;
    out.extend_from_slice(&shstrtab);
    while out.len() % 8 != 0 { out.push(0); }
    let shoff = out.len() as u64;

    // Section headers.
    write_elf64_shdr(&mut out, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    for i in 0..n_out {
        let os = &out_secs[i];
        write_elf64_shdr(&mut out, sec_name_offs[i], os.sh_type, os.flags,
            0, sec_offsets[i], os.size, 0, 0, os.align.max(1), os.entsize);
    }
    for i in 0..n_out {
        if rela_datas[i].is_empty() { continue; }
        write_elf64_shdr(&mut out, rela_name_offs[i], SHT_RELA, 0x40, // SHF_INFO_LINK
            0, rela_offsets[i], rela_datas[i].len() as u64,
            symtab_idx as u32, (i + 1) as u32, 8, 24);
    }
    write_elf64_shdr(&mut out, symtab_name, SHT_SYMTAB, 0,
        0, symtab_off, symtab_data.len() as u64,
        strtab_idx as u32, n_local as u32, 8, 24);
    write_elf64_shdr(&mut out, strtab_name, SHT_STRTAB, 0,
        0, strtab_off, strtab.len() as u64, 0, 0, 1, 0);
    write_elf64_shdr(&mut out, shstrtab_name, SHT_STRTAB, 0,
        0, shstrtab_off, shstrtab.len() as u64, 0, 0, 1, 0);

    // ELF header.
    out[0..4].copy_from_slice(&ELF_MAGIC);
    out[4] = ELFCLASS64; out[5] = ELFDATA2LSB; out[6] = 1;
    w16(&mut out, 16, ET_REL);
    w16(&mut out, 18, EM_X86_64);
    w32(&mut out, 20, 1);
    w64(&mut out, 24, 0);       // e_entry
    w64(&mut out, 32, 0);       // e_phoff
    w64(&mut out, 40, shoff);
    w32(&mut out, 48, 0);
    w16(&mut out, 52, 64);      // e_ehsize
    w16(&mut out, 54, 0);       // e_phentsize
    w16(&mut out, 56, 0);       // e_phnum
    w16(&mut out, 58, 64);      // e_shentsize
    w16(&mut out, 60, sh_count as u16);
    w16(&mut out, 62, shstrtab_idx as u16);

    std::fs::write(output_path, &out)
        .map_err(|e| format!("failed to write '{}': {}", output_path, e))
}

// STB_LOCAL/STB_GLOBAL imported for potential callers; silence unused warnings.
#[allow(dead_code)]
const _: (u8, u8) = (STB_LOCAL, STB_GLOBAL);

//! GNU ld's input-format switch, `-b FORMAT` / `--format FORMAT`, and its
//! `binary` input format.
//!
//! `-b` is positional and names the format of the *input files that follow*
//! (bfd `ld/lexsup.c`: `-b TARGET, --format TARGET  Specify target for
//! following input files`), until the next `-b`.  It is not an alias for
//! `--oformat`: bfd links `gcc main.c -Wl,-b,binary` by reading the driver's
//! trailing objects and libraries as raw data (and fails on the duplicate
//! `_binary_*` symbols), and the idioms that use it embed a file:
//!
//! ```text
//! ld -r -b binary blob.bin -o blob.o
//! cc main.c -Wl,-b,binary,blob.bin,-b,elf64-x86-64
//! ```
//!
//! lccc's loaders recognise ELF objects, archives and scripts by content, so
//! an ELF target name (or `default`) needs no action.  A `binary` input is
//! turned into the relocatable object bfd's `binary` target presents for it
//! (`bfd/binary.c`): one `.data` section (`SHF_ALLOC|SHF_WRITE`, alignment
//! 1) holding the file's bytes, and the global symbols
//! `_binary_<name>_start` / `_end` (in `.data`) and `_size` (`SHN_ABS`),
//! where `<name>` is the file name as written on the command line with every
//! byte that is not ASCII alphanumeric replaced by `_`.  The symbol table
//! layout matches `ld -r -b binary` (section symbol, then start, size, end).
//! Converting at the front ends keeps every backend loader (x86-64, i686,
//! AArch64, RISC-V, and `-r`) unchanged.  Other input formats are refused:
//! a raw object format lccc cannot read must not be linked as something else.

use std::path::PathBuf;

use crate::backend::elf::EM_386;

/// The ELF machines a wrapper object can be written for, by name, for the
/// standalone `lccc-ld` front end (which cannot see `backend::elf`).
pub mod machine {
    pub const X86_64: u16 = crate::backend::elf::EM_X86_64;
    pub const I386: u16 = crate::backend::elf::EM_386;
    pub const AARCH64: u16 = crate::backend::elf::EM_AARCH64;
    pub const RISCV: u16 = crate::backend::elf::EM_RISCV;
}

/// The format `-b` / `--format` selects for the inputs that follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    /// ELF objects, archives and scripts, recognised by content.
    Native,
    /// Raw bytes, wrapped in a one-section object (see the module docs).
    Binary,
}

/// Classify a `-b` / `--format` word.
pub fn parse_input_format(word: &str) -> Result<InputFormat, String> {
    if word == "binary" {
        Ok(InputFormat::Binary)
    } else if word == "default" || word.starts_with("elf") {
        Ok(InputFormat::Native)
    } else {
        Err(format!(
            "input format '{word}' (-b/--format) is not supported: lccc-ld reads ELF \
             objects, archives and linker scripts, and raw files under `-b binary`"
        ))
    }
}

/// Whether `-b<rest>` is the joined `-b FORMAT` spelling.  GNU ld parses
/// with `getopt_long_only`, so a single-dash word that begins a long option
/// (`-build-id`, `-bss-plt`, `-base-file`) is that option, never `-b`; only
/// a recognisable BFD target word makes it the short option.
pub fn is_joined_format(rest: &str) -> bool {
    const TARGETS: &[&str] = &[
        "binary",
        "default",
        "elf",
        "srec",
        "symbolsrec",
        "ihex",
        "verilog",
        "tekhex",
        "plugin",
        "pe-",
        "pei-",
    ];
    TARGETS.iter().any(|t| rest.starts_with(t)) || rest.starts_with('=')
}

/// The `-b` / `--format` word carried by `tok` (joined spellings), or, when
/// `tok` is the separated spelling, `Some(None)` to ask for the next word.
/// `None`: `tok` is not an input-format switch.
pub fn format_switch(tok: &str) -> Option<Option<&str>> {
    if tok == "-b" || tok == "--format" || tok == "-format" {
        return Some(None);
    }
    if let Some(v) = tok
        .strip_prefix("--format=")
        .or_else(|| tok.strip_prefix("-format="))
    {
        return Some(Some(v));
    }
    let rest = tok.strip_prefix("-b")?;
    is_joined_format(rest).then(|| Some(rest.strip_prefix('=').unwrap_or(rest)))
}

/// `_binary_<name>` for the file `name` as written (bfd `mangle_name`).
pub fn symbol_stem(name: &str) -> String {
    let mut stem = String::with_capacity(name.len() + 8);
    stem.push_str("_binary_");
    stem.extend(name.bytes().map(|b| {
        if b.is_ascii_alphanumeric() {
            b as char
        } else {
            '_'
        }
    }));
    stem
}

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHF_WRITE: u64 = 1;
const SHF_ALLOC: u64 = 2;
const SHN_ABS: u16 = 0xfff1;
const STT_SECTION: u8 = 3;
const STB_GLOBAL: u8 = 1;

/// One symbol of the wrapper object: name offset, info, section, value.
struct Sym(u32, u8, u16, u64);

/// One section header of the wrapper object.
struct Shdr {
    name: u32,
    ty: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
}

/// The relocatable object bfd's `binary` target presents for the file
/// `name` with contents `data`, for ELF machine `machine` (ELFCLASS32 for
/// `EM_386`, ELFCLASS64 otherwise; little-endian, as every lccc target).
pub fn binary_object(name: &str, data: &[u8], machine: u16) -> Result<Vec<u8>, String> {
    let class64 = machine != EM_386;
    let size = u64::try_from(data.len()).map_err(|_| format!("{name}: file too large"))?;
    if !class64 && u32::try_from(size).is_err() {
        return Err(format!("{name}: file too large for an ELFCLASS32 link"));
    }
    let stem = symbol_stem(name);
    let mut strtab = vec![0u8];
    let mut add_str = |s: &str| {
        let at = u32::try_from(strtab.len()).unwrap_or(u32::MAX);
        strtab.extend_from_slice(s.as_bytes());
        strtab.push(0);
        at
    };
    let start = add_str(&format!("{stem}_start"));
    let size_sym = add_str(&format!("{stem}_size"));
    let end = add_str(&format!("{stem}_end"));
    let global = STB_GLOBAL << 4;
    let syms = [
        Sym(0, 0, 0, 0),
        Sym(0, STT_SECTION, 1, 0),
        Sym(start, global, 1, 0),
        Sym(size_sym, global, SHN_ABS, size),
        Sym(end, global, 1, size),
    ];
    let shstrtab = b"\0.data\0.symtab\0.strtab\0.shstrtab\0";

    let (ehsize, symsize, shsize, word) = if class64 {
        (64u64, 24u64, 64u64, 8u64)
    } else {
        (52, 16, 40, 4)
    };
    let pad = |off: u64, align: u64| off.div_ceil(align) * align;
    let data_off = ehsize;
    let symtab_off = pad(data_off + size, word);
    let symtab_size = symsize * syms.len() as u64;
    let strtab_off = symtab_off + symtab_size;
    let strtab_size = strtab.len() as u64;
    let shstrtab_off = strtab_off + strtab_size;
    let shoff = pad(shstrtab_off + shstrtab.len() as u64, word);
    let shdrs = [
        Shdr {
            name: 0,
            ty: 0,
            flags: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            align: 0,
            entsize: 0,
        },
        Shdr {
            name: 1,
            ty: SHT_PROGBITS,
            flags: SHF_ALLOC | SHF_WRITE,
            offset: data_off,
            size,
            link: 0,
            info: 0,
            align: 1,
            entsize: 0,
        },
        Shdr {
            name: 7,
            ty: SHT_SYMTAB,
            flags: 0,
            offset: symtab_off,
            size: symtab_size,
            link: 3,
            info: 2, // first global: after the null and section symbols
            align: word,
            entsize: symsize,
        },
        Shdr {
            name: 15,
            ty: SHT_STRTAB,
            flags: 0,
            offset: strtab_off,
            size: strtab_size,
            link: 0,
            info: 0,
            align: 1,
            entsize: 0,
        },
        Shdr {
            name: 23,
            ty: SHT_STRTAB,
            flags: 0,
            offset: shstrtab_off,
            size: shstrtab.len() as u64,
            link: 0,
            info: 0,
            align: 1,
            entsize: 0,
        },
    ];

    let mut out = Vec::with_capacity(usize::try_from(shoff + shsize * 5).unwrap_or(0));
    // Writes the low `width` bytes of `v`; every field narrower than 8
    // bytes is range-checked by construction (offsets of a < 4 GiB file
    // under ELFCLASS32, small constants otherwise).
    let put = |out: &mut Vec<u8>, v: u64, width: usize| {
        out.extend_from_slice(&v.to_le_bytes()[..width]);
    };
    let w = if class64 { 8 } else { 4 };
    out.extend_from_slice(b"\x7fELF");
    out.push(if class64 { 2 } else { 1 }); // EI_CLASS
    out.push(1); // EI_DATA: little-endian
    out.push(1); // EI_VERSION
    out.extend_from_slice(&[0; 9]); // OSABI, ABI version, padding
    put(&mut out, 1, 2); // e_type: ET_REL
    put(&mut out, u64::from(machine), 2);
    put(&mut out, 1, 4); // e_version
    put(&mut out, 0, w); // e_entry
    put(&mut out, 0, w); // e_phoff
    put(&mut out, shoff, w);
    put(&mut out, 0, 4); // e_flags: none (RISC-V merges skip code-less objects)
    put(&mut out, ehsize, 2);
    put(&mut out, 0, 2); // e_phentsize
    put(&mut out, 0, 2); // e_phnum
    put(&mut out, shsize, 2);
    put(&mut out, shdrs.len() as u64, 2);
    put(&mut out, 4, 2); // e_shstrndx
    out.extend_from_slice(data);
    out.resize(usize::try_from(symtab_off).unwrap_or(usize::MAX), 0);
    for Sym(name, info, shndx, value) in &syms {
        put(&mut out, u64::from(*name), 4);
        if class64 {
            out.push(*info);
            out.push(0);
            put(&mut out, u64::from(*shndx), 2);
            put(&mut out, *value, 8);
            put(&mut out, 0, 8);
        } else {
            put(&mut out, *value, 4);
            put(&mut out, 0, 4);
            out.push(*info);
            out.push(0);
            put(&mut out, u64::from(*shndx), 2);
        }
    }
    out.extend_from_slice(&strtab);
    out.extend_from_slice(shstrtab);
    out.resize(usize::try_from(shoff).unwrap_or(usize::MAX), 0);
    for s in &shdrs {
        put(&mut out, u64::from(s.name), 4);
        put(&mut out, u64::from(s.ty), 4);
        put(&mut out, s.flags, w);
        put(&mut out, 0, w); // sh_addr
        put(&mut out, s.offset, w);
        put(&mut out, s.size, w);
        put(&mut out, u64::from(s.link), 4);
        put(&mut out, u64::from(s.info), 4);
        put(&mut out, s.align, w);
        put(&mut out, s.entsize, w);
    }
    Ok(out)
}

/// Wrapper objects written for `binary` inputs, removed when dropped.
#[derive(Debug, Default)]
pub struct BinaryObjects {
    dir: Option<PathBuf>,
    count: usize,
}

impl BinaryObjects {
    /// Read the raw input `name` (as written on the command line) and write
    /// its wrapper object for `machine`; returns the object's path.
    pub fn materialize(&mut self, name: &str, machine: u16) -> Result<String, String> {
        let data = std::fs::read(name).map_err(|e| format!("cannot find {name}: {e}"))?;
        let object = binary_object(name, &data, machine)?;
        let dir = match &self.dir {
            Some(dir) => dir.clone(),
            None => {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                let dir = std::env::temp_dir()
                    .join(format!("lccc-binary-{}-{stamp}", std::process::id()));
                std::fs::create_dir(&dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
                self.dir = Some(dir.clone());
                dir
            }
        };
        let path = dir.join(format!("{}.o", self.count));
        self.count += 1;
        std::fs::write(&path, object)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(path.to_string_lossy().into_owned())
    }
}

impl Drop for BinaryObjects {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// GNU ld options whose value is the next word, so that word is never an
/// input file (beyond `parse_linker_args`'s re-join list, which covers the
/// options lccc implements).
pub(crate) const VALUE_WORDS: &[&str] = &[
    "-o",
    "--output",
    "-T",
    "--script",
    "-L",
    "--library-path",
    "-l",
    "--library",
    "-y",
    "--trace-symbol",
    "--require-defined",
    "--section-start",
    "-Ttext",
    "-Tdata",
    "-Tbss",
    "-Ttext-segment",
    "-Trodata-segment",
    "-Tldata-segment",
    "-m",
    "-h",
    "-R",
    "--just-symbols",
    "-f",
    "--auxiliary",
    "-F",
    "--filter",
    "--retain-symbols-file",
    "--dependency-file",
    "-plugin",
    "--plugin",
    "-plugin-opt",
    "--plugin-opt",
    "-Y",
    "-A",
    "--architecture",
    "-c",
    "--mri-script",
    "-G",
    "--gpsize",
    "--sort-section",
    "--spare-dynamic-tags",
    "--task-link",
    "--unique",
    "--version-exports-section",
];

/// Rewrite a compiler driver's linker options (`-Wl,` groups, in order) so
/// that every file named inside them under `-b binary` becomes its wrapper
/// object and every `-b` / `--format` switch is consumed.  `value_taking`
/// is `parse_linker_args`'s list of options whose value is the next word.
///
/// The driver adds its own trailing inputs (crt objects, `-lc`) after these
/// options, and GNU ld would read them in the format still in effect, so a
/// `binary` state left open at the end is refused rather than half-honoured.
/// Inputs given to the driver outside `-Wl,` (in its object list) are not
/// positioned relative to these options and keep the native format.
pub fn rewrite_driver_args(
    user_args: &[String],
    machine: u16,
    value_taking: &[&str],
    objects: &mut BinaryObjects,
) -> Result<Option<Vec<String>>, String> {
    let words = |a: &String| a.strip_prefix("-Wl,").map(|w| w.split(',').count());
    let flat: Vec<(usize, &str)> = user_args
        .iter()
        .enumerate()
        .filter_map(|(g, a)| a.strip_prefix("-Wl,").map(|w| (g, w)))
        .flat_map(|(g, w)| w.split(',').map(move |t| (g, t)))
        .collect();
    if !flat.iter().any(|&(_, t)| format_switch(t).is_some()) {
        return Ok(None);
    }
    let takes_value = |t: &str| value_taking.contains(&t) || VALUE_WORDS.contains(&t);
    let mut kept: Vec<Vec<String>> = user_args
        .iter()
        .map(|a| Vec::with_capacity(words(a).unwrap_or(0)))
        .collect();
    let mut format = InputFormat::Native;
    let mut k = 0;
    while k < flat.len() {
        let (g, tok) = flat[k];
        k += 1;
        if let Some(word) = format_switch(tok) {
            let word = match word {
                Some(w) => w,
                None => {
                    let Some(&(_, w)) = flat.get(k) else {
                        return Err(format!("{tok}: missing input format"));
                    };
                    k += 1;
                    w
                }
            };
            format = parse_input_format(word)?;
            continue;
        }
        if takes_value(tok) {
            kept[g].push(tok.to_string());
            if let Some(&(vg, v)) = flat.get(k) {
                kept[vg].push(v.to_string());
                k += 1;
            }
            continue;
        }
        if format == InputFormat::Binary && !tok.starts_with('-') && !tok.is_empty() {
            kept[g].push(objects.materialize(tok, machine)?);
            continue;
        }
        if format == InputFormat::Binary && tok.starts_with("-l") {
            return Err(format!(
                "{tok}: a library searched under -b binary would be read as raw data; \
                 end the binary inputs with -b default first"
            ));
        }
        kept[g].push(tok.to_string());
    }
    if format == InputFormat::Binary {
        return Err(
            "-b binary is still in effect after the last linker option: GNU ld \
                    would read the driver's trailing objects and libraries as raw data; \
                    end the binary inputs with -b default (or the ELF target name)"
                .to_string(),
        );
    }
    let mut out = Vec::with_capacity(user_args.len());
    for (arg, toks) in user_args.iter().zip(kept) {
        if arg.starts_with("-Wl,") {
            if !toks.is_empty() {
                out.push(format!("-Wl,{}", toks.join(",")));
            }
        } else {
            out.push(arg.clone());
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::{EM_RISCV, EM_X86_64};

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn symbol_names_follow_bfd_mangling() {
        assert_eq!(symbol_stem("blob.bin"), "_binary_blob_bin");
        assert_eq!(symbol_stem("/usr/lib/x-1.a"), "_binary__usr_lib_x_1_a");
        // Non-ASCII bytes are replaced byte by byte, like bfd's ISALNUM.
        assert_eq!(symbol_stem("é"), "_binary___");
    }

    #[test]
    fn format_words_and_switches() {
        assert_eq!(parse_input_format("binary"), Ok(InputFormat::Binary));
        assert_eq!(parse_input_format("default"), Ok(InputFormat::Native));
        assert_eq!(parse_input_format("elf64-x86-64"), Ok(InputFormat::Native));
        assert!(parse_input_format("srec").is_err());
        assert_eq!(format_switch("-b"), Some(None));
        assert_eq!(format_switch("--format"), Some(None));
        assert_eq!(format_switch("-bbinary"), Some(Some("binary")));
        assert_eq!(format_switch("--format=binary"), Some(Some("binary")));
        assert_eq!(format_switch("-format=default"), Some(Some("default")));
        assert_eq!(format_switch("-b=binary"), Some(Some("binary")));
        // getopt_long_only: these are long options, not `-b`.
        assert_eq!(format_switch("-build-id"), None);
        assert_eq!(format_switch("-bss-plt"), None);
        assert_eq!(format_switch("-base-file"), None);
        assert_eq!(format_switch("--build-id=sha1"), None);
    }

    #[test]
    fn wrapper_object_matches_bfd_binary_layout() {
        let obj = binary_object("blob.bin", b"hello", EM_X86_64).expect("object");
        let parsed = crate::backend::linker_common::parse_elf64_object(&obj, "blob.o", EM_X86_64)
            .expect("parses as ELF64");
        let data = parsed
            .sections
            .iter()
            .position(|s| s.name == ".data")
            .expect(".data");
        let sec = &parsed.sections[data];
        assert_eq!(sec.flags, SHF_ALLOC | SHF_WRITE);
        assert_eq!(sec.addralign, 1);
        assert_eq!(&*parsed.section_data[data], b"hello");
        let find = |n: &str| {
            parsed
                .symbols
                .iter()
                .find(|s| s.name == n)
                .unwrap_or_else(|| panic!("{n}"))
        };
        let (start, end, size) = (
            find("_binary_blob_bin_start"),
            find("_binary_blob_bin_end"),
            find("_binary_blob_bin_size"),
        );
        assert_eq!((start.value, end.value, size.value), (0, 5, 5));
        assert_eq!(usize::from(start.shndx), data);
        assert_eq!(usize::from(end.shndx), data);
        assert_eq!(size.shndx, SHN_ABS);
        assert!([start, end, size].iter().all(|s| s.info >> 4 == STB_GLOBAL));
    }

    #[test]
    fn wrapper_object_class_follows_the_machine() {
        let i386 = binary_object("b", b"xy", EM_386).expect("object");
        assert_eq!(
            (i386[4], u16::from_le_bytes([i386[18], i386[19]])),
            (1, EM_386)
        );
        let rv = binary_object("b", b"xy", EM_RISCV).expect("object");
        assert_eq!((rv[4], u16::from_le_bytes([rv[18], rv[19]])), (2, EM_RISCV));
        assert_eq!(u32::from_le_bytes([rv[48], rv[49], rv[50], rv[51]]), 0);
    }

    #[test]
    fn driver_rewrite_wraps_only_binary_files() {
        let dir = std::env::temp_dir().join(format!("lccc-bin-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let blob = dir.join("blob.bin");
        std::fs::write(&blob, b"abc").expect("blob");
        let blob = blob.to_string_lossy().into_owned();
        let mut objects = BinaryObjects::default();

        // Untouched without a format switch.
        let plain = args(&["-Wl,--build-id=sha1", "-Wl,-Map,out.map"]);
        assert_eq!(
            rewrite_driver_args(&plain, EM_X86_64, &["-Map"], &mut objects),
            Ok(None)
        );

        let wl = format!("-Wl,-Map,map.txt,-b,binary,{blob},-b,elf64-x86-64,-lm");
        let out = rewrite_driver_args(&args(&[&wl]), EM_X86_64, &["-Map"], &mut objects)
            .expect("ok")
            .expect("rewritten");
        assert_eq!(out.len(), 1);
        let toks: Vec<&str> = out[0].trim_start_matches("-Wl,").split(',').collect();
        assert_eq!(&toks[..2], &["-Map", "map.txt"]);
        assert_eq!(toks[3], "-lm");
        assert!(toks[2].ends_with(".o") && std::path::Path::new(toks[2]).exists());

        // A pair split across groups, and the long spellings.
        let split = args(&[
            "-Wl,--format",
            "-Wl,binary",
            &format!("-Wl,{blob}"),
            "-Wl,--format=default",
        ]);
        let out = rewrite_driver_args(&split, EM_X86_64, &[], &mut objects)
            .expect("ok")
            .expect("rewritten");
        assert_eq!(out.len(), 1, "{out:?}");

        // An open binary state would swallow the driver's trailing inputs.
        let open = args(&[&format!("-Wl,-b,binary,{blob}")]);
        assert!(rewrite_driver_args(&open, EM_X86_64, &[], &mut objects).is_err());
        let lib = args(&["-Wl,-b,binary,-lfoo,-b,default"]);
        assert!(rewrite_driver_args(&lib, EM_X86_64, &[], &mut objects).is_err());
        let bad = args(&["-Wl,-b,srec"]);
        assert!(rewrite_driver_args(&bad, EM_X86_64, &[], &mut objects).is_err());
        let missing = args(&["-Wl,-b,binary,/nonexistent/lccc/blob,-b,default"]);
        assert!(rewrite_driver_args(&missing, EM_X86_64, &[], &mut objects).is_err());

        let made = objects.dir.clone().expect("dir");
        drop(objects);
        assert!(
            !made.exists(),
            "wrapper objects are removed with their owner"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

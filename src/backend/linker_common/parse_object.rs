//! ELF64 relocatable object file parser.
//!
//! This single function replaces the near-identical `parse_object()` functions
//! in x86/linker/elf.rs, arm/linker/elf.rs, and riscv/linker/elf_read.rs.
//! The only parameter that differed was the expected e_machine value.

use super::SymStr;
use super::filemap::FileBacking;
use super::secdata::SectionData;
use super::types::{Elf64Object, Elf64Rela, Elf64Section, Elf64Symbol};
use crate::backend::elf::{
    ELF_MAGIC, ELFCLASS64, ELFDATA2LSB, EM_RISCV, ET_REL, SHT_NOBITS, SHT_RELA, SHT_SYMTAB,
    read_cstr, read_cstr_ref, read_i64, read_u16, read_u32, read_u64, slice_at, table_entry,
};

/// Parse an ELF64 relocatable object file (.o).
///
/// `expected_machine` is the ELF e_machine value to validate (e.g., EM_X86_64,
/// EM_AARCH64, EM_RISCV). Pass 0 to skip machine validation.
/// Parse an ELF64 object, **sharing** the caller's file buffer.
///
/// `buf` is the whole file (or whole archive) already in memory; the object
/// occupies `buf[base .. base + size]`. Section contents become windows into
/// `buf` rather than copies of it — see `secdata.rs` for why this is an
/// `Arc<[u8]>` window and not a borrow with a lifetime.
///
/// Prefer this over [`parse_elf64_object`] on any hot path: the latter has to
/// copy the input into a fresh `Arc` because it only receives a slice.
pub fn parse_elf64_object_at(
    buf: &std::sync::Arc<[u8]>,
    base: usize,
    size: usize,
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    parse_elf64_object_backed(
        &FileBacking::owned(std::sync::Arc::clone(buf)),
        base,
        size,
        source_name,
        expected_machine,
    )
}

/// Parse an ELF64 object from a standalone slice.
///
/// Convenience wrapper for callers that do not hold an `Arc` buffer; it copies
/// `data` once so section windows have something to share. Hot paths should
/// use [`parse_elf64_object_at`].
pub fn parse_elf64_object(
    data: &[u8],
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    parse_elf64_object_inner(data, None, source_name, expected_machine)
}

/// Parse an object that lives inside a memory-mapped (or read) input file.
///
/// This is the true zero-copy path: section bytes are windows into the
/// kernel's page cache, so the file is never copied into the linker's heap at
/// all. `base`/`size` delimit the object within the file, which is what lets a
/// single mapping serve every member of an archive.
pub fn parse_elf64_object_backed(
    backing: &FileBacking,
    base: usize,
    size: usize,
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    let all = backing.as_slice();
    let Some(data) = all.get(base..base.checked_add(size).unwrap_or(usize::MAX)) else {
        return Err(format!(
            "{}: object extends past end of buffer",
            source_name
        ));
    };
    parse_elf64_object_inner(data, Some((backing, base)), source_name, expected_machine)
}

fn parse_elf64_object_inner(
    data: &[u8],
    shared: Option<(&FileBacking, usize)>,
    source_name: &str,
    expected_machine: u16,
) -> Result<Elf64Object, String> {
    if data.len() < 64 {
        return Err(format!("{}: file too small for ELF header", source_name));
    }
    if data[0..4] != ELF_MAGIC {
        return Err(format!("{}: not an ELF file", source_name));
    }
    if data[4] != ELFCLASS64 {
        return Err(format!("{}: not 64-bit ELF", source_name));
    }
    if data[5] != ELFDATA2LSB {
        return Err(format!("{}: not little-endian ELF", source_name));
    }

    let e_type = read_u16(data, 16);
    if e_type != ET_REL {
        return Err(format!(
            "{}: not a relocatable object (type={})",
            source_name, e_type
        ));
    }

    if expected_machine != 0 {
        let e_machine = read_u16(data, 18);
        if e_machine != expected_machine {
            return Err(format!(
                "{}: wrong machine type (expected={}, got={})",
                source_name, expected_machine, e_machine
            ));
        }
    }

    let e_shoff = read_u64(data, 40) as usize;
    let e_shentsize = read_u16(data, 58) as usize;
    let e_shnum = read_u16(data, 60) as usize;
    let e_shstrndx = read_u16(data, 62) as usize;

    if e_shoff == 0 || e_shnum == 0 {
        return Err(format!("{}: no section headers", source_name));
    }

    // Parse section headers
    // A malformed e_shentsize would make every subsequent field read land at
    // the wrong offset; reject it up front rather than parsing garbage.
    if e_shentsize < 64 {
        return Err(format!(
            "{}: bogus e_shentsize {} (need >= 64)",
            source_name, e_shentsize
        ));
    }
    // `Vec::with_capacity(e_shnum)` on an attacker-controlled u16 is bounded
    // (<= 65535 * 88 bytes), so no reservation guard is needed here.
    let mut sections = Vec::with_capacity(e_shnum);
    for i in 0..e_shnum {
        // Overflow-safe: e_shoff is a full u64 and i * e_shentsize can itself
        // overflow, so both operations are checked.
        if table_entry(data, e_shoff, i, e_shentsize).is_none() {
            return Err(format!(
                "{}: section header {} out of bounds",
                source_name, i
            ));
        }
        let off = e_shoff + i * e_shentsize;
        // ELF gABI: sh_addralign is 0 or a positive integral power of two.
        // A non-power-of-two value is not merely cosmetic: the layout engine
        // rounds section addresses up to it, so a value such as 0xffffffffff
        // demands a 1 TiB-aligned output and aborts the process in the
        // allocator (`memory allocation of 1099511627796 bytes failed`).
        // A mutation fuzzer found exactly that.  bfd and mold silently accept
        // the bogus value; wild rejects it, which is the spec-correct and
        // safe behaviour, so lccc rejects it too — with the offending value
        // in the message.
        let addralign = read_u64(data, off + 48);
        if addralign > 1 && !addralign.is_power_of_two() {
            return Err(format!(
                "{}: section header {} has invalid sh_addralign 0x{:x} \
                 (must be 0 or a power of two)",
                source_name, i, addralign
            ));
        }
        sections.push(Elf64Section {
            name_idx: read_u32(data, off),
            name: String::new(),
            sh_type: read_u32(data, off + 4),
            flags: read_u64(data, off + 8),
            addr: read_u64(data, off + 16),
            offset: read_u64(data, off + 24),
            size: read_u64(data, off + 32),
            link: read_u32(data, off + 40),
            info: read_u32(data, off + 44),
            addralign,
            entsize: read_u64(data, off + 56),
        });
    }

    // Read section name string table
    if e_shstrndx < sections.len() {
        let shstrtab = &sections[e_shstrndx];
        let strtab_off = shstrtab.offset as usize;
        let strtab_size = shstrtab.size as usize;
        // Overflow-safe: `strtab_off + strtab_size` wraps for offsets near
        // usize::MAX, which previously turned a malformed object into a panic.
        if let Some(strtab_data) = slice_at(data, strtab_off, strtab_size) {
            for sec in &mut sections {
                sec.name = read_cstr(strtab_data, sec.name_idx as usize);
            }
        }
    }

    // Point each section at its bytes.
    //
    // When the caller supplied the backing buffer (`shared`), this is a pure
    // window: one refcount bump, zero bytes copied. `std::fs::read` has
    // already paid for one copy of the file; the `to_vec()` this replaces was
    // a second one, worth 2.68% of a 20 000-symbol link.
    //
    // Callers that only have a slice fall back to owning a private copy of
    // the whole input once, which is still no worse than per-section copies.
    let owned_buf: Option<FileBacking> = match shared {
        Some(_) => None,
        None => Some(FileBacking::owned(std::sync::Arc::from(data))),
    };
    let (backing, origin) = match shared {
        Some((buf, base)) => (buf, base),
        // `owned_buf` is Some on this branch by construction.
        None => (owned_buf.as_ref().unwrap(), 0usize),
    };

    let mut section_data = Vec::with_capacity(e_shnum);
    for sec in sections.iter_mut() {
        if sec.sh_type == SHT_NOBITS || sec.size == 0 {
            section_data.push(SectionData::empty());
            continue;
        }
        let start = sec.offset as usize;
        let len = sec.size as usize;
        // Validate against the object's own extent first, so a section
        // that escapes this object but happens to land inside a larger
        // shared buffer (an archive member reading its neighbour) is still
        // rejected.
        if slice_at(data, start, len).is_none() {
            return Err(format!(
                "{}: section '{}' data out of bounds",
                source_name, sec.name
            ));
        }
        if sec.flags & SHF_COMPRESSED != 0 {
            section_data.push(expand_compressed_section(data, start, len, sec)?);
            continue;
        }
        let Some(sd) = SectionData::slice_backing(backing, origin + start, len) else {
            return Err(format!(
                "{}: section '{}' data out of bounds",
                source_name, sec.name
            ));
        };
        section_data.push(sd);
    }

    // Find symbol table and its string table
    let mut symbols = Vec::new();
    for i in 0..sections.len() {
        if sections[i].sh_type == SHT_SYMTAB {
            let strtab_idx = sections[i].link as usize;
            let strtab_data: &[u8] = if strtab_idx < section_data.len() {
                section_data[strtab_idx].as_slice()
            } else {
                continue;
            };
            // Bind the backing slice once. Indexing through `SectionData`'s
            // Deref re-derives a bounds-checked subslice on *every* access,
            // and this loop touches it ~6 times per symbol; measured, that
            // cost more than the section copy this type removed.
            let sym_data: &[u8] = section_data[i].as_slice();
            let sym_count = sym_data.len() / 24; // sizeof(Elf64_Sym) = 24
            // Reserve the exact count up front. Growing one element at a time
            // reallocated 15 times and memcpy'd 3.1 MB of Elf64Symbol on a
            // 20 000-symbol object (DHAT). `sym_count` is derived from the
            // section size, so this is exact, not a guess -- and it is bounded
            // by the section that is already in memory, so a malformed header
            // cannot make it request an absurd allocation.
            symbols.reserve(sym_count);
            for j in 0..sym_count {
                let off = j * 24;
                if off + 24 > sym_data.len() {
                    break;
                }
                let name_idx = read_u32(sym_data, off);
                // Borrow the name out of the string table and copy it straight
                // into the symbol's inline storage. Materialising a `String`
                // here first would add one heap allocation per symbol -- the
                // single largest source of allocator traffic in the linker.
                let name = match read_cstr_ref(strtab_data, name_idx as usize) {
                    // Strip the @PLT suffix some assemblers (including our own,
                    // historically) embed in the symbol name instead of using a
                    // R_X86_64_PLT32 relocation; resolution uses the base name.
                    Some(n) => SymStr::new(n.strip_suffix("@PLT").unwrap_or(n)),
                    // Non-UTF-8 name: rare but legal, take the allocating path.
                    None => {
                        let owned = read_cstr(strtab_data, name_idx as usize);
                        SymStr::new(owned.strip_suffix("@PLT").unwrap_or(&owned))
                    }
                };
                symbols.push(Elf64Symbol {
                    name_idx,
                    name,
                    info: sym_data[off + 4],
                    other: sym_data[off + 5],
                    shndx: read_u16(sym_data, off + 6),
                    value: read_u64(sym_data, off + 8),
                    size: read_u64(sym_data, off + 16),
                });
            }
            break;
        }
    }

    // Parse relocations - index by the section they apply to
    let mut relocations = vec![Vec::new(); e_shnum];
    for i in 0..sections.len() {
        if sections[i].sh_type == SHT_RELA {
            // `sh_info` is the index of the section this table applies to, and it
            // comes from the input file. An out-of-range value used to make the
            // whole table vanish silently, which is a wrong image with no
            // diagnostic: relocations that are dropped are relocations that are
            // not applied.
            let target_sec = sections[i].info as usize;
            if target_sec >= sections.len() {
                return Err(format!(
                    "{source_name}: relocation section {} has sh_info {target_sec}, \
                     but the object has only {} section(s)",
                    sections[i].name,
                    sections.len()
                ));
            }
            let target_size = sections[target_sec].size;
            let rela_data: &[u8] = section_data[i].as_slice();
            let rela_count = rela_data.len() / 24; // sizeof(Elf64_Rela) = 24
            let mut relas = Vec::with_capacity(rela_count);
            for j in 0..rela_count {
                let off = j * 24;
                if off + 24 > rela_data.len() {
                    break;
                }
                let r_info = read_u64(rela_data, off + 8);
                let r_offset = read_u64(rela_data, off);
                // `r_offset` is used downstream to compute an index into the
                // output image, so an input file controls where we write. Two
                // failures were measured before this check existed, both with an
                // exit status of 0:
                //
                //   r_offset = 2^64 - 16  the u64 sum wrapped, the unchecked
                //                         writer no-oped, and the image kept the
                //                         unrelocated instruction -- `lea` loaded
                //                         the address of the next instruction
                //                         instead of the symbol's;
                //   r_offset = size       the write landed at the attacker's
                //                         chosen offset, four bytes into whatever
                //                         follows the section.
                //
                // GNU ld refuses both ("reloc against `tgt': error 4", i.e.
                // bfd_reloc_outofrange). The stricter half of its rule -- the
                // field must not extend past the end of the section either --
                // needs the field width, which is architecture-specific, so that
                // half lives in reloc_field::offset_in_section and is applied
                // where the relocations are used.
                // R_RISCV_NONE has no storage operation. Its offset is not a
                // write index; keep the strict rule for all actual fields.
                let riscv_none = read_u16(data, 18) == EM_RISCV && r_info as u32 == 0;
                if !riscv_none && r_offset >= target_size {
                    return Err(format!(
                        "{source_name}: relocation {j} in {} has offset 0x{r_offset:x}, \
                         outside the section it patches (size 0x{target_size:x})",
                        sections[target_sec].name,
                    ));
                }
                relas.push(Elf64Rela {
                    offset: r_offset,
                    sym_idx: (r_info >> 32) as u32,
                    rela_type: (r_info & 0xffffffff) as u32,
                    addend: read_i64(rela_data, off + 16),
                });
            }
            relocations[target_sec] = relas;
        }
    }

    Ok(Elf64Object {
        sections,
        symbols,
        section_data,
        relocations,
        source_name: source_name.to_string(),
    })
}

/// `sh_flags` bit for a section whose bytes are an `Elf64_Chdr` plus a
/// compression stream (gABI).
const SHF_COMPRESSED: u64 = 0x800;
/// `ch_type` values we can decode: zlib (RFC 1950) and zstd.
const ELFCOMPRESS_ZLIB: u32 = 1;
const ELFCOMPRESS_ZSTD: u32 = 2;
/// Upper bound on a declared uncompressed size. No legitimate debug section
/// approaches 512 MiB; the cap stops a hostile `ch_size` from turning a
/// 20-byte input into an unbounded allocation (the same malicious-input
/// posture as the relocation offset checks).
const MAX_DECOMPRESSED_SECTION: u64 = 1 << 29;

/// Apply the `SHF_COMPRESSED` view adjustment for one section: decompress the
/// body once, publish the logical size and alignment, and clear the flag so
/// every downstream check sees one consistent view of the section.
///
/// `SHF_COMPRESSED` (measured): a GCC built with compressed debug defaults
/// emits relocations against the UNCOMPRESSED content (`ch_size`) while the
/// on-disk size holds the zlib/zstd stream. Comparing those offsets with the
/// on-disk size rejected every such object with `relocation ... outside the
/// section it patches`, where GNU ld links it fine.
fn expand_compressed_section(
    data: &[u8],
    start: usize,
    len: usize,
    sec: &mut Elf64Section,
) -> Result<SectionData, String> {
    let raw = &data[start..start + len];
    let logical = decompress_section(&sec.name, raw)?;
    sec.size = logical.len() as u64;
    sec.flags &= !SHF_COMPRESSED;
    // The stored stream is 8-aligned (the chdr needs it); the LOGICAL
    // content carries its own alignment (1 for GCC debug sections).
    // Keeping the outer 8 here would make the writer pad every
    // concatenated contribution to 8 and break DWARF's back-to-back
    // CU chain, which GNU ld produces unpadded (measured).
    // `decompress_section` validated the 24-byte header, so ch_addralign
    // sits at 16..24; default to 1 if that ever stops holding.
    let logical_align = raw
        .get(16..24)
        .and_then(|b| b.try_into().ok())
        .map(u64::from_le_bytes)
        .unwrap_or(1);
    sec.addralign = logical_align.max(1);
    Ok(SectionData::owned(logical))
}

/// Decode an `SHF_COMPRESSED` section body (`Elf64_Chdr` + stream) into its
/// logical bytes.
fn decompress_section(name: &str, raw: &[u8]) -> Result<Vec<u8>, String> {
    let Some(hdr) = raw.get(..24) else {
        return Err(format!(
            "section '{name}' has a truncated SHF_COMPRESSED header"
        ));
    };
    let Ok(ch_type) = <[u8; 4]>::try_from(&hdr[0..4]) else {
        return Err(format!(
            "section '{name}' has a malformed compression header"
        ));
    };
    let ch_type = u32::from_le_bytes(ch_type);
    let Ok(ch_size) = <[u8; 8]>::try_from(&hdr[8..16]) else {
        return Err(format!(
            "section '{name}' has a malformed compression header"
        ));
    };
    let ch_size = u64::from_le_bytes(ch_size);
    if ch_size > MAX_DECOMPRESSED_SECTION {
        return Err(format!(
            "section '{name}' declares a decompressed size of {ch_size} bytes (limit {MAX_DECOMPRESSED_SECTION})"
        ));
    }
    let capacity = ch_size as usize;
    let payload = &raw[24..];
    match ch_type {
        ELFCOMPRESS_ZLIB => zlib_inflate(name, payload, capacity),
        ELFCOMPRESS_ZSTD => zstd_decompress(name, payload, capacity),
        other => Err(format!(
            "section '{name}' uses unknown compression type {other}"
        )),
    }
}

/// Dynamic-loader bindings for the system compression libraries. They are
/// part of every distribution's base system (GNU ld itself links libz), and
/// resolving them at runtime keeps the linker crate dependency-free.
mod sysz {
    use std::os::raw::{c_char, c_int, c_uint, c_void};

    pub const RTLD_NOW: c_int = 2;

    unsafe extern "C" {
        pub fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        pub fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// `z_stream` with the exact C layout on LP64 (112 bytes; zlib validates
    /// that size in `inflateInit_`).
    #[repr(C)]
    #[derive(Default)]
    pub struct ZStream {
        pub next_in: *mut u8,
        pub avail_in: c_uint,
        pub total_in: std::os::raw::c_ulong,
        pub next_out: *mut u8,
        pub avail_out: c_uint,
        pub total_out: std::os::raw::c_ulong,
        pub msg: *mut c_char,
        pub state: *mut c_void,
        pub zalloc: *mut c_void,
        pub zfree: *mut c_void,
        pub opaque: *mut c_void,
        pub data_type: c_int,
        pub adler: std::os::raw::c_ulong,
        pub reserved: std::os::raw::c_ulong,
    }

    pub type ZStreamPtr = *mut ZStream;

    /// `dlopen` a library once and resolve the named symbols, cached for the
    /// process lifetime.
    pub fn open(lib: &str, symbols: &[&str]) -> Result<Vec<usize>, String> {
        let lib_c = std::ffi::CString::new(lib)
            .map_err(|_| format!("library name '{lib}' contains an interior NUL"))?;
        let sym_c = symbols
            .iter()
            .map(|s| {
                std::ffi::CString::new(*s)
                    .map_err(|_| format!("symbol name '{s}' contains an interior NUL"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        // SAFETY: dlopen/dlsym are the documented libc entry points; both
        // results are NULL-checked below before any use, and the CStrings
        // keep their buffers alive across the calls.
        unsafe {
            let handle = dlopen(lib_c.as_ptr(), RTLD_NOW);
            if handle.is_null() {
                return Err(format!("{lib} is unavailable (dlopen failed)"));
            }
            let mut out = Vec::with_capacity(symbols.len());
            for (i, sym) in sym_c.iter().enumerate() {
                let ptr = dlsym(handle, sym.as_ptr());
                if ptr.is_null() {
                    return Err(format!("{lib} does not export {}", symbols[i]));
                }
                out.push(ptr as usize);
            }
            Ok(out)
        }
    }
}

/// Inflate `payload` (RFC 1950 zlib) into exactly `capacity` bytes.
fn zlib_inflate(name: &str, payload: &[u8], capacity: usize) -> Result<Vec<u8>, String> {
    use std::os::raw::c_char;
    use sysz::{ZStream, ZStreamPtr};

    type VersionFn = unsafe extern "C" fn() -> *const c_char;
    type InitFn = unsafe extern "C" fn(ZStreamPtr, *const c_char, i32) -> i32;
    type InflateFn = unsafe extern "C" fn(ZStreamPtr, i32) -> i32;
    type EndFn = unsafe extern "C" fn(ZStreamPtr) -> i32;
    // resolved once per process
    static ZFUNCS: std::sync::OnceLock<Result<Vec<usize>, String>> = std::sync::OnceLock::new();
    let fns = ZFUNCS
        .get_or_init(|| {
            sysz::open(
                "libz.so.1",
                &["zlibVersion", "inflateInit_", "inflate", "inflateEnd"],
            )
        })
        .clone()
        .map_err(|e| format!("section '{name}': {e}"))?;
    let mut out = vec![0u8; capacity];
    let mut stream = ZStream {
        next_in: payload.as_ptr() as *mut u8,
        avail_in: payload.len() as u32,
        next_out: out.as_mut_ptr(),
        avail_out: capacity as u32,
        ..ZStream::default()
    };
    // SAFETY: `fns` holds addresses resolved by `dlsym` against libz.so.1
    // with every symbol NULL-checked at resolution; the transmutes map each
    // address to exactly the C signature zlib documents; the stream layout
    // is zlib's validated LP64 `z_stream`; and both buffers outlive the
    // whole inflate loop.
    unsafe {
        let (version, init, inflate, end): (VersionFn, InitFn, InflateFn, EndFn) = (
            std::mem::transmute::<usize, VersionFn>(fns[0]),
            std::mem::transmute::<usize, InitFn>(fns[1]),
            std::mem::transmute::<usize, InflateFn>(fns[2]),
            std::mem::transmute::<usize, EndFn>(fns[3]),
        );
        let ver = version();
        let ver = std::ffi::CStr::from_ptr(ver).to_string_lossy().into_owned();
        let ver = std::ffi::CString::new(ver)
            .map_err(|_| format!("section '{name}': zlibVersion() contains an interior NUL"))?;
        let rc = init(&mut stream, ver.as_ptr(), size_of::<ZStream>() as i32);
        if rc != 0 {
            return Err(format!("section '{name}': inflateInit_ failed ({rc})"));
        }
        const Z_OK: i32 = 0;
        const Z_STREAM_END: i32 = 1;
        let mut result = Ok(());
        let mut prev_in = u64::MAX;
        for _ in 0..=(payload.len() + capacity + 16) {
            let rc = inflate(&mut stream, 0);
            if rc == Z_STREAM_END {
                break;
            }
            if rc != Z_OK {
                result = Err(format!(
                    "section '{name}': zlib inflate failed ({rc}){}",
                    stream
                        .msg
                        .as_ref()
                        .map(|_| "")
                        .map_or_else(String::new, |_| {
                            if stream.msg.is_null() {
                                String::new()
                            } else {
                                format!(
                                    ": {}",
                                    std::ffi::CStr::from_ptr(stream.msg).to_string_lossy()
                                )
                            }
                        })
                ));
                break;
            }
            if stream.total_in == prev_in {
                result = Err(format!("section '{name}': zlib stream made no progress"));
                break;
            }
            prev_in = stream.total_in;
            if stream.avail_out == 0 {
                break; // capacity filled; verified below
            }
            if stream.avail_in == 0 {
                result = Err(format!(
                    "section '{name}': zlib stream truncated before its declared size"
                ));
                break;
            }
        }
        end(&mut stream);
        if let Err(e) = result {
            return Err(e);
        }
    }
    if stream.total_out as usize != capacity {
        return Err(format!(
            "section '{name}': zlib stream produced {} bytes, header declared {capacity}",
            stream.total_out
        ));
    }
    Ok(out)
}

/// One-shot zstd decompression into exactly `capacity` bytes.
fn zstd_decompress(name: &str, payload: &[u8], capacity: usize) -> Result<Vec<u8>, String> {
    type DecompressFn = unsafe extern "C" fn(
        *mut std::os::raw::c_void,
        usize,
        *const std::os::raw::c_void,
        usize,
    ) -> usize;
    type IsErrorFn = unsafe extern "C" fn(usize) -> std::os::raw::c_uint;
    type ErrorNameFn = unsafe extern "C" fn(usize) -> *const std::os::raw::c_char;
    static ZSTD_FUNCS: std::sync::OnceLock<Result<Vec<usize>, String>> = std::sync::OnceLock::new();
    let fns = ZSTD_FUNCS
        .get_or_init(|| {
            sysz::open(
                "libzstd.so.1",
                &["ZSTD_decompress", "ZSTD_isError", "ZSTD_getErrorName"],
            )
        })
        .clone()
        .map_err(|e| format!("section '{name}': {e}"))?;
    let mut out = vec![0u8; capacity];
    // SAFETY: `fns` holds addresses resolved by `dlsym` against
    // libzstd.so.1 with every symbol NULL-checked at resolution; each
    // transmute maps an address to exactly the C signature libzstd
    // documents; the output buffer is `capacity` bytes; and the input
    // slice outlives the one-shot call.
    let written = unsafe {
        let (decompress, is_error, error_name): (DecompressFn, IsErrorFn, ErrorNameFn) = (
            std::mem::transmute::<usize, DecompressFn>(fns[0]),
            std::mem::transmute::<usize, IsErrorFn>(fns[1]),
            std::mem::transmute::<usize, ErrorNameFn>(fns[2]),
        );
        let written = decompress(
            out.as_mut_ptr().cast(),
            capacity,
            payload.as_ptr().cast(),
            payload.len(),
        );
        if is_error(written) != 0 {
            let msg = error_name(written);
            let msg = if msg.is_null() {
                String::from("unknown zstd error")
            } else {
                std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned()
            };
            return Err(format!("section '{name}': zstd decompress failed: {msg}"));
        }
        written
    };
    if written != capacity {
        return Err(format!(
            "section '{name}': zstd stream produced {written} bytes, header declared {capacity}"
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod compressed_section_tests {
    use super::*;

    /// A zlib stream built from one stored (uncompressed) block: header
    /// `78 01`, BFINAL+stored block header, LEN/NLEN, payload, Adler-32.
    fn stored_zlib(payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() < 65536);
        let mut out = vec![0x78, 0x01, 0x01];
        out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(payload.len() as u16)).to_le_bytes());
        out.extend_from_slice(payload);
        // Adler-32 over the payload.
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in payload {
            a = (a + byte as u32) % 65521;
            b = (b + a) % 65521;
        }
        out.extend_from_slice(&((b << 16) | a).to_be_bytes());
        out
    }

    fn chdr(ch_type: u32, ch_size: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&ch_type.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&ch_size.to_le_bytes());
        out.extend_from_slice(&1u64.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn stored_block_roundtrip_and_relocated_size() {
        let logical = b"Compilation Unit header bytes";
        let body = chdr(
            ELFCOMPRESS_ZLIB,
            logical.len() as u64,
            &stored_zlib(logical),
        );
        let out = decompress_section(".debug_info", &body).expect("zlib roundtrip");
        assert_eq!(out, logical);
    }

    #[test]
    fn declared_size_mismatch_is_rejected() {
        let body = chdr(ELFCOMPRESS_ZLIB, 64, &stored_zlib(b"short"));
        let err = decompress_section(".debug_info", &body).expect_err("size mismatch");
        assert!(err.contains("declared 64"), "{err}");
    }

    #[test]
    fn oversized_declaration_is_rejected_before_allocating() {
        let body = chdr(ELFCOMPRESS_ZLIB, MAX_DECOMPRESSED_SECTION + 1, &[]);
        let err = decompress_section(".debug_info", &body).expect_err("cap");
        assert!(err.contains("limit"), "{err}");
    }

    #[test]
    fn unknown_compression_type_is_rejected() {
        let body = chdr(9, 4, b"abcd");
        let err = decompress_section(".debug_info", &body).expect_err("type");
        assert!(err.contains("unknown compression type 9"), "{err}");
    }

    #[test]
    fn truncated_header_is_rejected() {
        let err = decompress_section(".debug_info", &[0u8; 8]).expect_err("trunc");
        assert!(err.contains("truncated SHF_COMPRESSED"), "{err}");
    }
}

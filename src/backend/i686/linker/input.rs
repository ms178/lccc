//! Input resolution for the i686 linker.
//!
//! Turns the ordered command line (driver CRT objects, user objects, `-l`
//! libraries, linker scripts, positional `--whole-archive` / `--as-needed` /
//! `-Bstatic` state) into the three things the rest of the linker consumes:
//!
//! * the relocatable objects to link (direct inputs, whole-archive members
//!   and the archive members demand extraction pulled in);
//! * the dynamic symbols visible from the linked shared objects;
//! * the shared objects themselves, in command-line order, each with its
//!   `DT_SONAME` and as-needed flag, from which the emitters derive
//!   `DT_NEEDED` deterministically.
//!
//! Semantics follow GNU ld wherever the two can be compared, with one
//! deliberate generalisation: every archive on the line behaves as if the
//! whole link were wrapped in `--start-group`/`--end-group`, so a member can
//! satisfy a reference from an input that appears later.  That accepts
//! every GNU-valid ordering and also links the common "library before
//! object" mistake GNU rejects; for a valid GNU link line it only changes
//! which definition wins in the pathological case of one symbol defined in
//! two different archives, where the earlier archive's member is chosen.
//!
//! Shared objects and archives interact through positions: an archive
//! member is not extracted for a symbol that a shared object appearing
//! *before* the archive already defines (GNU: the name is no longer
//! undefined when the archive is scanned), while a shared object *after*
//! the archive does not prevent extraction (`-lgcc` before `-lgcc_s`).

use crate::backend::common::with_sysroot_prefix;
use crate::backend::linker_common::InputItem;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

use super::dynsym::*;
use super::parse::*;
use super::types::*;

/// Dynamic symbols visible to the link:
/// name → (soname, st_type, st_size, version, is_default_version, binding).
/// One shared-library export, as symbol resolution sees it.
#[derive(Clone, Debug)]
pub(super) struct DynlibSym {
    /// DT_SONAME (or file name) of the defining library.
    pub lib: String,
    pub sym_type: u8,
    pub size: u32,
    pub version: Option<String>,
    pub is_default_ver: bool,
    pub binding: u8,
    /// `st_value` inside the library (see `DynSymInfo::value`).
    pub value: u32,
}

impl DynlibSym {
    fn new(sym: DynSymInfo, lib_soname: &str) -> Self {
        DynlibSym {
            lib: lib_soname.to_string(),
            sym_type: sym.sym_type,
            size: sym.size,
            version: sym.version,
            is_default_ver: sym.is_default_ver,
            binding: sym.binding,
            value: sym.value,
        }
    }
}

/// Shared-library exports by name (the first library defining a name wins).
pub(super) type DynlibSyms = FxHashMap<String, DynlibSym>;

/// Linker scripts may include other linker scripts; GNU ld has no fixed
/// limit, but a cycle must not hang the link.  Real toolchains nest at most
/// one level (libc.so and libgcc_s.so name ELF objects directly).
const MAX_SCRIPT_DEPTH: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LinkItemKind {
    /// A file operand (object, archive, shared object or linker script).
    File(String),
    /// `-lNAME` (or `-l:FILE`), searched in the library directories.
    Lib(String),
}

/// One entry of the ordered input list with the positional state that was
/// in effect where it appeared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LinkItem {
    pub kind: LinkItemKind,
    pub whole_archive: bool,
    pub as_needed: bool,
    pub static_search: bool,
    /// lccc-driver convenience input (the implicit `-lm`): skipped when the
    /// library does not exist.  gcc itself never adds `-lm` for C, so its
    /// absence must not fail links gcc accepts.
    pub optional: bool,
    /// The driver's implicit `-lc`: when it resolves directly to an ELF
    /// `libc.so.6` instead of glibc's `libc.so` linker script (a sysroot
    /// without the script), the script's `libc_nonshared.a` is added
    /// explicitly so `atexit`, `stat` & co. still resolve.
    pub driver_libc: bool,
}

impl LinkItem {
    pub fn from_input(item: &InputItem) -> Self {
        LinkItem {
            kind: if item.is_lib {
                LinkItemKind::Lib(item.name.clone())
            } else {
                LinkItemKind::File(item.name.clone())
            },
            whole_archive: item.whole_archive,
            as_needed: item.as_needed,
            static_search: item.static_search,
            optional: false,
            driver_libc: false,
        }
    }

    pub fn file(path: &str) -> Self {
        LinkItem {
            kind: LinkItemKind::File(path.to_string()),
            whole_archive: false,
            as_needed: false,
            static_search: false,
            optional: false,
            driver_libc: false,
        }
    }

    pub fn lib(name: &str, as_needed: bool, static_search: bool) -> Self {
        LinkItem {
            kind: LinkItemKind::Lib(name.to_string()),
            whole_archive: false,
            as_needed,
            static_search,
            optional: false,
            driver_libc: false,
        }
    }
}

/// A shared object taking part in the link.
#[derive(Clone, Debug)]
pub(super) struct SharedLib {
    /// `DT_SONAME`, or — when the object has none — the name GNU ld records:
    /// the file name for a `-l` search, the operand as written otherwise.
    pub soname: String,
    /// `DT_NEEDED` only if a regular object's reference binds to it.
    pub as_needed: bool,
    /// Position on the command line (monotonic over all loaded inputs).
    pub pos: usize,
    /// Defined dynamic symbol names.
    pub defs: FxHashSet<String>,
    /// Undefined dynamic symbols `(name, is_weak)`.
    pub undefs: Vec<(String, bool)>,
}

pub(super) struct ResolvedInputs {
    pub objects: Vec<InputObject>,
    pub dynlib_syms: DynlibSyms,
    pub shared_libs: Vec<SharedLib>,
}

impl ResolvedInputs {
    /// Names the linked shared objects reference (strong or weak) or define.
    /// An executable must export its own definitions of these names (GNU
    /// ld: a regular definition that is also referenced or defined
    /// dynamically becomes dynamic): a reference could not otherwise bind
    /// to it, and a DSO's own definition would not be interposed — the
    /// DSO's calls to it must reach the executable's copy, as ELF symbol
    /// lookup order requires.
    pub fn dso_visible_names(&self) -> FxHashSet<String> {
        self.shared_libs
            .iter()
            .flat_map(|l| l.undefs.iter().map(|(n, _)| n).chain(l.defs.iter()))
            .cloned()
            .collect()
    }

    /// `DT_NEEDED` entries in command-line order: every no-as-needed shared
    /// object, plus each as-needed one whose soname satisfies at least one
    /// reference (`referenced` holds the sonames the symbol resolver bound
    /// references to).  Deterministic by construction; each soname appears
    /// once because `resolve_inputs` already merged duplicates.
    pub fn needed_sonames(&self, referenced: &FxHashSet<String>) -> Vec<String> {
        self.shared_libs
            .iter()
            .filter(|lib| !lib.as_needed || referenced.contains(&lib.soname))
            .map(|lib| lib.soname.clone())
            .collect()
    }
}

pub(super) struct ResolveRequest<'a> {
    pub items: &'a [LinkItem],
    pub lib_dirs: &'a [String],
    /// `-u` / `--undefined` names: seed demand extraction.
    pub undefined: &'a [String],
    /// `--defsym` pairs: a referenced alias demands the symbols its
    /// expression names.
    pub defsyms: &'a [(String, String)],
    /// False for `-static`: a shared object on the line is an error.
    pub allow_shared: bool,
    /// `--wrap` symbol names.
    pub wrap: &'a [String],
}

/// Resolve the ordered input list.  See the module documentation.
pub(super) fn resolve_inputs(req: &ResolveRequest<'_>) -> Result<ResolvedInputs, String> {
    let mut r = Resolver {
        dirs: req.lib_dirs,
        wrap: req.wrap,
        allow_shared: req.allow_shared,
        next_pos: 0,
        objects: Vec::new(),
        pool: Vec::new(),
        archives: FxHashMap::default(),
        shared: Vec::new(),
        shared_defs: Vec::new(),
        shared_by_soname: FxHashMap::default(),
    };
    for item in req.items {
        r.load_item(item)?;
    }
    r.finish(req.undefined, req.defsyms)
}

/// Is `sym` a definition an archive index (armap) would list?  GNU's armap
/// holds global, weak and GNU_UNIQUE definitions (COMMON included) — never
/// locals, which have file scope and cannot satisfy another file's
/// reference.
fn is_global_def(sym: &InputSymbol) -> bool {
    const STB_GNU_UNIQUE: u8 = 10;
    !sym.name.is_empty()
        && sym.section_index != SHN_UNDEF
        && sym.sym_type != STT_FILE
        && sym.sym_type != STT_SECTION
        && matches!(sym.binding, STB_GLOBAL | STB_WEAK | STB_GNU_UNIQUE)
}

/// A reference that demands a definition: undefined and not weak (GNU ld
/// never extracts an archive member for a weak undefined symbol).
fn is_strong_undef(sym: &InputSymbol) -> bool {
    !sym.name.is_empty()
        && sym.section_index == SHN_UNDEF
        && sym.sym_type != STT_FILE
        && sym.sym_type != STT_SECTION
        && sym.binding == STB_GLOBAL
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FileKind {
    Relocatable,
    SharedObject,
    Archive,
    ThinArchive,
    Script,
    /// An ELF file for another class/machine, or an ELF type that cannot be
    /// a link input (ET_EXEC, ET_CORE).
    Incompatible(&'static str),
    Unknown,
}

fn classify(data: &[u8]) -> FileKind {
    if data.len() >= 4 && data[0..4] == ELF_MAGIC {
        if data.len() < 52 || data[4] != ELFCLASS32 || data[5] != ELFDATA2LSB {
            return FileKind::Incompatible("not a little-endian ELF32 file");
        }
        if read_u16(data, 18) != EM_386 {
            return FileKind::Incompatible("not an i386 ELF file");
        }
        return match read_u16(data, 16) {
            ET_REL => FileKind::Relocatable,
            ET_DYN => FileKind::SharedObject,
            _ => FileKind::Incompatible("neither a relocatable object nor a shared object"),
        };
    }
    if data.len() >= 8 && &data[0..8] == b"!<arch>\n" {
        return FileKind::Archive;
    }
    if is_thin_archive(data) {
        return FileKind::ThinArchive;
    }
    if std::str::from_utf8(data).is_ok() {
        return FileKind::Script;
    }
    FileKind::Unknown
}

/// Would this `-l` candidate be usable?  GNU ld skips (with a "skipping
/// incompatible" note) files of the wrong class or machine and keeps
/// searching — how an amd64 host's `/usr/lib/libfoo.so` coexists with
/// `/usr/lib32/libfoo.so` on one search path.  An archive is judged by its
/// first ELF member; one without ELF members is compatible (empty archives
/// are legal inputs).
fn candidate_compatible(data: &[u8], path: &str) -> bool {
    match classify(data) {
        FileKind::Incompatible(_) | FileKind::Unknown => false,
        FileKind::Archive => match parse_archive_members(data) {
            Ok(members) => members
                .iter()
                .map(|&(_, off, size)| &data[off..off + size])
                .find(|m| m.len() >= 4 && m[0..4] == ELF_MAGIC)
                .is_none_or(|m| classify(m) == FileKind::Relocatable),
            Err(_) => false,
        },
        FileKind::ThinArchive => match parse_thin_archive_i686(data, path) {
            Ok(members) => members
                .first()
                .is_none_or(|(_, m)| classify(m) == FileKind::Relocatable),
            Err(_) => false,
        },
        _ => true,
    }
}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

struct Resolver<'a> {
    dirs: &'a [String],
    wrap: &'a [String],
    allow_shared: bool,
    next_pos: usize,
    objects: Vec<InputObject>,
    /// Lazily-linked archive members: (archive position, archive id, object).
    pool: Vec<(usize, usize, InputObject)>,
    /// Canonical archive path → (archive id, loaded as whole-archive).
    archives: FxHashMap<PathBuf, (usize, bool)>,
    shared: Vec<SharedLib>,
    /// Full dynamic symbol records, parallel to `shared`.
    shared_defs: Vec<Vec<DynSymInfo>>,
    shared_by_soname: FxHashMap<String, usize>,
}

/// Where a loaded input came from, for the name GNU records and for
/// positional state inherited by linker-script members.
struct LoadCtx<'p> {
    path: &'p str,
    recorded_name: &'p str,
    whole_archive: bool,
    as_needed: bool,
    static_search: bool,
    depth: usize,
}

impl Resolver<'_> {
    fn load_item(&mut self, item: &LinkItem) -> Result<(), String> {
        match &item.kind {
            LinkItemKind::File(path) => {
                let data = std::fs::read(path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        format!("cannot find {}: No such file or directory", path)
                    } else {
                        format!("cannot read {}: {}", path, e)
                    }
                })?;
                self.load_data(
                    &LoadCtx {
                        path,
                        recorded_name: path,
                        whole_archive: item.whole_archive,
                        as_needed: item.as_needed,
                        static_search: item.static_search,
                        depth: 0,
                    },
                    &data,
                )
            }
            LinkItemKind::Lib(name) => {
                let Some((path, data)) = self.search_lib(name, item.static_search)? else {
                    if item.optional {
                        return Ok(());
                    }
                    return Err(format!("cannot find -l{}", name));
                };
                let direct_shared = classify(&data) == FileKind::SharedObject;
                // GNU records a soname-less library found by `-l` under its
                // file name, never the directory it happened to live in.
                let recorded = file_name_of(&path);
                self.load_data(
                    &LoadCtx {
                        path: &path,
                        recorded_name: &recorded,
                        whole_archive: item.whole_archive,
                        as_needed: item.as_needed,
                        static_search: item.static_search,
                        depth: 0,
                    },
                    &data,
                )?;
                if item.driver_libc && direct_shared {
                    if let Some((p, d)) = self.search_exact("libc_nonshared.a")? {
                        self.load_data(
                            &LoadCtx {
                                path: &p,
                                recorded_name: &p,
                                whole_archive: false,
                                as_needed: false,
                                static_search: false,
                                depth: 0,
                            },
                            &d,
                        )?;
                    }
                }
                Ok(())
            }
        }
    }

    /// Search the library directories for `-lNAME` / `-l:FILE`: per
    /// directory `libNAME.so` then `libNAME.a` (only `.a` under
    /// `-Bstatic`), exactly GNU ld's order.
    fn search_lib(
        &self,
        name: &str,
        static_search: bool,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        if let Some(exact) = name.strip_prefix(':') {
            return self.search_exact(exact);
        }
        let so = format!("lib{}.so", name);
        let ar = format!("lib{}.a", name);
        let candidates: &[&str] = if static_search { &[&ar] } else { &[&so, &ar] };
        for dir in self.dirs {
            for cand in candidates {
                if let Some(found) = probe(dir, cand, name)? {
                    return Ok(Some(found));
                }
            }
        }
        Ok(None)
    }

    fn search_exact(&self, file: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        for dir in self.dirs {
            if let Some(found) = probe(dir, file, &format!(":{file}"))? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    fn load_data(&mut self, ctx: &LoadCtx<'_>, data: &[u8]) -> Result<(), String> {
        let pos = self.next_pos;
        self.next_pos += 1;
        match classify(data) {
            FileKind::Relocatable => {
                let mut obj = parse_elf32(data, ctx.path)?;
                apply_wrap(&mut obj, self.wrap);
                self.objects.push(obj);
                Ok(())
            }
            FileKind::SharedObject => self.load_shared(ctx, pos),
            FileKind::Archive | FileKind::ThinArchive => {
                self.load_archive(ctx.path, data, ctx.whole_archive, pos)
            }
            FileKind::Script => self.load_script(ctx, data),
            FileKind::Incompatible(why) => {
                Err(format!("{}: file in wrong format ({})", ctx.path, why))
            }
            FileKind::Unknown => Err(format!("{}: file format not recognized", ctx.path)),
        }
    }

    fn load_shared(&mut self, ctx: &LoadCtx<'_>, pos: usize) -> Result<(), String> {
        if !self.allow_shared {
            return Err(format!(
                "attempted static link of dynamic object `{}'",
                ctx.path
            ));
        }
        let dso = read_elf32_dso(ctx.path)?;
        let soname = dso.soname.unwrap_or_else(|| ctx.recorded_name.to_string());
        if let Some(&idx) = self.shared_by_soname.get(&soname) {
            // The same object named twice (once by a script, once
            // explicitly, ...): one DT_NEEDED, and a single no-as-needed
            // mention makes it unconditional.
            self.shared[idx].as_needed &= ctx.as_needed;
            return Ok(());
        }
        self.shared_by_soname
            .insert(soname.clone(), self.shared.len());
        self.shared.push(SharedLib {
            soname,
            as_needed: ctx.as_needed,
            pos,
            defs: dso.defs.iter().map(|d| d.name.clone()).collect(),
            undefs: dso.undefs,
        });
        self.shared_defs.push(dso.defs);
        Ok(())
    }

    fn load_script(&mut self, ctx: &LoadCtx<'_>, data: &[u8]) -> Result<(), String> {
        if ctx.depth >= MAX_SCRIPT_DEPTH {
            return Err(format!(
                "{}: linker scripts nested more than {} deep",
                ctx.path, MAX_SCRIPT_DEPTH
            ));
        }
        let text = String::from_utf8_lossy(data);
        let Some(entries) = crate::backend::elf::parse_linker_script_inputs(&text) else {
            return Err(format!(
                "{}: file format not recognized (only GROUP/INPUT linker scripts are \
                 supported as link inputs; pass a full script with -T)",
                ctx.path
            ));
        };
        let script_dir = Path::new(ctx.path)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        for input in entries {
            let (mpath, mdata, mrecorded) = match &input.entry {
                LinkerScriptEntry::Lib(name) => {
                    let Some((p, d)) = self.search_lib(name, ctx.static_search)? else {
                        return Err(format!(
                            "cannot find -l{} (referenced from linker script {})",
                            name, ctx.path
                        ));
                    };
                    let rec = file_name_of(&p);
                    (p, d, rec)
                }
                LinkerScriptEntry::Path(p) => {
                    let Some((rp, d)) = self.resolve_script_path(p, &script_dir)? else {
                        return Err(format!(
                            "cannot find {} (referenced from linker script {})",
                            p, ctx.path
                        ));
                    };
                    (rp, d, p.clone())
                }
            };
            self.load_data(
                &LoadCtx {
                    path: &mpath,
                    recorded_name: &mrecorded,
                    whole_archive: ctx.whole_archive,
                    as_needed: ctx.as_needed || input.as_needed,
                    static_search: ctx.static_search,
                    depth: ctx.depth + 1,
                },
                &mdata,
            )?;
        }
        Ok(())
    }

    /// Resolve a file named inside a linker script: an absolute path is
    /// looked up under `LCCC_SYSROOT` first (GNU ld prefixes the sysroot for
    /// scripts inside it), then as written; a relative one next to the
    /// script, then in the library directories.
    fn resolve_script_path(
        &self,
        p: &str,
        script_dir: &Path,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        let mut tries: Vec<String> = Vec::with_capacity(2);
        if p.starts_with('/') {
            let prefixed = with_sysroot_prefix(p);
            if prefixed != p {
                tries.push(prefixed);
            }
            tries.push(p.to_string());
        } else {
            tries.push(script_dir.join(p).to_string_lossy().into_owned());
        }
        for t in tries {
            if Path::new(&t).is_file() {
                let d = std::fs::read(&t).map_err(|e| format!("cannot read {}: {}", t, e))?;
                return Ok(Some((t, d)));
            }
        }
        if p.starts_with('/') {
            return Ok(None);
        }
        self.search_exact(p)
    }

    fn load_archive(
        &mut self,
        path: &str,
        data: &[u8],
        whole_archive: bool,
        pos: usize,
    ) -> Result<(), String> {
        let key = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        if let Some(&(id, was_whole)) = self.archives.get(&key) {
            // Re-listing an archive adds nothing under group semantics —
            // unless it is now whole-archive: then every member not yet
            // extracted joins the link.
            if whole_archive && !was_whole {
                let (take, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pool)
                    .into_iter()
                    .partition(|m| m.1 == id);
                self.pool = keep;
                self.objects.extend(take.into_iter().map(|(_, _, obj)| obj));
                self.archives.insert(key, (id, true));
            }
            return Ok(());
        }
        let id = self.archives.len();
        self.archives.insert(key, (id, whole_archive));
        let members = if is_thin_archive(data) {
            parse_thin_archive_i686(data, path)?
        } else {
            parse_archive(data, path)?
        };
        for (name, mdata) in members {
            let member_name = format!("{}({})", path, name);
            match classify(&mdata) {
                FileKind::Relocatable => {
                    let mut obj = parse_elf32(&mdata, &member_name)?;
                    apply_wrap(&mut obj, self.wrap);
                    if whole_archive {
                        self.objects.push(obj);
                    } else {
                        self.pool.push((pos, id, obj));
                    }
                }
                FileKind::Incompatible(why) if whole_archive => {
                    return Err(format!(
                        "{}: file in wrong format ({}) under --whole-archive",
                        member_name, why
                    ));
                }
                // Members of another class/machine (mixed archives) can
                // never satisfy an i386 reference, and parse_archive only
                // yields ELF-magic members, so nothing else reaches here.
                _ => {}
            }
        }
        Ok(())
    }

    /// Demand extraction to a fixed point, then assemble the result.
    fn finish(
        mut self,
        extra_undefined: &[String],
        defsyms: &[(String, String)],
    ) -> Result<ResolvedInputs, String> {
        // Lowest command-line position of a shared object defining a name.
        let mut shared_def_pos: FxHashMap<String, usize> = FxHashMap::default();
        for lib in &self.shared {
            for name in &lib.defs {
                shared_def_pos.entry(name.clone()).or_insert(lib.pos);
            }
        }

        let mut state = Extraction::default();
        for obj in &self.objects {
            state.add_defs(obj);
        }
        for obj in &self.objects {
            state.add_undefs(obj);
        }
        for lib in &self.shared {
            for (name, weak) in &lib.undefs {
                if !*weak {
                    state.demand(name);
                }
            }
        }
        for name in extra_undefined {
            state.demand(name);
        }
        // `--defsym alias=EXPR`: the alias is materialised by the linker; a
        // reference to it demands every symbol the expression names.
        for (alias, target) in defsyms {
            let referenced = extra_undefined.contains(alias)
                || state.undefined.contains(alias)
                || self.objects.iter().any(|o| {
                    o.symbols
                        .iter()
                        .any(|s| s.section_index == SHN_UNDEF && s.name == *alias)
                });
            state.defined.insert(alias.clone());
            state.undefined.remove(alias);
            if referenced {
                for t in expression_symbols(target) {
                    state.demand(&t);
                }
            }
        }

        loop {
            let before = self.pool.len();
            let mut i = 0;
            while i < self.pool.len() {
                let member_pos = self.pool[i].0;
                let wanted = self.pool[i].2.symbols.iter().any(|sym| {
                    is_global_def(sym)
                        && state.undefined.contains(&sym.name)
                        && shared_def_pos
                            .get(&sym.name)
                            .is_none_or(|&p| p > member_pos)
                });
                if !wanted {
                    i += 1;
                    continue;
                }
                let (_, _, obj) = self.pool.remove(i);
                state.add_defs(&obj);
                state.add_undefs(&obj);
                self.objects.push(obj);
            }
            if self.pool.len() == before {
                break;
            }
        }

        // First shared object (command-line order) defining a name wins;
        // within one object the default version wins (`insert_dynsym`).
        let mut dynlib_syms: DynlibSyms = FxHashMap::default();
        for (lib, defs) in self
            .shared
            .iter()
            .zip(std::mem::take(&mut self.shared_defs))
        {
            for sym in defs {
                insert_dynsym(&mut dynlib_syms, sym, &lib.soname);
            }
        }

        Ok(ResolvedInputs {
            objects: self.objects,
            dynlib_syms,
            shared_libs: self.shared,
        })
    }
}

/// Defined / still-demanded symbol sets for demand extraction.
#[derive(Default)]
struct Extraction {
    defined: FxHashSet<String>,
    undefined: FxHashSet<String>,
}

impl Extraction {
    fn add_defs(&mut self, obj: &InputObject) {
        for sym in obj.symbols.iter().filter(|s| is_global_def(s)) {
            self.undefined.remove(&sym.name);
            self.defined.insert(sym.name.clone());
        }
    }

    fn add_undefs(&mut self, obj: &InputObject) {
        for sym in obj.symbols.iter().filter(|s| is_strong_undef(s)) {
            self.demand(&sym.name);
        }
    }

    fn demand(&mut self, name: &str) {
        if !self.defined.contains(name) {
            self.undefined.insert(name.to_string());
        }
    }
}

/// GNU `--wrap=SYM`: undefined references to `SYM` become `__wrap_SYM`, and
/// undefined references to `__real_SYM` become `SYM`.  Definitions are
/// untouched.  Applied to every object before archive extraction, so a
/// `__wrap_SYM` living in an archive is pulled in like any other reference.
fn apply_wrap(obj: &mut InputObject, wrap: &[String]) {
    if wrap.is_empty() {
        return;
    }
    for sym in &mut obj.symbols {
        if sym.section_index != SHN_UNDEF || sym.name.is_empty() {
            continue;
        }
        if wrap.contains(&sym.name) {
            sym.name = format!("__wrap_{}", sym.name);
        } else if let Some(real) = sym.name.strip_prefix("__real_") {
            if wrap.iter().any(|w| w == real) {
                sym.name = real.to_string();
            }
        }
    }
}

/// Symbol names mentioned by a `--defsym` right-hand side (`b`, `b+4`,
/// `(end - start) / 2`): identifier-shaped tokens that are not numbers.
fn expression_symbols(expr: &str) -> Vec<String> {
    expr.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'))
        .filter(|t| !t.is_empty() && !t.as_bytes()[0].is_ascii_digit())
        .map(str::to_string)
        .collect()
}

/// Probe `dir/file` as a `-l` candidate.  Returns the file when it exists
/// and is compatible; an incompatible file is reported like GNU ld's
/// "skipping incompatible" and the search continues.
fn probe(dir: &str, file: &str, lib: &str) -> Result<Option<(String, Vec<u8>)>, String> {
    let path = format!("{}/{}", dir.trim_end_matches('/'), file);
    if !Path::new(&path).is_file() {
        return Ok(None);
    }
    let data = std::fs::read(&path).map_err(|e| format!("cannot read {}: {}", path, e))?;
    if candidate_compatible(&data, &path) {
        return Ok(Some((path, data)));
    }
    eprintln!(
        "lccc-ld: skipping incompatible {} when searching for -l{}",
        path, lib
    );
    Ok(None)
}

pub(super) fn insert_dynsym(dynlib_syms: &mut DynlibSyms, sym: DynSymInfo, lib_soname: &str) {
    use std::collections::hash_map::Entry;
    match dynlib_syms.entry(sym.name.clone()) {
        Entry::Vacant(e) => {
            e.insert(DynlibSym::new(sym, lib_soname));
        }
        // The same name exported under several versions by the SAME object
        // binds to the default (`@@`) one; a later object never overrides
        // an earlier one.
        Entry::Occupied(mut e) => {
            if e.get().lib == lib_soname && sym.is_default_ver && !e.get().is_default_ver {
                e.insert(DynlibSym::new(sym, lib_soname));
            }
        }
    }
}

/// Demand extraction for the `-T` script path (`load_inputs_for_script`):
/// the same armap semantics as `resolve_inputs`, without shared objects.
pub(super) fn resolve_archive_members(
    inputs: &mut Vec<InputObject>,
    archive_pool: &mut Vec<InputObject>,
    extra_undefined: &[String],
) {
    let mut state = Extraction::default();
    for obj in inputs.iter() {
        state.add_defs(obj);
    }
    for obj in inputs.iter() {
        state.add_undefs(obj);
    }
    for name in extra_undefined {
        state.demand(name);
    }
    loop {
        let before = archive_pool.len();
        let mut i = 0;
        while i < archive_pool.len() {
            let wanted = archive_pool[i]
                .symbols
                .iter()
                .any(|sym| is_global_def(sym) && state.undefined.contains(&sym.name));
            if !wanted {
                i += 1;
                continue;
            }
            let obj = archive_pool.remove(i);
            state.add_defs(&obj);
            state.add_undefs(&obj);
            inputs.push(obj);
        }
        if archive_pool.len() == before {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, shndx: u16, binding: u8) -> InputSymbol {
        InputSymbol {
            name: name.to_string(),
            value: 0,
            size: 0,
            binding,
            sym_type: STT_FUNC,
            visibility: STV_DEFAULT,
            section_index: shndx,
        }
    }

    fn obj(name: &str, symbols: Vec<InputSymbol>) -> InputObject {
        InputObject {
            sections: Vec::new(),
            symbols,
            filename: name.to_string(),
        }
    }

    #[test]
    fn locals_and_weak_undefs_never_drive_extraction() {
        let mut inputs = vec![obj(
            "main.o",
            vec![
                sym("helper", SHN_UNDEF, STB_WEAK),
                sym("need", SHN_UNDEF, STB_GLOBAL),
            ],
        )];
        let mut pool = vec![
            obj("weak.o", vec![sym("helper", 1, STB_GLOBAL)]),
            obj("local.o", vec![sym("need", 1, STB_LOCAL)]),
            obj("real.o", vec![sym("need", 1, STB_GLOBAL)]),
        ];
        resolve_archive_members(&mut inputs, &mut pool, &[]);
        let names: Vec<&str> = inputs.iter().map(|o| o.filename.as_str()).collect();
        assert_eq!(names, ["main.o", "real.o"]);
    }

    #[test]
    fn undefined_option_seeds_extraction_transitively() {
        let mut inputs = vec![obj("main.o", vec![])];
        let mut pool = vec![
            obj("b.o", vec![sym("b", 1, STB_GLOBAL)]),
            obj(
                "a.o",
                vec![sym("a", 1, STB_GLOBAL), sym("b", SHN_UNDEF, STB_GLOBAL)],
            ),
        ];
        resolve_archive_members(&mut inputs, &mut pool, &["a".to_string()]);
        assert_eq!(inputs.len(), 3);
        assert!(pool.is_empty());
    }

    #[test]
    fn wrap_renames_undefined_references_only() {
        let mut o = obj(
            "x.o",
            vec![
                sym("malloc", SHN_UNDEF, STB_GLOBAL),
                sym("__real_malloc", SHN_UNDEF, STB_GLOBAL),
                sym("free", 1, STB_GLOBAL),
                sym("__real_free", SHN_UNDEF, STB_GLOBAL),
            ],
        );
        apply_wrap(&mut o, &["malloc".to_string(), "free".to_string()]);
        let names: Vec<&str> = o.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["__wrap_malloc", "malloc", "free", "free"]);
    }

    #[test]
    fn defsym_expression_tokens() {
        assert_eq!(
            expression_symbols("(end - start) / 2 + 0x10"),
            ["end", "start"]
        );
        assert_eq!(expression_symbols("real_fn"), ["real_fn"]);
    }

    #[test]
    fn needed_order_is_command_line_order_and_as_needed_filtered() {
        let lib = |s: &str, as_needed: bool, pos: usize| SharedLib {
            soname: s.to_string(),
            as_needed,
            pos,
            defs: FxHashSet::default(),
            undefs: Vec::new(),
        };
        let r = ResolvedInputs {
            objects: Vec::new(),
            dynlib_syms: FxHashMap::default(),
            shared_libs: vec![
                lib("libz.so.1", true, 0),
                lib("libc.so.6", false, 1),
                lib("libm.so.6", true, 2),
            ],
        };
        let referenced: FxHashSet<String> = ["libm.so.6".to_string()].into_iter().collect();
        assert_eq!(r.needed_sonames(&referenced), ["libc.so.6", "libm.so.6"]);
    }

    #[test]
    fn classify_rejects_foreign_elf() {
        let mut hdr = vec![0u8; 64];
        hdr[0..4].copy_from_slice(&ELF_MAGIC);
        hdr[4] = 2; // ELFCLASS64
        hdr[5] = 1;
        assert!(matches!(classify(&hdr), FileKind::Incompatible(_)));
        hdr[4] = 1;
        hdr[16] = 1; // ET_REL
        hdr[18] = 62; // EM_X86_64
        assert!(matches!(classify(&hdr), FileKind::Incompatible(_)));
        hdr[18] = 3;
        assert_eq!(classify(&hdr), FileKind::Relocatable);
    }
}

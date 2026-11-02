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
//! Inputs are settled in one pass in command-line order, as GNU ld does:
//! each listing of an archive extracts the members defining a name
//! undefined at that point (and what those members need from the same
//! archive), and each `--as-needed` shared object is decided where it
//! appears, for good.  A name a linked shared object defines is no longer
//! undefined for the archives after it; a dropped one demands nothing.  So
//! a valid GNU link line links exactly what GNU ld links.
//!
//! One deliberate generalisation follows the pass: a name still undefined
//! at the end may be satisfied from any archive, as if the whole line were
//! one `--start-group`.  That only accepts lines GNU ld rejects (the
//! "library before object" mistake) and never changes what a valid line
//! links, because on a valid line nothing a member could define is left
//! undefined.

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
    /// True when `value` falls inside a `PF_X` LOAD segment of the defining
    /// library (see `DynSymInfo::in_exec_segment`).
    pub in_exec: bool,
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
            in_exec: sym.in_exec_segment,
        }
    }
}

/// Shared-library exports by name (the first library defining a name wins).
pub(super) type DynlibSyms = FxHashMap<String, DynlibSym>;

/// Linker scripts may include other linker scripts; the shared bound lives
/// next to the script parser so every backend rejects the same nesting.
const MAX_SCRIPT_DEPTH: usize = crate::backend::elf::MAX_LINKER_SCRIPT_DEPTH;

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
    /// `--as-needed` was in effect where the library was named.
    pub as_needed: bool,
    /// In the link: named `--no-as-needed`, or needed where it appears
    /// (`bind_shared_commons`).  Only a linked library's exports bind, and
    /// only a linked library gets `DT_NEEDED`.
    pub linked: bool,
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
    /// `--warn-common` diagnostics, in GNU ld's order (empty unless asked).
    pub common_warnings: Vec<String>,
    /// Names whose tentative definitions a shared library's data definition
    /// takes over (`bind_shared_commons`); `dynlib_syms` holds that
    /// definition for each.
    pub common_imports: FxHashSet<String>,
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

    /// `DT_NEEDED` entries in command-line order: every linked shared
    /// object.  GNU ld records an `--as-needed` library once it is needed
    /// where it appears, even if a later definition takes the reference
    /// that made it needed.  Each soname appears once because
    /// `resolve_inputs` already merged duplicates.
    pub fn needed_sonames(&self) -> Vec<String> {
        self.shared_libs
            .iter()
            .filter(|lib| lib.linked)
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
    /// `--warn-common`: collect GNU ld's COMMON-symbol diagnostics.
    pub warn_common: bool,
}

/// Resolve the ordered input list.  See the module documentation.
pub(super) fn resolve_inputs(req: &ResolveRequest<'_>) -> Result<ResolvedInputs, String> {
    let mut r = Resolver {
        dirs: req.lib_dirs,
        wrap: req.wrap,
        allow_shared: req.allow_shared,
        next_pos: 0,
        objects: Vec::new(),
        object_pos: Vec::new(),
        pool: Vec::new(),
        archives: FxHashMap::default(),
        archive_listings: Vec::new(),
        shared: Vec::new(),
        shared_defs: Vec::new(),
        shared_paths: Vec::new(),
        shared_by_soname: FxHashMap::default(),
    };
    for item in req.items {
        r.load_item(item)?;
    }
    r.finish(req.undefined, req.defsyms, req.warn_common)
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
    /// Command-line position of each object, parallel to `objects` (an
    /// archive member has its archive's).
    object_pos: Vec<usize>,
    /// Lazily-linked archive members: (archive position, archive id, object).
    pool: Vec<(usize, usize, InputObject)>,
    /// Canonical archive path → (archive id, loaded as whole-archive).
    archives: FxHashMap<PathBuf, (usize, bool)>,
    /// Every listing of an archive as (command-line position, archive id):
    /// GNU ld scans an archive again where it is named again (gcc names
    /// `-lgcc` twice).
    archive_listings: Vec<(usize, usize)>,
    shared: Vec<SharedLib>,
    /// Full dynamic symbol records, parallel to `shared`.
    shared_defs: Vec<Vec<DynSymInfo>>,
    /// The path each shared object was loaded from, parallel to `shared`.
    shared_paths: Vec<String>,
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
                    return Err(crate::backend::linker_common::cannot_find_lib(name));
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
                self.object_pos.push(pos);
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
            linked: true,
            pos,
            defs: dso.defs.iter().map(|d| d.name.clone()).collect(),
            undefs: dso.undefs,
        });
        self.shared_defs.push(dso.defs);
        self.shared_paths.push(ctx.path.to_string());
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
                    // bfd records the path the entry resolved to
                    // (`INPUT(./x.so)` in `./s.so`: `././x.so`).
                    let rec = rp.clone();
                    (rp, d, rec)
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
    /// looked up under `LCCC_SYSROOT` first, then with an already-baked
    /// sysroot prefix stripped, then as written; a relative one is tried next
    /// to the script and then in the library directories.
    fn resolve_script_path(
        &self,
        p: &str,
        script_dir: &Path,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        let mut tries: Vec<String> = Vec::with_capacity(3);
        if p.starts_with('/') {
            let prefixed = with_sysroot_prefix(p);
            if prefixed != p {
                tries.push(prefixed);
            }
            if let Some(stripped) = crate::backend::common::strip_sysroot_prefix(p) {
                tries.push(stripped);
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
            // Named again: scanned again there -- or, now whole-archive,
            // every member not yet extracted joins the link at this position.
            if whole_archive && !was_whole {
                let (take, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pool)
                    .into_iter()
                    .partition(|m| m.1 == id);
                self.pool = keep;
                for (_, _, obj) in take {
                    self.objects.push(obj);
                    self.object_pos.push(pos);
                }
                self.archives.insert(key, (id, true));
            } else {
                self.archive_listings.push((pos, id));
            }
            return Ok(());
        }
        let id = self.archives.len();
        self.archives.insert(key, (id, whole_archive));
        if !whole_archive {
            self.archive_listings.push((pos, id));
        }
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
                        self.object_pos.push(pos);
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

    /// Settle which shared objects are linked and which archive members are
    /// extracted, then assemble the result.
    fn finish(
        mut self,
        extra_undefined: &[String],
        defsyms: &[(String, String)],
        warn_common: bool,
    ) -> Result<ResolvedInputs, String> {
        let mut state = Extraction::default();
        for name in extra_undefined {
            state.demand(name);
        }
        // `--defsym alias=EXPR`: the alias is materialised by the linker; a
        // reference to it demands every symbol the expression names.
        for (alias, target) in defsyms {
            let referenced = extra_undefined.contains(alias)
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

        // The pass in command-line order (see the module documentation).
        // Objects and archive listings sharing a position cannot occur:
        // every input takes its own.
        enum Event {
            Object(usize),
            Archive(usize),
            Library(usize),
        }
        let mut events: Vec<(usize, Event)> = self
            .object_pos
            .iter()
            .enumerate()
            .map(|(i, &pos)| (pos, Event::Object(i)))
            .chain(
                self.archive_listings
                    .iter()
                    .map(|&(pos, id)| (pos, Event::Archive(id))),
            )
            .chain(
                self.shared
                    .iter()
                    .enumerate()
                    .map(|(j, lib)| (lib.pos, Event::Library(j))),
            )
            .collect();
        events.sort_by_key(|&(pos, _)| pos);
        let pool = std::mem::take(&mut self.pool);
        let mut taken = vec![false; pool.len()];
        // (pool index, command-line position) in extraction order.
        let mut extracted: Vec<(usize, usize)> = Vec::new();
        let mut binding = SharedBinding::new(self.shared.len());
        for (pos, event) in events {
            match event {
                Event::Object(i) => {
                    binding.object(&self.objects[i]);
                    state.add(&self.objects[i]);
                }
                Event::Archive(id) => loop {
                    let before = extracted.len();
                    for (k, (_, member_of, obj)) in pool.iter().enumerate() {
                        if *member_of == id && !taken[k] && state.wants(obj) {
                            taken[k] = true;
                            extracted.push((k, pos));
                            binding.object(obj);
                            state.add(obj);
                        }
                    }
                    if extracted.len() == before {
                        break;
                    }
                },
                Event::Library(j) => {
                    let (lib, defs) = (&self.shared[j], &self.shared_defs[j]);
                    if binding.library(j, lib, defs, None) {
                        state.provide(lib, defs);
                    }
                }
            }
        }
        let linked = binding.finish().1;

        // The generalisation: a name still undefined, from any archive.
        loop {
            let before = extracted.len();
            for (k, (pos, _, obj)) in pool.iter().enumerate() {
                if !taken[k] && state.wants(obj) {
                    taken[k] = true;
                    extracted.push((k, *pos));
                    state.add(obj);
                }
            }
            if extracted.len() == before {
                break;
            }
        }
        let mut pool: Vec<Option<InputObject>> =
            pool.into_iter().map(|(_, _, obj)| Some(obj)).collect();
        for (k, pos) in extracted {
            if let Some(obj) = pool[k].take() {
                self.objects.push(obj);
                self.object_pos.push(pos);
            }
        }
        // The libraries stay as decided where they appear; replaying the
        // final order with those decisions yields the common imports.
        let (common_imports, linked) =
            bind_shared_commons(&self.link_order(), self.shared.len(), Some(&linked));
        let common_warnings = if warn_common {
            self.common_warnings(&self.link_order(), &linked)
        } else {
            Vec::new()
        };
        for (lib, &in_link) in self.shared.iter_mut().zip(&linked) {
            lib.linked = in_link;
        }
        // First shared object (command-line order) defining a name wins;
        // within one object the default version wins (`insert_dynsym`).  A
        // name a common imports is the definition that took the common.
        let mut dynlib_syms: DynlibSyms = FxHashMap::default();
        for (j, (lib, defs)) in self
            .shared
            .iter()
            .zip(std::mem::take(&mut self.shared_defs))
            .enumerate()
            .filter(|(_, (lib, _))| lib.linked)
        {
            for sym in defs {
                if sym.is_default_ver && common_imports.get(&sym.name) == Some(&j) {
                    dynlib_syms.insert(sym.name.clone(), DynlibSym::new(sym, &lib.soname));
                } else {
                    insert_dynsym(&mut dynlib_syms, sym, &lib.soname);
                }
            }
        }

        Ok(ResolvedInputs {
            objects: self.objects,
            dynlib_syms,
            shared_libs: self.shared,
            common_warnings,
            common_imports: common_imports.into_keys().collect(),
        })
    }

    /// The inputs in command-line order, as GNU ld meets them: an archive
    /// member at the position of the listing that extracted it, members of
    /// one listing in extraction order (the sort is stable).
    fn link_order(&self) -> Vec<LinkStep<'_>> {
        let mut order: Vec<(usize, LinkStep<'_>)> =
            self.object_pos
                .iter()
                .zip(&self.objects)
                .map(|(&pos, obj)| (pos, LinkStep::Object(obj)))
                .chain(
                    self.shared.iter().zip(&self.shared_defs).enumerate().map(
                        |(index, (lib, defs))| (lib.pos, LinkStep::Library { index, lib, defs }),
                    ),
                )
                .collect();
        order.sort_by_key(|&(pos, _)| pos);
        order.into_iter().map(|(_, step)| step).collect()
    }

    /// `--warn-common` over the inputs in command-line order; a library
    /// `--as-needed` dropped (`linked[j]` false) takes no part.
    fn common_warnings(&self, order: &[LinkStep<'_>], linked: &[bool]) -> Vec<String> {
        use crate::backend::linker_common::warn_common::{
            CommonDiagnostics, dynamic_definition, object_symbol,
        };
        let mut common = CommonDiagnostics::default();
        for &step in order {
            match step {
                LinkStep::Object(obj) => {
                    let symbols = obj.symbols.iter().filter_map(|sym| {
                        object_symbol(
                            &sym.name,
                            sym.binding,
                            sym.sym_type,
                            sym.section_index,
                            u64::from(sym.size),
                            sym.visibility,
                        )
                    });
                    common.add_input(&obj.filename, false, symbols);
                }
                LinkStep::Library { index: j, defs, .. } => {
                    if !linked[j] {
                        continue;
                    }
                    let names = defs
                        .iter()
                        .filter(|def| {
                            def.is_default_ver
                                && dynamic_definition((def.binding << 4) | def.sym_type)
                        })
                        .map(|def| def.name.as_str());
                    common.add_shared_definitions(&self.shared_paths[j], names);
                }
            }
        }
        common.into_warnings()
    }
}

/// One input of the link, in command-line order.
#[derive(Clone, Copy)]
enum LinkStep<'a> {
    Object(&'a InputObject),
    /// The `index`th shared object, with its dynamic symbol records.
    Library {
        index: usize,
        lib: &'a SharedLib,
        defs: &'a [DynSymInfo],
    },
}

#[derive(Clone, Copy)]
enum Slot {
    Undefined,
    /// Referenced, only weakly so far.
    WeakUndefined,
    Common,
    Defined,
    Restricted,
    Shared {
        lib: usize,
        info: u8,
        took_common: bool,
    },
}

/// GNU ld's binding of tentative definitions to shared-library data (bfd
/// `elf_merge_symbol`), met input by input in command-line order: a common
/// and a linked library's strong data definition meet as an import of the
/// latter, whichever comes first ("definition overriding common" after the
/// common, "common overridden by definition" before it).  A weak library
/// definition yields to the common in either order, also one a reference
/// bound earlier; code, TLS (`shared_def_overrides_common`), a later regular
/// definition and a name an object gives non-default visibility never
/// import.  An `--as-needed` library is in the link only if, where it
/// appears, it satisfies a non-weak reference (bfd's `ref_regular_nonweak`:
/// a weak one binds to a linked library but brings none in) or takes a
/// common, or its soname already is: bfd unloads a dropped one, symbols
/// included.
///
/// [`SharedBinding::finish`] returns each imported name with the index of
/// the library providing it, and whether each library is linked.
struct SharedBinding<'a> {
    slots: FxHashMap<&'a str, Slot>,
    linked: Vec<bool>,
    linked_sonames: FxHashSet<&'a str>,
}

impl<'a> SharedBinding<'a> {
    fn new(libraries: usize) -> Self {
        SharedBinding {
            slots: FxHashMap::default(),
            linked: vec![false; libraries],
            linked_sonames: FxHashSet::default(),
        }
    }

    fn object(&mut self, obj: &'a InputObject) {
        use crate::backend::linker_common::shared_def_overrides_common;
        for sym in &obj.symbols {
            if sym.name.is_empty()
                || sym.binding == STB_LOCAL
                || matches!(sym.sym_type, STT_FILE | STT_SECTION)
            {
                continue;
            }
            let slot = self.slots.get(sym.name.as_str()).copied();
            let next = match (sym.section_index, slot) {
                _ if sym.visibility != STV_DEFAULT => Slot::Restricted,
                (_, Some(Slot::Restricted)) => continue,
                (SHN_UNDEF, None) if sym.binding == STB_WEAK => Slot::WeakUndefined,
                (SHN_UNDEF, None | Some(Slot::WeakUndefined)) => Slot::Undefined,
                (SHN_UNDEF, _) => continue,
                (SHN_COMMON, None | Some(Slot::Undefined | Slot::WeakUndefined)) => Slot::Common,
                (SHN_COMMON, Some(Slot::Shared { lib, info, .. }))
                    if shared_def_overrides_common(info) =>
                {
                    Slot::Shared {
                        lib,
                        info,
                        took_common: true,
                    }
                }
                (SHN_COMMON, Some(Slot::Shared { .. })) => Slot::Common,
                (SHN_COMMON, _) => continue,
                _ => Slot::Defined,
            };
            self.slots.insert(&sym.name, next);
        }
    }

    /// Meet the `index`th library; `decided` replaces the `--as-needed`
    /// decision.  Returns whether the library is linked.
    fn library(
        &mut self,
        index: usize,
        lib: &'a SharedLib,
        defs: &'a [DynSymInfo],
        decided: Option<bool>,
    ) -> bool {
        use crate::backend::linker_common::shared_def_overrides_common;
        let exports = || {
            defs.iter()
                .filter(|d| d.is_default_ver)
                .map(|d| (d.name.as_str(), (d.binding << 4) | d.sym_type))
        };
        let in_link = decided.unwrap_or_else(|| {
            !lib.as_needed
                || self.linked_sonames.contains(lib.soname.as_str())
                || exports().any(|(name, info)| match self.slots.get(name) {
                    Some(Slot::Undefined) => true,
                    Some(Slot::Common) => shared_def_overrides_common(info),
                    _ => false,
                })
        });
        if !in_link {
            return false;
        }
        self.linked[index] = true;
        self.linked_sonames.insert(&lib.soname);
        for (name, info) in exports() {
            let took_common = match self.slots.get(name) {
                None | Some(Slot::Undefined | Slot::WeakUndefined) => false,
                Some(Slot::Common) if shared_def_overrides_common(info) => true,
                _ => continue,
            };
            self.slots.insert(
                name,
                Slot::Shared {
                    lib: index,
                    info,
                    took_common,
                },
            );
        }
        true
    }

    fn finish(self) -> (FxHashMap<String, usize>, Vec<bool>) {
        let imports = self
            .slots
            .into_iter()
            .filter_map(|(name, slot)| match slot {
                Slot::Shared {
                    lib,
                    took_common: true,
                    ..
                } => Some((name.to_string(), lib)),
                _ => None,
            })
            .collect();
        (imports, self.linked)
    }
}

/// [`SharedBinding`] over `steps`, with each library's `decided` linked flag
/// when given.
fn bind_shared_commons(
    steps: &[LinkStep<'_>],
    libraries: usize,
    decided: Option<&[bool]>,
) -> (FxHashMap<String, usize>, Vec<bool>) {
    let mut binding = SharedBinding::new(libraries);
    for step in steps {
        match *step {
            LinkStep::Object(obj) => binding.object(obj),
            LinkStep::Library { index, lib, defs } => {
                binding.library(
                    index,
                    lib,
                    defs,
                    decided.and_then(|d| d.get(index).copied()),
                );
            }
        }
    }
    binding.finish()
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

    fn add(&mut self, obj: &InputObject) {
        self.add_defs(obj);
        self.add_undefs(obj);
    }

    /// Would an archive index list `obj` for a name still undefined?
    fn wants(&self, obj: &InputObject) -> bool {
        obj.symbols
            .iter()
            .any(|sym| is_global_def(sym) && self.undefined.contains(&sym.name))
    }

    /// A linked shared object: the names it exports (default versions --
    /// `foo@OLD` answers no plain `foo`) are defined for the inputs after
    /// it, and its own strong references demand definitions.
    fn provide(&mut self, lib: &SharedLib, defs: &[DynSymInfo]) {
        for def in defs.iter().filter(|d| d.is_default_ver) {
            self.undefined.remove(&def.name);
            self.defined.insert(def.name.clone());
        }
        for (name, weak) in &lib.undefs {
            if !*weak {
                self.demand(name);
            }
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

    /// A library exporting `defs` as `(name, binding, type, default version)`.
    fn library(
        soname: &str,
        as_needed: bool,
        defs: &[(&str, u8, u8, bool)],
    ) -> (SharedLib, Vec<DynSymInfo>) {
        let infos: Vec<DynSymInfo> = defs
            .iter()
            .map(|&(name, binding, sym_type, is_default_ver)| DynSymInfo {
                name: name.to_string(),
                sym_type,
                size: 4,
                binding,
                version: None,
                is_default_ver,
                value: 0x1000,
                in_exec_segment: false,
                soname: Some(soname.to_string()),
            })
            .collect();
        let lib = SharedLib {
            soname: soname.to_string(),
            as_needed,
            linked: true,
            pos: 0,
            defs: infos.iter().map(|d| d.name.clone()).collect(),
            undefs: Vec::new(),
        };
        (lib, infos)
    }

    fn object(name: &str, symbols: &[(&str, u16, u8)]) -> InputObject {
        InputObject {
            sections: Vec::new(),
            symbols: symbols
                .iter()
                .map(|&(sym, section_index, visibility)| InputSymbol {
                    name: sym.to_string(),
                    value: 4,
                    size: 4,
                    binding: STB_GLOBAL,
                    sym_type: STT_OBJECT,
                    visibility,
                    section_index,
                })
                .collect(),
            filename: name.to_string(),
        }
    }

    /// `bind_shared_commons` over `inputs` in order, where `Err(k)` is the
    /// `k`th of `libs`: the imported names (sorted, with their library) and
    /// the linked flags.
    fn bind(
        objs: &[InputObject],
        libs: &[(SharedLib, Vec<DynSymInfo>)],
        inputs: &[Result<usize, usize>],
    ) -> (Vec<(String, usize)>, Vec<bool>) {
        let steps: Vec<LinkStep<'_>> = inputs
            .iter()
            .map(|&input| match input {
                Ok(i) => LinkStep::Object(&objs[i]),
                Err(k) => LinkStep::Library {
                    index: k,
                    lib: &libs[k].0,
                    defs: &libs[k].1,
                },
            })
            .collect();
        let (imports, linked) = bind_shared_commons(&steps, libs.len(), None);
        let mut imports: Vec<(String, usize)> = imports.into_iter().collect();
        imports.sort();
        (imports, linked)
    }

    /// GNU ld's common-vs-library matrix, each cell measured with bfd 2.47
    /// (`tests/linker` `common_binds_shared_data` checks the same on real
    /// links): only a strong data definition takes the common, in either
    /// order; weak data, code, TLS and non-default versions never do.
    #[test]
    fn shared_commons_follow_gnu_order_rules() {
        let common = || object("c.o", &[("s", SHN_COMMON, STV_DEFAULT)]);
        let lib = |binding, sym_type, default_ver| {
            library("libl.so", false, &[("s", binding, sym_type, default_ver)])
        };
        let cases = [
            (
                "strong after common",
                lib(STB_GLOBAL, STT_OBJECT, true),
                true,
                true,
            ),
            ("unique after common", lib(10, STT_OBJECT, true), true, true),
            (
                "weak after common",
                lib(STB_WEAK, STT_OBJECT, true),
                true,
                false,
            ),
            (
                "function after common",
                lib(STB_GLOBAL, STT_FUNC, true),
                true,
                false,
            ),
            (
                "tls after common",
                lib(STB_GLOBAL, STT_TLS, true),
                true,
                false,
            ),
            (
                "non-default version",
                lib(STB_GLOBAL, STT_OBJECT, false),
                true,
                false,
            ),
            (
                "strong before common",
                lib(STB_GLOBAL, STT_OBJECT, true),
                false,
                true,
            ),
            (
                "weak before common",
                lib(STB_WEAK, STT_OBJECT, true),
                false,
                false,
            ),
            (
                "function before common",
                lib(STB_GLOBAL, STT_FUNC, true),
                false,
                false,
            ),
            (
                "tls before common",
                lib(STB_GLOBAL, STT_TLS, true),
                false,
                false,
            ),
        ];
        for (what, l, common_first, imported) in cases {
            let order = if common_first {
                [Ok(0), Err(0)]
            } else {
                [Err(0), Ok(0)]
            };
            let (imports, linked) = bind(&[common()], &[l], &order);
            let want: Vec<(String, usize)> = if imported {
                vec![("s".to_string(), 0)]
            } else {
                Vec::new()
            };
            assert_eq!(imports, want, "{what}");
            assert_eq!(linked, [true], "{what}: a no-as-needed library is linked");
        }
    }

    /// An `--as-needed` library is linked only if, where it appears, it
    /// satisfies a reference or takes a common: met before the common it is
    /// dropped (bfd unloads it, so the common stays and no warning names it).
    #[test]
    fn shared_commons_respect_as_needed_position() {
        let objs = [object("c.o", &[("s", SHN_COMMON, STV_DEFAULT)])];
        let libs = [library(
            "libl.so",
            true,
            &[("s", STB_GLOBAL, STT_OBJECT, true)],
        )];
        assert_eq!(
            bind(&objs, &libs, &[Ok(0), Err(0)]),
            (vec![("s".to_string(), 0)], vec![true])
        );
        assert_eq!(
            bind(&objs, &libs, &[Err(0), Ok(0)]),
            (Vec::new(), vec![false])
        );
        // A reference made before the library keeps it, and with it the
        // later common's binding.
        let objs = [
            object("r.o", &[("s", SHN_UNDEF, STV_DEFAULT)]),
            object("c.o", &[("s", SHN_COMMON, STV_DEFAULT)]),
        ];
        assert_eq!(
            bind(&objs, &libs, &[Ok(0), Err(0), Ok(1)]),
            (vec![("s".to_string(), 0)], vec![true])
        );
    }

    /// Only a non-weak reference made before an `--as-needed` library brings
    /// it in (bfd 2.47: `mw.o --as-needed ./libfoo.so` leaves the weak
    /// `foo` null and records no `DT_NEEDED`); a strong reference upgrades
    /// an earlier weak one, and one made after the library is too late.
    #[test]
    fn weak_references_never_need_a_library() {
        let weak = {
            let mut obj = object("w.o", &[("foo", SHN_UNDEF, STV_DEFAULT)]);
            obj.symbols[0].binding = STB_WEAK;
            obj
        };
        let objs = [weak, object("r.o", &[("foo", SHN_UNDEF, STV_DEFAULT)])];
        let libs = [library(
            "libfoo.so",
            true,
            &[("foo", STB_GLOBAL, STT_FUNC, true)],
        )];
        let linked = |inputs: &[Result<usize, usize>]| bind(&objs, &libs, inputs).1;
        assert_eq!(linked(&[Ok(0), Err(0)]), [false]);
        assert_eq!(linked(&[Ok(0), Err(0), Ok(1)]), [false]);
        assert_eq!(linked(&[Ok(0), Ok(1), Err(0)]), [true]);
        assert_eq!(linked(&[Ok(1), Ok(0), Err(0)]), [true]);
    }

    /// A weak library definition met after the common leaves it a common, so
    /// a later library's strong definition still takes it (first-export-wins
    /// would bind nothing); a later regular definition or a non-default
    /// visibility anywhere keeps the name out of every library.
    #[test]
    fn shared_commons_first_strong_definition_and_restrictions() {
        let libs = [
            library("libw.so", false, &[("s", STB_WEAK, STT_OBJECT, true)]),
            library("libl.so", false, &[("s", STB_GLOBAL, STT_OBJECT, true)]),
        ];
        let objs = [object("c.o", &[("s", SHN_COMMON, STV_DEFAULT)])];
        assert_eq!(
            bind(&objs, &libs, &[Ok(0), Err(0), Err(1)]),
            (vec![("s".to_string(), 1)], vec![true, true])
        );
        let objs = [
            object("c.o", &[("s", SHN_COMMON, STV_DEFAULT)]),
            object("d.o", &[("s", 1, STV_DEFAULT)]),
            object("h.o", &[("s", SHN_UNDEF, STV_HIDDEN)]),
        ];
        let lib = [library(
            "libl.so",
            false,
            &[("s", STB_GLOBAL, STT_OBJECT, true)],
        )];
        assert_eq!(
            bind(&objs, &lib, &[Err(0), Ok(0), Ok(1)]).0,
            Vec::<(String, usize)>::new()
        );
        assert_eq!(
            bind(&objs, &lib, &[Err(0), Ok(0), Ok(2)]).0,
            Vec::<(String, usize)>::new()
        );
        assert_eq!(
            bind(&objs, &lib, &[Ok(2), Ok(0), Err(0)]).0,
            Vec::<(String, usize)>::new()
        );
    }

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
    fn needed_order_is_command_line_order_of_linked_libraries() {
        let lib = |s: &str, linked: bool, pos: usize| SharedLib {
            soname: s.to_string(),
            as_needed: true,
            linked,
            pos,
            defs: FxHashSet::default(),
            undefs: Vec::new(),
        };
        let r = ResolvedInputs {
            objects: Vec::new(),
            dynlib_syms: FxHashMap::default(),
            shared_libs: vec![
                lib("libz.so.1", false, 0),
                lib("libc.so.6", true, 1),
                lib("libm.so.6", true, 2),
            ],
            common_warnings: Vec::new(),
            common_imports: FxHashSet::default(),
        };
        assert_eq!(r.needed_sonames(), ["libc.so.6", "libm.so.6"]);
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

    /// One input of [`resolve`]'s command line.
    enum Input {
        Object(InputObject),
        /// A new archive with these members.
        Archive(Vec<InputObject>),
        /// The `n`th archive named again.
        Again(usize),
        Library((SharedLib, Vec<DynSymInfo>)),
    }

    /// The resolver's verdict on `inputs`, in command-line order.
    fn resolve(inputs: Vec<Input>) -> ResolvedInputs {
        let mut r = Resolver {
            dirs: &[],
            wrap: &[],
            allow_shared: true,
            next_pos: 0,
            objects: Vec::new(),
            object_pos: Vec::new(),
            pool: Vec::new(),
            archives: FxHashMap::default(),
            archive_listings: Vec::new(),
            shared: Vec::new(),
            shared_defs: Vec::new(),
            shared_paths: Vec::new(),
            shared_by_soname: FxHashMap::default(),
        };
        let mut archives = 0;
        for (pos, input) in inputs.into_iter().enumerate() {
            match input {
                Input::Object(obj) => {
                    r.objects.push(obj);
                    r.object_pos.push(pos);
                }
                Input::Archive(members) => {
                    r.archive_listings.push((pos, archives));
                    r.pool
                        .extend(members.into_iter().map(|m| (pos, archives, m)));
                    archives += 1;
                }
                Input::Again(id) => r.archive_listings.push((pos, id)),
                Input::Library((mut lib, defs)) => {
                    lib.pos = pos;
                    r.shared.push(lib);
                    r.shared_defs.push(defs);
                    r.shared_paths.push(String::new());
                }
            }
        }
        match r.finish(&[], &[], false) {
            Ok(resolved) => resolved,
            Err(e) => panic!("resolve: {e}"),
        }
    }

    fn linked_objects(resolved: &ResolvedInputs) -> Vec<&str> {
        resolved
            .objects
            .iter()
            .map(|o| o.filename.as_str())
            .collect()
    }

    fn linked_libraries(resolved: &ResolvedInputs) -> Vec<&str> {
        resolved
            .shared_libs
            .iter()
            .filter(|lib| lib.linked)
            .map(|lib| lib.soname.as_str())
            .collect()
    }

    const DEF: u16 = 1;

    fn func_lib(soname: &str, defs: &[&str], undefs: &[&str]) -> (SharedLib, Vec<DynSymInfo>) {
        let defs: Vec<_> = defs
            .iter()
            .map(|&d| (d, STB_GLOBAL, STT_FUNC, true))
            .collect();
        let (mut lib, infos) = library(soname, true, &defs);
        lib.undefs = undefs.iter().map(|&u| (u.to_string(), false)).collect();
        (lib, infos)
    }

    /// bfd 2.47: `libbar.a mf.o ./libfoo.so` never re-scans libbar.a, so
    /// `foo` binds to libfoo.so (`tests/linker` `positional_shared_operands`
    /// "archive-before-user-lib" checks the real link on every target).
    #[test]
    fn an_archive_before_its_user_yields_to_a_later_library() {
        let resolved = resolve(vec![
            Input::Archive(vec![object("bar.o", &[("foo", DEF, STV_DEFAULT)])]),
            Input::Object(object("mf.o", &[("foo", SHN_UNDEF, STV_DEFAULT)])),
            Input::Library(func_lib("libfoo.so", &["foo"], &[])),
        ]);
        assert_eq!(linked_objects(&resolved), ["mf.o"]);
        assert_eq!(linked_libraries(&resolved), ["libfoo.so"]);
    }

    /// `mf.o libA.so libX.a libB.so libZ.a`, all `--as-needed`: libX.a's
    /// member (pulled for libA.so's `x`) defines `y` before libB.so, so
    /// libB.so is dropped -- and its own reference to `z` pulls nothing.
    #[test]
    fn a_dropped_library_demands_nothing() {
        let resolved = resolve(vec![
            Input::Object(object(
                "mf.o",
                &[("a", SHN_UNDEF, STV_DEFAULT), ("y", SHN_UNDEF, STV_DEFAULT)],
            )),
            Input::Library(func_lib("libA.so", &["a"], &["x"])),
            Input::Archive(vec![object(
                "m.o",
                &[("x", DEF, STV_DEFAULT), ("y", DEF, STV_DEFAULT)],
            )]),
            Input::Library(func_lib("libB.so", &["y"], &["z"])),
            Input::Archive(vec![object("mz.o", &[("z", DEF, STV_DEFAULT)])]),
        ]);
        assert_eq!(linked_objects(&resolved), ["mf.o", "m.o"]);
        assert_eq!(linked_libraries(&resolved), ["libA.so"]);
    }

    /// gcc names `-lgcc` twice: the second listing is scanned where it
    /// stands, before the library after it is decided.
    #[test]
    fn an_archive_named_again_is_scanned_again_there() {
        let resolved = resolve(vec![
            Input::Archive(vec![object("f.o", &[("f", DEF, STV_DEFAULT)])]),
            Input::Object(object("mf.o", &[("f", SHN_UNDEF, STV_DEFAULT)])),
            Input::Again(0),
            Input::Library(func_lib("libf.so", &["f"], &[])),
        ]);
        assert_eq!(linked_objects(&resolved), ["mf.o", "f.o"]);
        assert!(linked_libraries(&resolved).is_empty());
    }

    /// The generalisation: a name nothing defines by the end of the line is
    /// taken from an archive named before its user (GNU ld rejects this).
    #[test]
    fn a_name_left_undefined_comes_from_any_archive() {
        let resolved = resolve(vec![
            Input::Archive(vec![object("bar.o", &[("foo", DEF, STV_DEFAULT)])]),
            Input::Object(object("mf.o", &[("foo", SHN_UNDEF, STV_DEFAULT)])),
        ]);
        assert_eq!(linked_objects(&resolved), ["mf.o", "bar.o"]);
    }
}

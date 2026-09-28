//! Link options for the i686 userspace linker, and the capability check.
//!
//! Every GNU ld option that changes the *meaning* of the output is either
//! implemented by this backend or rejected here with a clear error before
//! any input is read.  Options that only steer optimisation (where doing
//! nothing still yields a correct image) are accepted with a warning, never
//! silently.
//!
//! | option | executable | `-shared` |
//! |---|---|---|
//! | `-e/--entry` | entry point (symbol, else number) | `e_entry` |
//! | `-rpath`, `--enable/disable-new-dtags` | DT_RUNPATH / DT_RPATH | same |
//! | `-z now` / `-z lazy` | DT_FLAGS BIND_NOW + DF_1_NOW | same |
//! | `-z relro` / `-z norelro` | PT_GNU_RELRO | same |
//! | `-E/--export-dynamic` | export all globals | (always) |
//! | `-soname` | DT_SONAME | DT_SONAME |
//! | `--build-id[=sha1\|tree]` | `.note.gnu.build-id` + PT_NOTE | same |
//! | `--hash-style=gnu\|sysv\|both` | DT_GNU_HASH / DT_HASH | same |
//! | `-Bsymbolic[-functions]` | no effect (as GNU) | local binding (+ DF_SYMBOLIC) |
//! | `--no-undefined` / `-z defs` | (always) | error on unresolved |
//! | `-z execstack/noexecstack` | PT_GNU_STACK flags | same |
//! | `-z origin/nodelete/nodlopen/initfirst/interpose/nodefaultlib/global` | DF_* / DF_1_* | same |
//! | `-z text` | error on text relocations | same |
//! | `-z ibt`, `-z shstk`, `-z x86-64-{baseline,v2,v3,v4}` | merged `.note.gnu.property` + PT_GNU_PROPERTY | same |
//! | `-z lam-u48`, `-z lam-u57` | warning: ignored (as GNU: x86-64 only) | same |
//! | `--wrap=SYM` | reference renaming | same |
//! | `--gc-sections`, `--icf`, `--sort-common`, `--sort-section`, `-O<n>` | warning: ignored | same |
//! | `--version-script`, `--dynamic-list`, `--exclude-libs`, `-Map`, `--emit-relocs`, `-pie`, `-r`, `-N`, `-Ttext`…, `-z nocopyreloc`, `-z max-page-size≠4096` | error | error |

/// Shared with the x86-64 linker, which parses the options identically.
pub(super) use crate::backend::linker_common::Symbolic;
use crate::backend::linker_common::cet::PropertyLinkFlags;
use crate::backend::linker_common::{HashStyle, LinkerArgs};

// DT_FLAGS bits the emitters derive themselves; the keyword-requested ones
// come from `LinkerArgs::requested_dyn_flags`.
pub(super) const DF_SYMBOLIC: u32 = 0x2;
pub(super) const DF_TEXTREL: u32 = 0x4;
pub(super) const DF_BIND_NOW: u32 = 0x8;
pub(super) const DF_STATIC_TLS: u32 = 0x10;
// DT_FLAGS_1 bits.
pub(super) const DF_1_NOW: u32 = 0x1;

/// Options the emitters act on.
#[derive(Clone, Debug, Default)]
pub(super) struct LinkOptions {
    pub entry: Option<String>,
    /// `-rpath` entries joined with ':' in command-line order, or None.
    pub rpath: Option<String>,
    /// DT_RUNPATH (true, the modern default) or DT_RPATH.
    pub use_runpath: bool,
    pub bind_now: bool,
    pub relro: bool,
    pub export_dynamic: bool,
    pub soname: Option<String>,
    pub exec_stack: Option<bool>,
    pub build_id: bool,
    pub hash_style: HashStyle,
    pub symbolic: Symbolic,
    pub no_undefined: bool,
    /// `-z text`: text relocations are an error.
    pub z_text: bool,
    /// DT_FLAGS / DT_FLAGS_1 bits requested by `-z` keywords (BIND_NOW,
    /// SYMBOLIC and TEXTREL are derived separately).
    pub dt_flags: u32,
    pub dt_flags_1: u32,
    pub wrap: Vec<String>,
    /// `-z ibt` / `-z shstk` / `-z x86-64-*` for the property note merge
    /// (`linker_common::cet`).  The LAM bits are always clear: `elf_i386`
    /// does not source `x86-64-lam.sh`, so GNU ld ignores `-z lam-*`.
    pub properties: PropertyLinkFlags,
}

impl LinkOptions {
    /// Final DT_FLAGS value.
    pub fn flags(&self, textrel: bool, static_tls: bool) -> u32 {
        let mut f = self.dt_flags;
        if static_tls {
            f |= DF_STATIC_TLS;
        }
        if self.bind_now {
            f |= DF_BIND_NOW;
        }
        if textrel {
            f |= DF_TEXTREL;
        }
        if self.symbolic == Symbolic::All {
            f |= DF_SYMBOLIC;
        }
        f
    }

    /// Final DT_FLAGS_1 value.
    pub fn flags_1(&self) -> u32 {
        let mut f = self.dt_flags_1;
        if self.bind_now {
            f |= DF_1_NOW;
        }
        f
    }

    /// The optional `.dynamic` entries these options add, in emission
    /// order: (tag, value-kind).  Shared by the sizing pass and the writer
    /// of both emitters so the two can never disagree.
    ///
    /// `static_tls` sets `DF_STATIC_TLS` (a shared object using a static
    /// TLS model).
    pub fn extra_dynamic_tags(
        &self,
        textrel: bool,
        symbolic_tag: bool,
        static_tls: bool,
    ) -> Vec<(i32, DynValue)> {
        let mut v = Vec::new();
        if let Some(sn) = &self.soname {
            v.push((DT_SONAME, DynValue::Str(sn.clone())));
        }
        if let Some(rp) = &self.rpath {
            let tag = if self.use_runpath {
                DT_RUNPATH
            } else {
                DT_RPATH
            };
            v.push((tag, DynValue::Str(rp.clone())));
        }
        if symbolic_tag && self.symbolic == Symbolic::All {
            v.push((DT_SYMBOLIC, DynValue::Num(0)));
        }
        if textrel {
            v.push((DT_TEXTREL, DynValue::Num(0)));
        }
        let flags = self.flags(textrel, static_tls);
        if flags != 0 {
            v.push((DT_FLAGS, DynValue::Num(flags)));
        }
        let flags_1 = self.flags_1();
        if flags_1 != 0 {
            v.push((DT_FLAGS_1, DynValue::Num(flags_1)));
        }
        v
    }
}

/// Value of an option-derived `.dynamic` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DynValue {
    /// Offset of this string in `.dynstr`.
    Str(String),
    Num(u32),
}

pub(super) const DT_SONAME: i32 = 14;
pub(super) const DT_RPATH: i32 = 15;
pub(super) const DT_SYMBOLIC: i32 = 16;
pub(super) const DT_TEXTREL: i32 = 22;
pub(super) const DT_FLAGS: i32 = 30;
pub(super) const DT_RUNPATH: i32 = 29;
pub(super) const DT_FLAGS_1: i32 = 0x6fff_fffb;

/// Flatten top-level and `-Wl,` arguments into linker tokens.
fn tokens(user_args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    for a in user_args {
        if let Some(wl) = a.strip_prefix("-Wl,") {
            out.extend(wl.split(','));
        } else {
            out.push(a.as_str());
        }
    }
    out
}

fn unsupported<T>(what: &str) -> Result<T, String> {
    Err(format!(
        "{what} is not supported by the elf_i386 userspace linker"
    ))
}

/// Tokens (exact or prefix) naming semantic options this backend does not
/// implement.
fn rejected_option(t: &str) -> Option<&'static str> {
    const EXACT: &[(&str, &str)] = &[
        ("-r", "relocatable output (-r)"),
        ("--relocatable", "relocatable output (-r)"),
        ("-i", "relocatable output (-r)"),
        ("-N", "-N/--omagic"),
        ("--omagic", "-N/--omagic"),
        ("--nmagic", "--nmagic"),
        ("--default-symver", "--default-symver"),
        ("--default-imported-symver", "--default-imported-symver"),
        ("--export-dynamic-symbol", "--export-dynamic-symbol"),
        (
            "--export-dynamic-symbol-list",
            "--export-dynamic-symbol-list",
        ),
    ];
    const PREFIX: &[(&str, &str)] = &[
        ("--dynamic-list", "--dynamic-list"),
        ("--export-dynamic-symbol=", "--export-dynamic-symbol"),
        (
            "--export-dynamic-symbol-list=",
            "--export-dynamic-symbol-list",
        ),
        ("-Ttext", "-Ttext"),
        ("-Tdata", "-Tdata"),
        ("-Tbss", "-Tbss"),
        ("--section-start", "--section-start"),
        ("--image-base", "--image-base"),
        ("--unresolved-symbols", "--unresolved-symbols"),
    ];
    EXACT
        .iter()
        .find(|(k, _)| *k == t)
        .or_else(|| PREFIX.iter().find(|(p, _)| t.starts_with(p)))
        .map(|(_, what)| *what)
}

fn is_optimisation_only(t: &str) -> bool {
    matches!(t, "--gc-sections" | "--sort-common" | "--sort-section")
        || t.starts_with("--icf")
        || t.starts_with("--sort-common=")
        || t.starts_with("--sort-section=")
        || (t.len() == 3 && t.starts_with("-O") && t.as_bytes()[2].is_ascii_digit())
}

/// Validate the parsed arguments for an i686 userspace link and produce the
/// emitter options.  `is_shared` selects the `-shared` rules.
pub(super) fn check_capabilities(
    args: &LinkerArgs,
    user_args: &[String],
    is_shared: bool,
) -> Result<LinkOptions, String> {
    if args.version_script.is_some() {
        return unsupported("--version-script");
    }
    if !args.exclude_libs.is_empty() {
        return unsupported("--exclude-libs");
    }
    if args.map_path.is_some() {
        return unsupported("-Map/--print-map");
    }
    if args.emit_relocs {
        return unsupported("--emit-relocs");
    }
    if args.is_pie {
        return unsupported("PIE output (-pie; link with -no-pie)");
    }
    let toks = tokens(user_args);
    if let Some(what) = toks.iter().find_map(|t| rejected_option(t)) {
        return unsupported(what);
    }
    for t in toks.iter().filter(|t| is_optimisation_only(t)) {
        eprintln!(
            "lccc-ld: warning: {t} ignored by the elf_i386 userspace linker (optimisation only; the output is still correct)"
        );
    }

    if let Some(style) = args.build_id_style.as_deref() {
        if !matches!(style, "sha1" | "tree" | "none" | "0") {
            return unsupported(&format!("--build-id={style} (only sha1/tree)"));
        }
    }

    // All DT_FLAGS / DT_FLAGS_1 bits are in the low 32 bits.
    let (dt_flags, dt_flags_1) = {
        let (f, f1) = args.requested_dyn_flags();
        (f as u32, f1 as u32)
    };
    let mut z_text = false;
    for kw in &args.z_other_keywords {
        let (key, val) = match kw.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (kw.as_str(), None),
        };
        match (key, val) {
            // DF_1_GLOBAL: already in `requested_dyn_flags`.
            ("global", None) => {}
            ("text", None) => z_text = true,
            ("notext" | "textoff", None) => z_text = false,
            // The layout always keeps headers, code and read-only data in
            // separate segments; `noseparate-code` would only save a page.
            ("separate-code" | "noseparate-code" | "combreloc" | "nocombreloc", None) => {}
            // Duplicate strong definitions resolve to the first one
            // (never diagnosed by this linker), which is exactly muldefs.
            ("muldefs", None) => {}
            ("max-page-size" | "common-page-size", Some(v)) => {
                let n = crate::backend::linker_common::defsym::parse_number(v);
                if n != Some(u64::from(super::types::PAGE_SIZE)) {
                    return unsupported(&format!("-z {key}={v} (the page size is fixed at 4096)"));
                }
            }
            ("nocopyreloc", None) => return unsupported("-z nocopyreloc"),
            _ => eprintln!("lccc-ld: warning: -z {kw} ignored"),
        }
    }
    // GNU property note flags.  `elf_i386` sources binutils' `cet.sh` and
    // `x86-64-level.sh` (so `-z ibt`, `-z shstk`, `-z x86-64-*` and the
    // invalid-level fatal all apply) but not `x86-64-lam.sh`: `-z lam-u48`
    // and `-z lam-u57` are unknown keywords there, warned about with GNU's
    // wording (measured: `ld -m elf_i386 -z lam-u48` → `warning: -z
    // lam-u48 ignored`).
    let mut properties = args.property_link_flags()?;
    for (set, kw) in [
        (properties.lam_u48, "lam-u48"),
        (properties.lam_u57, "lam-u57"),
    ] {
        if set {
            eprintln!("lccc-ld: warning: -z {kw} ignored");
        }
    }
    properties.lam_u48 = false;
    properties.lam_u57 = false;
    for kw in &args.z_ignored_keywords {
        eprintln!("lccc-ld: warning: -z {kw} ignored");
    }

    // The last of -Bsymbolic / -Bsymbolic-functions / -Bno-symbolic
    // decides, as in GNU ld (parsed in `parse_linker_args`).  An
    // executable's definitions are never preemptible anyway.
    let symbolic = if is_shared {
        args.symbolic
    } else {
        Symbolic::None
    };

    Ok(LinkOptions {
        entry: args.entry_symbol.clone(),
        rpath: (!args.rpath_entries.is_empty()).then(|| args.rpath_entries.join(":")),
        use_runpath: args.use_runpath,
        bind_now: args.z_now,
        relro: args.z_relro,
        export_dynamic: args.export_dynamic,
        soname: args.soname.clone(),
        exec_stack: args.z_execstack,
        build_id: args.build_id,
        hash_style: args.hash_style,
        symbolic,
        no_undefined: args.no_undefined,
        z_text,
        dt_flags,
        dt_flags_1,
        wrap: args.wrap_symbols.clone(),
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::linker_common::parse_linker_args;

    fn check(args: &[&str], shared: bool) -> Result<LinkOptions, String> {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        check_capabilities(&parse_linker_args(&v), &v, shared)
    }

    #[test]
    fn semantic_options_map_to_emitter_options() {
        let o = check(
            &[
                "-Wl,-e,my_entry",
                "-Wl,-rpath,/a",
                "-Wl,-rpath=/b",
                "-Wl,-z,now",
                "-Wl,-z,origin",
                "-Wl,--export-dynamic",
                "-Wl,--build-id",
                "-Wl,--hash-style=both",
            ],
            false,
        )
        .unwrap();
        assert_eq!(o.entry.as_deref(), Some("my_entry"));
        assert_eq!(o.rpath.as_deref(), Some("/a:/b"));
        assert!(o.use_runpath && o.bind_now && o.export_dynamic && o.build_id);
        assert_eq!(o.hash_style, HashStyle::Both);
        use crate::backend::linker_common::dyn_flags as df;
        assert_eq!(
            o.flags(false, false),
            (df::DF_ORIGIN | df::DF_BIND_NOW) as u32
        );
        assert_eq!(o.flags_1(), (df::DF_1_NOW | df::DF_1_ORIGIN) as u32);
        assert!(o.relro, "RELRO is the default");
        let tags: Vec<i32> = o
            .extra_dynamic_tags(true, false, false)
            .iter()
            .map(|t| t.0)
            .collect();
        assert_eq!(tags, [DT_RUNPATH, DT_TEXTREL, DT_FLAGS, DT_FLAGS_1]);
    }

    #[test]
    fn bsymbolic_variants_are_distinguished_for_shared_only() {
        assert_eq!(
            check(&["-Wl,-Bsymbolic"], true).unwrap().symbolic,
            Symbolic::All
        );
        assert_eq!(
            check(&["-Wl,-Bsymbolic-functions"], true).unwrap().symbolic,
            Symbolic::Functions
        );
        assert_eq!(
            check(&["-Wl,-Bsymbolic"], false).unwrap().symbolic,
            Symbolic::None
        );
        // GNU ld: the last spelling wins, in either order, and
        // -Bno-symbolic resets.
        for (args, want) in [
            (
                &["-Wl,-Bsymbolic", "-Wl,-Bsymbolic-functions"][..],
                Symbolic::Functions,
            ),
            (
                &["-Wl,-Bsymbolic-functions", "-Wl,-Bsymbolic"][..],
                Symbolic::All,
            ),
            (&["-Wl,-Bsymbolic", "-Wl,-Bno-symbolic"][..], Symbolic::None),
            (
                &["-Wl,-Bno-symbolic,-Bsymbolic-functions"][..],
                Symbolic::Functions,
            ),
        ] {
            assert_eq!(check(args, true).unwrap().symbolic, want, "{args:?}");
        }
        assert_eq!(
            check(&["-Wl,-Bsymbolic"], true)
                .unwrap()
                .flags(false, false),
            DF_SYMBOLIC
        );
    }

    #[test]
    fn unimplemented_semantic_options_are_rejected() {
        for bad in [
            &["-Wl,--version-script=v.map"][..],
            &["-Wl,--exclude-libs,ALL"],
            &["-Wl,-Map=x.map"],
            &["-Wl,--emit-relocs"],
            &["-pie"],
            &["-Wl,--dynamic-list=x"],
            &["-Wl,-z,max-page-size=0x200000"],
            &["-Wl,-z,nocopyreloc"],
            &["-Wl,--build-id=md5"],
            &["-Wl,-Ttext=0x1000"],
            &["-r"],
        ] {
            let e = check(bad, false).expect_err(&format!("{bad:?} must be rejected"));
            assert!(e.contains("not supported"), "{e}");
        }
    }

    #[test]
    fn layout_neutral_z_keywords_are_accepted() {
        let o = check(
            &[
                "-Wl,-z,max-page-size=4096",
                "-Wl,-z,separate-code",
                "-Wl,-z,text",
                "-Wl,-z,noexecstack",
            ],
            false,
        )
        .unwrap();
        assert!(o.z_text);
        assert_eq!(o.exec_stack, Some(false));
    }
}

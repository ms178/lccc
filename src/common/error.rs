//! Structured diagnostic infrastructure for the compiler.
//!
//! Provides a `DiagnosticEngine` that collects errors, warnings, and notes
//! with source locations and renders them in GCC-compatible format with
//! optional source snippet display and ANSI color output.
//!
//! # Warning control
//! The engine supports GCC-compatible warning flags:
//! - `-Werror`: promote all warnings to errors
//! - `-Werror=<name>`: promote a specific warning to an error
//! - `-Wno-error=<name>`: demote a specific warning back from error
//! - `-Wall`: enable standard warning set
//! - `-Wextra`: enable additional warnings
//! - `-W<name>`: enable a specific warning
//! - `-Wno-<name>`: disable a specific warning
//!
//! Flags are processed left-to-right, so `-Wall -Wno-unused-variable` enables
//! all warnings except unused-variable.
//!
//! # Color output
//! Controlled by `-fdiagnostics-color={auto,always,never}`:
//! - `auto` (default): colorize when stderr is a terminal
//! - `always`: always emit ANSI color codes
//! - `never`: plain text output
//!
//! Color scheme (matching GCC):
//! - Location (file:line:col): **bold white**
//! - `error:` label: **bold red**
//! - `warning:` label: **bold magenta**
//! - `note:` label: **bold cyan**
//! - Caret/underline (`^~~~`): **bold green**
//! - Fix-it hints: **bold green**
//!
//! # Output format
//! ```text
//! file.c:10:5: error: expected ';', got '}'
//!     int x = 42
//!             ^
//! ```

use crate::common::source::{SourceManager, Span};

/// Controls whether diagnostic output uses ANSI color escape codes.
///
/// Matches GCC's `-fdiagnostics-color` flag with the same three modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    /// Colorize output only when stderr is a terminal (default).
    #[default]
    Auto,
    /// Always emit ANSI color codes.
    Always,
    /// Never emit color codes; plain text output.
    Never,
}

impl ColorMode {
    /// Parse a `-fdiagnostics-color` flag value.
    /// Returns `None` for unrecognized values.
    pub fn from_flag(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(ColorMode::Auto),
            "always" => Some(ColorMode::Always),
            "never" => Some(ColorMode::Never),
            _ => None,
        }
    }

    /// Resolve whether colors should actually be used, considering
    /// the mode and whether stderr is a terminal.
    fn use_color(self) -> bool {
        match self {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => {
                use std::io::IsTerminal;
                std::io::stderr().is_terminal()
            }
        }
    }
}

/// Categories of warnings, matching GCC's -W<name> flag names.
///
/// Each variant corresponds to a `-W<name>` flag. For example,
/// `WarningKind::ImplicitFunctionDeclaration` maps to
/// `-Wimplicit-function-declaration`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningKind {
    /// An undeclared identifier was used.
    /// NOTE: As of the fix in sema.rs, undeclared variables are now hard errors
    /// (not warnings). This variant is kept for CLI flag parsing compatibility
    /// (-Wno-undeclared, -Werror=undeclared) but is no longer emitted.
    Undeclared,
    /// A function was called without a prior declaration (C89 implicit int).
    /// GCC flag: -Wimplicit-function-declaration
    ImplicitFunctionDeclaration,
    /// A `#warning` preprocessor directive was encountered.
    /// GCC flag: -Wcpp
    Cpp,
    /// An evaluated #if/#elif operand names an undefined identifier.
    Undef,
    /// An externally visible definition lacks a previous file-scope prototype.
    MissingPrototypes,
    /// An externally visible definition lacks a previous file-scope declaration.
    MissingDeclarations,
    /// Control reaches end of non-void function, or non-void function does
    /// not return a value in all control paths.
    /// GCC flag: -Wreturn-type
    ReturnType,
    IncompatiblePointerTypes,
    /// An attribute that does not apply to its declaration is ignored
    /// (enabled by default, as in GCC). GCC flag: -Wattributes
    Attributes,
    /// A block-scope object is never referenced.
    /// GCC flag: -Wunused-variable (enabled by -Wall).
    UnusedVariable,
    /// Integer operands of a comparison have different signedness.
    /// GCC flag: -Wsign-compare (enabled by -Wextra for C).
    SignCompare,
    /// An integer division or remainder has a constant zero divisor.
    /// GCC flag: -Wdiv-by-zero (on by default in C, as GCC does).
    DivByZero,
    /// A function declaration without a parameter-type list: `int f();`.
    /// GCC flag: -Wstrict-prototypes (off by default; C23 makes `()`
    /// a prototype, so GCC stays silent there and so must we).
    StrictPrototypes,
    /// A printf-family format string does not match its variadic arguments.
    /// GCC flag: -Wformat (enabled by -Wall).
    Format,
    /// A printf-family call uses a non-literal format with no format arguments.
    /// GCC flag: -Wformat-security (must be combined with -Wformat).
    FormatSecurity,
    /// A declaration omitted its type specifier, defaulting to `int`.
    /// GCC flag: -Wimplicit-int
    ImplicitInt,
    /// `return;` in a function returning non-void (or a missing return
    /// value at the end of a non-void function's `return expr;`).
    /// GCC flag: -Wreturn-mismatch
    ReturnMismatch,
    /// An integer initializer/assignment/argument was used where a pointer
    /// is expected without a null-pointer constant or an explicit cast.
    /// GCC flag: -Wint-conversion
    IntConversion,
    /// A pointer was cast to an integer type whose object size differs from
    /// the pointer's (C11 6.5.4p2 permits the cast, GCC 16.2 warns by
    /// default in EVERY dialect and never as a dialect default error —
    /// measured; `_Bool` and enumeration targets never warn).
    /// GCC flag: -Wpointer-to-int-cast
    PointerToIntCast,
    /// An implicit pointer conversion drops a qualifier of the pointed-to
    /// type (`char *p = (const char *)q;`, C23 6.5.16.1p1).
    /// GCC flag: -Wdiscarded-qualifiers (enabled by default).
    DiscardedQualifiers,
    /// The same for a pointer to an array, whose element type carries the
    /// qualifier. GCC flag: -Wdiscarded-array-qualifiers (enabled by default).
    DiscardedArrayQualifiers,
    /// An integer was cast to a pointer type whose object size differs from
    /// the integer's (measured against GCC 16.2: default-on in every
    /// dialect, never a dialect default error; `_Bool` and enumeration
    /// sources never warn, and integer constant expressions are folded
    /// silently — `(void *)42` is silent while `(void *)i32` warns, and
    /// the exemption is the constant-ness: `const int cj = 5; (void *)cj`
    /// warns).
    /// GCC flag: -Wint-to-pointer-cast
    IntToPointerCast,
    /// `==`/`!=`/ordering between pointers to distinct pointee types
    /// (measured against GCC 16.2: default-on in every dialect, never a
    /// dialect default error; `void *` absorbs the other side at the
    /// OUTERMOST level only; the test is C23 6.2.7 compatibility of the
    /// unqualified pointees (`int **` vs `int *const *` is silent, `int **`
    /// vs `const int **` warns), so `int *` vs
    /// `unsigned *` or `char *` vs `signed char *` warn, while an enum
    /// against its own underlying type, `int (*)[3]` against `int (*)[]`
    /// and a prototype against an unprototyped function type are silent).
    /// GCC flag: -Wcompare-distinct-pointer-types
    CompareDistinctPointerTypes,
    /// ISO C language-extension constraints (the `-pedantic` family).
    /// Measured on GCC 16.2: `-pedantic`/`-Wpedantic` warn and
    /// `-pedantic-errors` errors with the PLAIN `[-Wpedantic]` label,
    /// while `-Werror=pedantic` prints `[-Werror=pedantic]`.
    /// GCC flag: -Wpedantic / -pedantic / -pedantic-errors
    Pedantic,
    /// A block-scope array bound is not an integer constant expression
    /// (measured GCC 16.2: default-off, NOT in -Wall/-Wextra, message
    /// `ISO C90 forbids variable length array '<name>'`).
    /// GCC flag: -Wvla
    Vla,
    /// `return` of the address of an automatic object, a parameter or a
    /// label (measured against GCC 16.2: default-on in every dialect, never
    /// a dialect default error, front-end walk identical at -O0..-O3; see
    /// `sema::analysis::return_local_addr`).
    /// GCC flag: -Wreturn-local-addr
    ReturnLocalAddr,
    /// `multi-character character constant`.
    ///
    /// GCC 16.2 enables it by default and it is a leaf option: `-Wno-multichar`
    /// silences it and `-Werror=multichar` promotes it, while a literal of five
    /// or more characters is reported as an unconditional warning instead
    /// (`multi-character literal with N characters exceeds 'int' size of 4
    /// bytes`, which `-Wno-multichar` does NOT silence — both measured).
    Multichar,
}

/// Supported warning groups. `-Wall` and `-Wextra` are broad groups;
/// `-Wimplicit` and `-Wunused` are more specific subgroups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningGroup {
    All,
    Extra,
    Implicit,
    Unused,
    // Reserved for warnings implemented by the future GP-10 pedantic work.
    Pedantic,
}

impl WarningGroup {
    fn from_flag_name(name: &str) -> Option<Self> {
        match name {
            "all" => Some(Self::All),
            "extra" => Some(Self::Extra),
            "implicit" => Some(Self::Implicit),
            "unused" => Some(Self::Unused),
            // Do not claim -Wpedantic is implemented merely because its
            // membership slot exists: no pedantic diagnostics are implemented.
            _ => None,
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::All | Self::Extra | Self::Pedantic => 1,
            Self::Implicit | Self::Unused => 2,
        }
    }
}

/// Single source of truth for each modeled warning's spelling and policy.
#[derive(Debug, Clone, Copy)]
pub struct WarningDescriptor {
    pub kind: WarningKind,
    pub name: &'static str,
    pub default_on: bool,
    pub groups: &'static [WarningGroup],
    pub c99_default_error: bool,
    pub description: &'static str,
}

macro_rules! warning {
    ($kind:ident, $name:literal, $default:expr, [$($group:ident),* $(,)?], $c99:expr, $description:literal) => {
        WarningDescriptor {
            kind: WarningKind::$kind,
            name: $name,
            default_on: $default,
            groups: &[$(WarningGroup::$group),*],
            c99_default_error: $c99,
            description: $description,
        }
    };
}

/// GCC 16.2 C warning membership, measured with positive and negative flags.
/// `Undeclared` is a compatibility-only switch; the corresponding condition is
/// a hard error, not an emitted warning. Pedantic has no implemented members.
pub const WARNING_REGISTRY: &[WarningDescriptor] = &[
    warning!(
        Undeclared,
        "undeclared",
        true,
        [],
        false,
        "compatibility-only undeclared name switch"
    ),
    warning!(
        ImplicitFunctionDeclaration,
        "implicit-function-declaration",
        true,
        [All, Implicit],
        true,
        "undeclared function call"
    ),
    warning!(Cpp, "cpp", true, [], false, "preprocessor #warning"),
    warning!(
        Undef,
        "undef",
        false,
        [],
        false,
        "undefined preprocessor conditional identifier"
    ),
    warning!(
        MissingPrototypes,
        "missing-prototypes",
        false,
        [],
        false,
        "external function definition without a prior prototype"
    ),
    warning!(
        MissingDeclarations,
        "missing-declarations",
        false,
        [],
        false,
        "external function definition without a prior declaration"
    ),
    warning!(
        ReturnType,
        "return-type",
        false,
        [All],
        false,
        "control reaches end of a non-void function"
    ),
    warning!(
        IncompatiblePointerTypes,
        "incompatible-pointer-types",
        true,
        [],
        true,
        "incompatible pointer conversion"
    ),
    warning!(
        Attributes,
        "attributes",
        true,
        [],
        false,
        "ignored or inapplicable attribute"
    ),
    warning!(
        UnusedVariable,
        "unused-variable",
        false,
        [All, Unused],
        false,
        "unused block-scope variable"
    ),
    warning!(
        SignCompare,
        "sign-compare",
        false,
        [Extra],
        false,
        "mixed-sign integer comparison"
    ),
    warning!(
        DivByZero,
        "div-by-zero",
        true,
        [],
        false,
        "integer division or remainder by a constant zero"
    ),
    warning!(
        StrictPrototypes,
        "strict-prototypes",
        false,
        [],
        false,
        "function declaration without a parameter-type list"
    ),
    warning!(
        Format,
        "format",
        false,
        [All],
        false,
        "invalid printf-family argument"
    ),
    warning!(
        FormatSecurity,
        "format-security",
        false,
        [],
        false,
        "nonliteral format without arguments"
    ),
    warning!(
        ImplicitInt,
        "implicit-int",
        true,
        [All, Implicit],
        true,
        "implicit int declaration"
    ),
    warning!(
        ReturnMismatch,
        "return-mismatch",
        true,
        [],
        true,
        "return value incompatible with function type"
    ),
    warning!(
        IntConversion,
        "int-conversion",
        true,
        [],
        true,
        "integer and pointer conversion without a cast"
    ),
    warning!(
        PointerToIntCast,
        "pointer-to-int-cast",
        true,
        [],
        false,
        "pointer-to-integer cast of a different width"
    ),
    warning!(
        DiscardedQualifiers,
        "discarded-qualifiers",
        true,
        [],
        false,
        "pointer conversion dropping a pointee qualifier"
    ),
    warning!(
        DiscardedArrayQualifiers,
        "discarded-array-qualifiers",
        true,
        [],
        false,
        "pointer-to-array conversion dropping an element qualifier"
    ),
    warning!(
        IntToPointerCast,
        "int-to-pointer-cast",
        true,
        [],
        false,
        "integer-to-pointer cast of a different width"
    ),
    warning!(
        CompareDistinctPointerTypes,
        "compare-distinct-pointer-types",
        true,
        [],
        false,
        "comparison of pointers with distinct pointee types"
    ),
    warning!(
        Pedantic,
        "pedantic",
        false,
        [],
        false,
        "ISO C language-extension constraints (statement expressions, ...)"
    ),
    warning!(
        Vla,
        "vla",
        false,
        [],
        false,
        "variable length array at block scope"
    ),
    warning!(
        ReturnLocalAddr,
        "return-local-addr",
        true,
        [],
        false,
        "return of the address of a local variable, parameter or label"
    ),
    warning!(
        Multichar,
        "multichar",
        true,
        [],
        false,
        "multi-character character constant"
    ),
];

impl WarningKind {
    fn descriptor(self) -> &'static WarningDescriptor {
        WARNING_REGISTRY
            .iter()
            .find(|entry| entry.kind == self)
            .expect("every WarningKind must have a registry row")
    }

    /// The GCC-compatible flag name (without the -W prefix).
    pub fn flag_name(self) -> &'static str {
        self.descriptor().name
    }

    /// How GCC labels this warning inside `[-W…]` / `[-Werror=…]`
    /// diagnostics. `-Wformat` is the one leveled option and GCC prints it
    /// with the trailing `=` (`[-Wformat=]`, `[-Werror=format=]`,
    /// measured on gcc-16.2); every other kind's label equals its flag
    /// name.
    pub fn flag_suffix(self) -> &'static str {
        match self {
            WarningKind::Format => "format=",
            other => other.flag_name(),
        }
    }

    /// Parse a *leaf* warning name. `implicit` is a group, not an alias for
    /// implicit-function-declaration: it also enables implicit-int.
    pub fn from_flag_name(name: &str) -> Option<Self> {
        WARNING_REGISTRY
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.kind)
    }
}

/// Warning selection and severity. More specific enable/disable options beat
/// less specific groups regardless of ordering; within a specificity level the
/// last option wins (GCC 16.2). Severity demotion has its own GCC rules.
#[derive(Debug, Clone)]
pub struct WarningConfig {
    enabled: crate::common::fx_hash::FxHashSet<WarningKind>,
    /// Highest CLI selection specificity seen for a kind; defaults have no
    /// entry, broad groups have priority 1, subgroups 2, leaf switches 3.
    enable_priority: crate::common::fx_hash::FxHashMap<WarningKind, u8>,
    errors: crate::common::fx_hash::FxHashSet<WarningKind>,
    not_errors: crate::common::fx_hash::FxHashSet<WarningKind>,
    default_errors: crate::common::fx_hash::FxHashSet<WarningKind>,
    explicit_enable: crate::common::fx_hash::FxHashSet<WarningKind>,
    /// `-Wsystem-headers`: report warnings located in system headers (off by
    /// default, as in GCC).
    system_headers: bool,
    /// Pre-C99 implicit return types belong to -Wreturn-type under -Wall
    /// or an explicit -Wreturn-type request, not to the C99 constraint class.
    legacy_implicit_return_type: bool,
    permissive: Option<bool>,
    bare_return_enabled: bool,
    pub werror_all: bool,
    inhibit_all: bool,
}

impl WarningConfig {
    pub fn new() -> Self {
        let enabled = WARNING_REGISTRY
            .iter()
            .filter(|entry| entry.default_on)
            .map(|entry| entry.kind)
            .collect();
        Self {
            enabled,
            enable_priority: crate::common::fx_hash::FxHashMap::default(),
            errors: crate::common::fx_hash::FxHashSet::default(),
            not_errors: crate::common::fx_hash::FxHashSet::default(),
            default_errors: crate::common::fx_hash::FxHashSet::default(),
            explicit_enable: crate::common::fx_hash::FxHashSet::default(),
            system_headers: false,
            legacy_implicit_return_type: false,
            permissive: None,
            bare_return_enabled: true,
            werror_all: false,
            inhibit_all: false,
        }
    }

    pub fn set_inhibit_all(&mut self) {
        self.inhibit_all = true;
    }

    pub fn set_permissive(&mut self, enabled: bool) {
        self.permissive = Some(enabled);
    }

    fn select(&mut self, kind: WarningKind, enabled: bool, priority: u8) {
        if self
            .enable_priority
            .get(&kind)
            .is_some_and(|&old| old > priority)
        {
            return;
        }
        self.enable_priority.insert(kind, priority);
        if enabled {
            self.enabled.insert(kind);
        } else {
            self.enabled.remove(&kind);
        }
    }

    fn select_group(&mut self, group: WarningGroup, enabled: bool) {
        for entry in WARNING_REGISTRY {
            if entry.groups.contains(&group) {
                self.select(entry.kind, enabled, group.priority());
            }
        }
    }

    pub fn enable(&mut self, kind: WarningKind) {
        self.select(kind, true, 3);
        self.explicit_enable.insert(kind);
    }

    pub fn disable(&mut self, kind: WarningKind) {
        self.select(kind, false, 3);
    }

    pub fn enable_wall(&mut self) {
        self.select_group(WarningGroup::All, true);
    }

    pub fn enable_wextra(&mut self) {
        self.select_group(WarningGroup::Extra, true);
    }

    pub fn set_werror(&mut self, kind: WarningKind) {
        self.errors.insert(kind);
        self.not_errors.remove(&kind);
        self.enable(kind);
    }

    /// `-pedantic-errors`: the diagnostic becomes an error with GCC's
    /// PLAIN `[-Wpedantic]` label (the same default-severity promotion
    /// class as the dialect default errors), and a later spelling wins
    /// over an earlier `-Wno-error=` (measured).
    pub fn set_dialect_error(&mut self, kind: WarningKind) {
        self.enable(kind);
        self.not_errors.remove(&kind);
        self.default_errors.insert(kind);
    }

    pub fn clear_werror(&mut self, kind: WarningKind) {
        self.errors.remove(&kind);
        self.default_errors.remove(&kind);
        self.not_errors.insert(kind);
    }

    /// GCC 16.2 expands `-Werror=<group>` to error classifications for each
    /// leaf as well as enabling the group. Unlike enablement specificity,
    /// these explicit classifications replace earlier per-leaf demotions.
    fn set_group_werror(&mut self, group: WarningGroup) {
        for entry in WARNING_REGISTRY {
            if entry.groups.contains(&group) {
                self.select(entry.kind, true, group.priority());
                self.errors.insert(entry.kind);
                self.not_errors.remove(&entry.kind);
            }
        }
    }

    pub fn apply_gcc162_c99_default_errors(&mut self) {
        for entry in WARNING_REGISTRY {
            if entry.c99_default_error
                && self.permissive != Some(true)
                && !self.not_errors.contains(&entry.kind)
            {
                self.default_errors.insert(entry.kind);
            }
        }
    }

    /// In pre-C99 modes, implicit diagnostics are not on by default but can
    /// still be selected by -Wall, -Wimplicit or their leaf switches. GCC's
    /// bare-return subcase is unusual: mentioning -Wreturn-type *or its
    /// negation*, including -Wno-all, enables -Wreturn-mismatch in gnu90.
    pub fn suppress_gcc162_pre_c99_defaults(&mut self) {
        self.legacy_implicit_return_type = self.enabled.contains(&WarningKind::ReturnType);
        self.bare_return_enabled = self.enable_priority.contains_key(&WarningKind::ReturnType);
        for kind in [
            WarningKind::ImplicitInt,
            WarningKind::ImplicitFunctionDeclaration,
        ] {
            if !self.enable_priority.contains_key(&kind) {
                self.enabled.remove(&kind);
            }
        }
        if self.permissive == Some(false) {
            self.apply_gcc162_c99_default_errors();
        }
    }

    pub fn is_enabled(&self, kind: WarningKind) -> bool {
        self.enabled.contains(&kind)
            && (!self.inhibit_all || (self.default_errors.contains(&kind) && self.is_error(kind)))
    }

    pub fn is_explicitly_requested(&self, kind: WarningKind) -> bool {
        self.explicit_enable.contains(&kind)
    }

    /// `-Wsystem-headers` / `-Wno-system-headers`.
    pub fn set_system_headers(&mut self, enabled: bool) {
        self.system_headers = enabled;
    }

    /// Check if a warning of the given kind should be promoted to an error.
    pub fn is_error(&self, kind: WarningKind) -> bool {
        ((self.errors.contains(&kind) || self.default_errors.contains(&kind))
            && !self.not_errors.contains(&kind))
            || (self.werror_all && !self.not_errors.contains(&kind))
    }

    /// Whether a positive name denotes a supported warning or group. Shared
    /// with `__has_warning` so preprocessor capability claims use this table.
    pub fn recognizes_name(name: &str) -> bool {
        name == "error"
            || WarningGroup::from_flag_name(name).is_some()
            || WarningKind::from_flag_name(name).is_some()
    }

    /// Process one option after its `-W` prefix, returning false for unknown
    /// names so the driver can apply its option-contract policy.
    pub fn process_flag(&mut self, flag: &str) -> bool {
        match flag {
            "error" => {
                self.werror_all = true;
                true
            }
            "no-error" => {
                self.werror_all = false;
                true
            }
            "all" => {
                self.enable_wall();
                true
            }
            "extra" => {
                self.enable_wextra();
                true
            }
            // `-Wsystem-headers`: report warnings located in system headers.
            // Not a warning category of its own, so it is not a WarningKind.
            "system-headers" => {
                self.system_headers = true;
                true
            }
            "no-system-headers" => {
                self.system_headers = false;
                true
            }
            _ if flag.starts_with("error=") => {
                let name = &flag[6..];
                if let Some(kind) = WarningKind::from_flag_name(name) {
                    self.set_werror(kind);
                    true
                } else if let Some(group) = WarningGroup::from_flag_name(name) {
                    self.set_group_werror(group);
                    true
                } else if let Some((base, level)) = name.split_once('=') {
                    // `-Werror=format=0` (measured): level 0 disables the
                    // warning entirely, promotion or not.
                    self.apply_level(base, level, true)
                } else {
                    false
                }
            }
            _ if flag.starts_with("no-error=") => {
                let name = &flag[9..];
                if let Some(kind) = WarningKind::from_flag_name(name) {
                    self.clear_werror(kind);
                    true
                } else {
                    // GCC 16.2 accepts group demotions but does not expand
                    // them to the leaf warning classifications we emit.
                    WarningGroup::from_flag_name(name).is_some()
                }
            }
            _ if flag.starts_with("no-") => {
                let name = &flag[3..];
                if let Some(kind) = WarningKind::from_flag_name(name) {
                    self.disable(kind);
                    true
                } else if let Some(group) = WarningGroup::from_flag_name(name) {
                    self.select_group(group, false);
                    true
                } else {
                    false
                }
            }
            _ => {
                if let Some(kind) = WarningKind::from_flag_name(flag) {
                    self.enable(kind);
                    true
                } else if let Some(group) = WarningGroup::from_flag_name(flag) {
                    self.select_group(group, true);
                    true
                } else if let Some((base, level)) = flag.split_once('=') {
                    // `-W<name>=<level>`: only leveled warnings accept
                    // levels. Measured on gcc-16.2: `-Wformat=0|1|2` are
                    // recognized (2 = the -Wformat family, of which LCCC
                    // implements -Wformat-security); every other
                    // `-W<name>=N` is a driver error there, and false lets
                    // the tolerated-option diagnostic stand in.
                    if !level.is_empty() && level.bytes().all(|b| b.is_ascii_digit()) {
                        self.apply_level(base, level, false)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
    }

    /// Apply a numeric level to a leveled warning: GCC 16.2 accepts levels
    /// only for `-Wformat` (measured — `=0` disables, `=1` enables, `=2`
    /// enables the -Wformat family, of which LCCC implements the core
    /// warning and -Wformat-security). `promote` marks the
    /// `-Werror=<name>=<level>` spelling, where `>=1` also promotes.
    /// Returns false for anything GCC would not recognize here.
    fn apply_level(&mut self, base: &str, level: &str, promote: bool) -> bool {
        let Some(kind) = WarningKind::from_flag_name(base) else {
            return false;
        };
        if kind != WarningKind::Format || !matches!(level, "0" | "1" | "2") {
            return false;
        }
        match level {
            // Measured: `-Werror -Wformat=0` and `-Werror=format=0` are
            // both completely silent, so =0 is a full disable.
            "0" => self.disable(kind),
            _ => {
                self.enable(kind);
                if level == "2" {
                    self.enable(WarningKind::FormatSecurity);
                }
                if promote {
                    self.set_werror(kind);
                }
            }
        }
        true
    }
}

impl Default for WarningConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Severity level for a diagnostic message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A fatal error that prevents compilation from continuing.
    Error,
    /// A warning that does not prevent compilation.
    Warning,
    /// A supplementary note attached to a previous error or warning.
    Note,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Note => write!(f, "note"),
        }
    }
}

/// A single diagnostic message with severity, location, and message text.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    /// Optional warning category for filtering and -Werror promotion.
    pub warning_kind: Option<WarningKind>,
    /// Optional follow-up notes providing additional context.
    pub notes: Vec<Diagnostic>,
    /// Optional fix-it hint: a short suggestion for how to fix the problem.
    /// Rendered below the snippet as "fix-it hint: insert ';'" etc.
    pub fix_hint: Option<String>,
    /// Explicit source location string for diagnostics without a span.
    /// Used for preprocessor-phase diagnostics (e.g., `#error`, `#warning`,
    /// missing `#include`) where the source manager is not yet available.
    /// Format: "file:line:" (rendered as the location prefix in output).
    pub explicit_location: Option<String>,
    /// Disambiguates two diagnostics that are otherwise identical: same
    /// severity, message and location. The engine drops a repeated warning
    /// with the same key (the same issue reported twice by two phases), but
    /// GCC reports one verdict per attribute list, so a caller that emits
    /// several verdicts at one location sets this to a value that differs per
    /// list (the list's own span). Zero means "no disambiguation".
    pub dedup_salt: u64,
}

impl Diagnostic {
    /// Create a new error diagnostic.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span: None,
            warning_kind: None,
            notes: Vec::new(),
            fix_hint: None,
            explicit_location: None,
            dedup_salt: 0,
        }
    }

    /// Create a new warning diagnostic with a category.
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            warning_kind: None,
            notes: Vec::new(),
            fix_hint: None,
            explicit_location: None,
            dedup_salt: 0,
        }
    }

    /// Create a new warning diagnostic with a specific kind for filtering.
    pub fn warning_with_kind(message: impl Into<String>, kind: WarningKind) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            warning_kind: Some(kind),
            notes: Vec::new(),
            fix_hint: None,
            explicit_location: None,
            dedup_salt: 0,
        }
    }

    /// Create a new note diagnostic.
    pub fn note(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Note,
            message: message.into(),
            span: None,
            warning_kind: None,
            notes: Vec::new(),
            fix_hint: None,
            explicit_location: None,
            dedup_salt: 0,
        }
    }

    /// Attach a source span to this diagnostic.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Add a follow-up note to this diagnostic.
    /// Used for "note: expression has type '...'" and similar context.
    pub fn with_note(mut self, note: Diagnostic) -> Self {
        self.notes.push(note);
        self
    }

    /// Add a fix-it hint suggestion.
    /// Rendered below the source snippet, e.g., "fix-it hint: insert ';' after expression".
    pub fn with_fix_hint(mut self, hint: impl Into<String>) -> Self {
        self.fix_hint = Some(hint.into());
        self
    }

    /// Set an explicit source location string for diagnostics without a span.
    /// Used for preprocessor-phase diagnostics where the source manager is
    /// not yet available. The location string is rendered as the prefix
    /// (e.g., "file.c:10:1:") before the severity label.
    pub fn with_location(mut self, file: &str, line: usize, col: usize) -> Self {
        self.explicit_location = Some(format!("{}:{}:{}:", file, line, col));
        self
    }
}

/// Collects and renders compiler diagnostics with source context.
///
/// The engine is designed to be threaded through compilation phases. Each
/// phase calls `emit()` to report errors and warnings. After each phase,
/// the driver checks `has_errors()` to decide whether to continue.
///
/// Diagnostics are printed to stderr immediately on emit, matching GCC
/// behavior where errors appear as soon as they are discovered.
///
/// Warning filtering and -Werror promotion are controlled by the
/// `WarningConfig` set on the engine.
pub struct DiagnosticEngine {
    error_count: usize,
    warning_count: usize,
    /// Warning configuration: controls which warnings are enabled and
    /// which are promoted to errors.
    warning_config: WarningConfig,
    /// Reference to the source manager for span resolution and snippet display.
    /// Set after the preprocessing/lexing phase creates the SourceManager.
    source_manager: Option<std::rc::Rc<SourceManager>>,
    /// Whether to use ANSI color codes in diagnostic output.
    /// Resolved once at creation from the ColorMode setting.
    use_color: bool,
    /// The file for which we last emitted an "In file included from" chain.
    /// Used to avoid repeating the same chain for consecutive errors in the
    /// same included file (matching GCC behavior).
    last_include_trace_file: Option<String>,
    /// Set once the rendered-error limit note has been emitted (see
    /// `render_error`); the count keeps growing afterwards, rendering stops.
    error_limit_noted: bool,
    /// Same latch for the warning cap (see `render_warning`).
    warning_limit_noted: bool,
    /// Warnings already reported, keyed by (file, span, message hash).
    ///
    /// GCC reports one diagnostic per offending token, however many internal
    /// paths notice the mistake. Two independent analyses of the same
    /// declaration (the parser's placement check and the declaration-level
    /// check) reach the same verdict on the same attribute; without this the
    /// user sees the line twice. Warnings are therefore idempotent per
    /// (span, message).
    emitted_warnings: crate::common::fx_hash::FxHashSet<(u32, u32, u32, u64, u64)>,
}

impl DiagnosticEngine {
    /// Clang-style cap on rendered errors per engine. One malformed input can
    /// trigger tens of thousands of cascading recovery errors; rendering every
    /// one of them (each with source snippets) blows up output and memory and
    /// buries the root cause. Errors keep COUNTING past the cap, so the
    /// translation unit is rejected exactly as before — only the printing
    /// stops (with a single "too many errors" note).
    pub const MAX_RENDERED_ERRORS: usize = 200;

    /// Cap on RENDERED warnings per engine, mirroring the error cap.
    /// Clang's equivalent is 20 (DiagnosticOptions::WarningLimit); lccc
    /// deliberately chooses 200:
    ///   * The corpus pins exact warning output for its translation
    ///     units — Clang's own limit is what forces it to truncate, and
    ///     matching 20 would silently drop pinned warnings on larger
    ///     real-world inputs before the root cause is ever visible.
    ///   * Warnings never abort the TU, so the cost of a few extra
    ///     rendered lines is low, while the cap still bounds genuinely
    ///     pathological floods (tens of thousands of cascading warnings
    ///     from one hostile header) that would otherwise explode stderr
    ///     and memory.
    ///   * 200 keeps the warning budget on the same scale as the error
    ///     budget above, which the rest of the diagnostic pipeline was
    ///     sized against.
    /// Warnings keep COUNTING past the cap (warning_count is unaffected),
    /// so -Werror semantics and the "N warning(s) generated" summary
    /// stay exact — only the printing stops, with a single note.
    pub const MAX_RENDERED_WARNINGS: usize = 200;
}

impl DiagnosticEngine {
    /// Create a new diagnostic engine with no source manager and default warning config.
    /// Color mode defaults to `Auto` (colorize when stderr is a terminal).
    pub fn new() -> Self {
        Self {
            error_count: 0,
            warning_count: 0,
            warning_config: WarningConfig::new(),
            source_manager: None,
            use_color: ColorMode::Auto.use_color(),
            last_include_trace_file: None,
            error_limit_noted: false,
            warning_limit_noted: false,
            emitted_warnings: crate::common::fx_hash::FxHashSet::default(),
        }
    }

    fn message_hash(message: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        message.hash(&mut hasher);
        hasher.finish()
    }

    /// Remember a *salted* warning occurrence only when it is actually going
    /// to be rendered (or promoted to an error).  A suppressed warning is not
    /// an emission: if `-Wsystem-headers`, `-Wno-<name>`, or `-w` hid the first
    /// attempt, a later visible attempt must not be deduplicated against it.
    /// Unsalted warnings are never deduplicated: GCC reports one diagnostic
    /// per offending token, and two independent problems at the same span are
    /// two diagnostics.
    fn record_warning_emission(&mut self, diag: &Diagnostic) -> bool {
        if diag.severity != Severity::Warning || diag.dedup_salt == 0 {
            return true;
        }
        let Some(span) = diag.span else {
            return true;
        };
        self.emitted_warnings.insert((
            span.file_id,
            span.start,
            span.end,
            Self::message_hash(&diag.message),
            diag.dedup_salt,
        ))
    }

    /// Render an error-severity diagnostic subject to `MAX_RENDERED_ERRORS`.
    fn render_error(&mut self, diag: &Diagnostic) {
        if self.error_count < Self::MAX_RENDERED_ERRORS {
            self.render_diagnostic(diag);
        } else if !self.error_limit_noted {
            self.error_limit_noted = true;
            let note = Diagnostic::error(format!(
                "too many errors emitted; stopping diagnostic emission (limit {})",
                Self::MAX_RENDERED_ERRORS
            ));
            self.render_diagnostic(&note);
        }
        self.error_count += 1;
    }

    /// Render a warning-severity diagnostic subject to
    /// `MAX_RENDERED_WARNINGS` (policy rationale on the constant). The
    /// count keeps growing past the cap.
    fn render_warning(&mut self, diag: &Diagnostic) {
        if self.warning_count < Self::MAX_RENDERED_WARNINGS {
            self.render_diagnostic(diag);
        } else if !self.warning_limit_noted {
            self.warning_limit_noted = true;
            let note = Diagnostic::warning(format!(
                "too many warnings emitted; stopping warning emission (limit {})",
                Self::MAX_RENDERED_WARNINGS
            ));
            self.render_diagnostic(&note);
        }
        self.warning_count += 1;
    }

    /// `-Wsystem-headers` / `-Wno-system-headers`.
    pub fn set_system_headers(&mut self, enabled: bool) {
        self.warning_config.set_system_headers(enabled);
    }
    /// Set the warning configuration (parsed from CLI flags).
    pub fn set_warning_config(&mut self, config: WarningConfig) {
        self.warning_config = config;
    }

    /// Check whether a warning is enabled before doing optional, potentially
    /// expensive analysis for that warning category.
    pub fn warning_enabled(&self, kind: WarningKind) -> bool {
        self.warning_config.is_enabled(kind)
    }

    /// Whether the user asked for this warning class with a positive
    /// `-W<name>` or `-Werror=<name>` flag (not merely its default state).
    /// Used by the attribute inventory: GCC-known-but-unimplemented
    /// attributes diagnose only on an explicit `-Wattributes` request.
    pub fn warning_explicitly_requested(&self, kind: WarningKind) -> bool {
        self.warning_config.is_explicitly_requested(kind)
    }

    /// Set the source manager for span resolution and snippet display.
    pub fn set_source_manager(&mut self, sm: SourceManager) {
        self.source_manager = Some(std::rc::Rc::new(sm));
    }

    /// Share the attached source manager with another owner WITHOUT detaching
    /// it from this engine.
    ///
    /// Why: the driver used to `take_source_manager()` right after sema so it
    /// could hand the manager to codegen for debug-info emission -- which left
    /// the lowering phase with no source manager, so every lowering diagnostic
    /// printed as a bare `error: ...` without `file:line:col` (AUD-ICE-1).
    /// Sharing by handle keeps the locations in both phases.
    pub fn share_source_manager(&self) -> Option<std::rc::Rc<SourceManager>> {
        self.source_manager.clone()
    }

    /// Take ownership of the source manager (for passing to codegen for debug info).
    pub fn take_source_manager(&mut self) -> Option<std::rc::Rc<SourceManager>> {
        self.source_manager.take()
    }

    /// Set the color mode for diagnostic output.
    /// Resolves the mode immediately (e.g., checking isatty for Auto).
    pub fn set_color_mode(&mut self, mode: ColorMode) {
        self.use_color = mode.use_color();
    }

    /// Emit a diagnostic: apply warning filtering/promotion, print to stderr,
    /// and update counts.
    ///
    /// For warnings with a `WarningKind`:
    /// - If the warning is disabled in the config, it is silently suppressed.
    /// - If -Werror or -Werror=<name> applies, it is promoted to an error.
    /// - The GCC-style `[-W<name>]` suffix is appended to the message.
    pub fn emit(&mut self, diag: &Diagnostic) {
        // GCC suppresses diagnostics that come from a system header unless
        // -Wsystem-headers asks for them; the suppression applies to warnings
        // only. Without this, a project that merely includes <pthread.h>
        // would see our honest `'returns_twice' attribute ignored` note on a
        // declaration it does not own. It runs before the per-occurrence
        // dedupe below: a suppressed warning must not consume the occurrence
        // slot of the same diagnostic emitted later under `-Wsystem-headers`.
        if diag.severity == Severity::Warning
            && !self.warning_config.system_headers
            && diag
                .span
                .is_some_and(|span| self.span_is_in_system_header(span))
        {
            return;
        }

        match diag.severity {
            Severity::Warning => {
                if let Some(mut kind) = diag.warning_kind {
                    if kind == WarningKind::ImplicitInt
                        && self.warning_config.legacy_implicit_return_type
                        && self.warning_config.is_enabled(WarningKind::ReturnType)
                    {
                        kind = WarningKind::ReturnType;
                    }
                    // Check if this warning category is enabled
                    if !self.warning_config.is_enabled(kind) {
                        return; // suppressed by -Wno-<name>
                    }
                    if !self.record_warning_emission(diag) {
                        return;
                    }

                    // Check if this warning should be promoted to an error
                    if self.warning_config.is_error(kind) {
                        // Promote: render as error with [-Werror=<name>] suffix
                        // GCC 16.2 prints the plain `[-W<name>]` suffix when
                        // the error comes from the dialect's default severity
                        // but `[-Werror=<name>]` for an explicit promotion
                        // (measured against gcc-16.2.0).
                        let suffix = if self.warning_config.default_errors.contains(&kind) {
                            // Default-severity error (dialect promotion):
                            // GCC prints the plain flag label (measured).
                            format!("[-W{}]", kind.flag_suffix())
                        } else {
                            // Explicit -Werror=<name> or global -Werror
                            // promoting a live warning: GCC prints the
                            // -Werror= label (measured).
                            format!("[-Werror={}]", kind.flag_suffix())
                        };
                        let promoted = Diagnostic {
                            severity: Severity::Error,
                            message: format!("{} {}", diag.message, suffix),
                            span: diag.span,
                            warning_kind: diag.warning_kind,
                            notes: diag.notes.clone(),
                            fix_hint: diag.fix_hint.clone(),
                            explicit_location: diag.explicit_location.clone(),
                            dedup_salt: diag.dedup_salt,
                        };
                        self.render_error(&promoted);
                        return;
                    }

                    // Render as warning with [-W<name>] suffix
                    let annotated = Diagnostic {
                        severity: Severity::Warning,
                        message: format!("{} [-W{}]", diag.message, kind.flag_suffix()),
                        span: diag.span,
                        warning_kind: diag.warning_kind,
                        notes: diag.notes.clone(),
                        fix_hint: diag.fix_hint.clone(),
                        explicit_location: diag.explicit_location.clone(),
                        dedup_salt: diag.dedup_salt,
                    };
                    self.render_warning(&annotated);
                } else {
                    // Warning without a kind: emit unless `-w` inhibits all
                    // warnings (GCC 16.2: `-w` silences unclassified user-code
                    // warnings too), subject to global -Werror otherwise.
                    if self.warning_config.inhibit_all {
                        return;
                    }
                    if !self.record_warning_emission(diag) {
                        return;
                    }
                    if self.warning_config.werror_all {
                        let promoted = Diagnostic {
                            severity: Severity::Error,
                            message: format!("{} [-Werror]", diag.message),
                            span: diag.span,
                            warning_kind: None,
                            notes: diag.notes.clone(),
                            fix_hint: diag.fix_hint.clone(),
                            explicit_location: diag.explicit_location.clone(),
                            dedup_salt: diag.dedup_salt,
                        };
                        self.render_error(&promoted);
                    } else {
                        self.render_warning(diag);
                    }
                }
            }
            Severity::Error => {
                self.render_error(diag);
            }
            Severity::Note => {
                self.render_diagnostic(diag);
                // notes don't count separately
            }
        }
    }

    /// Whether `span` resolves into a system header.
    ///
    /// The test is the one GCC's driver applies when it classifies include
    /// directories: the compiler's own `<built-in>` buffer, `/usr/include*`,
    /// the GCC installation's include directory, `/usr/local/include*` and
    /// `/usr/lib*/gcc/*/include*`. `-I`/`-isystem` directories are not
    /// consulted here; those are ordinary user directories from the point of
    /// view of this predicate, so a warning inside them is still reported.
    fn span_is_in_system_header(&self, span: Span) -> bool {
        let Some(sm) = self.source_manager.as_ref() else {
            return false;
        };
        let file = sm.resolve_span(span).file;
        is_system_header_path(&file)
    }

    /// Emit an error with a message and span.
    pub fn error(&mut self, message: impl Into<String>, span: Span) {
        let diag = Diagnostic::error(message).with_span(span);
        self.emit(&diag);
    }

    /// Emit a `note:` line that belongs to the diagnostic just reported, at
    /// the same location GCC uses for it. Notes never change the exit status
    /// and are not counted in the diagnostic totals.
    pub fn note(&mut self, message: impl Into<String>, span: Span) {
        let diag = Diagnostic::note(message).with_span(span);
        self.emit(&diag);
    }

    /// Emit a warning with a message and span.
    pub fn warning(&mut self, message: impl Into<String>, span: Span) {
        let diag = Diagnostic::warning(message).with_span(span);
        self.emit(&diag);
    }

    /// Emit a categorized warning with a message, span, and kind.
    pub fn warning_with_kind(&mut self, message: impl Into<String>, span: Span, kind: WarningKind) {
        let diag = Diagnostic::warning_with_kind(message, kind).with_span(span);
        self.emit(&diag);
    }

    /// Emit a categorized warning that must not be folded into an earlier
    /// identical one. `salt` distinguishes the occurrences; see
    /// [`Diagnostic::dedup_salt`].
    pub fn warning_with_kind_salted(
        &mut self,
        message: impl Into<String>,
        span: Span,
        kind: WarningKind,
        salt: u64,
    ) {
        let mut diag = Diagnostic::warning_with_kind(message, kind).with_span(span);
        diag.dedup_salt = salt;
        self.emit(&diag);
    }

    /// Diagnose the bare-return subcase without suppressing the independent
    /// return-from-void subcase of the same warning class in pre-C99 modes.
    pub fn missing_return_value(&mut self, span: Span) {
        if self.warning_config.bare_return_enabled {
            self.warning_with_kind(
                "'return' with no value, in function returning non-void",
                span,
                WarningKind::ReturnMismatch,
            );
        }
    }

    /// Returns true if any errors have been emitted (including promoted warnings).
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Number of errors emitted so far (including promoted warnings).
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Number of warnings emitted so far (excludes promoted warnings).
    pub fn warning_count(&self) -> usize {
        self.warning_count
    }

    /// Render "In file included from X:Y:" traces for errors in included files.
    ///
    /// GCC emits these traces before the first diagnostic in an included file,
    /// showing the chain of #include directives that led to the current file.
    /// Example output:
    /// ```text
    /// In file included from main.c:4:
    /// In file included from header1.h:2:
    /// header2.h:10:5: error: unknown type name 'foo'
    /// ```
    ///
    /// The trace is only emitted once per file -- consecutive errors in the
    /// same included file do not repeat the chain.
    fn render_include_trace(&mut self, diag: &Diagnostic) {
        // Only emit traces for diagnostics that have a span and are errors/warnings
        // (not for notes, which are sub-diagnostics)
        if diag.severity == Severity::Note {
            return;
        }

        let span = match diag.span {
            Some(s) => s,
            None => return,
        };

        let sm = match &self.source_manager {
            Some(sm) => sm,
            None => return,
        };

        let loc = sm.resolve_span(span);

        // Check if we already emitted a trace for this file
        if let Some(ref last_file) = self.last_include_trace_file {
            if *last_file == loc.file {
                return; // Same file as last diagnostic, skip trace
            }
        }

        // Get the include chain for this file
        let chain = sm.get_include_chain(&loc.file);
        if chain.is_empty() {
            // Main file or no include info -- no trace needed
            self.last_include_trace_file = Some(loc.file.clone());
            return;
        }

        // Emit the include trace matching GCC format:
        //   In file included from header1.h:3,
        //                    from main.c:1:
        // The first entry uses "In file included from", subsequent entries use
        // "from" with indentation to align with the first entry.
        for (i, origin) in chain.iter().enumerate() {
            let is_last = i == chain.len() - 1;
            let suffix = if is_last { ":" } else { "," };
            if i == 0 {
                if self.use_color {
                    eprintln!(
                        "\x1b[1mIn file included from {}:{}{}\x1b[0m",
                        origin.file, origin.line, suffix
                    );
                } else {
                    eprintln!(
                        "In file included from {}:{}{}",
                        origin.file, origin.line, suffix
                    );
                }
            } else {
                // "                 from " aligns with "In file included from "
                if self.use_color {
                    eprintln!(
                        "\x1b[1m                 from {}:{}{}\x1b[0m",
                        origin.file, origin.line, suffix
                    );
                } else {
                    eprintln!(
                        "                 from {}:{}{}",
                        origin.file, origin.line, suffix
                    );
                }
            }
        }

        self.last_include_trace_file = Some(loc.file.clone());
    }

    /// Render a single diagnostic to stderr, including location, severity,
    /// message, source snippet with caret, fix-it hints, and any follow-up notes.
    ///
    /// When color is enabled, output matches GCC's color scheme:
    /// - Location prefix: bold white
    /// - "error:": bold red
    /// - "warning:": bold magenta
    /// - "note:": bold cyan
    /// - Message text: bold white
    /// - Source snippet: no special color
    /// - Caret/underline: bold green
    /// - Fix-it hints: bold green
    fn render_diagnostic(&mut self, diag: &Diagnostic) {
        // Emit "In file included from" traces for errors in included files
        self.render_include_trace(diag);

        use std::fmt::Write;
        let mut msg = String::new();

        if self.use_color {
            // Colored output matching GCC
            // Location prefix in bold white
            if let Some(span) = diag.span {
                if let Some(ref sm) = self.source_manager {
                    let loc = sm.resolve_span(span);
                    let _ = write!(
                        msg,
                        "\x1b[1m{}:{}:{}: \x1b[0m",
                        loc.file, loc.line, loc.column
                    );
                }
            } else if let Some(ref loc) = diag.explicit_location {
                let _ = write!(msg, "\x1b[1m{} \x1b[0m", loc);
            }
            // Severity label in its color, message in bold white
            let (sev_color, sev_text) = match diag.severity {
                Severity::Error => ("\x1b[1;31m", "error"), // bold red
                Severity::Warning => ("\x1b[1;35m", "warning"), // bold magenta
                Severity::Note => ("\x1b[1;36m", "note"),   // bold cyan
            };
            let _ = write!(
                msg,
                "{}{}\x1b[0m: \x1b[1m{}\x1b[0m",
                sev_color, sev_text, diag.message
            );
        } else {
            // Plain text output
            if let Some(span) = diag.span {
                if let Some(ref sm) = self.source_manager {
                    let loc = sm.resolve_span(span);
                    let _ = write!(msg, "{}:{}:{}: ", loc.file, loc.line, loc.column);
                }
            } else if let Some(ref loc) = diag.explicit_location {
                let _ = write!(msg, "{} ", loc);
            }
            let _ = write!(msg, "{}: {}", diag.severity, diag.message);
        }
        eprintln!("{}", msg);

        // Source snippet with caret underline
        if let Some(span) = diag.span {
            self.render_snippet(span);
        }

        // Render fix-it hint if present
        if let Some(ref hint) = diag.fix_hint {
            if self.use_color {
                eprintln!("  \x1b[1;32mfix-it hint:\x1b[0m {}", hint);
            } else {
                eprintln!("  fix-it hint: {}", hint);
            }
        }

        // Render "in expansion of macro 'X'" note for errors in macro expansions.
        // Only for primary diagnostics (errors/warnings), not for sub-notes.
        if diag.severity != Severity::Note {
            self.render_macro_expansion_trace(diag);
        }

        // Render any follow-up notes
        for note in &diag.notes {
            self.render_diagnostic(note);
        }
    }

    /// Render "note: in expansion of macro 'X'" traces for diagnostics that
    /// occur in lines produced by macro expansion.
    ///
    /// GCC emits these notes when an error or warning is triggered inside
    /// macro-expanded code, helping the user understand that the error is
    /// in the expansion of a specific macro.
    ///
    /// Example output:
    /// ```text
    /// file.c:10:5: error: expected ';' before '}' token
    ///    10 |   FOO(x)
    ///       |   ^
    /// file.c:10:5: note: in expansion of macro 'FOO'
    /// ```
    fn render_macro_expansion_trace(&self, diag: &Diagnostic) {
        let span = match diag.span {
            Some(s) => s,
            None => return,
        };

        let sm = match &self.source_manager {
            Some(sm) => sm,
            None => return,
        };

        let macro_names = match sm.get_macro_expansion_at(span) {
            Some(names) => names,
            None => return,
        };

        // Filter out predefined/builtin macros that are expanded ubiquitously
        // and would produce noise in diagnostics. Only show user-defined macros
        // that are likely relevant to the error.
        let interesting: Vec<&str> = macro_names
            .iter()
            .filter(|name| !is_uninteresting_macro(name))
            .map(|s| s.as_str())
            .collect();

        if interesting.is_empty() {
            return;
        }

        let loc = sm.resolve_span(span);

        // Emit a note for each macro in the expansion chain (outermost first).
        // Typically there is only one, but nested expansions may have several.
        // Limit to 3 levels to avoid noisy output for deeply nested macros.
        for name in interesting.iter().take(3) {
            if self.use_color {
                eprintln!(
                    "\x1b[1m{}:{}:{}: \x1b[0m\x1b[1;36mnote:\x1b[0m \x1b[1min expansion of macro '{}'\x1b[0m",
                    loc.file, loc.line, loc.column, name
                );
            } else {
                eprintln!(
                    "{}:{}:{}: note: in expansion of macro '{}'",
                    loc.file, loc.line, loc.column, name
                );
            }
        }
    }

    /// Render a source code snippet with a caret pointing to the error location.
    ///
    /// Output format:
    /// ```text
    ///     int x = 42
    ///             ^~
    /// ```
    ///
    /// When color is enabled, the caret and underline are rendered in bold green
    /// (matching GCC's behavior).
    fn render_snippet(&self, span: Span) {
        let sm = match &self.source_manager {
            Some(sm) => sm,
            None => return,
        };

        let source_line = match sm.get_source_line(span) {
            Some(line) => line,
            None => return,
        };

        // Don't render snippets for empty or whitespace-only lines
        if source_line.trim().is_empty() {
            return;
        }

        // Resolve the column for caret positioning.
        //
        // Coordinate systems (kept strictly separate):
        //   * `col` from resolve_span is a 1-based BYTE offset;
        //   * the snippet window and caret padding are CHAR-based, since
        //     the rendered line is a char sequence;
        //   * tabs render as single columns here (the renderer does not
        //     expand them), so display column == char index + 1.
        let loc = sm.resolve_span(span);
        let col = loc.column as usize;
        let line_bytes = source_line.as_bytes();
        let col_byte0 = col.saturating_sub(1).min(line_bytes.len());
        // Chars preceding the caret = non-continuation bytes preceding it.
        let col_char0 = line_bytes[..col_byte0]
            .iter()
            .filter(|&&b| (b & 0xC0) != 0x80)
            .count();

        // Window overly long lines around the caret (generated/minified code
        // can put 10k+ characters on one line; dumping it buries the caret
        // and floods logs — the frame-budget reproducers hit exactly this).
        const SNIPPET_WINDOW: usize = 160;
        let n_chars = source_line.chars().count();
        let (shown, caret_pad): (String, usize) = if n_chars <= SNIPPET_WINDOW {
            (source_line.clone(), col_char0 + 1)
        } else {
            let half = SNIPPET_WINDOW / 2;
            let start = col_char0
                .saturating_sub(half)
                .min(n_chars.saturating_sub(SNIPPET_WINDOW));
            let end = (start + SNIPPET_WINDOW).min(n_chars);
            let window: String = source_line.chars().skip(start).take(end - start).collect();
            // Caret alignment: 1 leading space + 3 for the "..." prefix +
            // 0-based char offset of the caret inside the window (+1 for
            // the 1-based column).
            let pad = (col_char0 - start) + 1 + if start > 0 { 3 } else { 0 };
            let shown = format!(
                "{}{}{}",
                if start > 0 { "..." } else { "" },
                window,
                if end < n_chars { "..." } else { "" }
            );
            (shown, pad)
        };

        // Print the source line with indentation
        eprintln!(" {}", shown);

        // Build the caret line: spaces up to the column, then ^ with tildes
        if caret_pad > 0 {
            let padding = " ".repeat(caret_pad);
            // Span length in CHARS (the squiggle is drawn over the
            // char-based window): count non-continuation bytes of the line
            // covered by [span.start, span.end).
            let line_start_byte = (span.start as usize).saturating_sub(col_byte0);
            let span_end_in_line = (span.end as usize).saturating_sub(line_start_byte);
            let lo = col_byte0.min(line_bytes.len());
            let hi = span_end_in_line.min(line_bytes.len()).max(lo);
            let span_len = line_bytes[lo..hi]
                .iter()
                .filter(|&&b| (b & 0xC0) != 0x80)
                .count()
                .max(usize::from(span_end_in_line > lo));
            // Never draw the squiggle past the rendered window.
            let span_len = span_len.min(shown.chars().count().saturating_sub(caret_pad));
            let underline = if span_len > 1 {
                format!("^{}", "~".repeat(span_len - 1))
            } else {
                "^".to_string()
            };
            if self.use_color {
                eprintln!("{}\x1b[1;32m{}\x1b[0m", padding, underline);
            } else {
                eprintln!("{}{}", padding, underline);
            }
        }
    }
}

impl Default for DiagnosticEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a macro name is a predefined/builtin macro that should not be shown
/// in "in expansion of macro" diagnostic notes. These macros are expanded
/// ubiquitously in C code and showing them would produce noise rather than
/// helping the user understand the error.
fn is_uninteresting_macro(name: &str) -> bool {
    // GCC/Clang builtin function-style macros
    if name.starts_with("__builtin_") {
        return true;
    }
    // Common standard library macros that are uninteresting
    matches!(
        name,
        "NULL"
            | "EOF"
            | "CHAR_BIT"
            | "CHAR_MAX"
            | "CHAR_MIN"
            | "INT_MAX"
            | "INT_MIN"
            | "UINT_MAX"
            | "LONG_MAX"
            | "LONG_MIN"
            | "ULONG_MAX"
            | "LLONG_MAX"
            | "LLONG_MIN"
            | "ULLONG_MAX"
            | "SIZE_MAX"
            | "PTRDIFF_MAX"
            | "PTRDIFF_MIN"
            | "true"
            | "false"
            | "bool"
            | "INT8_MAX"
            | "INT16_MAX"
            | "INT32_MAX"
            | "INT64_MAX"
            | "UINT8_MAX"
            | "UINT16_MAX"
            | "UINT32_MAX"
            | "UINT64_MAX"
            | "INT8_MIN"
            | "INT16_MIN"
            | "INT32_MIN"
            | "INT64_MIN"
            | "WCHAR_MAX"
            | "WCHAR_MIN"
            | "INTPTR_MAX"
            | "INTPTR_MIN"
            | "UINTPTR_MAX"
            | "SSIZE_MAX"
            | "PATH_MAX"
            | "SEEK_SET"
            | "SEEK_CUR"
            | "SEEK_END"
            | "STDIN_FILENO"
            | "STDOUT_FILENO"
            | "STDERR_FILENO"
            | "EXIT_SUCCESS"
            | "EXIT_FAILURE"
    )
}

#[cfg(test)]
mod dialect_severity_redteam_tests {
    use super::*;

    #[test]
    fn warning_registry_covers_each_kind_exactly_once() {
        use std::collections::HashSet;
        let kinds = [
            WarningKind::Undeclared,
            WarningKind::ImplicitFunctionDeclaration,
            WarningKind::Cpp,
            WarningKind::Undef,
            WarningKind::MissingPrototypes,
            WarningKind::MissingDeclarations,
            WarningKind::ReturnType,
            WarningKind::IncompatiblePointerTypes,
            WarningKind::Attributes,
            WarningKind::UnusedVariable,
            WarningKind::SignCompare,
            WarningKind::DivByZero,
            WarningKind::StrictPrototypes,
            WarningKind::Format,
            WarningKind::FormatSecurity,
            WarningKind::ImplicitInt,
            WarningKind::ReturnMismatch,
            WarningKind::IntConversion,
            WarningKind::PointerToIntCast,
            WarningKind::DiscardedQualifiers,
            WarningKind::DiscardedArrayQualifiers,
            WarningKind::IntToPointerCast,
            WarningKind::CompareDistinctPointerTypes,
            WarningKind::Pedantic,
            WarningKind::Vla,
            WarningKind::Multichar,
            WarningKind::ReturnLocalAddr,
        ];
        let expected: HashSet<_> = kinds.into_iter().collect();
        assert_eq!(
            kinds.len(),
            expected.len(),
            "duplicate expected warning kind"
        );
        let registered: HashSet<_> = WARNING_REGISTRY.iter().map(|entry| entry.kind).collect();
        let names: HashSet<_> = WARNING_REGISTRY.iter().map(|entry| entry.name).collect();
        // The registry must be a bijection onto the measured warning kinds:
        // no missing leaf, no duplicate row, and no duplicate flag spelling.
        assert_eq!(WARNING_REGISTRY.len(), expected.len());
        assert_eq!(registered, expected);
        assert_eq!(names.len(), expected.len());
        for kind in expected {
            assert_eq!(WarningKind::from_flag_name(kind.flag_name()), Some(kind));
        }
        for group in ["all", "extra", "implicit", "unused"] {
            assert!(WarningConfig::recognizes_name(group));
        }
        // GP-10 landed the leaf: -Wpedantic is now a recognised name, and
        // the Pedantic *group* slot stays reserved (no members yet).
        assert!(WarningConfig::recognizes_name("pedantic"));
    }

    #[test]
    fn quiet_mode_preserves_default_errors_but_suppresses_promoted_warnings() {
        let mut config = WarningConfig::new();
        config.apply_gcc162_c99_default_errors();
        config.set_inhibit_all();
        assert!(config.is_enabled(WarningKind::IntConversion));
        config.set_werror(WarningKind::Cpp);
        assert!(!config.is_enabled(WarningKind::Cpp));
        config.clear_werror(WarningKind::IntConversion);
        assert!(!config.is_enabled(WarningKind::IntConversion));
    }

    #[test]
    fn permissive_defaults_do_not_erase_specific_promotions() {
        for explicit_first in [false, true] {
            let mut config = WarningConfig::new();
            if explicit_first {
                config.set_werror(WarningKind::IntConversion);
            }
            config.set_permissive(true);
            if !explicit_first {
                config.set_werror(WarningKind::IntConversion);
            }
            config.apply_gcc162_c99_default_errors();
            assert!(config.is_error(WarningKind::IntConversion));
            assert!(!config.default_errors.contains(&WarningKind::IntConversion));
            assert!(!config.is_error(WarningKind::ImplicitInt));
        }
    }

    #[test]
    fn pre_c99_return_subcases_have_independent_enablement() {
        let mut config = WarningConfig::new();
        config.suppress_gcc162_pre_c99_defaults();
        assert!(config.is_enabled(WarningKind::ReturnMismatch));
        assert!(!config.bare_return_enabled);
        let mut config = WarningConfig::new();
        config.enable_wall();
        config.suppress_gcc162_pre_c99_defaults();
        assert!(config.bare_return_enabled);
        assert!(config.legacy_implicit_return_type);
    }

    #[test]
    fn explicit_nonpermissive_policy_promotes_live_pre_c99_constraints_only() {
        let mut config = WarningConfig::new();
        config.set_permissive(false);
        config.suppress_gcc162_pre_c99_defaults();
        assert!(config.is_error(WarningKind::IntConversion));
        assert!(config.is_enabled(WarningKind::IntConversion));
        assert!(!config.is_enabled(WarningKind::ImplicitInt));
        assert!(!config.bare_return_enabled);
    }

    /// The measured membership cells that the command-line gate re-verifies
    /// against GCC 16.2, stated against the registry as it stands.
    #[test]
    fn registry_matches_the_measured_gcc162_cells() {
        // Defaults: every enabled-by-default kind except the
        // compatibility-only `undeclared` switch.
        let mut default_on: Vec<&str> = WARNING_REGISTRY
            .iter()
            .filter(|entry| entry.default_on)
            .map(|entry| entry.name)
            .collect();
        default_on.sort_unstable();
        let mut expected_defaults = [
            "implicit-function-declaration",
            "cpp",
            "incompatible-pointer-types",
            "attributes",
            "implicit-int",
            "return-mismatch",
            "int-conversion",
            "pointer-to-int-cast",
            "int-to-pointer-cast",
            "compare-distinct-pointer-types",
            "discarded-qualifiers",
            "discarded-array-qualifiers",
            "div-by-zero",
            "return-local-addr",
            // `-Wmultichar` is on by default in GCC 16.2: `'ab'` warns with no
            // flag at all, `-Wno-multichar` silences it (measured).
            "multichar",
            "undeclared",
        ];
        expected_defaults.sort_unstable();
        assert_eq!(default_on, expected_defaults);
        // Exactly the measured five dialect default errors from C99 on.
        let mut c99: Vec<&str> = WARNING_REGISTRY
            .iter()
            .filter(|entry| entry.c99_default_error)
            .map(|entry| entry.name)
            .collect();
        c99.sort_unstable();
        assert_eq!(
            c99,
            [
                "implicit-function-declaration",
                "implicit-int",
                "incompatible-pointer-types",
                "int-conversion",
                "return-mismatch",
            ]
        );
        // Group membership is only observable for default-off warnings
        // (a group never re-enables a `-Wno-`'d warning, measured), and
        // these are exactly the members that turn them on.
        let wall: Vec<&str> = WARNING_REGISTRY
            .iter()
            .filter(|entry| !entry.default_on && entry.groups.contains(&WarningGroup::All))
            .map(|entry| entry.name)
            .collect();
        assert_eq!(wall, ["return-type", "unused-variable", "format"]);
        let wextra: Vec<&str> = WARNING_REGISTRY
            .iter()
            .filter(|entry| !entry.default_on && entry.groups.contains(&WarningGroup::Extra))
            .map(|entry| entry.name)
            .collect();
        assert_eq!(wextra, ["sign-compare"]);
        assert!(
            !WARNING_REGISTRY
                .iter()
                .any(|entry| entry.groups.contains(&WarningGroup::Pedantic))
        );
        // Diagnostic labels: `-Wformat` prints with a trailing `=`.
        for entry in WARNING_REGISTRY {
            let suffix = entry.kind.flag_suffix();
            assert_eq!(
                suffix,
                if entry.kind == WarningKind::Format {
                    "format="
                } else {
                    entry.name
                },
                "{} label",
                entry.name
            );
        }
    }

    /// GCC 16.2 (measured): a group flag never re-enables a warning an
    /// explicit `-Wno-<name>` turned off, in either order, while a
    /// positive `-W<name>` or `-Werror=<name>` does.
    #[test]
    fn group_flags_never_reenable_an_explicit_disable() {
        for group_first in [true, false] {
            for kind in [WarningKind::UnusedVariable, WarningKind::SignCompare] {
                let mut config = WarningConfig::new();
                let apply_group = |config: &mut WarningConfig, kind: WarningKind| {
                    if kind == WarningKind::UnusedVariable {
                        config.enable_wall();
                    } else {
                        config.enable_wextra();
                    }
                };
                if group_first {
                    apply_group(&mut config, kind);
                    config.disable(kind);
                } else {
                    config.disable(kind);
                    apply_group(&mut config, kind);
                }
                assert!(
                    !config.is_enabled(kind),
                    "{kind:?} group_first={group_first}"
                );
            }
            // The same rule for a default-on kind and -Wall.
            let mut config = WarningConfig::new();
            if group_first {
                config.enable_wall();
                config.disable(WarningKind::ReturnType);
            } else {
                config.disable(WarningKind::ReturnType);
                config.enable_wall();
            }
            assert!(!config.is_enabled(WarningKind::ReturnType));
        }
        // The positive forms win over the disable.
        let mut config = WarningConfig::new();
        config.disable(WarningKind::ReturnType);
        config.enable(WarningKind::ReturnType);
        assert!(config.is_enabled(WarningKind::ReturnType));
        let mut config = WarningConfig::new();
        config.disable(WarningKind::Format);
        config.set_werror(WarningKind::Format);
        assert!(config.is_enabled(WarningKind::Format));
        assert!(config.is_error(WarningKind::Format));
    }

    /// `-Wformat=0|1|2` are the only level spellings GCC 16.2 accepts
    /// (measured; every other `-W<name>=N` is a driver error there, which
    /// LCCC answers with its tolerated-option diagnostic via `false`).
    #[test]
    fn format_levels_follow_gcc() {
        let mut config = WarningConfig::new();
        assert!(config.process_flag("format=0"));
        assert!(!config.is_enabled(WarningKind::Format));
        config.werror_all = true;
        assert!(!config.is_enabled(WarningKind::Format));
        let mut config = WarningConfig::new();
        assert!(config.process_flag("format=1"));
        assert!(config.is_enabled(WarningKind::Format));
        assert!(!config.is_enabled(WarningKind::FormatSecurity));
        let mut config = WarningConfig::new();
        assert!(config.process_flag("format=2"));
        assert!(config.is_enabled(WarningKind::Format));
        assert!(config.is_enabled(WarningKind::FormatSecurity));
        assert!(!config.process_flag("sign-compare=1"));
        // `-Werror=format=0` disables outright (measured).
        let mut config = WarningConfig::new();
        assert!(config.process_flag("error=format=0"));
        assert!(!config.is_enabled(WarningKind::Format));
    }
}

/// Whether a resolved source path names a system header.
///
/// This is the path test GCC's driver applies when it marks include
/// directories as `-isystem` implicitly; it is deliberately narrow so that a
/// warning inside a project's own headers always survives.
pub fn is_system_header_path(path: &str) -> bool {
    // `<built-in>` and `<command line>` are the compiler's own pseudo-files:
    // GCC reports them as system headers, so their warnings follow
    // -Wsystem-headers like any other. (A warning that originates in *user*
    // code always carries a user-file location, so this cannot hide one.)
    if path == "<built-in>" || path == "<command line>" {
        return true;
    }
    let Some((dir, _)) = path.rsplit_once('/') else {
        // A bare relative path is a project file.
        return false;
    };
    // The compiler's own bundled headers (`include/` next to the binary) are
    // system headers by definition; they are recognised by shape below only if
    // they live under an install prefix, so name them explicitly.
    if dir.ends_with("/lib/gcc") || dir.contains("/lib/gcc/") {
        return true;
    }
    is_system_include_dir(dir)
}

/// Whether `dir` is one of the include directories GCC marks as system
/// implicitly: anything under `/usr/include`, `/usr/local/include`, the GCC or
/// LLVM installation trees, or a cross sysroot's `<triple>/include` (Debian
/// spells those `/usr/<triple>/include` and `/usr/lib/<triple>/include`).
///
/// The test is on *directory shape*, not on a list of prefixes, so a sysroot
/// passed with `--sysroot=/opt/sysroot` is covered too. A project that happens
/// to install headers under `.../usr/include` is treated as a system directory,
/// which is also what GCC's `-isystem` defaults do.
pub fn is_system_include_dir(dir: &str) -> bool {
    // Path-boundary comparison: `/usr/include_local` is NOT `/usr/include`.
    let under = |base: &str| dir == base || dir.starts_with(&format!("{base}/"));
    if under("/usr/include") || under("/usr/local/include") {
        return true;
    }
    // `/usr/lib/gcc/<triple>/<version>/include`, `/usr/lib/llvm-<n>/lib/clang/
    // <version>/include`, `/usr/lib/<triple>/include`, `/opt/sysroot/usr/lib/
    // gcc/...`: the directory itself ends in `include` beneath a compiler or
    // sysroot tree.
    let ends_with_include = dir.ends_with("/include")
        || dir
            .rsplit_once('/')
            .is_some_and(|(_, last)| last == "include");
    ends_with_include
        && (dir.contains("/lib/gcc/")
            || dir.contains("/lib/clang/")
            || dir.starts_with("/opt/")
            || dir.contains("/usr/")
            || dir.contains("-linux-gnu")
            || dir.contains("-linux-musl")
            || dir.contains("-unknown-"))
}

#[cfg(test)]
mod system_header_tests {
    use super::is_system_header_path;

    #[test]
    fn system_headers_are_recognised() {
        assert!(is_system_header_path("/usr/include/pthread.h"));
        assert!(is_system_header_path(
            "/usr/lib/gcc/x86_64-linux-gnu/12/include/stddef.h"
        ));
        assert!(is_system_header_path("<built-in>"));
    }

    #[test]
    fn project_headers_are_not() {
        assert!(!is_system_header_path("src/main.c"));
        assert!(!is_system_header_path("/home/user/project/include/x.h"));
        assert!(!is_system_header_path("/usr/include_local/x.h"));
        assert!(!is_system_header_path("/home/user/project/usr/x.h"));
    }

    #[test]
    fn cross_sysroots_are_system_headers() {
        // The AArch64/RISC-V lanes compile against a Debian cross sysroot; a
        // warning inside it must not reach the user any more than one in the
        // native `/usr/include` does.
        for p in [
            "/usr/aarch64-linux-gnu/include/stdio.h",
            "/usr/include/i386-linux-gnu/bits/types.h",
            "/usr/lib/gcc/aarch64-linux-gnu/14/include/stddef.h",
            "/opt/sysroot/usr/include/zlib.h",
            "/usr/lib/llvm-19/lib/clang/19/include/immintrin.h",
        ] {
            assert!(is_system_header_path(p), "{p}");
        }
    }

    #[test]
    fn warnings_from_system_headers_are_suppressed_unless_requested() {
        use super::DiagnosticEngine;
        use crate::common::source::{SourceManager, Span};
        let mut sm = SourceManager::new();
        let sys = sm.add_file("/usr/include/x.h".to_string(), "int v;\n".to_string());
        let usr = sm.add_file("src/main.c".to_string(), "int v;\n".to_string());
        let mut engine = DiagnosticEngine::new();
        engine.set_source_manager(sm);

        // Default: a warning the user cannot act on is not printed, but it is
        // still counted as suppressed (not as emitted).
        engine.warning("in a system header", Span::new(0, 5, sys));
        assert_eq!(
            engine.warning_count, 0,
            "system-header warning must be suppressed"
        );
        // A warning in the user's own file is unaffected.
        engine.warning("in the user's file", Span::new(0, 5, usr));
        assert_eq!(engine.warning_count, 1);

        // `-Wsystem-headers` turns the suppression off.
        engine.set_system_headers(true);
        engine.warning("in a system header", Span::new(0, 5, sys));
        assert_eq!(engine.warning_count, 2);

        // Errors are never suppressed: a broken system header is still a
        // broken translation unit.
        engine.error("in a system header", Span::new(0, 5, sys));
        assert_eq!(engine.error_count, 1);
    }

    #[test]
    fn command_line_and_builtin_locations_are_system() {
        assert!(is_system_header_path("<built-in>"));
        assert!(is_system_header_path("<command line>"));
    }
}

#[cfg(test)]
mod warning_dedup_tests {
    use super::{DiagnosticEngine, WarningConfig, WarningKind};
    use crate::common::source::{SourceManager, Span};

    fn engine_with_user_file() -> (DiagnosticEngine, Span) {
        let mut sm = SourceManager::new();
        let file = sm.add_file("src/main.c".to_string(), "int v;\n".to_string());
        let mut engine = DiagnosticEngine::new();
        engine.set_source_manager(sm);
        (engine, Span::new(0, 3, file))
    }

    #[test]
    fn unsalted_identical_warnings_are_not_folded() {
        let (mut engine, span) = engine_with_user_file();
        engine.warning("same text", span);
        engine.warning("same text", span);
        assert_eq!(engine.warning_count(), 2);
    }

    #[test]
    fn salted_identical_warnings_fold_per_salt() {
        let (mut engine, span) = engine_with_user_file();
        engine.warning_with_kind_salted("same text", span, WarningKind::Attributes, 1);
        engine.warning_with_kind_salted("same text", span, WarningKind::Attributes, 1);
        assert_eq!(engine.warning_count(), 1);
        engine.warning_with_kind_salted("same text", span, WarningKind::Attributes, 2);
        assert_eq!(engine.warning_count(), 2);
    }

    #[test]
    fn suppressed_salted_warning_does_not_consume_a_later_visible_emission() {
        let (mut engine, span) = engine_with_user_file();
        let mut config = WarningConfig::new();
        config.disable(WarningKind::Attributes);
        engine.set_warning_config(config.clone());
        engine.warning_with_kind_salted("same text", span, WarningKind::Attributes, 7);
        assert_eq!(engine.warning_count(), 0);
        config.enable(WarningKind::Attributes);
        engine.set_warning_config(config);
        engine.warning_with_kind_salted("same text", span, WarningKind::Attributes, 7);
        assert_eq!(engine.warning_count(), 1);
    }

    #[test]
    fn system_header_suppression_does_not_consume_salted_dedup_slot() {
        let mut sm = SourceManager::new();
        let sys = sm.add_file("/usr/include/x.h".to_string(), "int v;\n".to_string());
        let usr = sm.add_file("src/main.c".to_string(), "int v;\n".to_string());
        let mut engine = DiagnosticEngine::new();
        engine.set_source_manager(sm);
        let sys_span = Span::new(0, 3, sys);
        let usr_span = Span::new(0, 3, usr);
        engine.warning_with_kind_salted("same text", sys_span, WarningKind::Attributes, 9);
        assert_eq!(engine.warning_count(), 0);
        engine.warning_with_kind_salted("same text", usr_span, WarningKind::Attributes, 9);
        assert_eq!(engine.warning_count(), 1);
    }
}

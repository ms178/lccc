//! Language semantics carried with IR, never ambient worker-thread state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PassPolicy {
    pub wrap_signed_overflow: bool,
    pub fp_environment_observable: bool,
    /// Which standard-library functions may be assumed to BE the library
    /// function (and so be synthesized, folded into, or expanded inline).
    pub builtins: BuiltinPolicy,
}

impl PassPolicy {
    pub const fn new(wrap_signed_overflow: bool, fp_environment_observable: bool) -> Self {
        Self {
            wrap_signed_overflow,
            fp_environment_observable,
            builtins: BuiltinPolicy::HOSTED,
        }
    }

    pub const fn with_builtins(self, builtins: BuiltinPolicy) -> Self {
        Self { builtins, ..self }
    }
}

// Declares `Libcall`, its complete `ALL` table and `name()` from ONE list: a
// variant cannot exist without its name, and cannot be missing from `ALL`
// (the array length is derived from the same list), so the exhaustive
// admission tests always cover every synthesizable library name.
macro_rules! libcalls {
    ($(#[$doc:meta])* $($variant:ident = $name:literal,)+) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Libcall {
            $($variant,)+
        }

        impl Libcall {
            /// Every variant, in declaration order.
            pub const ALL: [Libcall; [$(stringify!($variant)),+].len()] =
                [$(Libcall::$variant),+];

            /// The C-visible symbol name of this library function.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Libcall::$variant => $name,)+
                }
            }
        }
    };
}

libcalls! {
    /// The library functions lccc creates calls to (by synthesis or by folding
    /// one call into another).  These are the names whose status a
    /// function-level policy must carry: a pass about to CREATE a call has no
    /// call site to read [`CallInfo::assume_library`] from.  Passes that only
    /// interpret an existing call (strlen folding, math intrinsics, purity
    /// tables) read that per-call fact instead, which covers every name.
    ///
    /// [`CallInfo::assume_library`]: crate::ir::reexports::CallInfo::assume_library
    Memset = "memset",
    Memcpy = "memcpy",
    Memmove = "memmove",
    MemsetChk = "__memset_chk",
    MemcpyChk = "__memcpy_chk",
    Printf = "printf",
    Vprintf = "vprintf",
    Fprintf = "fprintf",
    Vfprintf = "vfprintf",
    PrintfChk = "__printf_chk",
    VprintfChk = "__vprintf_chk",
    FprintfChk = "__fprintf_chk",
    VfprintfChk = "__vfprintf_chk",
    Puts = "puts",
    Putchar = "putchar",
    Fputs = "fputs",
    Fputc = "fputc",
    Fwrite = "fwrite",
}

// `bit()` packs one variant per bit of a u32 mask: the table must fit.
const _: () = assert!(Libcall::ALL.len() <= u32::BITS as usize);

impl Libcall {
    pub fn from_name(name: &str) -> Option<Libcall> {
        Libcall::ALL.into_iter().find(|l| l.name() == name)
    }

    const fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// Builtin status, as GCC 16.2 defines it (measured against the oracle;
/// see `driver::cli` for the flag table):
///
/// * `-fno-builtin` / `-ffreestanding` / `-fno-hosted` withdraw every
///   library name; `-fbuiltin` / `-fhosted` / `-fno-freestanding` restore
///   them (last one wins).  Reserved implementation names (`__` prefix:
///   `__builtin_*`, libgcc's `__fabstf2`, ...) are the compiler's own and
///   never withdrawn -- except the fortified entry points among the
///   [`Libcall`] names (`__printf_chk` stays a call under `-fno-builtin`);
/// * `-fno-builtin-<name>` withdraws one name and is STICKY: a later blanket
///   restore does not bring it back;
/// * per function, `passes::builtin_policy::apply` withdraws each
///   [`Libcall`] whose synthesis there could execute inside the translation
///   unit's own definition of that function (or of one the library
///   implementation calls): creating `call memset` inside the TU's `memset`,
///   or inside a helper it reaches, recurses forever.
///
/// A withdrawn name is never synthesized by a pass, and a call to it is
/// folded or expanded only when it was spelled `__builtin_<name>` (which is
/// recorded per call: `CallInfo::assume_library`).  This Copy value carries
/// only the [`Libcall`] bits; per-name withdrawals of other names live in
/// the driver's [`BuiltinFlags`], which reaches every call site once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinPolicy {
    blanket_withdrawn: bool,
    /// [`Libcall`] bits: the command line's sticky `-fno-builtin-<name>`s
    /// plus this function's synthesis withdrawals.
    withdrawn: u32,
}

impl Default for BuiltinPolicy {
    fn default() -> Self {
        Self::HOSTED
    }
}

impl BuiltinPolicy {
    /// Every name is the library function (a hosted TU, no flags).
    pub const HOSTED: BuiltinPolicy = BuiltinPolicy {
        blanket_withdrawn: false,
        withdrawn: 0,
    };

    /// `-fno-builtin` / `-ffreestanding` / `-fno-hosted`.
    pub fn withdraw_all(&mut self) {
        self.blanket_withdrawn = true;
    }

    /// `-fbuiltin` / `-fhosted` / `-fno-freestanding`: undoes the blanket
    /// withdrawal only.
    pub fn restore_all(&mut self) {
        self.blanket_withdrawn = false;
    }

    /// `-fno-builtin-<name>` for a [`Libcall`] name (other names: see
    /// [`BuiltinFlags::withdraw_name`]).
    pub fn withdraw_name(&mut self, name: &str) {
        if let Some(l) = Libcall::from_name(name) {
            self.withdraw_libcall(l);
        }
    }

    /// No call to `l` may be created (or folded into) here.
    pub fn withdraw_libcall(&mut self, l: Libcall) {
        self.withdrawn |= l.bit();
    }

    /// May a call spelled `name` (not `__builtin_name`) be treated as the
    /// library function, as far as this policy knows?  (A per-name
    /// withdrawal of a non-[`Libcall`] name is [`BuiltinFlags`]' to apply.)
    pub fn may_assume(self, name: &str) -> bool {
        match Libcall::from_name(name) {
            Some(l) => self.may_assume_libcall(l),
            None => !self.blanket_withdrawn || name.starts_with("__"),
        }
    }

    /// May a pass create a NEW call to `l` (or fold a call into one)?
    pub fn may_synthesize(self, l: Libcall) -> bool {
        self.may_assume_libcall(l)
    }

    fn may_assume_libcall(self, l: Libcall) -> bool {
        self.withdrawn & l.bit() == 0 && !self.blanket_withdrawn
    }
}

/// The command line's builtin flags: the [`BuiltinPolicy`] that travels
/// with the IR, plus `-fno-builtin-<name>` for names outside [`Libcall`]
/// (`strlen`, `sqrt`, ...), which only per-call facts need and which
/// `passes::builtin_policy::apply` folds into `CallInfo::assume_library`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuiltinFlags {
    policy: BuiltinPolicy,
    withdrawn_other: Vec<String>,
}

impl BuiltinFlags {
    pub fn withdraw_all(&mut self) {
        self.policy.withdraw_all();
    }

    pub fn restore_all(&mut self) {
        self.policy.restore_all();
    }

    /// `-fno-builtin-<name>`: sticky, for any name.
    pub fn withdraw_name(&mut self, name: &str) {
        if Libcall::from_name(name).is_some() {
            self.policy.withdraw_name(name);
        } else if !self.withdrawn_other.iter().any(|n| n == name) {
            self.withdrawn_other.push(name.to_string());
        }
    }

    pub fn policy(&self) -> BuiltinPolicy {
        self.policy
    }

    /// The command line's answer for a call spelled `name`.
    pub fn may_assume(&self, name: &str) -> bool {
        self.policy.may_assume(name) && !self.withdrawn_other.iter().any(|n| n == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_are_independent() {
        for wrap in [false, true] {
            for fenv in [false, true] {
                let policy = PassPolicy::new(wrap, fenv);
                assert_eq!(policy.wrap_signed_overflow, wrap);
                assert_eq!(policy.fp_environment_observable, fenv);
                assert_eq!(policy.builtins, BuiltinPolicy::HOSTED);
            }
        }
    }

    #[test]
    fn names_are_unique_and_the_table_fits_the_mask() {
        let mut names: Vec<&str> = Libcall::ALL.iter().map(|l| l.name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate Libcall name");
        assert!(Libcall::ALL.len() <= u32::BITS as usize);
        // Every variant owns a distinct mask bit.
        let mask = Libcall::ALL.iter().fold(0u32, |m, l| m | l.bit());
        assert_eq!(mask.count_ones() as usize, Libcall::ALL.len());
    }

    #[test]
    fn from_name_is_exact() {
        // No prefix, suffix, case or `__builtin_` aliasing: a spelled builtin
        // is a per-call fact (`CallInfo::assume_library`), not a Libcall name.
        for name in [
            "",
            "mem",
            "memsetx",
            "MEMSET",
            "__builtin_memset",
            " memset",
        ] {
            assert_eq!(Libcall::from_name(name), None, "{name:?}");
        }
    }

    #[test]
    fn names_round_trip() {
        for l in Libcall::ALL {
            assert_eq!(Libcall::from_name(l.name()), Some(l));
        }
        assert_eq!(Libcall::from_name("strlen"), None);
    }

    #[test]
    fn blanket_withdrawal_is_last_wins_and_spares_reserved_names() {
        let mut p = BuiltinPolicy::HOSTED;
        p.withdraw_all();
        for name in ["memset", "__printf_chk", "__memset_chk", "strlen", "sqrt"] {
            assert!(!p.may_assume(name), "{name}");
        }
        for name in ["__builtin_fabs", "__fabstf2", "__copysigntf3"] {
            assert!(p.may_assume(name), "{name}");
        }
        p.restore_all();
        assert!(p.may_assume("memset") && p.may_assume("strlen"));
    }

    #[test]
    fn flags_withdraw_any_name_stickily() {
        let mut f = BuiltinFlags::default();
        f.withdraw_name("strlen");
        f.withdraw_name("memcpy");
        f.withdraw_name("strlen");
        f.restore_all();
        assert!(!f.may_assume("strlen") && !f.may_assume("memcpy"));
        assert!(f.may_assume("sqrt") && f.may_assume("memset"));
        assert!(!f.policy().may_synthesize(Libcall::Memcpy));
        assert!(f.policy().may_synthesize(Libcall::Memset));
        // Only Libcall names live in the Copy policy.
        assert!(f.policy().may_assume("strlen"));
    }

    #[test]
    fn per_name_withdrawal_is_sticky() {
        let mut p = BuiltinPolicy::HOSTED;
        p.withdraw_name("memset");
        p.restore_all();
        assert!(!p.may_assume("memset"));
        assert!(p.may_assume("memcpy"));
        p.withdraw_name("__memset_chk");
        assert!(!p.may_assume("__memset_chk"));
    }

    #[test]
    fn a_libcall_withdrawal_is_exact() {
        let mut p = BuiltinPolicy::HOSTED;
        p.withdraw_libcall(Libcall::Memset);
        assert!(!p.may_synthesize(Libcall::Memset));
        assert!(!p.may_assume("memset"));
        for l in Libcall::ALL.into_iter().filter(|&l| l != Libcall::Memset) {
            assert!(p.may_synthesize(l), "{l:?}");
        }
    }
}

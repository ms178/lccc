//! ELF thread-local-storage access models: the one place that decides which
//! model a `__thread`/`_Thread_local` symbol is accessed with.
//!
//! Every backend (and the RISC-V pre-codegen lowering) asks [`select_model`]
//! through [`model_table`], so the four targets cannot disagree about a symbol
//! and the driver flag, the declaration attribute and the binding rules are
//! applied in exactly one order.
//!
//! # The rule (measured against GCC 16.2, `decl_default_tls_model`)
//!
//! *Shared-library code* (`flag_shlib`) is code compiled with `-fPIC`/`-fpic`
//! or for `-shared`; PIE and non-PIC code is executable code.
//!
//! | binds locally? | executable | shared-library code |
//! |---|---|---|
//! | yes | Local-Exec | Local-Dynamic |
//! | no  | Initial-Exec | Global-Dynamic |
//!
//! The result is then raised (never lowered) to the `-ftls-model=` floor and to
//! the `tls_model` attribute, in the order GD < LD < IE < LE: a request can
//! only ask for a *more* general access than the defaults would pick, and an
//! attribute naming a more general model than the binding allows is upgraded
//! (`__thread int x __attribute__((tls_model("global-dynamic")))` defined in a
//! PIE is still Local-Exec, measured).
//!
//! "Binds locally" (measured, GCC 16.2 x86-64):
//! * executable: the symbol is defined in this translation unit (a `weak`
//!   definition included — the executable's definition dominates), or it has
//!   hidden/internal/protected visibility (an `extern` declaration included);
//! * shared-library code: the symbol is `static`, or hidden/internal.
//!   **Protected does not count** and neither does a default-visibility
//!   definition: both are `@tlsgd` under `-fPIC`.
//!
//! Local-Dynamic is kept as a distinct value (it is what `-ftls-model=` and
//! the attribute can name, and what the floor ordering needs) but every
//! backend lowers it with the same per-symbol sequence as Global-Dynamic.
//! That is exactly GCC's `-O0` behaviour and is ABI-identical; the module-base
//! form only pays off when one function touches several distinct local TLS
//! symbols, and the descriptor dialects lccc uses make the per-symbol call a
//! two-instruction fast path once the module's TLS block exists.

use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::ir::reexports::IrModule;
use std::cell::RefCell;

thread_local! {
    /// TLS symbols of the module being compiled whose address sequence is a
    /// real call on this target (see [`set_call_sequence_symbols`]).
    static CALL_SEQUENCE_SYMBOLS: RefCell<FxHashSet<String>> =
        RefCell::new(FxHashSet::default());
}

/// Install, for the module about to be compiled, the TLS symbols whose
/// address materialisation is a branch-and-link the backend's text peephole
/// models as an ordinary call (AArch64 `.tlsdesccall x; blr x1`).
///
/// The register allocator must then treat each such `GlobalAddr` as a call
/// point (`liveness::instruction_is_call_point`): the peephole's call model
/// assumes every caller-saved register dies there, so no value may be
/// allocated to one across it. The descriptor ABI preserves far more than
/// that, but over-approximating the clobbers is sound and confined to the
/// dynamic models, which already pay for a call. Per-module (thread-local,
/// like the other target facts in `common::types`); replaced wholesale.
pub fn set_call_sequence_symbols(names: FxHashSet<String>) {
    CALL_SEQUENCE_SYMBOLS.with(|s| *s.borrow_mut() = names);
}

/// Whether the current target emits the dynamic models as a real call:
/// AArch64 (`blr` to the descriptor, modelled as a full call by the text
/// peephole) and RISC-V (`call __tls_get_addr`). x86-64/i686 TLS descriptors
/// are pinned opaque `rax`/`eax`-only instructions that preserve every other
/// register, so they are not call points.
fn dynamic_access_is_call() -> bool {
    const EM_AARCH64: u16 = 183;
    const EM_RISCV: u16 = 243;
    matches!(
        crate::common::types::target_elf_machine(),
        EM_AARCH64 | EM_RISCV
    )
}

/// [`set_call_sequence_symbols`] from a module's model table (the driver
/// installs it before the live-range splitters, codegen again from its own
/// table; both come from [`model_table`] with the same policy).
pub fn install_call_sequence_symbols(models: &FxHashMap<String, TlsModel>) {
    let names = if dynamic_access_is_call() {
        models
            .iter()
            .filter(|(_, m)| m.is_dynamic())
            .map(|(n, _)| n.clone())
            .collect()
    } else {
        FxHashSet::default()
    };
    set_call_sequence_symbols(names);
}

/// Whether taking the address of TLS symbol `name` is a call point in the
/// module currently being compiled.
pub fn address_is_call_sequence(name: &str) -> bool {
    CALL_SEQUENCE_SYMBOLS.with(|s| {
        let s = s.borrow();
        !s.is_empty() && s.contains(name)
    })
}

/// An ELF TLS access model, ordered from most general to most optimized
/// (GCC's `enum tls_model` order), so `max` picks the stricter of two
/// requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TlsModel {
    /// Any module, resolved at run time per symbol (`__tls_get_addr` or a TLS
    /// descriptor).
    GlobalDynamic,
    /// The defining module, resolved at run time (lowered like Global-Dynamic).
    LocalDynamic,
    /// A module in the static TLS block, offset read from the GOT.
    InitialExec,
    /// The executable's own TLS block, offset fixed at link time.
    LocalExec,
}

impl TlsModel {
    /// Parse GCC's spelling, as used by `-ftls-model=` and
    /// `__attribute__((tls_model("...")))`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "global-dynamic" => Some(Self::GlobalDynamic),
            "local-dynamic" => Some(Self::LocalDynamic),
            "initial-exec" => Some(Self::InitialExec),
            "local-exec" => Some(Self::LocalExec),
            _ => None,
        }
    }

    /// GCC's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GlobalDynamic => "global-dynamic",
            Self::LocalDynamic => "local-dynamic",
            Self::InitialExec => "initial-exec",
            Self::LocalExec => "local-exec",
        }
    }

    /// Whether the access needs the run-time resolver (a call).
    pub fn is_dynamic(self) -> bool {
        matches!(self, Self::GlobalDynamic | Self::LocalDynamic)
    }
}

/// The compilation-wide inputs of the model choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TlsPolicy {
    /// Shared-library code: `-fPIC`/`-fpic` or `-shared` (GCC `flag_shlib`).
    /// PIE and non-PIC code is executable code.
    pub shlib: bool,
    /// `-ftls-model=` (GCC `flag_tls_default`); `None` = not given.
    pub floor: Option<TlsModel>,
}

/// The model GCC 16.2 selects for one symbol (see the module docs).
pub fn select_model(policy: TlsPolicy, binds_locally: bool, attr: Option<TlsModel>) -> TlsModel {
    let default = match (policy.shlib, binds_locally) {
        (false, true) => TlsModel::LocalExec,
        (false, false) => TlsModel::InitialExec,
        (true, true) => TlsModel::LocalDynamic,
        (true, false) => TlsModel::GlobalDynamic,
    };
    let mut model = default;
    if let Some(floor) = policy.floor {
        model = model.max(floor);
    }
    if let Some(attr) = attr {
        model = model.max(attr);
    }
    model
}

/// Whether a TLS symbol binds to its definition in this module.
///
/// `defined` is whether this translation unit defines the symbol (a weak
/// definition counts); `visibility` is the declared ELF visibility.
pub fn binds_locally(
    shlib: bool,
    is_static: bool,
    defined: bool,
    visibility: Option<&str>,
) -> bool {
    if is_static || matches!(visibility, Some("hidden" | "internal")) {
        return true;
    }
    // Executable: everything defined in the program binds to that definition,
    // and a protected symbol is by definition defined in this module.
    !shlib && (defined || visibility == Some("protected"))
}

/// The final access model of every TLS symbol the module names.
///
/// Redeclarations are merged the way GCC merges declaration attributes: the
/// symbol is defined if any entry defines it, `static` if any entry is, the
/// first explicit visibility wins, and the `tls_model` attribute comes from
/// `module.tls_model_attrs` (the last declaration's, decided by lowering).
pub fn model_table(module: &IrModule, policy: TlsPolicy) -> FxHashMap<String, TlsModel> {
    struct Facts<'a> {
        is_static: bool,
        defined: bool,
        visibility: Option<&'a str>,
    }
    let mut facts: FxHashMap<&str, Facts<'_>> = FxHashMap::default();
    for g in module.globals.iter().filter(|g| g.is_thread_local) {
        let f = facts.entry(g.name.as_str()).or_insert(Facts {
            is_static: false,
            defined: false,
            visibility: None,
        });
        f.is_static |= g.is_static;
        f.defined |= !g.is_extern;
        if f.visibility.is_none() {
            f.visibility = g.visibility.as_deref();
        }
    }
    // A visibility written on a separate (non-TLS-typed) redeclaration, e.g.
    // `extern __thread int x; extern int x __attribute__((visibility("hidden")));`
    // reaches the module only through `symbol_attrs`.
    for (name, _, vis) in &module.symbol_attrs {
        if let (Some(f), Some(v)) = (facts.get_mut(name.as_str()), vis.as_deref()) {
            if f.visibility.is_none() {
                f.visibility = Some(v);
            }
        }
    }
    facts
        .into_iter()
        .map(|(name, f)| {
            let local = binds_locally(policy.shlib, f.is_static, f.defined, f.visibility);
            let attr = module.tls_model_attrs.get(name).copied();
            (name.to_string(), select_model(policy, local, attr))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::TlsModel::*;
    use super::*;

    const EXEC: TlsPolicy = TlsPolicy {
        shlib: false,
        floor: None,
    };
    const SHLIB: TlsPolicy = TlsPolicy {
        shlib: true,
        floor: None,
    };

    #[test]
    fn spelling_round_trips_and_rejects_unknown() {
        for m in [GlobalDynamic, LocalDynamic, InitialExec, LocalExec] {
            assert_eq!(TlsModel::parse(m.as_str()), Some(m));
        }
        assert_eq!(TlsModel::parse("initial_exec"), None);
        assert_eq!(TlsModel::parse("traditional"), None);
    }

    /// The GCC 16.2 table measured on `t3.c` (-O2, x86-64).
    #[test]
    fn matches_gcc_16_binding_table() {
        // __attribute__((weak)) __thread int w = 1;
        assert_eq!(
            select_model(EXEC, binds_locally(false, false, true, None), None),
            LocalExec
        );
        assert_eq!(
            select_model(SHLIB, binds_locally(true, false, true, None), None),
            GlobalDynamic
        );
        // extern __thread int hx __attribute__((visibility("hidden")));
        assert!(binds_locally(true, false, false, Some("hidden")));
        assert_eq!(select_model(EXEC, true, None), LocalExec);
        assert_eq!(select_model(SHLIB, true, None), LocalDynamic);
        // __thread int pr __attribute__((visibility("protected"))) = 2;
        assert!(binds_locally(false, false, true, Some("protected")));
        assert!(!binds_locally(true, false, true, Some("protected")));
        // extern (default visibility)
        assert!(!binds_locally(false, false, false, None));
        assert_eq!(select_model(EXEC, false, None), InitialExec);
        // static __thread
        assert!(binds_locally(true, true, true, None));
    }

    #[test]
    fn requests_only_raise_the_model() {
        // tls_model("global-dynamic") on a PIE definition stays Local-Exec.
        assert_eq!(select_model(EXEC, true, Some(GlobalDynamic)), LocalExec);
        // tls_model("local-exec") on a static in -fPIC code is honoured.
        assert_eq!(select_model(SHLIB, true, Some(LocalExec)), LocalExec);
        // extern tls_model("initial-exec") under -fPIC.
        assert_eq!(select_model(SHLIB, false, Some(InitialExec)), InitialExec);
        // -fPIC -ftls-model=initial-exec: every non-attributed symbol is IE.
        let ie = TlsPolicy {
            shlib: true,
            floor: Some(InitialExec),
        };
        assert_eq!(select_model(ie, false, None), InitialExec);
        assert_eq!(select_model(ie, true, None), InitialExec);
        assert_eq!(select_model(ie, true, Some(LocalExec)), LocalExec);
        // -fPIE -ftls-model=global-dynamic changes nothing.
        let gd = TlsPolicy {
            shlib: false,
            floor: Some(GlobalDynamic),
        };
        assert_eq!(select_model(gd, true, None), LocalExec);
        assert_eq!(select_model(gd, false, None), InitialExec);
    }

    #[test]
    fn dynamic_models_are_the_resolver_ones() {
        assert!(GlobalDynamic.is_dynamic() && LocalDynamic.is_dynamic());
        assert!(!InitialExec.is_dynamic() && !LocalExec.is_dynamic());
    }
}

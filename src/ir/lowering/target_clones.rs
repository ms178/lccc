//! GCC `__attribute__((target_clones(...)))` / `[[gnu::target_clones(...)]]`
//! function multiversioning via CPUID + `STT_GNU_IFUNC`.
//!
//! Given a function definition annotated with `target_clones("default", "avx2",
//! "arch=x86-64-v3", ...)`:
//! 1. Splits comma-separated clone specifications and validates that `"default"`
//!    is present and every non-default specification maps to a known x86 CPUID
//!    probe set.
//! 2. Lowers one internal `IrFunction` per clone (`<name>.default`,
//!    `<name>.avx2`, `<name>.arch_x86_64_v3`, ...) with `target_features`
//!    populated so both the middle-end vectorizer and x86 codegen compile each
//!    clone under its exact ISA permissions (e.g. 256-bit AVX2+FMA for
//!    `arch=x86-64-v3`, 256-bit AVX2 for `avx2`, 128-bit SSE2 for `default`).
//! 3. Synthesizes `<name>.resolver` using `crate::common::cpu_probe` (pure
//!    TU-local RIP/PC-relative state, zero libgcc dependency, safe during
//!    dynamic-linker `IRELATIVE` relocation) ordered by descending ISA priority,
//!    and binds `<name>` as an `STT_GNU_IFUNC` symbol pointing to
//!    `<name>.resolver`.

use super::func_state::FunctionBuildState;
use super::lower::Lowerer;
use crate::backend::Target;
use crate::common::cpu_probe::{self, Probe};
use crate::common::types::IrType;
use crate::frontend::parser::ast::FunctionDef;
use crate::ir::module::IrFunction;
use crate::ir::reexports::{Instruction, Operand, Terminator};

/// Parse a `target_clones` attribute string into deduplicated clone targets.
pub(super) fn parse_target_clones_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            continue;
        }
        if !out.contains(&trimmed) {
            out.push(trimmed);
        }
    }
    out
}

/// Mangle a clone target string into GCC's `.symtab` clone suffix
/// (`"arch=x86-64-v3"` -> `"arch_x86_64_v3"`, `"sse4.2"` -> `"sse4_2"`).
fn mangle_clone_suffix(spec: &str) -> String {
    spec.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Map a single non-`"default"` `target_clones` entry to `(priority, probes)`.
/// Higher `priority` clones are tested earlier by the IFUNC resolver, matching
/// GCC's `get_builtin_code_for_version` ISA hierarchy.
fn resolve_clone_spec(spec: &str) -> Result<(u32, Vec<Probe>), String> {
    let lookup_all = |names: &[&str]| -> Result<Vec<Probe>, String> {
        let mut v = Vec::with_capacity(names.len());
        for &n in names {
            let p = cpu_probe::lookup(n)
                .ok_or_else(|| format!("target_clones attribute '{spec}' is not supported"))?;
            v.push(p);
        }
        Ok(v)
    };

    // An `arch=` clone is compiled with that CPU's GCC feature set
    // (`X86Isa::with_target_features`, the intrinsic permission), so the
    // resolver probes every one of those features: a Nehalem has the whole
    // x86-64-v2 set but no PCLMUL, and must not run the `arch=westmere`
    // clone, whose code may use `_mm_clmulepi64_si128`. The rows carry the
    // CPU's whole AVX-512 set, so a Sapphire Rapids clone also needs
    // AVX512-FP16 and a Tiger Lake one AVX512-VP2INTERSECT.
    let arch_probes = |arch: &str| -> Result<Vec<Probe>, String> {
        let row = crate::backend::x86::isa::X86Isa::arch_row(arch)
            .filter(|row| row.x86_64)
            .ok_or_else(|| format!("target_clones attribute '{spec}' is not supported"))?;
        let names: Vec<&str> = crate::common::x86_features::X86Feature::ALL
            .iter()
            .filter(|&&feature| row.has(feature))
            .map(|feature| feature.name())
            .collect();
        lookup_all(&names)
    };
    let arch = spec.strip_prefix("arch=").unwrap_or("");

    match spec {
        "arch=x86-64" => Ok((gcc_prio::X86_64_BASELINE, arch_probes(arch)?)),
        "arch=nehalem" | "arch=silvermont" | "arch=goldmont" | "arch=tremont" => {
            Ok((gcc_prio::PROC_SSE4_2, arch_probes(arch)?))
        }
        "arch=x86-64-v2" => Ok((gcc_prio::X86_64_V2, arch_probes(arch)?)),
        // GCC resolves `arch=westmere` (NEHALEM + PCLMUL) to P_PCLMUL.
        "arch=westmere" => Ok((gcc_prio::PCLMUL, arch_probes(arch)?)),
        "arch=sandybridge" | "arch=ivybridge" => Ok((gcc_prio::PROC_AVX, arch_probes(arch)?)),
        "arch=haswell" | "arch=broadwell" | "arch=skylake" | "arch=alderlake"
        | "arch=raptorlake" | "arch=meteorlake" | "arch=arrowlake" | "arch=znver1"
        | "arch=znver2" | "arch=znver3" => Ok((gcc_prio::PROC_AVX2, arch_probes(arch)?)),
        "arch=x86-64-v3" => Ok((gcc_prio::X86_64_V3, arch_probes(arch)?)),
        "arch=skylake-avx512"
        | "arch=cascadelake"
        | "arch=cooperlake"
        | "arch=icelake-client"
        | "arch=icelake-server"
        | "arch=tigerlake"
        | "arch=sapphirerapids"
        | "arch=emeraldrapids"
        | "arch=znver4"
        | "arch=znver5" => Ok((gcc_prio::PROC_AVX512F, arch_probes(arch)?)),
        "arch=x86-64-v4" => Ok((gcc_prio::X86_64_V4, arch_probes(arch)?)),
        "arch=graniterapids" => Ok((gcc_prio::PROC_AVX10_1, arch_probes(arch)?)),
        other => {
            let probe = cpu_probe::lookup(other).ok_or_else(|| {
                format!("ISA '{other}' is not supported in 'target_clones' attribute")
            })?;
            Ok((gcc_prio::feature(other), vec![probe]))
        }
    }
}

/// GCC 16.2's dispatch order (`enum feature_priority`,
/// gcc/common/config/i386/i386-cpuinfo.h; values are the enumerator
/// ordinals). Feature names take their `isa_names_table` priority
/// (i386-isas.h) and `arch=` names their `processor_alias_table` one,
/// with P_PROC_DYNAMIC and the `x86-64-v*` levels resolved as
/// `get_builtin_code_for_version` does. Mirroring it makes the clone a
/// CPU runs the one GCC would run whenever both are eligible.
///
/// One deliberate difference: GCC dispatches a named CPU (`arch=haswell`)
/// with `__builtin_cpu_is`, lccc with the CPU's feature set. Every `arch=`
/// clone's code generation and intrinsics are confined to that CPU's GCC
/// feature set (`X86_ARCH_TABLE`), all of which its resolver probes, so
/// this is safe, and it lets other CPUs with those features run the clone.
mod gcc_prio {
    pub(super) const NONE: u32 = 0;
    pub(super) const X86_64_BASELINE: u32 = 4;
    pub(super) const PROC_SSE4_2: u32 = 12;
    pub(super) const X86_64_V2: u32 = 14;
    pub(super) const PCLMUL: u32 = 16;
    pub(super) const PROC_AVX: u32 = 18;
    pub(super) const PROC_AVX2: u32 = 28;
    pub(super) const X86_64_V3: u32 = 29;
    pub(super) const PROC_AVX512F: u32 = 31;
    pub(super) const X86_64_V4: u32 = 32;
    pub(super) const PROC_AVX10_1: u32 = 35;

    /// `isa_names_table` priority of a single feature name. Names GCC
    /// lists as P_NONE (`lzcnt`, `movbe`, `cmpxchg16b`, `f16c`, the AVX-512
    /// sub-features, ...) and spellings it does not list rank lowest; ties
    /// keep the user's order.
    pub(super) fn feature(name: &str) -> u32 {
        match name {
            "mmx" => 1,
            "sse" => 2,
            "sse2" => 3,
            "sse3" => 5,
            "ssse3" => 6,
            "sse4a" => 8,
            "sse4.1" | "sse4_1" => 10,
            "sse4.2" | "sse4_2" => 11,
            "popcnt" => 13,
            "aes" => 15,
            "pclmul" => PCLMUL,
            "avx" => 17,
            "bmi" => 19,
            "fma4" => 21,
            "xop" => 22,
            "fma" => 24,
            "bmi2" => 26,
            "avx2" => 27,
            "avx512f" => 30,
            _ => NONE,
        }
    }
}

/// One of GCC 16.2's errors for an x86 `target("...")` string.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetAttributeError {
    message: String,
    /// GCC reports it at the function's name (the argument errors); the
    /// CPU-name errors go to the start of the declaration.
    at_name: bool,
}

/// GCC 16.2's errors for an x86 `target("...")` string, in GCC's order;
/// empty when it is accepted. Measured on `gcc-16 -c`, message for message.
///
/// GCC reads the string twice, and an error of the first reading ends the
/// second.
///
/// 1. Argument by argument, split at the commas and taken as written -- a
///    blank is part of the name, so `"avx2, sse"` names ` sse` -- with only
///    a trailing comma forgiven. An argument is a plain name GCC knows
///    ([`X86_TARGET_ATTRIBUTE_NAMES`], 119 of them), or one of the four
///    `KEY=value` forms, either behind an optional `no-`. Anything else is
///    "attribute 'target' argument '...' is unknown", once per argument.
///    lccc took every such string in silence: `target("avx2,ssse4.1")`
///    compiled its function without the misspelt extension, and
///    `target("bmi 2")`, `target("AVX2")` and `target("-mavx2")` without
///    any, where GCC stops the build. A closed-list key with a value off
///    its list (`fpmath=`, `prefer-vector-width=`), a second `arch=` or
///    `tune=`, and `no-general-regs-only` are errors of this reading too;
///    the last one ends it on the spot.
/// 2. The CPU names: `arch=generic`, a name GCC does not know (`native`
///    included), a 32-bit-only CPU on x86-64, then the same for `tune=`
///    (which does take `native` and `generic`).
///
/// A name of the list that lccc does not model (`rdseed`, `clwb`, ...)
/// is accepted and changes nothing, as before.
///
/// [`X86_TARGET_ATTRIBUTE_NAMES`]: crate::backend::x86::arch_table::X86_TARGET_ATTRIBUTE_NAMES
fn x86_target_attribute_errors(raw: &str, x86_64: bool) -> Vec<TargetAttributeError> {
    use crate::backend::x86::arch_table::{
        X86_TARGET_ATTRIBUTE_FPMATH, X86_TARGET_ATTRIBUTE_NO_NEGATION,
        X86_TARGET_ATTRIBUTE_VECTOR_WIDTH, x86_target_attribute_name_known, x86_tune_name_valid,
    };
    let at_name = |message: String| TargetAttributeError {
        message,
        at_name: true,
    };
    let at_declaration = |message: String| TargetAttributeError {
        message,
        at_name: false,
    };
    // The whole string `default` is the function-multiversioning spelling
    // and no option; as one argument among others it is unknown.
    if raw == "default" {
        return Vec::new();
    }
    let mut arguments: Vec<&str> = raw.split(',').collect();
    if arguments.last() == Some(&"") {
        arguments.pop();
    }
    let mut errors = Vec::new();
    let (mut arch, mut tune) = (None, None);
    for argument in arguments {
        let body = argument.strip_prefix("no-").unwrap_or(argument);
        let unknown = || {
            at_name(format!(
                "attribute 'target' argument '{argument}' is unknown"
            ))
        };
        let Some((key, value)) = body.split_once('=') else {
            if !x86_target_attribute_name_known(body) {
                errors.push(unknown());
            } else if body.len() != argument.len()
                && X86_TARGET_ATTRIBUTE_NO_NEGATION.contains(&body)
            {
                errors.push(at_name(format!(
                    "pragma or attribute 'target(\"{body}\")' does not allow a negated form"
                )));
                return errors;
            }
            continue;
        };
        let cpu = match key {
            _ if value.is_empty() => {
                errors.push(unknown());
                continue;
            }
            "arch" => &mut arch,
            "tune" => &mut tune,
            "fpmath" | "prefer-vector-width" => {
                let values = if key == "fpmath" {
                    X86_TARGET_ATTRIBUTE_FPMATH
                } else {
                    X86_TARGET_ATTRIBUTE_VECTOR_WIDTH
                };
                if !values.contains(&value) {
                    errors.push(at_name(format!(
                        "attribute value '{body}' is unknown in 'target' attribute"
                    )));
                }
                continue;
            }
            _ => {
                errors.push(unknown());
                continue;
            }
        };
        if cpu.is_some() {
            errors.push(at_name(format!(
                "attribute value '{body}' was already specified in 'target' attribute"
            )));
        } else {
            *cpu = Some(value);
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    const NOT_X86_64: &str = "CPU you selected does not support x86-64 instruction set";
    if let Some(arch) = arch {
        if arch == "generic" {
            errors.push(at_declaration(
                "'generic' CPU can be used only for 'target(\"tune=\")' attribute".into(),
            ));
            return errors;
        }
        match crate::backend::x86::isa::X86Isa::arch_row(arch) {
            None => errors.push(at_declaration(format!(
                "bad value '{arch}' for 'target(\"arch=\")' attribute"
            ))),
            Some(row) if x86_64 && !row.x86_64 => {
                errors.push(at_declaration(NOT_X86_64.into()));
                return errors;
            }
            Some(_) => {}
        }
    }
    if let Some(tune) = tune.filter(|&tune| tune != "native" && !x86_tune_name_valid(tune, x86_64))
    {
        errors.push(at_declaration(
            if x86_64 && x86_tune_name_valid(tune, false) {
                NOT_X86_64.into()
            } else {
                format!("bad value '{tune}' for 'target(\"tune=\")' attribute")
            },
        ));
    }
    errors
}

impl Lowerer {
    /// Diagnose a function's x86 `target("...")` string as GCC does.
    pub(super) fn diagnose_x86_target_attribute(&self, func: &FunctionDef) {
        if !matches!(self.target, Target::X86_64 | Target::I686) {
            return;
        }
        let Some(raw) = func.attrs.target.as_deref() else {
            return;
        };
        for error in x86_target_attribute_errors(raw, self.target == Target::X86_64) {
            let span = match func.name_span {
                Some(name) if error.at_name => name,
                _ => func.span,
            };
            self.diagnostics.borrow_mut().error(error.message, span);
        }
    }

    /// Lower a `FunctionDef` carrying `__attribute__((target_clones(...)))`.
    pub(super) fn lower_target_clones_function(&mut self, func: &FunctionDef, raw_clones: &str) {
        // GCC 16.2 ignores `target_clones` beside `target` (with this
        // warning) and compiles the plain function under its `target`.
        if func
            .attrs
            .target
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
        {
            self.diagnostics.borrow_mut().warning_with_kind(
                "ignoring attribute 'target_clones' because it conflicts with attribute 'target'",
                func.span,
                crate::common::error::WarningKind::Attributes,
            );
            let mut plain = func.clone();
            plain.attrs.target_clones = None;
            self.lower_function(&plain);
            return;
        }
        let specs = parse_target_clones_list(raw_clones);
        if !specs.iter().any(|s| s == "default") {
            self.diagnostics.borrow_mut().error(
                "'target_clones' attribute requires a \"default\" target",
                func.span,
            );
            let mut plain = func.clone();
            plain.attrs.target_clones = None;
            self.lower_function(&plain);
            return;
        }

        // Non-x86 targets: if only `"default"` or target-neutral fallback is
        // needed, lower `"default"` directly unless an x86-only ISA is named.
        if !matches!(self.target, Target::X86_64 | Target::I686) {
            for s in &specs {
                if s != "default" && crate::common::x86_features::X86Feature::parse(s).is_some() {
                    self.diagnostics.borrow_mut().error(
                        format!(
                            "ISA '{s}' is not supported in 'target_clones' attribute on this target"
                        ),
                        func.span,
                    );
                    return;
                }
            }
            let mut plain = func.clone();
            plain.attrs.target_clones = None;
            self.lower_function(&plain);
            return;
        }

        // Single `"default"` clone degenerates to the plain function.
        if specs.len() == 1 {
            let mut plain = func.clone();
            plain.attrs.target_clones = None;
            self.lower_function(&plain);
            return;
        }

        let mut resolved_non_default: Vec<(usize, u32, String, String, Vec<Probe>)> = Vec::new();
        for (idx, spec) in specs.iter().enumerate() {
            if spec == "default" {
                continue;
            }
            match resolve_clone_spec(spec) {
                Ok((prio, probes)) => {
                    let suffix = mangle_clone_suffix(spec);
                    let clone_sym = format!("{}.{}", func.name, suffix);
                    resolved_non_default.push((idx, prio, spec.clone(), clone_sym, probes));
                }
                Err(msg) => {
                    self.diagnostics.borrow_mut().error(msg, func.span);
                    return;
                }
            }
        }

        let default_sym = format!("{}.default", func.name);

        // Emit `<name>.default` + every specialized clone as internal functions.
        let mut emit_one = |clone_name: &str, extra_target: Option<&str>| {
            let mut cloned = func.clone();
            cloned.name = clone_name.to_string();
            cloned.attrs.target_clones = None;
            cloned.attrs.set_static(true);
            cloned.attrs.set_extern(false);
            cloned.attrs.set_used(true);
            cloned.attrs.visibility = None;
            cloned.attrs.symver = None;
            // No `target` here: one beside `target_clones` returned above.
            cloned.attrs.target = extra_target.map(str::to_string);
            self.lower_function(&cloned);
        };

        emit_one(&default_sym, None);
        for (_, _, spec, clone_sym, _) in &resolved_non_default {
            emit_one(clone_sym, Some(spec));
        }

        // Sort clones by descending ISA priority (tie-breaking by earlier user order).
        resolved_non_default.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        // Synthesize `<name>.resolver(void) -> void*`.
        let resolver_name = format!("{}.resolver", func.name);
        self.build_target_clones_resolver(&resolver_name, &default_sym, &resolved_non_default);

        // Bind `<name>` (honoring `__asm__("label")` if any) as STT_GNU_IFUNC -> `<name>.resolver`.
        let asm_name = self
            .asm_label_map
            .get(&func.name)
            .cloned()
            .unwrap_or_else(|| func.name.clone());
        let effective_vis =
            self.effective_def_visibility(func.attrs.is_static(), &func.attrs.visibility);
        self.defined_functions.insert(func.name.clone());
        self.module.ifuncs.push((
            asm_name,
            resolver_name,
            func.attrs.is_static(),
            effective_vis,
        ));
    }

    fn build_target_clones_resolver(
        &mut self,
        resolver_name: &str,
        default_sym: &str,
        candidates: &[(usize, u32, String, String, Vec<Probe>)],
    ) {
        let prev_state = self.func_state.take();
        let entry_label = self.fresh_label();
        let mut state = FunctionBuildState::new(resolver_name.to_string(), IrType::Ptr, false);
        state.current_label = entry_label;
        state.cpu_probe_init = true;
        cpu_probe::mark_runtime_needed();
        self.func_state = Some(state);

        for (_, _, _, clone_sym, probes) in candidates {
            let next_candidate = self.fresh_label();
            for (pi, &probe) in probes.iter().enumerate() {
                let pass_label = self.fresh_label();
                let bit_ok = self.emit_cpu_probe_read(probe);
                let is_last = pi + 1 == probes.len();
                let target_true = pass_label;
                self.terminate(Terminator::CondBranch {
                    cond: Operand::Value(bit_ok),
                    true_label: target_true,
                    false_label: next_candidate,
                });
                self.start_block(pass_label);
                if is_last {
                    let fn_addr = self.fresh_value();
                    self.emit(Instruction::GlobalAddr {
                        dest: fn_addr,
                        name: clone_sym.clone(),
                    });
                    self.terminate(Terminator::Return(Some(Operand::Value(fn_addr))));
                }
            }
            self.start_block(next_candidate);
        }

        let def_addr = self.fresh_value();
        self.emit(Instruction::GlobalAddr {
            dest: def_addr,
            name: default_sym.to_string(),
        });
        self.terminate(Terminator::Return(Some(Operand::Value(def_addr))));

        self.splice_cpu_probe_init();

        let Some(built) = self.func_state.take() else {
            self.func_state = prev_state;
            return;
        };
        self.func_state = prev_state;

        let ir_func = IrFunction {
            debug_param_indices: Vec::new(),
            debug_locals: Vec::new(),
            debug_scopes: Vec::new(),
            debug_inline_instances: Vec::new(),
            pass_policy: self.pass_policy,
            name: resolver_name.to_string(),
            return_type: IrType::Ptr,
            params: Vec::new(),
            blocks: built.blocks,
            is_variadic: false,
            is_declaration: false,
            is_static: true,
            is_inline: false,
            is_always_inline: false,
            x86_isa_needs: Default::default(),
            is_noinline: true,
            target_features: Vec::new(),
            next_value_id: built.next_value,
            fp_expr_tags: Default::default(),
            // Every label above came from the lowerer's global counter, as
            // for ordinary and nested functions (`func_lowering.rs`). A 0
            // here hands the next label-minting pass a live label.
            next_label: self.next_label,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: true,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            calling_convention: crate::common::types::CallingConvention::Default,
            is_fastcall: false,
            regparm: None,
            is_naked: false,
            no_instrument: true,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            ret_is_f128_sse: false,
            ret_is_vector128: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
            ssp: crate::ir::module::SspInfo {
                no_stack_protector: true,
                ..Default::default()
            },
            // Synthesized resolver: no source body.
            decl_span: crate::common::source::Span::dummy(),
            body_span: crate::common::source::Span::dummy(),
            terminator_spans: Default::default(),
        };
        self.defined_functions.insert(resolver_name.to_string());
        self.module.functions.push(ir_func);
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_clone_spec, x86_target_attribute_errors};

    /// The first verdict, as the tests below read it.
    fn x86_target_attribute_error(raw: &str, x86_64: bool) -> Option<String> {
        x86_target_attribute_errors(raw, x86_64)
            .into_iter()
            .next()
            .map(|error| error.message)
    }

    /// Every argument GCC 16.2 does not know is an error of its own, at the
    /// function's name; a known name lccc does not model is not one.
    #[test]
    fn unknown_target_attribute_arguments_are_errors() {
        let messages = |raw: &str| -> Vec<String> {
            let errors = x86_target_attribute_errors(raw, true);
            assert!(errors.iter().all(|e| e.at_name), "{raw}: {errors:?}");
            errors.into_iter().map(|e| e.message).collect()
        };
        let unknown =
            |argument: &str| format!("attribute 'target' argument '{argument}' is unknown");
        // Accepted: modelled names, names GCC knows and lccc does not model,
        // flags, negations, the four keys, a trailing comma.
        for raw in [
            "avx2",
            "no-avx512f,avx2",
            "rdseed",
            "clwb,no-xop,sse4a",
            "cld,no-recip,80387,no-ieee-fp",
            "general-regs-only",
            "sse4.1,sse4.2,sse4,no-sse4",
            "fpmath=sse",
            "fpmath=sse+387",
            "fpmath=sse,fpmath=387",
            "prefer-vector-width=256,prefer-vector-width=none",
            "arch=haswell,tune=native",
            "no-arch=k8",
            "avx2,sse,",
            "",
            "default",
        ] {
            assert_eq!(messages(raw), Vec::<String>::new(), "{raw}");
        }
        // Refused: GCC's wording, every unknown argument named as written.
        for (raw, argument) in [
            ("foo", "foo"),
            ("no-foo", "no-foo"),
            ("AVX2", "AVX2"),
            ("mavx2", "mavx2"),
            ("-mavx2", "-mavx2"),
            ("sse4_1", "sse4_1"),
            ("avx2, sse", " sse"),
            (" avx2", " avx2"),
            ("avx2 ", "avx2 "),
            (",avx2", ""),
            ("avx2,,sse", ""),
            ("no-", "no-"),
            ("no-no-sse", "no-no-sse"),
            ("avx2,bar=3", "bar=3"),
            ("sse=1", "sse=1"),
            ("arch=", "arch="),
            ("branch-cost=2", "branch-cost=2"),
            ("fpmath=sse,387", "387"),
            ("foo,arch=zzz", "foo"),
            ("arch=haswell,foo", "foo"),
            ("default,avx2", "default"),
            ("avx2,default", "default"),
            ("default,", "default"),
            ("no-default", "no-default"),
        ] {
            assert_eq!(messages(raw), vec![unknown(argument)], "{raw}");
        }
        assert_eq!(
            messages("sse,foo,bar"),
            vec![unknown("foo"), unknown("bar")]
        );
        assert_eq!(
            messages("fpmath=bogus"),
            vec!["attribute value 'fpmath=bogus' is unknown in 'target' attribute"]
        );
        assert_eq!(
            messages("prefer-vector-width=64,foo"),
            vec![
                "attribute value 'prefer-vector-width=64' is unknown in 'target' attribute"
                    .to_string(),
                unknown("foo"),
            ]
        );
        // The negated form GCC refuses ends the reading.
        assert_eq!(
            messages("no-general-regs-only,foo"),
            vec![
                "pragma or attribute 'target(\"general-regs-only\")' does not allow a negated form"
            ]
        );
        assert_eq!(
            messages("arch=k8,arch=k8,foo"),
            vec![
                "attribute value 'arch=k8' was already specified in 'target' attribute".to_string(),
                unknown("foo"),
            ]
        );
        assert_eq!(
            messages("no-arch=k8,arch=k8"),
            vec!["attribute value 'arch=k8' was already specified in 'target' attribute"]
        );
    }

    /// The CPU names of `arch=` and `tune=`, judged after the arguments and
    /// reported at the declaration.
    #[test]
    fn target_attribute_cpu_names_are_judged_like_gcc() {
        let messages = |raw: &str, x86_64: bool| -> Vec<String> {
            let errors = x86_target_attribute_errors(raw, x86_64);
            assert!(errors.iter().all(|e| !e.at_name), "{raw}: {errors:?}");
            errors.into_iter().map(|e| e.message).collect()
        };
        let bad = |value: &str, key: &str| {
            format!("bad value '{value}' for 'target(\"{key}=\")' attribute")
        };
        for raw in [
            "tune=native",
            "tune=generic",
            "tune=intel",
            "arch=haswell,tune=znver4",
        ] {
            assert_eq!(messages(raw, true), Vec::<String>::new(), "{raw}");
        }
        assert_eq!(messages("tune=zzz", true), vec![bad("zzz", "tune")]);
        assert_eq!(
            messages("tune=x86-64-v3", true),
            vec![bad("x86-64-v3", "tune")]
        );
        // Both names, `arch=` first whatever the order written.
        for raw in ["arch=zzz,tune=yyy", "tune=yyy,arch=zzz"] {
            assert_eq!(
                messages(raw, true),
                vec![bad("zzz", "arch"), bad("yyy", "tune")]
            );
        }
        let not_x86_64 = "CPU you selected does not support x86-64 instruction set";
        assert_eq!(messages("tune=i386", true), vec![not_x86_64]);
        assert_eq!(messages("arch=pentium4,tune=i386", true), vec![not_x86_64]);
        assert_eq!(messages("tune=i386", false), Vec::<String>::new());
        assert_eq!(
            messages("tune=generic,arch=generic", true),
            vec!["'generic' CPU can be used only for 'target(\"tune=\")' attribute"]
        );
    }

    /// The verdicts GCC 16.2 gives (measured on `gcc-16 -c`).
    #[test]
    fn target_attribute_strings_are_judged_like_gcc() {
        fn x64(raw: &str) -> Option<String> {
            x86_target_attribute_error(raw, true)
        }
        assert_eq!(x64("arch=haswell,no-avx2"), None);
        assert_eq!(x64("avx2,arch=x86-64,tune=znver4"), None);
        assert_eq!(x64("arch=core2"), None);
        assert_eq!(
            x64("arch=haswell,arch=x86-64").as_deref(),
            Some("attribute value 'arch=x86-64' was already specified in 'target' attribute")
        );
        assert_eq!(
            x64("tune=haswell,arch=x86-64,tune=znver4").as_deref(),
            Some("attribute value 'tune=znver4' was already specified in 'target' attribute")
        );
        assert_eq!(
            x64("arch=generic").as_deref(),
            Some("'generic' CPU can be used only for 'target(\"tune=\")' attribute")
        );
        for bad in ["bogus", "native", "x86-64-v1", "raptor-lake", "knl"] {
            assert_eq!(
                x64(&format!("arch={bad}")),
                Some(format!(
                    "bad value '{bad}' for 'target(\"arch=\")' attribute"
                ))
            );
        }
        assert_eq!(
            x64("arch=pentium4").as_deref(),
            Some("CPU you selected does not support x86-64 instruction set")
        );
        assert_eq!(x86_target_attribute_error("arch=pentium4", false), None);
        assert_eq!(x86_target_attribute_error("arch=i386,sse2", false), None);
    }

    fn prio(spec: &str) -> u32 {
        resolve_clone_spec(spec).expect(spec).0
    }

    /// An `arch=` clone is dispatched only where every feature its code may
    /// use is present (GCC 16.2's feature set for the CPU).
    #[test]
    fn arch_clones_probe_the_cpus_whole_feature_set() {
        use crate::common::cpu_probe::lookup;
        let probes = |spec: &str| resolve_clone_spec(spec).expect(spec).1;
        let has = |spec: &str, name: &str| probes(spec).contains(&lookup(name).expect(name));
        // Nehalem has the v2 set but no PCLMUL; the Westmere clone may use it.
        assert!(has("arch=westmere", "pclmul") && has("arch=westmere", "popcnt"));
        assert!(!has("arch=x86-64-v2", "pclmul"));
        assert!(has("arch=haswell", "rdrnd") && !has("arch=haswell", "aes"));
        assert!(has("arch=skylake", "aes"));
        assert!(has("arch=x86-64-v4", "avx512vl") && has("arch=znver4", "avx512bw"));
        assert!(!has("arch=x86-64-v3", "avx512f"));
        // Every `arch=` clone requires at least its level's features.
        for spec in [
            "arch=x86-64-v3",
            "arch=haswell",
            "arch=znver3",
            "arch=alderlake",
        ] {
            for name in [
                "avx2", "fma", "bmi2", "movbe", "cx16", "popcnt", "f16c", "lzcnt",
            ] {
                assert!(has(spec, name), "{spec} {name}");
            }
        }
    }

    #[test]
    fn clone_priorities_follow_gcc_dispatch_order() {
        // The orderings the previous table inverted.
        assert!(prio("popcnt") > prio("sse4.2"));
        assert!(prio("bmi") > prio("avx"));
        assert!(prio("bmi2") > prio("fma"));
        // GCC P_NONE features rank below every prioritised one.
        for none in ["lzcnt", "movbe", "f16c", "avx512bw", "avx512vl"] {
            assert!(prio(none) < prio("mmx"), "{none}");
        }
        // Levels outrank the CPUs of the same level; graniterapids is AVX10.1.
        let chain = [
            "sse2",
            "arch=x86-64",
            "sse4.2",
            "arch=nehalem",
            "popcnt",
            "arch=x86-64-v2",
            "arch=westmere",
            "avx",
            "arch=sandybridge",
            "bmi",
            "fma",
            "bmi2",
            "avx2",
            "arch=haswell",
            "arch=x86-64-v3",
            "avx512f",
            "arch=znver4",
            "arch=x86-64-v4",
            "arch=graniterapids",
        ];
        for pair in chain.windows(2) {
            assert!(prio(pair[0]) < prio(pair[1]), "{} < {}", pair[0], pair[1]);
        }
    }
}

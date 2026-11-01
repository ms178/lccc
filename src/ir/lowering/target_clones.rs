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

/// GCC 16.2's verdict on an x86 `target("...")` string, or `None` when it
/// is accepted: a repeated `key=` option, `arch=generic`, a CPU name GCC
/// does not know (`native` included), and a 32-bit-only CPU on x86-64 are
/// errors there, worded as GCC words them.
fn x86_target_attribute_error(raw: &str, x86_64: bool) -> Option<String> {
    let options: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .collect();
    for (index, option) in options.iter().enumerate() {
        if let Some((key, _)) = option.split_once('=')
            && options[..index]
                .iter()
                .any(|prev| prev.split_once('=').is_some_and(|(k, _)| k == key))
        {
            return Some(format!(
                "attribute value '{option}' was already specified in 'target' attribute"
            ));
        }
    }
    let arch = options.iter().find_map(|o| o.strip_prefix("arch="))?;
    if arch == "generic" {
        return Some("'generic' CPU can be used only for 'target(\"tune=\")' attribute".into());
    }
    match crate::backend::x86::isa::X86Isa::arch_row(arch) {
        None => Some(format!(
            "bad value '{arch}' for 'target(\"arch=\")' attribute"
        )),
        Some(row) if x86_64 && !row.x86_64 => {
            Some("CPU you selected does not support x86-64 instruction set".into())
        }
        Some(_) => None,
    }
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
        if let Some(message) = x86_target_attribute_error(raw, self.target == Target::X86_64) {
            self.diagnostics.borrow_mut().error(message, func.span);
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
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
            ssp: crate::ir::module::SspInfo {
                no_stack_protector: true,
                ..Default::default()
            },
            // Synthesized resolver: no source body.
            body_span: crate::common::source::Span::dummy(),
            terminator_spans: Default::default(),
        };
        self.defined_functions.insert(resolver_name.to_string());
        self.module.functions.push(ir_func);
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_clone_spec, x86_target_attribute_error};

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

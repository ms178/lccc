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

    const V2_FEATS: &[&str] = &["cx16", "popcnt", "sse3", "ssse3", "sse4.1", "sse4.2"];
    const V3_FEATS: &[&str] = &[
        "avx", "avx2", "bmi", "bmi2", "cx16", "f16c", "fma", "lzcnt", "movbe", "popcnt", "sse3",
        "ssse3", "sse4.1", "sse4.2",
    ];
    const V4_FEATS: &[&str] = &[
        "avx", "avx2", "avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl", "bmi", "bmi2",
        "cx16", "f16c", "fma", "lzcnt", "movbe", "popcnt", "sse3", "ssse3", "sse4.1", "sse4.2",
    ];

    match spec {
        "arch=x86-64" => Ok((200, lookup_all(&["sse2"])?)),
        "arch=x86-64-v2" | "arch=nehalem" | "arch=westmere" | "arch=silvermont"
        | "arch=goldmont" | "arch=tremont" => Ok((500, lookup_all(V2_FEATS)?)),
        "arch=sandybridge" | "arch=ivybridge" => Ok((
            610,
            lookup_all(&["avx", "cx16", "popcnt", "sse4.1", "sse4.2"])?,
        )),
        "arch=x86-64-v3" | "arch=haswell" | "arch=broadwell" | "arch=skylake"
        | "arch=alderlake" | "arch=raptorlake" | "arch=meteorlake" | "arch=arrowlake"
        | "arch=znver1" | "arch=znver2" | "arch=znver3" => Ok((800, lookup_all(V3_FEATS)?)),
        "arch=x86-64-v4"
        | "arch=skylake-avx512"
        | "arch=cascadelake"
        | "arch=cooperlake"
        | "arch=icelake-client"
        | "arch=icelake-server"
        | "arch=tigerlake"
        | "arch=sapphirerapids"
        | "arch=emeraldrapids"
        | "arch=graniterapids"
        | "arch=znver4"
        | "arch=znver5" => Ok((900, lookup_all(V4_FEATS)?)),
        other => {
            let probe = cpu_probe::lookup(other).ok_or_else(|| {
                format!("ISA '{other}' is not supported in 'target_clones' attribute")
            })?;
            let prio = match other {
                s if s.starts_with("avx512") => 850,
                "avx2" => 700,
                "fma" => 650,
                "avx" | "f16c" => 600,
                "bmi2" => 550,
                "sse4.2" | "sse4_2" => 490,
                "sse4.1" | "sse4_1" | "sse4" => 450,
                "popcnt" | "lzcnt" | "abm" | "bmi" | "movbe" | "cx16" => 420,
                "ssse3" => 350,
                "sse3" => 300,
                "sse2" => 200,
                "sse" | "mmx" => 150,
                _ => 100,
            };
            Ok((prio, vec![probe]))
        }
    }
}

impl Lowerer {
    /// Lower a `FunctionDef` carrying `__attribute__((target_clones(...)))`.
    pub(super) fn lower_target_clones_function(&mut self, func: &FunctionDef, raw_clones: &str) {
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
            if let Some(extra) = extra_target {
                cloned.attrs.target = Some(match &func.attrs.target {
                    Some(prev) if !prev.is_empty() => format!("{prev},{extra}"),
                    _ => extra.to_string(),
                });
            }
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
            is_noinline: true,
            target_features: Vec::new(),
            next_value_id: built.next_value,
            fp_expr_tags: Default::default(),
            next_label: 0,
            section: None,
            visibility: None,
            is_weak: false,
            is_used: true,
            has_inlined_calls: false,
            param_alloca_values: Vec::new(),
            uses_sret: false,
            is_fastcall: false,
            regparm: None,
            is_naked: false,
            no_instrument: true,
            global_init_label_blocks: Vec::new(),
            ret_eightbyte_classes: Vec::new(),
            ret_is_f128_sse: false,
            is_gnu_inline_def: false,
            loop_promoted_f64_values: Vec::new(),
        };
        self.defined_functions.insert(resolver_name.to_string());
        self.module.functions.push(ir_func);
    }
}

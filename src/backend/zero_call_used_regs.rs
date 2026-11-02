//! Post-peephole `-fzero-call-used-regs=` / `__attribute__((zero_call_used_regs("...")))`
//! register zeroing pass across x86-64, i686, AArch64, and RISC-V 64 (`MB-07e`).
//!
//! Runs after target peephole optimization (so dead-move peepholes such as
//! AArch64 `eliminate_dead_pre_ret_moves` cannot strip intentional zeroing
//! before `ret`) and before x86-64 `ipa_ra::verify` / CFI boundary synthesis.

use crate::backend::Target;
use crate::common::fx_hash::{FxHashMap, FxHashSet};
use crate::common::types::{EightbyteClass, IrType};
use crate::ir::instruction::Instruction;
use crate::ir::module::{IrFunction, IrModule};

/// GCC 16.2 `-fzero-call-used-regs=` / `__attribute__((zero_call_used_regs("...")))` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZeroCallUsedRegs {
    #[default]
    Skip,
    UsedGprArg,
    UsedGpr,
    UsedArg,
    Used,
    LeafyGprArg,
    LeafyGpr,
    LeafyArg,
    Leafy,
    AllGprArg,
    AllGpr,
    AllArg,
    All,
}

impl ZeroCallUsedRegs {
    /// Parse any of the 13 GCC 16.2 mode names.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "skip" => Some(Self::Skip),
            "used-gpr-arg" => Some(Self::UsedGprArg),
            "used-gpr" => Some(Self::UsedGpr),
            "used-arg" => Some(Self::UsedArg),
            "used" => Some(Self::Used),
            "leafy-gpr-arg" => Some(Self::LeafyGprArg),
            "leafy-gpr" => Some(Self::LeafyGpr),
            "leafy-arg" => Some(Self::LeafyArg),
            "leafy" => Some(Self::Leafy),
            "all-gpr-arg" => Some(Self::AllGprArg),
            "all-gpr" => Some(Self::AllGpr),
            "all-arg" => Some(Self::AllArg),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Resolve `leafy*` modes into `used*` (for leaf functions) or `all*` (for non-leaf).
    #[inline]
    pub fn resolve_leafy(self, is_leaf: bool) -> Self {
        match self {
            Self::LeafyGprArg => {
                if is_leaf {
                    Self::UsedGprArg
                } else {
                    Self::AllGprArg
                }
            }
            Self::LeafyGpr => {
                if is_leaf {
                    Self::UsedGpr
                } else {
                    Self::AllGpr
                }
            }
            Self::LeafyArg => {
                if is_leaf {
                    Self::UsedArg
                } else {
                    Self::AllArg
                }
            }
            Self::Leafy => {
                if is_leaf {
                    Self::Used
                } else {
                    Self::All
                }
            }
            other => other,
        }
    }

    #[inline]
    fn only_used(self) -> bool {
        matches!(
            self,
            Self::UsedGprArg | Self::UsedGpr | Self::UsedArg | Self::Used
        )
    }

    #[inline]
    fn only_arg(self) -> bool {
        matches!(
            self,
            Self::UsedGprArg | Self::UsedArg | Self::AllGprArg | Self::AllArg
        )
    }

    #[inline]
    fn include_fp(self) -> bool {
        matches!(self, Self::UsedArg | Self::Used | Self::AllArg | Self::All)
    }
}

/// Options controlling target-specific register zeroing encodings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ZeroRegsConfig {
    pub default_mode: ZeroCallUsedRegs,
    pub avx: bool,
    pub apx: bool,
    pub fp_simd_allowed: bool,
    pub global_regparm: u8,
}

/// Effective mode for `func`, honoring per-function `zero_call_used_regs` attribute override.
pub(crate) fn effective_func_mode(
    func: &IrFunction,
    module: &IrModule,
    default_mode: ZeroCallUsedRegs,
) -> ZeroCallUsedRegs {
    if func.is_naked || func.is_declaration {
        return ZeroCallUsedRegs::Skip;
    }
    module
        .zero_call_used_regs_attrs
        .get(&func.name)
        .copied()
        .unwrap_or(default_mode)
}

/// Apply `-fzero-call-used-regs=` across every defined function in `asm`.
pub(crate) fn apply_zero_call_used_regs(
    asm: String,
    target: Target,
    module: &IrModule,
    cfg: ZeroRegsConfig,
) -> String {
    let mut func_map: FxHashMap<&str, (&IrFunction, ZeroCallUsedRegs)> = FxHashMap::default();
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        let mode = effective_func_mode(func, module, cfg.default_mode);
        if mode != ZeroCallUsedRegs::Skip {
            let asm_name = module
                .asm_labels
                .get(&func.name)
                .map(|s| s.as_str())
                .unwrap_or(func.name.as_str());
            func_map.insert(asm_name, (func, mode));
        }
    }
    if func_map.is_empty() {
        return asm;
    }

    let lines: Vec<&str> = asm.lines().collect();
    let mut out = String::with_capacity(asm.len() + 256);
    let mut idx = 0usize;
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if let Some(label) = trimmed.strip_suffix(':')
            && let Some(&(func, raw_mode)) = func_map.get(label)
        {
            let end = find_function_end(&lines, idx + 1, label);
            rewrite_function_slice(&lines[idx..end], target, func, raw_mode, cfg, &mut out);
            idx = end;
            continue;
        }
        out.push_str(lines[idx]);
        out.push('\n');
        idx += 1;
    }
    out
}

/// Locate the exclusive end line index of `label:`'s body (`... .size label, .-label`).
fn find_function_end(lines: &[&str], start: usize, label: &str) -> usize {
    let mut in_app = false;
    for (offset, line) in lines[start..].iter().enumerate() {
        let t = line.trim();
        if t == "#APP" {
            in_app = true;
            continue;
        }
        if t == "#NO_APP" {
            in_app = false;
            continue;
        }
        if in_app {
            continue;
        }
        if let Some(rest) = t.strip_prefix(".size ")
            && let Some((name, _)) = rest.split_once(',')
            && name.trim() == label
        {
            return start + offset + 1;
        }
        if t == ".cfi_endproc" {
            return start + offset + 1;
        }
    }
    lines.len()
}

fn rewrite_function_slice(
    slice: &[&str],
    target: Target,
    func: &IrFunction,
    raw_mode: ZeroCallUsedRegs,
    cfg: ZeroRegsConfig,
    out: &mut String,
) {
    let is_leaf = !ir_has_calls(func) && !asm_has_calls(slice, target);
    let mode = raw_mode.resolve_leafy(is_leaf);
    if mode == ZeroCallUsedRegs::Skip {
        for line in slice {
            out.push_str(line);
            out.push('\n');
        }
        return;
    }

    let zero_insns = build_zero_sequence(slice, target, func, mode, cfg);
    if zero_insns.is_empty() {
        for line in slice {
            out.push_str(line);
            out.push('\n');
        }
        return;
    }

    let mut in_app = false;
    for line in slice {
        let t = line.trim();
        if t == "#APP" {
            in_app = true;
        } else if t == "#NO_APP" {
            in_app = false;
        } else if !in_app && is_return_insn(t, target) {
            for z in &zero_insns {
                out.push_str("    ");
                out.push_str(z);
                out.push('\n');
            }
        }
        out.push_str(line);
        out.push('\n');
    }
}

fn ir_has_calls(func: &IrFunction) -> bool {
    func.blocks.iter().any(|b| {
        b.instructions.iter().any(|i| {
            matches!(
                i,
                Instruction::Call { .. } | Instruction::CallIndirect { .. }
            )
        })
    })
}

fn asm_has_calls(slice: &[&str], target: Target) -> bool {
    let mut in_app = false;
    for line in slice {
        let t = line.trim();
        if t == "#APP" {
            in_app = true;
            continue;
        }
        if t == "#NO_APP" {
            in_app = false;
            continue;
        }
        if in_app || t.is_empty() || t.starts_with('.') || t.starts_with('#') || t.ends_with(':') {
            continue;
        }
        let mnem = t.split_whitespace().next().unwrap_or("");
        let is_call = match target {
            Target::X86_64 | Target::I686 => mnem == "call" || mnem == "callq" || mnem == "calll",
            Target::Aarch64 => mnem == "bl" || mnem == "blr",
            Target::Riscv64 => {
                mnem == "call" || mnem == "jalr" || mnem.starts_with("jal\t") || mnem == "jal"
            }
        };
        if is_call {
            return true;
        }
    }
    false
}

fn is_return_insn(trimmed: &str, target: Target) -> bool {
    match target {
        Target::X86_64 | Target::I686 => {
            trimmed == "ret"
                || trimmed == "retq"
                || trimmed == "retl"
                || trimmed.starts_with("ret $")
                || trimmed == "jmp __x86_return_thunk"
        }
        Target::Aarch64 | Target::Riscv64 => trimmed == "ret",
    }
}

#[derive(Clone, Copy)]
struct RegCandidate {
    /// Canonical register key (`"rdi"`, `"xmm0"`, `"x0"`, `"v0"`, `"a0"`, `"fa0"`).
    key: &'static str,
    is_arg: bool,
    is_fp: bool,
}

fn build_zero_sequence(
    slice: &[&str],
    target: Target,
    func: &IrFunction,
    mode: ZeroCallUsedRegs,
    cfg: ZeroRegsConfig,
) -> Vec<String> {
    let live = live_return_regs(target, func);
    let used = if mode.only_used() {
        Some(scan_used_regs(slice, target))
    } else {
        None
    };
    let candidates = target_candidates(target, func, cfg);
    let mut out = Vec::new();
    for cand in candidates {
        if cand.is_fp && (!mode.include_fp() || !cfg.fp_simd_allowed) {
            continue;
        }
        if mode.only_arg() && !cand.is_arg {
            continue;
        }
        if live.contains(cand.key) {
            continue;
        }
        if let Some(ref used_set) = used
            && !used_set.contains(cand.key)
        {
            continue;
        }
        out.push(format_zero_insn(target, cand.key, cfg.avx));
    }
    out
}

fn target_candidates(target: Target, func: &IrFunction, cfg: ZeroRegsConfig) -> Vec<RegCandidate> {
    let mut v = Vec::with_capacity(32);
    match target {
        Target::X86_64 => {
            for k in ["rdi", "rsi", "rdx", "rcx", "r8", "r9"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: false,
                });
            }
            for k in ["rax", "r10", "r11"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: false,
                });
            }
            if cfg.apx {
                for k in [
                    "r16", "r17", "r18", "r19", "r20", "r21", "r22", "r23", "r24", "r25", "r26",
                    "r27", "r28", "r29", "r30", "r31",
                ] {
                    v.push(RegCandidate {
                        key: k,
                        is_arg: false,
                        is_fp: false,
                    });
                }
            }
            for k in [
                "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6", "xmm7",
            ] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: true,
                });
            }
            for k in [
                "xmm8", "xmm9", "xmm10", "xmm11", "xmm12", "xmm13", "xmm14", "xmm15",
            ] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: true,
                });
            }
        }
        Target::I686 => {
            let rp = func.regparm.unwrap_or(cfg.global_regparm).min(3) as usize;
            let (eax_arg, edx_arg, ecx_arg) = if func.is_fastcall {
                (false, true, true)
            } else {
                (rp >= 1, rp >= 2, rp >= 3)
            };
            v.push(RegCandidate {
                key: "eax",
                is_arg: eax_arg,
                is_fp: false,
            });
            v.push(RegCandidate {
                key: "edx",
                is_arg: edx_arg,
                is_fp: false,
            });
            v.push(RegCandidate {
                key: "ecx",
                is_arg: ecx_arg,
                is_fp: false,
            });
            for k in ["xmm0", "xmm1", "xmm2"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: true,
                });
            }
            for k in ["xmm3", "xmm4", "xmm5", "xmm6", "xmm7"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: true,
                });
            }
        }
        Target::Aarch64 => {
            for k in ["x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: false,
                });
            }
            // x8..x17 are caller-saved non-argument GPRs; x18 is the AAPCS64
            // platform register and must never be clobbered.
            for k in [
                "x8", "x9", "x10", "x11", "x12", "x13", "x14", "x15", "x16", "x17",
            ] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: false,
                });
            }
            for k in ["v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: true,
                });
            }
            // v8..v15 have callee-saved low 64 bits (d8..d15) under AAPCS64;
            // only v16..v31 are purely caller-saved non-argument SIMD regs.
            for k in [
                "v16", "v17", "v18", "v19", "v20", "v21", "v22", "v23", "v24", "v25", "v26", "v27",
                "v28", "v29", "v30", "v31",
            ] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: true,
                });
            }
        }
        Target::Riscv64 => {
            for k in ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: false,
                });
            }
            for k in ["t0", "t1", "t2", "t3", "t4", "t5", "t6"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: false,
                });
            }
            for k in ["fa0", "fa1", "fa2", "fa3", "fa4", "fa5", "fa6", "fa7"] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: true,
                    is_fp: true,
                });
            }
            for k in [
                "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "ft8", "ft9", "ft10",
                "ft11",
            ] {
                v.push(RegCandidate {
                    key: k,
                    is_arg: false,
                    is_fp: true,
                });
            }
        }
    }
    v
}

fn format_zero_insn(target: Target, key: &str, avx: bool) -> String {
    match target {
        Target::X86_64 => {
            if key.starts_with("xmm") {
                if avx {
                    format!("vxorps %{key}, %{key}, %{key}")
                } else {
                    format!("xorps %{key}, %{key}")
                }
            } else {
                let r32 = x86_64_reg32_name(key);
                format!("xorl %{r32}, %{r32}")
            }
        }
        Target::I686 => {
            if key.starts_with("xmm") {
                if avx {
                    format!("vxorps %{key}, %{key}, %{key}")
                } else {
                    format!("xorps %{key}, %{key}")
                }
            } else {
                format!("xorl %{key}, %{key}")
            }
        }
        Target::Aarch64 => {
            if key.starts_with('v') {
                format!("movi {key}.2d, #0")
            } else {
                format!("mov {key}, #0")
            }
        }
        Target::Riscv64 => {
            if key.starts_with('f') {
                format!("fmv.d.x {key}, zero")
            } else {
                format!("li {key}, 0")
            }
        }
    }
}

fn x86_64_reg32_name(key: &str) -> &'static str {
    match key {
        "rax" => "eax",
        "rcx" => "ecx",
        "rdx" => "edx",
        "rsi" => "esi",
        "rdi" => "edi",
        "r8" => "r8d",
        "r9" => "r9d",
        "r10" => "r10d",
        "r11" => "r11d",
        "r16" => "r16d",
        "r17" => "r17d",
        "r18" => "r18d",
        "r19" => "r19d",
        "r20" => "r20d",
        "r21" => "r21d",
        "r22" => "r22d",
        "r23" => "r23d",
        "r24" => "r24d",
        "r25" => "r25d",
        "r26" => "r26d",
        "r27" => "r27d",
        "r28" => "r28d",
        "r29" => "r29d",
        "r30" => "r30d",
        "r31" => "r31d",
        _ => "eax",
    }
}

fn sets_second_fp(func: &IrFunction) -> (bool, bool) {
    let mut fp = false;
    let mut f128 = false;
    for b in &func.blocks {
        for inst in &b.instructions {
            match inst {
                Instruction::SetReturnF32Second { .. } | Instruction::SetReturnF64Second { .. } => {
                    fp = true;
                }
                Instruction::SetReturnF128Second { .. } => {
                    f128 = true;
                }
                _ => {}
            }
        }
    }
    (fp, f128)
}

fn live_return_regs(target: Target, func: &IrFunction) -> FxHashSet<&'static str> {
    let mut live = FxHashSet::default();
    let (sets_fp2, sets_f128_2) = sets_second_fp(func);
    match target {
        Target::X86_64 => {
            if func.uses_sret {
                live.insert("rax");
                return live;
            }
            if func.return_type == IrType::Void {
                return live;
            }
            let vec_ret =
                crate::common::sysv_classify::vector_reg_bytes_of(&func.ret_eightbyte_classes)
                    .is_some();
            let single_sse = func.ret_eightbyte_classes.len() == 1
                && func.ret_eightbyte_classes[0] == EightbyteClass::Sse;
            let sse_count = func
                .ret_eightbyte_classes
                .iter()
                .filter(|c| **c == EightbyteClass::Sse)
                .count();
            let int_count = func
                .ret_eightbyte_classes
                .iter()
                .filter(|c| **c == EightbyteClass::Integer)
                .count();

            if func.ret_is_f128_sse
                || vec_ret
                || single_sse
                || sse_count >= 1
                || matches!(
                    func.return_type,
                    IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64
                )
            {
                live.insert("xmm0");
            }
            if sets_fp2 || sse_count >= 2 {
                live.insert("xmm1");
            }
            if matches!(func.return_type, IrType::I128 | IrType::U128) {
                if func.ret_eightbyte_classes.is_empty() {
                    live.insert("rax");
                    live.insert("rdx");
                } else {
                    if int_count >= 1 {
                        live.insert("rax");
                    }
                    if int_count >= 2 {
                        live.insert("rdx");
                    }
                }
            } else if !func.ret_is_f128_sse
                && !vec_ret
                && !single_sse
                && !matches!(
                    func.return_type,
                    IrType::F32 | IrType::F64 | IrType::F128 | IrType::D32 | IrType::D64
                )
            {
                live.insert("rax");
            }
        }
        Target::I686 => {
            if func.uses_sret {
                live.insert("eax");
                return live;
            }
            if func.return_type == IrType::Void {
                return live;
            }
            let returns_half_xmm = func.return_type == IrType::U16 && func.ret_is_f128_sse;
            let returns_i686_xmm = func.return_type == IrType::I128
                && func.ret_eightbyte_classes.first() == Some(&EightbyteClass::Sse);
            let returns_gp3 = func.return_type == IrType::I128
                && func.ret_eightbyte_classes.len() == 3
                && func
                    .ret_eightbyte_classes
                    .iter()
                    .all(|c| *c == EightbyteClass::Integer);
            if returns_half_xmm || returns_i686_xmm {
                live.insert("xmm0");
            } else if !matches!(func.return_type, IrType::F32 | IrType::F64 | IrType::F128) {
                live.insert("eax");
                if matches!(
                    func.return_type,
                    IrType::I64 | IrType::U64 | IrType::I128 | IrType::U128
                ) {
                    live.insert("edx");
                }
                if returns_gp3 {
                    live.insert("ecx");
                }
            }
        }
        Target::Aarch64 => {
            // Note: on AAPCS64 sret passes the buffer address in x8; the caller
            // does NOT read x0 or x8 after return.
            if func.uses_sret || func.return_type == IrType::Void {
                return live;
            }
            if func.ret_is_f128_sse
                || matches!(
                    func.return_type,
                    IrType::F32 | IrType::F64 | IrType::F128 | IrType::D32 | IrType::D64
                )
            {
                live.insert("v0");
                if sets_fp2 || sets_f128_2 {
                    live.insert("v1");
                }
            } else {
                live.insert("x0");
                if matches!(func.return_type, IrType::I128 | IrType::U128) {
                    live.insert("x1");
                }
            }
        }
        Target::Riscv64 => {
            if func.uses_sret || func.return_type == IrType::Void {
                return live;
            }
            let half_fp = func.return_type == IrType::U16 && func.ret_is_f128_sse;
            if half_fp
                || matches!(
                    func.return_type,
                    IrType::F32 | IrType::F64 | IrType::D32 | IrType::D64
                )
            {
                live.insert("fa0");
                if sets_fp2 {
                    live.insert("fa1");
                }
            } else {
                live.insert("a0");
                if matches!(func.return_type, IrType::I128 | IrType::U128 | IrType::F128) {
                    live.insert("a1");
                }
            }
        }
    }
    live
}

fn scan_used_regs(slice: &[&str], target: Target) -> FxHashSet<&'static str> {
    let mut used = FxHashSet::default();
    for line in slice {
        let t = line.trim();
        if t.is_empty()
            || t.starts_with('.')
            || t.starts_with('#')
            || t.starts_with("//")
            || t.ends_with(':')
        {
            continue;
        }
        // Strip trailing comments.
        let code = t.split('#').next().unwrap_or(t);
        let code = code.split("//").next().unwrap_or(code);
        match target {
            Target::X86_64 => scan_x86_64_line(code, &mut used),
            Target::I686 => scan_i686_line(code, &mut used),
            Target::Aarch64 => scan_aarch64_line(code, &mut used),
            Target::Riscv64 => scan_riscv_line(code, &mut used),
        }
    }
    used
}

fn scan_x86_64_line(code: &str, used: &mut FxHashSet<&'static str>) {
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            if let Some(canon) = canon_x86_64_reg(&code[start..end]) {
                used.insert(canon);
            }
            i = end;
        } else {
            i += 1;
        }
    }
}

fn canon_x86_64_reg(r: &str) -> Option<&'static str> {
    match r {
        "rax" | "eax" | "ax" | "al" | "ah" => Some("rax"),
        "rcx" | "ecx" | "cx" | "cl" | "ch" => Some("rcx"),
        "rdx" | "edx" | "dx" | "dl" | "dh" => Some("rdx"),
        "rsi" | "esi" | "si" | "sil" => Some("rsi"),
        "rdi" | "edi" | "di" | "dil" => Some("rdi"),
        "r8" | "r8d" | "r8w" | "r8b" => Some("r8"),
        "r9" | "r9d" | "r9w" | "r9b" => Some("r9"),
        "r10" | "r10d" | "r10w" | "r10b" => Some("r10"),
        "r11" | "r11d" | "r11w" | "r11b" => Some("r11"),
        "r16" | "r16d" | "r16w" | "r16b" => Some("r16"),
        "r17" | "r17d" | "r17w" | "r17b" => Some("r17"),
        "r18" | "r18d" | "r18w" | "r18b" => Some("r18"),
        "r19" | "r19d" | "r19w" | "r19b" => Some("r19"),
        "r20" | "r20d" | "r20w" | "r20b" => Some("r20"),
        "r21" | "r21d" | "r21w" | "r21b" => Some("r21"),
        "r22" | "r22d" | "r22w" | "r22b" => Some("r22"),
        "r23" | "r23d" | "r23w" | "r23b" => Some("r23"),
        "r24" | "r24d" | "r24w" | "r24b" => Some("r24"),
        "r25" | "r25d" | "r25w" | "r25b" => Some("r25"),
        "r26" | "r26d" | "r26w" | "r26b" => Some("r26"),
        "r27" | "r27d" | "r27w" | "r27b" => Some("r27"),
        "r28" | "r28d" | "r28w" | "r28b" => Some("r28"),
        "r29" | "r29d" | "r29w" | "r29b" => Some("r29"),
        "r30" | "r30d" | "r30w" | "r30b" => Some("r30"),
        "r31" | "r31d" | "r31w" | "r31b" => Some("r31"),
        "xmm0" | "ymm0" | "zmm0" => Some("xmm0"),
        "xmm1" | "ymm1" | "zmm1" => Some("xmm1"),
        "xmm2" | "ymm2" | "zmm2" => Some("xmm2"),
        "xmm3" | "ymm3" | "zmm3" => Some("xmm3"),
        "xmm4" | "ymm4" | "zmm4" => Some("xmm4"),
        "xmm5" | "ymm5" | "zmm5" => Some("xmm5"),
        "xmm6" | "ymm6" | "zmm6" => Some("xmm6"),
        "xmm7" | "ymm7" | "zmm7" => Some("xmm7"),
        "xmm8" | "ymm8" | "zmm8" => Some("xmm8"),
        "xmm9" | "ymm9" | "zmm9" => Some("xmm9"),
        "xmm10" | "ymm10" | "zmm10" => Some("xmm10"),
        "xmm11" | "ymm11" | "zmm11" => Some("xmm11"),
        "xmm12" | "ymm12" | "zmm12" => Some("xmm12"),
        "xmm13" | "ymm13" | "zmm13" => Some("xmm13"),
        "xmm14" | "ymm14" | "zmm14" => Some("xmm14"),
        "xmm15" | "ymm15" | "zmm15" => Some("xmm15"),
        _ => None,
    }
}

fn scan_i686_line(code: &str, used: &mut FxHashSet<&'static str>) {
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                end += 1;
            }
            let r = &code[start..end];
            let canon = match r {
                "eax" | "ax" | "al" | "ah" => Some("eax"),
                "ecx" | "cx" | "cl" | "ch" => Some("ecx"),
                "edx" | "dx" | "dl" | "dh" => Some("edx"),
                "xmm0" | "ymm0" => Some("xmm0"),
                "xmm1" | "ymm1" => Some("xmm1"),
                "xmm2" | "ymm2" => Some("xmm2"),
                "xmm3" | "ymm3" => Some("xmm3"),
                "xmm4" | "ymm4" => Some("xmm4"),
                "xmm5" | "ymm5" => Some("xmm5"),
                "xmm6" | "ymm6" => Some("xmm6"),
                "xmm7" | "ymm7" => Some("xmm7"),
                _ => None,
            };
            if let Some(c) = canon {
                used.insert(c);
            }
            i = end;
        } else {
            i += 1;
        }
    }
}

fn operands_part(code: &str) -> &str {
    let trimmed = code.trim();
    match trimmed.find(|c: char| c.is_ascii_whitespace()) {
        Some(pos) => trimmed[pos..].trim(),
        None => "",
    }
}

fn scan_aarch64_line(code: &str, used: &mut FxHashSet<&'static str>) {
    let mnem = code.split_whitespace().next().unwrap_or("");
    if matches!(mnem, "bl" | "b") || mnem.starts_with("b.") {
        return;
    }
    if mnem == "adrp" {
        if let Some(first_op) = operands_part(code).split(',').next() {
            let reg = first_op.trim();
            if let Some(c) = canon_aarch64_gpr(reg) {
                used.insert(c);
            }
        }
        return;
    }
    let ops = strip_aarch64_relocs(operands_part(code));
    for tok in ops.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if tok.is_empty() {
            continue;
        }
        let canon = canon_aarch64_gpr(tok).or_else(|| canon_aarch64_simd_reg(tok));
        if let Some(c) = canon {
            used.insert(c);
        }
    }
}

fn canon_aarch64_gpr(tok: &str) -> Option<&'static str> {
    match tok {
        "x0" | "w0" => Some("x0"),
        "x1" | "w1" => Some("x1"),
        "x2" | "w2" => Some("x2"),
        "x3" | "w3" => Some("x3"),
        "x4" | "w4" => Some("x4"),
        "x5" | "w5" => Some("x5"),
        "x6" | "w6" => Some("x6"),
        "x7" | "w7" => Some("x7"),
        "x8" | "w8" => Some("x8"),
        "x9" | "w9" => Some("x9"),
        "x10" | "w10" => Some("x10"),
        "x11" | "w11" => Some("x11"),
        "x12" | "w12" => Some("x12"),
        "x13" | "w13" => Some("x13"),
        "x14" | "w14" => Some("x14"),
        "x15" | "w15" => Some("x15"),
        "x16" | "w16" => Some("x16"),
        "x17" | "w17" => Some("x17"),
        _ => None,
    }
}

/// Remove `:lo12:sym` / `:got:sym` modifier-symbol pairs before register scanning.
fn strip_aarch64_relocs(ops: &str) -> String {
    let mut out = String::with_capacity(ops.len());
    let bytes = ops.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            // Skip `:modifier:symbol`.
            i += 1;
            while i < bytes.len() && bytes[i] != b':' && bytes[i] != b',' && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b':' {
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn canon_aarch64_simd_reg(tok: &str) -> Option<&'static str> {
    let first = tok.as_bytes().first().copied()?;
    if !matches!(first, b'v' | b'q' | b'd' | b's' | b'h' | b'b') {
        return None;
    }
    match &tok[1..] {
        "0" => Some("v0"),
        "1" => Some("v1"),
        "2" => Some("v2"),
        "3" => Some("v3"),
        "4" => Some("v4"),
        "5" => Some("v5"),
        "6" => Some("v6"),
        "7" => Some("v7"),
        "16" => Some("v16"),
        "17" => Some("v17"),
        "18" => Some("v18"),
        "19" => Some("v19"),
        "20" => Some("v20"),
        "21" => Some("v21"),
        "22" => Some("v22"),
        "23" => Some("v23"),
        "24" => Some("v24"),
        "25" => Some("v25"),
        "26" => Some("v26"),
        "27" => Some("v27"),
        "28" => Some("v28"),
        "29" => Some("v29"),
        "30" => Some("v30"),
        "31" => Some("v31"),
        _ => None,
    }
}

fn scan_riscv_line(code: &str, used: &mut FxHashSet<&'static str>) {
    let mnem = code.split_whitespace().next().unwrap_or("");
    if matches!(mnem, "call" | "tail" | "j" | "lla" | "la") {
        // `la rd, sym` / `lla rd, sym`: only the first operand is a register.
        if matches!(mnem, "lla" | "la")
            && let Some(first_op) = operands_part(code).split(',').next()
            && let Some(c) = canon_riscv_reg(first_op.trim())
        {
            used.insert(c);
        }
        return;
    }
    let ops = strip_riscv_relocs(operands_part(code));
    for tok in ops.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if tok.is_empty() {
            continue;
        }
        if let Some(c) = canon_riscv_reg(tok) {
            used.insert(c);
        }
    }
}

/// Strip `%hi(sym)` / `%lo(sym)` / `%pcrel_hi(sym)` / `%pcrel_lo(sym)` modifiers
/// while preserving any base register `(reg)` that follows.
fn strip_riscv_relocs(ops: &str) -> String {
    let mut out = String::with_capacity(ops.len());
    let bytes = ops.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'(' {
                let mut depth = 1usize;
                i += 1;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'(' {
                        depth += 1;
                    } else if bytes[i] == b')' {
                        depth -= 1;
                    }
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn canon_riscv_reg(tok: &str) -> Option<&'static str> {
    match tok {
        "a0" | "x10" => Some("a0"),
        "a1" | "x11" => Some("a1"),
        "a2" | "x12" => Some("a2"),
        "a3" | "x13" => Some("a3"),
        "a4" | "x14" => Some("a4"),
        "a5" | "x15" => Some("a5"),
        "a6" | "x16" => Some("a6"),
        "a7" | "x17" => Some("a7"),
        "t0" | "x5" => Some("t0"),
        "t1" | "x6" => Some("t1"),
        "t2" | "x7" => Some("t2"),
        "t3" | "x28" => Some("t3"),
        "t4" | "x29" => Some("t4"),
        "t5" | "x30" => Some("t5"),
        "t6" | "x31" => Some("t6"),
        "fa0" | "f10" => Some("fa0"),
        "fa1" | "f11" => Some("fa1"),
        "fa2" | "f12" => Some("fa2"),
        "fa3" | "f13" => Some("fa3"),
        "fa4" | "f14" => Some("fa4"),
        "fa5" | "f15" => Some("fa5"),
        "fa6" | "f16" => Some("fa6"),
        "fa7" | "f17" => Some("fa7"),
        "ft0" | "f0" => Some("ft0"),
        "ft1" | "f1" => Some("ft1"),
        "ft2" | "f2" => Some("ft2"),
        "ft3" | "f3" => Some("ft3"),
        "ft4" | "f4" => Some("ft4"),
        "ft5" | "f5" => Some("ft5"),
        "ft6" | "f6" => Some("ft6"),
        "ft7" | "f7" => Some("ft7"),
        "ft8" | "f28" => Some("ft8"),
        "ft9" | "f29" => Some("ft9"),
        "ft10" | "f30" => Some("ft10"),
        "ft11" | "f31" => Some("ft11"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_thirteen_gcc16_modes() {
        for (name, expected) in [
            ("skip", ZeroCallUsedRegs::Skip),
            ("used-gpr-arg", ZeroCallUsedRegs::UsedGprArg),
            ("used-gpr", ZeroCallUsedRegs::UsedGpr),
            ("used-arg", ZeroCallUsedRegs::UsedArg),
            ("used", ZeroCallUsedRegs::Used),
            ("leafy-gpr-arg", ZeroCallUsedRegs::LeafyGprArg),
            ("leafy-gpr", ZeroCallUsedRegs::LeafyGpr),
            ("leafy-arg", ZeroCallUsedRegs::LeafyArg),
            ("leafy", ZeroCallUsedRegs::Leafy),
            ("all-gpr-arg", ZeroCallUsedRegs::AllGprArg),
            ("all-gpr", ZeroCallUsedRegs::AllGpr),
            ("all-arg", ZeroCallUsedRegs::AllArg),
            ("all", ZeroCallUsedRegs::All),
        ] {
            assert_eq!(ZeroCallUsedRegs::parse(name), Some(expected));
        }
        assert_eq!(ZeroCallUsedRegs::parse("bogus"), None);
    }

    #[test]
    fn leafy_resolves_leaf_to_used_and_non_leaf_to_all() {
        assert_eq!(
            ZeroCallUsedRegs::LeafyGpr.resolve_leafy(true),
            ZeroCallUsedRegs::UsedGpr
        );
        assert_eq!(
            ZeroCallUsedRegs::LeafyGpr.resolve_leafy(false),
            ZeroCallUsedRegs::AllGpr
        );
        assert_eq!(
            ZeroCallUsedRegs::LeafyArg.resolve_leafy(true),
            ZeroCallUsedRegs::UsedArg
        );
        assert_eq!(
            ZeroCallUsedRegs::LeafyArg.resolve_leafy(false),
            ZeroCallUsedRegs::AllArg
        );
    }
}

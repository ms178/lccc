//! Stack Smashing Protection (`-fstack-protector{,-strong,-all,-explicit}`) and
//! target stack-guard configuration (`-mstack-protector-guard*`).
//!
//! Architecture:
//! - Source-level local variable classification (`SspInfo`) is recorded on each
//!   `IrFunction` during lowering and propagated across inlining.
//! - Right before target code generation (after all SSA optimization passes and
//!   phi elimination have completed), [`instrument_module`] inserts:
//!   1. A dedicated pointer-sized canary `Alloca` recorded in
//!      `func.ssp.stack_guard_alloca`, which `calculate_stack_space_common`
//!      places at the highest-address local slot (immediately adjacent to the
//!      saved frame/return state on all four architectures).
//!   2. Entry-block canary initialization from the configured guard source
//!      (TLS `%fs:40` on x86-64, TLS `%gs:20` on i686, `__stack_chk_guard` on
//!      AArch64/RISC-V, or explicit `-mstack-protector-guard*` overrides).
//!   3. Pre-return verification before every `Terminator::Return`, branching on
//!      mismatch to a shared cold failure block calling `__stack_chk_fail`.

use crate::backend::Target;
use crate::common::fx_hash::FxHashSet;
use crate::common::source::Span;
use crate::common::types::{AddressSpace, IrType};
use crate::ir::instruction::{
    BasicBlock, BlockId, CallInfo, Instruction, Operand, Terminator, Value,
};
use crate::ir::intrinsics::IntrinsicOp;
use crate::ir::module::{IrFunction, IrModule};
use crate::ir::ops::IrCmpOp;

/// Stack-protector policy selected on the CLI (`-fstack-protector*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackProtectorMode {
    /// `-fno-stack-protector` (default).
    #[default]
    None,
    /// `-fstack-protector`: functions with `char` arrays >= `ssp-buffer-size`,
    /// dynamic `alloca`/VLAs, or `__attribute__((stack_protect))`.
    Standard,
    /// `-fstack-protector-strong`: functions with any array local (including
    /// arrays inside structs/unions), address-taken locals, dynamic
    /// `alloca`/VLAs, or `__attribute__((stack_protect))`.
    Strong,
    /// `-fstack-protector-all`: every non-naked function without
    /// `no_stack_protector`.
    All,
    /// `-fstack-protector-explicit`: only functions carrying
    /// `__attribute__((stack_protect))`.
    Explicit,
}

impl StackProtectorMode {
    /// Predefined GCC feature macro (`__SSP*__`) for this mode, if active.
    pub fn predefined_macro(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::None => None,
            Self::Standard => Some(("__SSP__", "1")),
            Self::All => Some(("__SSP_ALL__", "2")),
            Self::Strong => Some(("__SSP_STRONG__", "3")),
            Self::Explicit => Some(("__SSP_EXPLICIT__", "4")),
        }
    }
}

/// Guard source kind (`-mstack-protector-guard={tls,global,sysreg}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackProtectorGuardKind {
    Tls,
    Global,
    Sysreg,
}

/// CLI-configured overrides for `-mstack-protector-guard*`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StackProtectorGuard {
    pub kind: Option<StackProtectorGuardKind>,
    pub reg: Option<String>,
    pub offset: Option<i64>,
    pub symbol: Option<String>,
}

/// Concrete guard access lowered for a specific [`Target`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveStackGuard {
    /// Segment/register-relative TLS read (`%fs:40`, `%gs:20`, `%gs:sym(%rip)`,
    /// `ld off(tp)`).
    Tls {
        reg: String,
        offset: i64,
        symbol: Option<String>,
    },
    /// External/TU global word (`__stack_chk_guard` or custom symbol).
    Global { symbol: String },
    /// AArch64 system register + byte offset (`mrs xN, sysreg; ldr x0, [xN, #off]`).
    Sysreg { reg: String, offset: i64 },
}

fn is_valid_asm_symbol(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || matches!(first, '_' | '.' | '$')) {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'))
}

impl StackProtectorGuard {
    /// Validate `-mstack-protector-guard*` options against the final [`Target`].
    pub fn validate_for_target(&self, target: Target) -> Result<(), String> {
        if let Some(ref sym) = self.symbol {
            if !is_valid_asm_symbol(sym) {
                return Err(format!(
                    "invalid '-mstack-protector-guard-symbol={sym}': not a valid symbol name"
                ));
            }
        }
        match target {
            Target::X86_64 | Target::I686 => {
                if self.kind == Some(StackProtectorGuardKind::Sysreg) {
                    return Err(
                        "'-mstack-protector-guard=sysreg' is not supported on x86 targets"
                            .to_string(),
                    );
                }
                if let Some(ref r) = self.reg {
                    let norm = r.strip_prefix('%').unwrap_or(r);
                    if !matches!(norm, "fs" | "gs") {
                        return Err(format!(
                            "invalid register '{r}' for '-mstack-protector-guard-reg='; expected 'fs' or 'gs'"
                        ));
                    }
                }
            }
            Target::Aarch64 => {
                if self.kind == Some(StackProtectorGuardKind::Sysreg) && self.reg.is_none() {
                    return Err(
                        "'-mstack-protector-guard=sysreg' requires '-mstack-protector-guard-reg='"
                            .to_string(),
                    );
                }
                if let Some(ref r) = self.reg {
                    if !is_valid_asm_symbol(r) {
                        return Err(format!(
                            "invalid system register '{r}' for '-mstack-protector-guard-reg='"
                        ));
                    }
                }
            }
            Target::Riscv64 => {
                if self.kind == Some(StackProtectorGuardKind::Sysreg) {
                    return Err(
                        "'-mstack-protector-guard=sysreg' is not supported on riscv64".to_string(),
                    );
                }
                if let Some(ref r) = self.reg {
                    if !is_valid_asm_symbol(r) {
                        return Err(format!(
                            "invalid register '{r}' for '-mstack-protector-guard-reg='"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve the concrete guard source for `target`.
    pub fn resolve(&self, target: Target) -> EffectiveStackGuard {
        let default_kind = match target {
            Target::X86_64 | Target::I686 => StackProtectorGuardKind::Tls,
            Target::Aarch64 | Target::Riscv64 => StackProtectorGuardKind::Global,
        };
        match self.kind.unwrap_or(default_kind) {
            StackProtectorGuardKind::Global => EffectiveStackGuard::Global {
                symbol: self
                    .symbol
                    .clone()
                    .unwrap_or_else(|| "__stack_chk_guard".to_string()),
            },
            StackProtectorGuardKind::Sysreg => EffectiveStackGuard::Sysreg {
                reg: self.reg.clone().unwrap_or_else(|| "sp_el0".to_string()),
                offset: self.offset.unwrap_or(0),
            },
            StackProtectorGuardKind::Tls => match target {
                Target::X86_64 => EffectiveStackGuard::Tls {
                    reg: self
                        .reg
                        .as_deref()
                        .map(|r| r.strip_prefix('%').unwrap_or(r).to_string())
                        .unwrap_or_else(|| "fs".to_string()),
                    offset: self.offset.unwrap_or(40),
                    symbol: self.symbol.clone(),
                },
                Target::I686 => EffectiveStackGuard::Tls {
                    reg: self
                        .reg
                        .as_deref()
                        .map(|r| r.strip_prefix('%').unwrap_or(r).to_string())
                        .unwrap_or_else(|| "gs".to_string()),
                    offset: self.offset.unwrap_or(20),
                    symbol: self.symbol.clone(),
                },
                Target::Aarch64 => EffectiveStackGuard::Sysreg {
                    reg: self.reg.clone().unwrap_or_else(|| "tpidr_el0".to_string()),
                    offset: self.offset.unwrap_or(40),
                },
                Target::Riscv64 => EffectiveStackGuard::Tls {
                    reg: self.reg.clone().unwrap_or_else(|| "tp".to_string()),
                    offset: self.offset.unwrap_or(0),
                    symbol: None,
                },
            },
        }
    }
}

fn has_dyn_alloca(func: &IrFunction) -> bool {
    func.blocks.iter().any(|b| {
        b.instructions
            .iter()
            .any(|i| matches!(i, Instruction::DynAlloca { .. }))
    })
}

/// True if any non-parameter local `Alloca` has its address taken (used as any
/// operand other than the direct `ptr` target of `Load` or `Store`).
fn has_escaped_alloca(func: &IrFunction) -> bool {
    let param_set: FxHashSet<u32> = func.param_alloca_values.iter().map(|v| v.0).collect();
    let mut local_allocas = FxHashSet::default();
    for block in &func.blocks {
        for inst in &block.instructions {
            if let Instruction::Alloca { dest, .. } = inst {
                if !param_set.contains(&dest.0) {
                    local_allocas.insert(dest.0);
                }
            }
        }
    }
    if local_allocas.is_empty() {
        return false;
    }
    for block in &func.blocks {
        for inst in &block.instructions {
            match inst {
                Instruction::Load { .. } => {}
                Instruction::Store { val, .. } => {
                    if let Operand::Value(v) = val {
                        if local_allocas.contains(&v.0) {
                            return true;
                        }
                    }
                }
                Instruction::GetElementPtr { base, offset, .. } => {
                    if local_allocas.contains(&base.0) {
                        return true;
                    }
                    if let Operand::Value(v) = offset {
                        if local_allocas.contains(&v.0) {
                            return true;
                        }
                    }
                }
                Instruction::Memcpy { dest, src, .. } => {
                    if local_allocas.contains(&dest.0) || local_allocas.contains(&src.0) {
                        return true;
                    }
                }
                _ => {
                    let mut escaped = false;
                    inst.for_each_used_value(|id| {
                        if local_allocas.contains(&id) {
                            escaped = true;
                        }
                    });
                    if escaped {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Decide whether `func` requires a stack-protector canary under `mode`.
pub fn function_needs_canary(
    func: &IrFunction,
    mode: StackProtectorMode,
    ssp_buffer_size: usize,
) -> bool {
    if func.is_declaration
        || func.is_naked
        || func.blocks.is_empty()
        || func.ssp.no_stack_protector
        || func.ssp.stack_guard_alloca.is_some()
    {
        return false;
    }
    match mode {
        StackProtectorMode::None => false,
        StackProtectorMode::Explicit => func.ssp.stack_protect,
        StackProtectorMode::All => true,
        StackProtectorMode::Standard => {
            func.ssp.stack_protect
                || has_dyn_alloca(func)
                || func.ssp.max_char_array_bytes as usize >= ssp_buffer_size
        }
        StackProtectorMode::Strong => {
            func.ssp.stack_protect
                || has_dyn_alloca(func)
                || func.ssp.has_array
                || has_escaped_alloca(func)
        }
    }
}

fn emit_guard_load(
    insts: &mut Vec<Instruction>,
    spans: &mut Vec<Span>,
    track_spans: bool,
    span: Span,
    next_val: &mut u32,
    effective: &EffectiveStackGuard,
    word_ty: IrType,
) -> Value {
    match effective {
        EffectiveStackGuard::Global { symbol } => {
            let addr = Value(*next_val);
            *next_val += 1;
            let val = Value(*next_val);
            *next_val += 1;
            insts.push(Instruction::GlobalAddr {
                dest: addr,
                name: symbol.clone(),
            });
            insts.push(Instruction::Load {
                dest: val,
                ptr: addr,
                ty: word_ty,
                seg_override: AddressSpace::Default,
                volatile: true,
            });
            if track_spans {
                spans.push(span);
                spans.push(span);
            }
            val
        }
        EffectiveStackGuard::Tls { .. } | EffectiveStackGuard::Sysreg { .. } => {
            let val = Value(*next_val);
            *next_val += 1;
            insts.push(Instruction::Intrinsic {
                dest: Some(val),
                op: IntrinsicOp::LoadStackGuard,
                dest_ptr: None,
                args: Vec::new(),
            });
            if track_spans {
                spans.push(span);
            }
            val
        }
    }
}

/// Instrument eligible functions in `module` with stack-protector canaries.
pub fn instrument_module(
    module: &mut IrModule,
    target: Target,
    mode: StackProtectorMode,
    ssp_buffer_size: usize,
    guard: &StackProtectorGuard,
) {
    if mode == StackProtectorMode::None {
        return;
    }
    let effective = guard.resolve(target);
    let ptr_size = if target == Target::I686 { 4 } else { 8 };
    let word_ty = if target == Target::I686 {
        IrType::U32
    } else {
        IrType::U64
    };
    let mut next_blk = module
        .functions
        .iter()
        .map(|f| {
            let max_in_blocks = f
                .blocks
                .iter()
                .map(|b| b.label.0)
                .chain(f.global_init_label_blocks.iter().map(|b| b.0))
                .max()
                .map_or(0, |m| m.saturating_add(1));
            std::cmp::max(f.next_label, max_in_blocks)
        })
        .max()
        .unwrap_or(0);
    for func in &mut module.functions {
        if !function_needs_canary(func, mode, ssp_buffer_size) {
            continue;
        }
        instrument_function(func, &effective, ptr_size, word_ty, &mut next_blk);
    }
}

fn instrument_function(
    func: &mut IrFunction,
    effective: &EffectiveStackGuard,
    ptr_size: usize,
    word_ty: IrType,
    next_blk: &mut u32,
) {
    let mut next_val = std::cmp::max(func.next_value_id, func.max_value_id().saturating_add(1));

    let canary_alloca = Value(next_val);
    next_val += 1;
    func.ssp.stack_guard_alloca = Some(canary_alloca);

    // 1. Insert canary alloca + guard load + volatile store in the entry block
    // right after leading Alloca / ParamRef instructions.
    {
        let entry = &mut func.blocks[0];
        let track_spans =
            !entry.source_spans.is_empty() && entry.source_spans.len() == entry.instructions.len();
        let entry_span = entry
            .source_spans
            .first()
            .copied()
            .unwrap_or_else(Span::dummy);
        let insert_pos = entry
            .instructions
            .iter()
            .position(|i| !matches!(i, Instruction::Alloca { .. } | Instruction::ParamRef { .. }))
            .unwrap_or(entry.instructions.len());

        let mut prologue_insts = Vec::with_capacity(4);
        let mut prologue_spans = Vec::with_capacity(4);
        prologue_insts.push(Instruction::Alloca {
            dest: canary_alloca,
            ty: word_ty,
            size: ptr_size,
            align: ptr_size,
            volatile: true,
            semantic_volatile: true,
        });
        if track_spans {
            prologue_spans.push(entry_span);
        }
        let g0 = emit_guard_load(
            &mut prologue_insts,
            &mut prologue_spans,
            track_spans,
            entry_span,
            &mut next_val,
            effective,
            word_ty,
        );
        prologue_insts.push(Instruction::Store {
            val: Operand::Value(g0),
            ptr: canary_alloca,
            ty: word_ty,
            seg_override: AddressSpace::Default,
            volatile: true,
        });
        if track_spans {
            prologue_spans.push(entry_span);
        }

        entry.instructions.splice(0..0, prologue_insts.drain(..1));
        if track_spans {
            entry.source_spans.splice(0..0, prologue_spans.drain(..1));
        }
        let store_pos = insert_pos + 1;
        entry
            .instructions
            .splice(store_pos..store_pos, prologue_insts);
        if track_spans {
            entry
                .source_spans
                .splice(store_pos..store_pos, prologue_spans);
        }
    }

    // 2. Rewrite every Return block to check the canary before returning.
    let orig_len = func.blocks.len();
    let mut fail_block_id: Option<BlockId> = None;
    let mut new_blocks: Vec<BasicBlock> = Vec::new();

    for bi in 0..orig_len {
        let ret_operand = match &func.blocks[bi].terminator {
            Terminator::Return(op) => *op,
            _ => continue,
        };
        let fail_lbl = *fail_block_id.get_or_insert_with(|| {
            let lbl = BlockId(*next_blk);
            *next_blk += 1;
            lbl
        });
        let ret_lbl = BlockId(*next_blk);
        *next_blk += 1;

        let blk = &mut func.blocks[bi];
        let track_spans =
            !blk.source_spans.is_empty() && blk.source_spans.len() == blk.instructions.len();
        let tail_span = blk.source_spans.last().copied().unwrap_or_else(Span::dummy);

        // Keep trailing SetReturn*Second instructions paired with Return in ret_block.
        let mut moved_insts = Vec::new();
        let mut moved_spans = Vec::new();
        while blk.instructions.last().is_some_and(|inst| {
            matches!(
                inst,
                Instruction::SetReturnF64Second { .. }
                    | Instruction::SetReturnF32Second { .. }
                    | Instruction::SetReturnF128Second { .. }
            )
        }) {
            if let Some(inst) = blk.instructions.pop() {
                moved_insts.push(inst);
            }
            if track_spans {
                if let Some(sp) = blk.source_spans.pop() {
                    moved_spans.push(sp);
                }
            }
        }
        moved_insts.reverse();
        moved_spans.reverse();

        let saved_val = Value(next_val);
        next_val += 1;
        blk.instructions.push(Instruction::Load {
            dest: saved_val,
            ptr: canary_alloca,
            ty: word_ty,
            seg_override: AddressSpace::Default,
            volatile: true,
        });
        if track_spans {
            blk.source_spans.push(tail_span);
        }
        let expected_val = emit_guard_load(
            &mut blk.instructions,
            &mut blk.source_spans,
            track_spans,
            tail_span,
            &mut next_val,
            effective,
            word_ty,
        );
        let cmp_val = Value(next_val);
        next_val += 1;
        blk.instructions.push(Instruction::Cmp {
            dest: cmp_val,
            op: IrCmpOp::Ne,
            lhs: Operand::Value(saved_val),
            rhs: Operand::Value(expected_val),
            ty: word_ty,
        });
        if track_spans {
            blk.source_spans.push(tail_span);
        }
        blk.terminator = Terminator::CondBranch {
            cond: Operand::Value(cmp_val),
            true_label: fail_lbl,
            false_label: ret_lbl,
        };

        new_blocks.push(BasicBlock {
            label: ret_lbl,
            instructions: moved_insts,
            terminator: Terminator::Return(ret_operand),
            source_spans: moved_spans,
        });
    }

    if let Some(fail_lbl) = fail_block_id {
        let info = CallInfo {
            dest: None,
            args: Vec::new(),
            arg_types: Vec::new(),
            return_type: IrType::Void,
            is_variadic: false,
            num_fixed_args: 0,
            struct_arg_sizes: Vec::new(),
            struct_arg_aligns: Vec::new(),
            struct_arg_classes: Vec::new(),
            struct_arg_riscv_float_classes: Vec::new(),
            struct_arg_is_f128_sse: Vec::new(),
            is_sret: false,
            calling_convention: crate::common::types::CallingConvention::Default,
            is_fastcall: false,
            regparm: None,
            is_pure: false,
            is_const: false,
            ret_eightbyte_classes: Vec::new(),
            assume_library: true,
            ret_is_f128_sse: false,
            cold: true,
        };
        new_blocks.push(BasicBlock {
            label: fail_lbl,
            instructions: vec![Instruction::Call {
                func: "__stack_chk_fail".to_string(),
                info,
            }],
            terminator: Terminator::Unreachable,
            source_spans: Vec::new(),
        });
    }

    func.blocks.extend(new_blocks);
    func.next_value_id = next_val;
    func.next_label = *next_blk;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::constants::IrConst;

    fn sample_func() -> IrFunction {
        let mut f = IrFunction::new("test_fn".to_string(), IrType::I32, Vec::new(), false);
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: Vec::new(),
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(42)))),
            source_spans: Vec::new(),
        });
        f.next_label = 1;
        f.next_value_id = 1;
        f
    }

    #[test]
    fn mode_selection_matches_gcc_triggers() {
        let mut f = sample_func();
        assert!(!function_needs_canary(&f, StackProtectorMode::Standard, 8));
        assert!(!function_needs_canary(&f, StackProtectorMode::Strong, 8));
        assert!(function_needs_canary(&f, StackProtectorMode::All, 8));

        f.ssp.max_char_array_bytes = 4;
        f.ssp.has_array = true;
        assert!(!function_needs_canary(&f, StackProtectorMode::Standard, 8));
        assert!(function_needs_canary(&f, StackProtectorMode::Strong, 8));

        f.ssp.max_char_array_bytes = 8;
        assert!(function_needs_canary(&f, StackProtectorMode::Standard, 8));

        f.ssp.no_stack_protector = true;
        assert!(!function_needs_canary(&f, StackProtectorMode::All, 8));
    }

    #[test]
    fn guard_validation_rejects_bad_registers() {
        let mut g = StackProtectorGuard {
            reg: Some("rax".to_string()),
            ..Default::default()
        };
        assert!(g.validate_for_target(Target::X86_64).is_err());
        g.reg = Some("gs".to_string());
        assert!(g.validate_for_target(Target::X86_64).is_ok());
    }
}

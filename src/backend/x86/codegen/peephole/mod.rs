//! x86-64 peephole optimizer for assembly text.
//!
//! Operates on generated assembly text to eliminate redundant patterns from the
//! stack-based codegen. Lines are pre-parsed into `LineInfo` structs so hot-path
//! pattern matching uses integer/enum comparisons instead of string parsing.
//!
//! ## Pass structure
//!
//! 1. **Local passes** (iterative, up to 8 rounds): `combined_local_pass` merges
//!    7 single-scan patterns (self-moves, reverse-moves, redundant jumps,
//!    conditional branch inversion, adjacent store/load, redundant zero/sign
//!    extensions, redundant xorl %eax,%eax) plus push/pop elimination and
//!    binary-op push/pop rewriting.
//!
//! 2. **Global passes** (once): global store forwarding (across fallthrough
//!    labels), register copy propagation, dead register move elimination,
//!    dead store elimination, compare-and-branch fusion, and memory operand
//!    folding.
//!
//! 3. **Post-global cleanup + loop trampolines + tail calls + never-read stores
//!    + callee-save elimination + frame compaction**: see `passes/mod.rs`.

mod passes;
mod types;

pub(crate) use passes::{
    peephole_optimize, peephole_optimize_pair_with_config, peephole_optimize_with_config,
};

/// Lower the opaque TLS-descriptor pseudo after optimization. During the
/// peephole it must be an unknown `%rax`-transforming instruction, not a normal
/// SysV call: the TLSDESC resolver ABI preserves all other register homes.
pub(crate) fn lower_tls_call_markers(asm: String) -> String {
    if !asm.contains("lccc_tls_call") {
        return asm;
    }
    asm.split_inclusive('\n')
        .map(|line| {
            let trimmed = line.trim();
            if let Some(symbol) = trimmed.strip_prefix("lccc_tls_call ") {
                format!("    call *{}@TLSCALL(%rax)\n", symbol)
            } else {
                line.to_string()
            }
        })
        .collect()
}

/// Delete the `# LCCC_VOLATILE_BEGIN` / `# LCCC_VOLATILE_END` markers from
/// assembly that bypasses the peephole (`LCCC_NO_PEEPHOLE=1`). The optimized
/// path deletes them in `pin_volatile_access_regions`; this keeps the escape
/// hatch's output marker-free too, so the bisection tool does not change what
/// the assembler or an asm-diff sees.
pub(crate) fn strip_volatile_markers(asm: String) -> String {
    crate::backend::peephole_common::strip_volatile_markers(asm)
}

#[cfg(test)]
mod tls_call_tests {
    use super::lower_tls_call_markers;

    #[test]
    fn tls_call_marker_lowers_only_after_optimization() {
        let input = "    leaq x@TLSDESC(%rip), %rax\n    lccc_tls_call x\n    addq %fs:0, %rax\n";
        let output = lower_tls_call_markers(input.to_string());
        assert_eq!(
            output,
            "    leaq x@TLSDESC(%rip), %rax\n    call *x@TLSCALL(%rax)\n    addq %fs:0, %rax\n"
        );
        assert!(!output.contains("lccc_tls_call"));
    }
}

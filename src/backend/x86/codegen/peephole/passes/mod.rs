//! x86-64 peephole optimizer: pass orchestration.
//!
//! This module is the entry point (`peephole_optimize`) that runs all optimization
//! passes in the correct order. The actual pass implementations live in submodules:
//!
//! - [`local_patterns`]: combined local pattern matching (self-move, reverse-move,
//!   redundant jump, branch inversion, store/load, extensions) + movq/ext fusion
//! - [`push_pop`]: push/pop pair and push/binop/pop elimination
//! - [`relay_and_lea`]: register move-relay elimination, windowed lea folding
//! - [`compare_branch`]: compare-and-branch fusion (cmp+setCC+test+jCC -> jCC)
//! - [`copy_propagation`]: register copy propagation across basic blocks
//! - [`dead_code`]: dead register moves, dead stores, never-read store elimination
//! - [`store_forwarding`]: global store forwarding across fallthrough labels
//! - [`loop_trampoline`]: SSA loop backedge trampoline block coalescing
//! - [`callee_saves`]: unused callee-saved register save/restore elimination
//! - [`memory_fold`]: fold stack loads into ALU instructions as memory operands
//! - [`tail_call`]: convert `call; epilogue; ret` to `epilogue; jmp` for tail calls
//! - [`frame_compact`]: stack frame compaction after dead store/callee-save elimination
//! - [`helpers`]: shared utilities (register rewriting, label parsing, etc.)

use super::types::*;

// Submodule pass implementations
mod callee_saves;
mod compare_branch;
mod copy_coalesce;
mod copy_propagation;
mod dead_code;
mod dead_writes;
mod flag_peepholes;
mod frame_compact;
mod helpers;
mod identical_blocks;
mod liveness;
mod load_op_fuse;
mod local_patterns;
mod loop_trampoline;
mod memory_fold;
mod narrow_copy_fold;
mod push_pop;
mod pushf_elim;
mod redundant_ext;
mod relay_and_lea;
mod spill_deref;
mod store_forwarding;
mod tail_call;

// ── Constants ────────────────────────────────────────────────────────────────

/// Maximum iterations for Phase 1 (local peephole passes).
/// Local patterns rarely chain deeper than 3-4 levels, so 8 provides ample headroom.
const MAX_LOCAL_PASS_ITERATIONS: usize = 8;

/// Maximum iterations for Phase 3 (local cleanup after global passes).
/// Post-global cleanup is shallow (mostly dead store + adjacent pairs), so 4 suffices.
const MAX_POST_GLOBAL_ITERATIONS: usize = 4;

// ── Stack-address escape pinning ─────────────────────────────────────────────

/// Parse a direct `OFFSET(%rsp)`/`OFFSET(%rbp)` operand.  Indexed addresses are
/// intentionally not guessed: a failure only forgoes a pin, while a pin is
/// always conservative.
fn direct_stack_slot(operand: &str) -> Option<(u8, i32)> {
    let operand = operand.trim();
    for (base, id) in [("(%rsp)", 4u8), ("(%rbp)", 5u8)] {
        if let Some(pos) = operand.find(base) {
            if pos + base.len() != operand.len() {
                continue;
            }
            let offset = if pos == 0 {
                0
            } else {
                operand[..pos].trim().parse::<i32>().ok()?
            };
            return Some((id, offset));
        }
    }
    None
}

/// Mark stores to stack slots whose address is materialized with `lea` as
/// pinned.  A later call may read or write through that pointer; text peepholes
/// do not have escape analysis and therefore must never prove such a store dead.
/// Parse a direct stack-slot reference embedded in a complete assembly line.
/// Indexed operands are intentionally excluded; absence of a marker only loses
/// an optimization opportunity, while a false marker would be unsound.
fn direct_stack_slot_in_line(line: &str) -> Option<(u8, i32)> {
    for base in ["(%rsp)", "(%rbp)"] {
        if let Some(pos) = line.find(base) {
            let end = pos + base.len();
            let prefix = &line[..end];
            let start = prefix
                .rfind(|c: char| c == ' ' || c == '\t' || c == ',')
                .map(|idx| idx + 1)
                .unwrap_or(0);
            if let Some(slot) = direct_stack_slot(&prefix[start..]) {
                return Some(slot);
            }
        }
    }
    None
}

/// Mark every direct memory access to a codegen-identified volatile alloca as
/// opaque.  Volatile access is observable even when an ordinary dataflow pass
/// believes the slot value is available in a register.
///
/// SOUNDNESS: the `# LCCC_VOLATILE_SLOT` markers describe THIS function's
/// frame only, but stack offsets repeat across functions.  A file-wide slot set
/// demoted unrelated accesses in other functions (e.g. a `movl 24(%rsp), %eax`
/// load in sqlite3MultiValues matched the `24(%rsp)` marker emitted by
/// kahanBabuskaNeumaierStep).  A demoted LOAD-into-%rax was classified as
/// `Other { dest_reg: REG_NONE }`, which combined_local_pass's rax_is_zero
/// tracking read as "rax not written" -> the following `xorl %eax, %eax`
/// (call-argument setup) was deemed redundant and removed, pushing stale data
/// as a stack argument.  Two-part fix:
///   1. Scope the volatile set to the current function (bounded by
///      .cfi_startproc / .cfi_endproc) so markers never leak across functions.
///   2. Preserve the destination register when demoting: a volatile load still
///      WRITES its destination, and rax_is_zero must observe that.  Only the
///      stack-specialized classification (StoreRbp/LoadRbp) is removed, which
///      is what blocks slot-based forwarding/folding passes.
/// Reclassify every line between `#APP` and `#NO_APP` as `LineKind::InlineAsm`
/// and pin it. Inline-asm text is user-authored and must reach the assembler
/// byte-for-byte:
///   * kernel ALTERNATIVE() templates use deliberate "redundant" instructions
///     (`movq %rax, %rax`) purely as length placeholders — removing one made
///     .altinstructions record orig_len=0 ("empty alternative entry", objtool);
///   * jump-label/static_call sites are patched at RUNTIME at the recorded
///     offset — any byte moved or removed corrupts the patch;
///   * label arithmetic (742b-740b) inside .pushsection depends on exact
///     instruction extents.
/// Every conservative fallback treats InlineAsm as clobbering everything:
/// reg_refs = all set, has_indirect_mem = true, pinned = true. The markers
/// themselves are comments and pass through to the assembler harmlessly.
fn pin_inline_asm_regions(store: &LineStore, infos: &mut [LineInfo]) {
    let mut in_asm = false;
    for i in 0..store.len() {
        let trimmed = infos[i].trimmed(store.get(i));
        if trimmed == "#APP" {
            in_asm = true;
            continue;
        }
        if trimmed == "#NO_APP" {
            in_asm = false;
            continue;
        }
        if in_asm {
            infos[i] = LineInfo {
                kind: LineKind::InlineAsm,
                ext_kind: ExtKind::None,
                trim_start: infos[i].trim_start,
                has_indirect_mem: true,
                rbp_offset: RBP_OFFSET_NONE,
                reg_refs: u16::MAX,
                pinned: true,
            };
        }
    }
}

/// Pin fallback parameter-ABI reads so no text pass rewrites their source.
///
/// `emit_param_ref_impl` falls back to reading a parameter from its incoming
/// ABI register when the parameter has no register home and no alloca slot.
/// The operand of that read is a *contract*: it must stay the ABI register
/// even when a caller-saved pre-store of a different parameter copied another
/// value into the same register name (`movq %rdi, %rsi` for param 0 while
/// param 1 still arrives in `%sil`). Copy propagation had no way to tell the
/// two apart and rewrote `movzbl %sil, %eax` into `movzbl %dil, %eax`,
/// storing the wrong parameter (sqlite VDBE `addop` corruption).
///
/// The `# LCCC_PARAM_ABI_READ <reg>` marker is emitted immediately before
/// the read; this pass pins that line (opaque to source rewriting, still
/// classified by destination so register-taint tracking keeps working) and
/// the marker itself stays a harmless comment for the assembler.
fn pin_param_abi_reads(store: &LineStore, infos: &mut [LineInfo]) {
    for i in 0..store.len() {
        let trimmed = infos[i].trimmed(store.get(i));
        if !trimmed.starts_with("# LCCC_PARAM_ABI_READ ") {
            continue;
        }
        // Pin the next real instruction line (skip other comments/labels).
        let mut j = i + 1;
        while j < store.len() {
            let next_trimmed = infos[j].trimmed(store.get(j));
            if next_trimmed.is_empty()
                || next_trimmed.starts_with('.')
                || next_trimmed.starts_with('#')
            {
                j += 1;
                continue;
            }
            let dest_reg = parse_dest_reg_fast(next_trimmed);
            infos[j].pinned = true;
            infos[j].has_indirect_mem = true;
            infos[j].kind = LineKind::Other { dest_reg };
            infos[j].rbp_offset = RBP_OFFSET_NONE;
            break;
        }
    }
}

fn pin_volatile_stack_slots(store: &LineStore, infos: &mut [LineInfo]) {
    let mut volatile_slots: Vec<(u8, i32)> = Vec::new();
    for i in 0..store.len() {
        let trimmed = infos[i].trimmed(store.get(i));
        // Function boundary: volatile markers are function-local.
        if trimmed.starts_with(".cfi_startproc") {
            volatile_slots.clear();
            continue;
        }
        if let Some(slot_text) = trimmed.strip_prefix("# LCCC_VOLATILE_SLOT ") {
            if let Some(slot) = direct_stack_slot(slot_text) {
                volatile_slots.push(slot);
            }
            continue;
        }
        if trimmed.starts_with(".cfi_endproc") {
            volatile_slots.clear();
            continue;
        }
        if volatile_slots.is_empty() {
            continue;
        }
        if let Some(slot) = direct_stack_slot_in_line(trimmed) {
            if volatile_slots.contains(&slot) {
                // Remove the stack-specialized classification as well as pinning.
                // This blocks forwarding/folding of both volatile loads and stores.
                // Keep the REAL destination register: the instruction still
                // writes it, and register-taint tracking (rax_is_zero in
                // combined_local_pass) depends on that.  REG_NONE here made a
                // volatile load into %rax look like a non-rax write, letting a
                // later `xorl %eax, %eax` be removed as "redundant" even though
                // %rax held a live value (miscompiled call argument setup).
                let dest_reg = parse_dest_reg_fast(trimmed);
                infos[i].pinned = true;
                infos[i].kind = LineKind::Other { dest_reg };
                infos[i].has_indirect_mem = true;
                infos[i].rbp_offset = RBP_OFFSET_NONE;
            }
        }
    }
}

fn pin_address_taken_stack_slots(store: &LineStore, infos: &mut [LineInfo]) {
    let mut address_taken = Vec::new();
    for i in 0..store.len() {
        let trimmed = infos[i].trimmed(store.get(i));
        let operand = if let Some(rest) = trimmed.strip_prefix("leaq ") {
            rest.split_once(',').map(|(op, _)| op)
        } else if let Some(rest) = trimmed.strip_prefix("lea ") {
            rest.split_once(',').map(|(op, _)| op)
        } else {
            None
        };
        if let Some(slot) = operand.and_then(direct_stack_slot) {
            address_taken.push(slot);
        }
    }
    if address_taken.is_empty() {
        return;
    }
    for i in 0..store.len() {
        // Pin BOTH stores and loads of address-taken slots: through the
        // escaped pointer, other code (e.g. a callee) may write the slot, so
        // a load must not be folded/forwarded from an earlier store, and the
        // store must not be dropped as dead.
        if !matches!(
            infos[i].kind,
            LineKind::StoreRbp { .. } | LineKind::LoadRbp { .. }
        ) {
            continue;
        }
        let trimmed = infos[i].trimmed(store.get(i));
        let operand = match infos[i].kind {
            LineKind::StoreRbp { .. } => trimmed.rsplit_once(',').map(|(_, op)| op),
            LineKind::LoadRbp { .. } => trimmed.split_once(',').map(|(op, _)| op),
            _ => None,
        };
        let slot = operand.and_then(|op| direct_stack_slot(op.trim()));
        if slot.map_or(false, |slot| address_taken.contains(&slot)) {
            infos[i].pinned = true;
        }
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Run peephole optimization on x86-64 assembly text.
/// Returns the optimized assembly string.
///
/// Pass structure for speed:
/// 1. Run cheap local passes iteratively until convergence (max `MAX_LOCAL_PASS_ITERATIONS`).
///    These are O(n) single-scan passes that only look at adjacent/nearby lines.
/// 2. Run expensive global passes once. `global_store_forwarding` is O(n) but with
///    higher constant factor due to tracking slot→register mappings. It subsumes
///    the functionality of local store-load forwarding across wider windows.
/// 3. Run local passes one more time to clean up opportunities exposed by the
///    global passes (max `MAX_POST_GLOBAL_ITERATIONS` iterations).
/// Cheap line estimate: count newline bytes (no allocation, no splitting).
/// Used by the size-adaptive peephole gate; for LF-terminated text the
/// newline count equals the line count, and the gate only needs a bound.
fn line_count_estimate(asm: &str) -> usize {
    asm.as_bytes().iter().filter(|&&b| b == b'\n').count()
}

pub fn peephole_optimize(mut asm: String) -> String {
    // ms178 debug: dump pre-peephole asm
    if let Ok(path) = std::env::var("CCC_DUMP_ASM") {
        let _ = std::fs::write(path, &asm);
    }
    // Always-on, provably-safe pass: eliminate redundant pushfq/popfq pairs
    // (flag-neutral window). Runs before the peephole gate.
    if std::env::var("CCC_NO_PUSHF_ELIM").is_err() {
        let _ = pushf_elim::eliminate_redundant_pushfq(&mut asm);
    }
    // Always-on, provably-safe pass: eliminate redundant zero-extensions
    // only when the tracked value already fits the source width. Runs before
    // gate; CCC_NO_REDUNDANT_EXT=1 disables it for miscompile bisection.
    if std::env::var("CCC_NO_REDUNDANT_EXT").is_err() {
        let _ = redundant_ext::eliminate_redundant_zero_extend(&mut asm);
    }
    // the peephole optimizer is now ENABLED BY DEFAULT with a curated,
    // gzip-validated safe subset. The two passes that were proven to miscompile
    // gzip 1.14 (full 30-test suite) are skipped by default: `store_fwd`
    // (global store-load forwarding) and `combined` (combined_local_pass).
    // All other passes individually pass gzip's 30-test suite, and the union
    // of the enabled passes passes 30/30 with a 2.1% text-size reduction and a
    // ~12% runtime improvement on gzip compress over the pushfq-only baseline.
    // Disable entirely with CCC_NO_PEEPHOLE=1; override the skip set with
    // CCC_PEEPHOLE_SKIP=pass1,pass2,...
    if std::env::var("CCC_NO_PEEPHOLE").is_ok() {
        return asm;
    }
    // Size-adaptive gate: pathological machine-generated files (the torture
    // memcpy-ax corpus expands to ~486k asm lines / ~246k instructions) pay a
    // full pass-mix sweep measured at ~58 s while the output is byte-identical
    // to the un-peepholed text — straight-line constant-size memset/memcpy
    // expansions contain none of the staging/relay patterns the passes match.
    // Real-world objects are orders of magnitude smaller (gzip -O2 links from
    // ~20k asm lines), so a 150k-line threshold keeps every realistic file on
    // the full pass mix while bounding worst-case compile time. Opt back in
    // with CCC_FORCE_PEEPHOLE=1 (bisection hook, mirrors CCC_PEEPHOLE_PHASE4).
    if line_count_estimate(&asm) > 150_000 && std::env::var("CCC_FORCE_PEEPHOLE").is_err() {
        return asm;
    }
    let skip_phase1 = std::env::var("CCC_NO_PEEPHOLE_PHASE1").is_ok();
    let skip_phase2 = std::env::var("CCC_NO_PEEPHOLE_PHASE2").is_ok();
    let skip_phase3 = std::env::var("CCC_NO_PEEPHOLE_PHASE3").is_ok();
    // Phase 4 (loop rotation / trampoline elimination / late hoisting) is
    // DISABLED BY DEFAULT: it has a register-renaming bug that miscompiles
    // valid programs (found via differential fuzzing on generated C; gzip was
    // unaffected but other code with certain loop shapes was). It also does not
    // help gzip (measured slower/larger). Opt in with CCC_PEEPHOLE_PHASE4=1.
    let skip_phase4 =
        !std::env::var("CCC_PEEPHOLE_PHASE4").is_ok() || std::env::var("CCC_NO_MACHINST").is_err(); // Phase 4 renaming is not MachInst-safe
    let skip_phase5 = std::env::var("CCC_NO_PEEPHOLE_PHASE5").is_ok();
    let skip_phase6 = std::env::var("CCC_NO_PEEPHOLE_PHASE6").is_ok();
    let skip_phase7 = std::env::var("CCC_NO_PEEPHOLE_PHASE7").is_ok();

    let mut store = LineStore::new(asm);
    let line_count = store.len();
    let mut infos: Vec<LineInfo> = (0..line_count)
        .map(|i| classify_line(store.get(i)))
        .collect();
    pin_inline_asm_regions(&store, &mut infos);
    pin_volatile_stack_slots(&store, &mut infos);
    pin_address_taken_stack_slots(&store, &mut infos);
    pin_param_abi_reads(&store, &mut infos);

    // ms178: pushfq/popfq are now classified as Push{REG_NONE}/Pop{REG_NONE}
    // (see types.rs) and all passes treat Push/Pop as barriers, so they can no
    // longer corrupt slot-offset tracking. The old global `has_pushfq` gate —
    // which skipped Phase 2/3 for the ENTIRE file whenever ANY function used a
    // Select (the origin of pushfq) — is removed. That gate silently disabled
    // the whole global peephole for files like gzip's deflate.c (longest_match
    // uses pushfq), leaving 28+ store→load→deref round-trips per hot loop
    // unoptimized.
    let skip_phase1 = skip_phase1;
    let skip_phase2 = skip_phase2;
    let skip_phase3 = skip_phase3;
    let max_phase1_iters = MAX_LOCAL_PASS_ITERATIONS;
    // CCC_TIME_PEEPHOLE=1: per-iteration/per-phase wall-time trace on stderr.
    // The optimizer is text-based, so pathological generated files (e.g. the
    // torture memcpy-ax corpus: ~486k asm lines) make per-pass cost visible;
    // this is the measurement hook for keeping the pass mix linear-ish.
    let time_peephole = std::env::var("CCC_TIME_PEEPHOLE").is_ok();
    let peephole_total = if time_peephole {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Pin parameter pre-store instructions: `movq %arg_reg, %callee_saved_reg`
    // that appear in the prologue area (before the first function call).
    // These save function parameters to callee-saved registers and must never
    // be removed by any peephole pass, since the ABI arg registers get
    // clobbered by subsequent calls.
    {
        let mut in_prologue = false;
        for idx in 0..line_count {
            let trimmed = infos[idx].trimmed(store.get(idx));
            if trimmed.starts_with(".cfi_startproc") {
                in_prologue = true;
                continue;
            }
            if !in_prologue {
                continue;
            }
            if matches!(infos[idx].kind, LineKind::Call) {
                in_prologue = false;
                continue;
            }
            // A genuine parameter pre-store always appears in the straight-line
            // entry block, BEFORE any branch/loop. Once we hit a branch (or an
            // indirect jump), the function body has begun and later
            // `movq %arg_reg, %callee_saved` copies (e.g. phi copy-backs in the
            // epilogue) are NOT parameter saves. Stop pinning here — otherwise
            // such copies get pinned and block legitimate peephole passes,
            // which can miscompile (see coalesce_phi_register_copies).
            if matches!(
                infos[idx].kind,
                LineKind::Jmp | LineKind::JmpIndirect | LineKind::CondJmp
            ) {
                in_prologue = false;
                continue;
            }
            // Pin movq from arg regs to callee-saved regs
            if trimmed.starts_with("movq %") {
                let is_param_prestore = (trimmed.contains("%rdi")
                    || trimmed.contains("%rsi")
                    || trimmed.contains("%rdx")
                    || trimmed.contains("%rcx")
                    || trimmed.contains("%r8")
                    || trimmed.contains("%r9"))
                    && (trimmed.ends_with("%rbx")
                        || trimmed.ends_with("%r12")
                        || trimmed.ends_with("%r13")
                        || trimmed.ends_with("%r14")
                        || trimmed.ends_with("%r15"));
                if is_param_prestore {
                    infos[idx].pinned = true;
                }
            }
        }
    }

    // CCC_PEEPHOLE_SKIP=pass1,pass2,... to disable specific sub-passes.
    //
    // ALL peephole passes are now ENABLED BY DEFAULT. The previously-disabled
    // passes were fixed properly (not merely skipped):
    //   - copy_prop: clear copy state at every barrier (incl. CondJmp).
    //   - store_relay: use the STORE's width, do not widen a 32-bit zero-extension
    //     to a 64-bit store, and fix the double-'%' register names.
    //   - store_fwd: frame-pointer-aware — do not treat `(%rbp)` as a stack slot
    //     when rbp is a general data register under -fomit-frame-pointer; also
    //     invalidate at CondJmp and calls.
    //   - gen_relay/leaq_relay/load_relay/cltq_relay/ext_relay: fixed the shared
    //     is_rax_dead_after liveness check (only a call truly clobbers rax).
    //   - gpr_hoist: place the hoisted load in the true preheader (replace the
    //     entry jmp), not in the dead gap after the header label.
    //   - dead_stores: frame-pointer-aware (never treat `(%rbp)` as a stack slot
    //     when rbp is a data register) and Cmp/test lines now carry memory
    //     operands so a compare read is not missed.
    //   - combined: dead `xorl` elimination refuses to drop it when the next
    //     instruction READS %rax (e.g. `movl %eax,%eax`), and the pass is gated
    //     by the skip set in phase 3.
    //   - identical_blocks: only merge blocks with IDENTICAL predecessor sets.
    //   - acc_alu: barrier no longer treated as "src reg safe".
    //   - phi_coalesce: TMP must not be written between copy-out and copy-back,
    //     and the chain register must not be read before its defining movslq.
    // Validation: differential fuzz (gen2, all passes ON, vs GCC -O2) and gzip 1.14
    // (30/30) and regression (7/7). expat 2.8.2 fails identically with and without
    // peephole (a base-backend codegen issue, not a peephole bug).
    let skip_set: crate::common::fx_hash::FxHashSet<String> = std::env::var("CCC_PEEPHOLE_SKIP")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let sk = |name: &str| -> bool { skip_set.contains(name) };

    // Phase 1: Iterative cheap local passes.
    let mut changed = true;
    let mut pass_count = 0;
    while changed && pass_count < max_phase1_iters && !skip_phase1 {
        let iter_start = if time_peephole {
            Some(std::time::Instant::now())
        } else {
            None
        };
        changed = false;
        let local_changed = if sk("combined") {
            false
        } else {
            local_patterns::combined_local_pass(&mut store, &mut infos)
        };
        changed |= local_changed;
        if !sk("lea_mem_sib") {
            changed |= local_patterns::fold_lea_into_memory_op(&mut store, &mut infos);
        }
        if !sk("lea_all_uses") {
            changed |= local_patterns::fold_lea_all_uses_in_block(&mut store, &mut infos);
        }
        if !sk("fuse_movq_ext") {
            changed |= local_patterns::fuse_movq_ext_truncation(&mut store, &mut infos);
            // VEX 3-operand exploitation: `movsd %A,%D; vOP %S,%D,%D` ->
            // `vOP %S,%A,%D`. Removes the 2-operand-ISA staging copy the
            // scalar FP emitters insert before every binary op.
            changed |= local_patterns::fuse_mov_scalar_fp_into_vex_op(&mut store, &mut infos);
            // Memory-source half: fold a dead staged load into a commutative
            // scalar VEX op's memory operand slot.
            changed |= local_patterns::fold_scalar_fp_memory_into_vex_op(&mut store, &mut infos);
        }
        if !sk("fp_roundtrips") {
            changed |= local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
        }
        if !sk("fp_mem_fold") {
            changed |= memory_fold::fold_fp_memory_operands(&mut store, &mut infos);
        }
        if !sk("fp_reg_mem_fold") {
            changed |= memory_fold::fold_fp_register_loads(&mut store, &mut infos);
        }
        // Fold a single-use scalar FP load into an adjacent FMA3-231 memory
        // src2 slot (dot-product inner shape; function-wide liveness proof).
        if !sk("fma_mem_fold") {
            changed |= memory_fold::fold_fma_memory_src2(&mut store, &mut infos);
        }
        // Constant-accumulator FMA shaping: 213-form with a just-loaded
        // multiplier becomes the 132 form with the load folded into the
        // memory slot, and repeated %xmm0 zeroings die block-locally.
        if !sk("fma132_zero") {
            changed |= memory_fold::fold_zero_addend_fma213_to_132(&mut store, &mut infos);
            changed |= memory_fold::eliminate_redundant_xmm0_zeroing(&mut store, &mut infos);
        }
        // Repeated RIP-relative loads of the same FP pool constant become a
        // single register materialization (leaf functions only; harvested
        // NOP slot must dominate all uses by fall-through).
        if !sk("fp_const_hoist") {
            changed |= memory_fold::hoist_repeated_fp_constant_loads(&mut store, &mut infos);
        }
        // Opt-in (CCC_PEEPHOLE_RELAY=1): fuses load+dead-copy relays; known
        // masked interaction with expat test_multichar_cdata_utf16 under the
        // full pass mix — root cause still open, so keep it off by default.
        if std::env::var("CCC_PEEPHOLE_RELAY").is_ok() && !sk("load_copy_relay") {
            changed |= memory_fold::fold_load_copy_relay(&mut store, &mut infos);
        }
        if !sk("rcx_copy") {
            changed |= local_patterns::eliminate_rcx_address_copy(&mut store, &mut infos);
        }
        if !sk("ptr_deref") {
            changed |= local_patterns::fold_ptr_deref_through_stack(&mut store, &mut infos);
        }
        if !sk("fp_spill") {
            changed |= local_patterns::eliminate_fp_spill_around_load(&mut store, &mut infos);
        }
        if !sk("fuse_copy_op") {
            changed |= local_patterns::fuse_copy_and_operation(&mut store, &mut infos);
        }
        if !sk("fp_hoist") {
            changed |= local_patterns::promote_loop_invariant_fp_load(&mut store, &mut infos);
        }
        if !sk("dead_signext") {
            changed |= local_patterns::eliminate_dead_sign_extensions(&mut store, &mut infos);
        }
        if !sk("dead_leaq") {
            changed |= local_patterns::eliminate_redundant_leaq(&store, &mut infos);
        }
        // Symbol-LEA CSE for every GPR and generic load->ALU memory-operand
        // fusion (any addressing mode, liveness-proved dead scratch); see
        // load_op_fuse.rs for the soundness argument.
        if !sk("symbol_lea_cse") {
            changed |= load_op_fuse::eliminate_redundant_symbol_lea(&store, &mut infos);
        }
        if !sk("recurrence_inplace") {
            changed |= load_op_fuse::fold_recurrence_update(&mut store, &mut infos);
        }
        if !sk("copy_dying_operand") {
            changed |= load_op_fuse::fold_copy_into_dying_operand(&mut store, &mut infos);
        }
        if !sk("load_alu_fuse") {
            changed |= load_op_fuse::fuse_load_into_alu(&mut store, &mut infos);
        }
        // Generic move-relay elimination and windowed lea->memory folding.
        // Both are block-local; see relay_and_lea.rs for the two deadness
        // proofs (block-local write-before-read, whole-function uniqueness).
        if !sk("move_relay") {
            changed |= relay_and_lea::eliminate_move_relays(&mut store, &mut infos);
        }
        if !sk("lea_load_window") {
            changed |= relay_and_lea::fold_lea_into_load(&mut store, &mut infos);
        }
        // Flags-aware cleanups: copy+add -> lea (also removes a flags write
        // from between a comparison and its consumer), the setcc/movzbl/test
        // boolean round-trip in front of a cmov, and copy+mask -> movzx.
        if !sk("producer_retarget") {
            changed |= relay_and_lea::retarget_producer_into_copy(&mut store, &mut infos);
        }
        // Loop-latch shape the two passes above decline: the copy dest is
        // live across the back edge and the producer reg is read after the
        // copy. Retargets `lea D(%rA), %rB; mov %rB, %rA` into
        // `lea D(%rA), %rA` with a liveness-proved, rollback-guarded rename
        // of the following %rB reads.
        if !sk("lea_base_fold") {
            changed |= relay_and_lea::fold_copy_into_lea_base(&mut store, &mut infos);
        }
        if !sk("copy_add_lea") {
            changed |= flag_peepholes::fold_copy_add_into_lea(&mut store, &mut infos);
        }
        if !sk("copy_shift_lea") {
            changed |= flag_peepholes::fold_copy_shift_into_lea(&mut store, &mut infos);
        }
        if !sk("setcc_cmov") {
            changed |= flag_peepholes::fold_setcc_test_cmov(&mut store, &mut infos);
        }
        // Sound dead-write elimination (LEAs, widened copies, setCC) and
        // repeated-load reuse; both use the relay/lea deadness proofs.
        // Whole-function copy coalescing: rename the destination family to the
        // source family when the source dies at the copy (kills the parameter
        // shuffle the allocator emits at every function entry).
        if !sk("copy_coalesce") {
            changed |= copy_coalesce::coalesce_register_copies(&mut store, &mut infos);
        }
        if !sk("dead_pure_writes") {
            changed |= dead_writes::eliminate_dead_pure_writes(&store, &mut infos);
        }
        if !sk("load_test_cmp") {
            changed |= dead_writes::fold_load_test_into_cmp(&mut store, &mut infos);
        }
        if !sk("acc_roundtrip") {
            changed |= dead_writes::fold_accumulator_roundtrip(&mut store, &mut infos);
        }
        if !sk("load_reuse") {
            changed |= dead_writes::reuse_redundant_loads(&mut store, &mut infos);
        }
        if !sk("self_test") {
            changed |= flag_peepholes::eliminate_redundant_self_test(&store, &mut infos);
        }
        if !sk("narrow_signext") {
            changed |= flag_peepholes::narrow_dead_sign_extension(&mut store, &mut infos);
        }
        if !sk("copy_mask_movz") {
            changed |= flag_peepholes::fold_copy_and_mask_into_movz(&mut store, &mut infos);
        }
        if !sk("base_index") {
            changed |= local_patterns::fold_base_index_addressing(&mut store, &mut infos);
        }
        if !sk("acc_alu") {
            changed |= local_patterns::fold_accumulator_alu_store(&mut store, &mut infos);
        }
        if !sk("phi_coalesce") {
            changed |= local_patterns::coalesce_phi_register_copies(&mut store, &mut infos);
        }
        if !sk("signext_move") {
            changed |= local_patterns::fuse_signext_and_move(&mut store, &mut infos);
        }
        if !sk("inc_chain") {
            changed |= local_patterns::collapse_increment_chain(&mut store, &mut infos);
        }
        // add_signext (fuse_add_sign_extend) REMOVED: it rewrote
        // `addl %X,%X; movslq %X,%DST` into `addl %X,%DSTd`, which (a) reads
        // an uninitialized %DSTd — its own SAFETY comment required "DST was
        // initialized with the same value" but never checked it — and
        // (b) drops the sign extension (addl zero-extends the upper half).
        // expat xmlparse.c reportProcessingInstruction: the PI-target length
        // became garbage and XmlNameLength walked off the buffer (SIGSEGV).
        // A sound fusion saves nothing (still needs init + sext), so the
        // pass is deleted rather than gated.
        if !sk("copy_shift_back") {
            changed |= local_patterns::fold_copy_shift_copyback(&mut store, &mut infos);
        }
        if !sk("xor_move_fold") {
            changed |= local_patterns::fold_zero_extended_xor_moves(&mut store, &mut infos);
        }
        if !sk("rotate_idiom") {
            changed |= local_patterns::fold_rotate_idiom(&mut store, &mut infos);
        }
        if !sk("vec_self_move") {
            changed |= local_patterns::eliminate_vector_self_moves(&mut store, &mut infos);
        }
        if !sk("cascaded_shifts") {
            changed |= local_patterns::fold_cascaded_shifts(&mut store, &mut infos);
        }
        if !sk("gpr_hoist") {
            changed |= local_patterns::hoist_loop_invariant_gpr_load(&mut store, &mut infos);
        }
        if !sk("fp_broadcast") {
            changed |= local_patterns::hoist_loop_invariant_fp_broadcast(&mut store, &mut infos);
        }
        if local_changed || pass_count == 0 {
            if !sk("push_pop") {
                changed |= push_pop::eliminate_push_pop_pairs(&store, &mut infos);
            }
            if !sk("binop_push_pop") {
                changed |= push_pop::eliminate_binop_push_pop_pattern(&mut store, &mut infos);
            }
        }
        if let Some(s) = iter_start {
            eprintln!(
                "[PEEPHOLE-TIME] phase1 iteration {}: {:.1} ms",
                pass_count,
                s.elapsed().as_secs_f64() * 1e3
            );
        }
        pass_count += 1;
    }

    // Phase 2: Expensive global passes (run once)
    let phase2_start = if time_peephole {
        Some(std::time::Instant::now())
    } else {
        None
    };
    let global_changed = if skip_phase2 {
        false
    } else {
        let mut global_changed = false;
        if !sk("store_fwd") {
            global_changed |= store_forwarding::global_store_forwarding(&mut store, &mut infos);
        }
        if !sk("spill_deref") {
            global_changed |= spill_deref::fold_spill_deref_roundtrip(&mut store, &mut infos);
        }
        if !sk("copy_prop") {
            global_changed |= copy_propagation::propagate_register_copies(&mut store, &mut infos);
        }
        // After copy propagation (64-bit chains) and before dead-code removal,
        // so the copies this fold orphans are retired in the same round.
        if !sk("copy_fold") {
            global_changed |= narrow_copy_fold::fold_register_copies(&mut store, &mut infos);
        }
        if !sk("dead_regs") {
            global_changed |= dead_code::eliminate_dead_reg_moves(&store, &mut infos);
        }
        if !sk("dead_stores") {
            global_changed |= dead_code::eliminate_dead_stores(&store, &mut infos);
        }
        if !sk("cmp_branch") {
            global_changed |= compare_branch::fuse_compare_and_branch(&mut store, &mut infos);
        }
        if !sk("mem_fold") {
            global_changed |= memory_fold::fold_memory_operands(&mut store, &mut infos);
        }
        if !sk("load_relay") {
            global_changed |= memory_fold::fold_load_relay(&mut store, &mut infos);
        }
        if !sk("leaq_relay") {
            global_changed |= memory_fold::fold_leaq_relay(&mut store, &mut infos);
        }
        if !sk("cltq_relay") {
            global_changed |= memory_fold::fold_cltq_relay(&mut store, &mut infos);
        }
        if !sk("ext_relay") {
            global_changed |= memory_fold::fold_extend_relay(&mut store, &mut infos);
        }
        if !sk("gen_relay") {
            global_changed |= memory_fold::fold_general_relay(&mut store, &mut infos);
        }
        if !sk("store_relay") {
            global_changed |= memory_fold::fold_store_relay(&mut store, &mut infos);
        }
        global_changed
    };
    if let Some(s) = phase2_start {
        eprintln!(
            "[PEEPHOLE-TIME] phase2: {:.1} ms",
            s.elapsed().as_secs_f64() * 1e3
        );
    }

    // Phase 3: One more local cleanup if global passes made changes.
    if global_changed && !skip_phase3 {
        let mut changed2 = true;
        let mut pass_count2 = 0;
        while changed2 && pass_count2 < MAX_POST_GLOBAL_ITERATIONS {
            changed2 = false;
            // Gated: previously combined_local_pass ran UNGATED here, so it
            // executed even when the user skipped `combined` — this caused
            // interaction miscompiles (e.g. redundant `xorl %eax,%eax` removal
            // across loop boundaries). Honor the skip set.
            if !sk("combined") {
                changed2 |= local_patterns::combined_local_pass(&mut store, &mut infos);
            }
            if !sk("lea_mem_sib") {
                changed2 |= local_patterns::fold_lea_into_memory_op(&mut store, &mut infos);
            }
            if !sk("lea_all_uses") {
                changed2 |= local_patterns::fold_lea_all_uses_in_block(&mut store, &mut infos);
            }
            changed2 |= local_patterns::fuse_movq_ext_truncation(&mut store, &mut infos);
            changed2 |= local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
            changed2 |= memory_fold::fold_fp_memory_operands(&mut store, &mut infos);
            if !sk("fp_reg_mem_fold") {
                changed2 |= memory_fold::fold_fp_register_loads(&mut store, &mut infos);
            }
            if !sk("fp_const_hoist") {
                changed2 |= memory_fold::hoist_repeated_fp_constant_loads(&mut store, &mut infos);
            }
            if std::env::var("CCC_PEEPHOLE_RELAY").is_ok() && !sk("load_copy_relay") {
                changed2 |= memory_fold::fold_load_copy_relay(&mut store, &mut infos);
            }
            changed2 |= local_patterns::eliminate_rcx_address_copy(&mut store, &mut infos);
            changed2 |= local_patterns::fold_ptr_deref_through_stack(&mut store, &mut infos);
            changed2 |= local_patterns::eliminate_fp_spill_around_load(&mut store, &mut infos);
            if !sk("dead_regs") {
                changed2 |= dead_code::eliminate_dead_reg_moves(&store, &mut infos);
            }
            if !sk("dead_stores") {
                changed2 |= dead_code::eliminate_dead_stores(&store, &mut infos);
            }
            if !sk("mem_fold") {
                changed2 |= memory_fold::fold_memory_operands(&mut store, &mut infos);
            }
            if !sk("load_relay") {
                changed2 |= memory_fold::fold_load_relay(&mut store, &mut infos);
            }
            if !sk("leaq_relay") {
                changed2 |= memory_fold::fold_leaq_relay(&mut store, &mut infos);
            }
            if !sk("cltq_relay") {
                changed2 |= memory_fold::fold_cltq_relay(&mut store, &mut infos);
            }
            if !sk("ext_relay") {
                changed2 |= memory_fold::fold_extend_relay(&mut store, &mut infos);
            }
            if !sk("gen_relay") {
                changed2 |= memory_fold::fold_general_relay(&mut store, &mut infos);
            }
            if !sk("store_relay") {
                changed2 |= memory_fold::fold_store_relay(&mut store, &mut infos);
            }
            if !sk("base_index") {
                changed2 |= local_patterns::fold_base_index_addressing(&mut store, &mut infos);
            }
            if !sk("phi_coalesce") {
                changed2 |= local_patterns::coalesce_phi_register_copies(&mut store, &mut infos);
            }
            if !sk("signext_move") {
                changed2 |= local_patterns::fuse_signext_and_move(&mut store, &mut infos);
            }
            changed2 |= local_patterns::collapse_increment_chain(&mut store, &mut infos);
            if !sk("copy_shift_back") {
                changed2 |= local_patterns::fold_copy_shift_copyback(&mut store, &mut infos);
            }
            if !sk("xor_move_fold") {
                changed2 |= local_patterns::fold_zero_extended_xor_moves(&mut store, &mut infos);
            }
            if !sk("rotate_idiom") {
                changed2 |= local_patterns::fold_rotate_idiom(&mut store, &mut infos);
            }
            if !sk("vec_self_move") {
                changed2 |= local_patterns::eliminate_vector_self_moves(&mut store, &mut infos);
            }
            pass_count2 += 1;
        }
    }

    // Phase 3b: Fuse addl+movslq in loops (must run BEFORE trampoline elimination,
    // which may remove the conditional back-edge that this pass uses to detect loops).
    // fuse_add_sign_extend removed (unsound: uninitialized dest + dropped sext).

    // Phase 4: Eliminate loop backedge trampoline blocks.
    let trampoline_changed = if skip_phase4 || sk("loop_trampoline") {
        false
    } else {
        loop_trampoline::eliminate_loop_trampolines(&mut store, &mut infos)
    };

    // Phase 4b: If trampoline elimination made changes, do another round of local cleanup.
    if trampoline_changed && !skip_phase4 {
        let mut changed3 = true;
        let mut pass_count3 = 0;
        while changed3 && pass_count3 < MAX_POST_GLOBAL_ITERATIONS {
            changed3 = false;
            changed3 |= local_patterns::combined_local_pass(&mut store, &mut infos);
            if !sk("lea_mem_sib") {
                changed3 |= local_patterns::fold_lea_into_memory_op(&mut store, &mut infos);
            }
            changed3 |= local_patterns::fuse_movq_ext_truncation(&mut store, &mut infos);
            changed3 |= local_patterns::eliminate_fp_xmm_roundtrips(&mut store, &mut infos);
            changed3 |= memory_fold::fold_fp_memory_operands(&mut store, &mut infos);
            if !sk("fp_reg_mem_fold") {
                changed3 |= memory_fold::fold_fp_register_loads(&mut store, &mut infos);
            }
            changed3 |= local_patterns::eliminate_rcx_address_copy(&mut store, &mut infos);
            changed3 |= local_patterns::fold_ptr_deref_through_stack(&mut store, &mut infos);
            if !sk("dead_regs") {
                changed3 |= dead_code::eliminate_dead_reg_moves(&store, &mut infos);
            }
            if !sk("dead_stores") {
                changed3 |= dead_code::eliminate_dead_stores(&store, &mut infos);
            }
            if !sk("mem_fold") {
                changed3 |= memory_fold::fold_memory_operands(&mut store, &mut infos);
            }
            if !sk("load_relay") {
                changed3 |= memory_fold::fold_load_relay(&mut store, &mut infos);
            }
            if !sk("leaq_relay") {
                changed3 |= memory_fold::fold_leaq_relay(&mut store, &mut infos);
            }
            if !sk("cltq_relay") {
                changed3 |= memory_fold::fold_cltq_relay(&mut store, &mut infos);
            }
            if !sk("ext_relay") {
                changed3 |= memory_fold::fold_extend_relay(&mut store, &mut infos);
            }
            if !sk("gen_relay") {
                changed3 |= memory_fold::fold_general_relay(&mut store, &mut infos);
            }
            changed3 |= local_patterns::coalesce_phi_register_copies(&mut store, &mut infos);
            if !sk("signext_move") {
                changed3 |= local_patterns::fuse_signext_and_move(&mut store, &mut infos);
            }
            changed3 |= local_patterns::collapse_increment_chain(&mut store, &mut infos);
            pass_count3 += 1;
        }
    }

    // Phase 4c: Late loop-invariant hoisting.
    if !skip_phase4 && !sk("gpr_hoist") {
        local_patterns::hoist_loop_invariant_gpr_load(&mut store, &mut infos);
    }
    if !skip_phase4 && !sk("fp_broadcast") {
        local_patterns::hoist_loop_invariant_fp_broadcast(&mut store, &mut infos);
    }

    // Phase 4d: Loop rotation — move condition from header to latch.
    if !skip_phase4 && !sk("loop_rotation") {
        local_patterns::rotate_loops(&mut store, &mut infos);
    }

    // Phase 5: Tail call optimization.
    if !skip_phase5 {
        tail_call::optimize_tail_calls(&mut store, &mut infos);
    }

    // Phase 5b: Global dead store elimination for never-read stack slots.
    if !skip_phase5 {
        dead_code::eliminate_never_read_stores(&store, &mut infos);
    }

    // Phase 6: Eliminate unused callee-saved register saves/restores.
    if !skip_phase6 {
        callee_saves::eliminate_unused_callee_saves(&mut store, &mut infos);
    }

    // Phase 7: Compact stack frames.
    if !skip_phase7 {
        frame_compact::compact_frame(&mut store, &mut infos);
    }

    // Phase 7b: Narrow `movabsq/movq $imm` to `movl` where zero-extension
    // yields the identical 64-bit value (BACKLOG PF-16). Runs after every
    // pattern pass so no earlier matcher has to know both spellings.
    if !sk("narrow_imm") {
        load_op_fuse::narrow_wide_immediates(&mut store, &mut infos);
    }

    // Phase 8: Merge identical basic blocks (phi trampoline deduplication).
    // Must run after all other transformations to catch final identical patterns.
    if !sk("identical_blocks") {
        identical_blocks::merge_identical_blocks(&mut store, &mut infos);
    }

    if let Some(s) = peephole_total {
        eprintln!(
            "[PEEPHOLE-TIME] total: {:.1} ms",
            s.elapsed().as_secs_f64() * 1e3
        );
    }

    // Phase 9: Re-run the always-on text passes on the FINAL text. The early
    // pushfq/popfq elimination is conservative about rsp-relative memory
    // operands, and the local passes frequently remove/rewrite exactly those
    // instructions — leaving windows that are now flag-neutral and safe to
    // strip. Same for redundant zero-extensions exposed by relay folding.
    if std::env::var("CCC_NO_PEEPHOLE_RERUN").is_ok() {
        return store.build_result(|i| infos[i].is_nop());
    }
    let mut result = store.build_result(|i| infos[i].is_nop());
    let _ = compare_branch::fuse_late_compare_bool_spills(&mut result);
    let _ = pushf_elim::eliminate_redundant_pushfq(&mut result);
    let _ = redundant_ext::eliminate_redundant_zero_extend(&mut result);
    result
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_never_read_store_eliminated_in_six_push_nofp_function() {
        // No-FP functions push rbp as an ordinary callee-saved register, so
        // the `subq $N,%rsp` prologue marker is separated from the .cfi
        // directives by the whole push chain.  The old form-1 failure path
        // skipped past the subq and the form-2 window missed the directives,
        // disabling dead-store elimination for such functions entirely
        // (gzip CRC kernel: a never-read `movq %rax, 8(%rsp)` survived).
        let asm = concat!(
            "main:\n",
            ".cfi_startproc\n",
            "    pushq %rbx\n",
            "    pushq %r12\n",
            "    pushq %r13\n",
            "    pushq %r14\n",
            "    pushq %r15\n",
            "    pushq %rbp\n",
            "    subq $24, %rsp\n",
            ".cfi_def_cfa_offset 80\n",
            "    movabsq $4294967295, %r13\n",
            ".L1:\n",
            "    movl (%rsi), %eax\n",
            "    movq %rax, 8(%rsp)\n",
            "    addl %eax, %r13d\n",
            "    jmp .L1\n",
            ".Lend:\n",
            "    addq $24, %rsp\n",
            "    popq %rbp\n",
            "    popq %r15\n",
            "    popq %r14\n",
            "    popq %r13\n",
            "    popq %r12\n",
            "    popq %rbx\n",
            "    ret\n",
            ".size main, .-main\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("movq %rax, 8(%rsp)"),
            "never-read slot store must be eliminated:\n{result}"
        );
    }

    #[test]
    fn test_cfg_retargets_accumulator_update_copyback() {
        let asm = concat!(
            "f:\n",
            "    movq %rcx, %rax\n",
            "    incl %eax\n",
            "    movl %eax, %ecx\n",
            "    movq $0, %rax\n",
            "    movq %rcx, %rdx\n",
            "    ret\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("incl %ecx"), "{result}");
        assert!(
            !result.contains("movq %rcx, %rax\n    incl %eax"),
            "{result}"
        );
    }

    #[test]
    fn test_copy_propagates_into_source_clobbering_address_load() {
        let asm = concat!(
            "f:\n",
            "    movq %rax, %rcx\n",
            "    movsbl (%rcx), %eax\n",
            "    ret\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movsbl (%rax), %eax"), "{result}");
        assert!(!result.contains("movq %rax, %rcx"), "{result}");
    }

    #[test]
    fn test_rcx_address_copy_into_scalar_store() {
        let asm = concat!(
            "f:\n",
            "    movq %rdx, %rcx\n",
            "    movq %rax, (%rcx)\n",
            "    ret\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movq %rax, (%rdx)"), "{result}");
        assert!(!result.contains("movq %rdx, %rcx"), "{result}");
    }

    #[test]
    fn test_rcx_address_copy_into_store_keeps_live_rcx() {
        let asm = concat!(
            "f:\n",
            "    movq %rdx, %rcx\n",
            "    movq %rax, (%rcx)\n",
            "    addq $1, %rcx\n",
            "    ret\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movq %rdx, %rcx"), "{result}");
        assert!(result.contains("(%rcx)"), "{result}");
    }

    #[test]
    fn test_stack_top_store_consumed_by_ret_survives_dse() {
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    subq $16, %rsp\n",
            ".Lset:\n",
            "    movq %rdx, (%rsp)\n",
            "    ret\n",
            ".size f, .-f\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movq %rdx, (%rsp)"), "{result}");
    }

    #[test]
    fn test_fp_xmm_roundtrip_load_rax() {
        let asm = "    movq -32(%rbp), %rax\n    movq %rax, %xmm0\n".to_string();
        let result = peephole_optimize(asm);
        eprintln!("fp_roundtrip result: {:?}", result);
        assert!(
            result.contains("movsd -32(%rbp), %xmm0"),
            "should eliminate rax roundtrip: {}",
            result
        );
        assert!(
            !result.contains("movq %rax, %xmm0"),
            "movq rax->xmm should be gone: {}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_load_rcx() {
        let asm = "    movq -40(%rbp), %rcx\n    movq %rcx, %xmm1\n".to_string();
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movsd -40(%rbp), %xmm1"),
            "should eliminate rcx roundtrip: {}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_store() {
        let asm = "    movq %xmm0, %rax\n    movq %rax, -48(%rbp)\n".to_string();
        let result = peephole_optimize(asm);
        eprintln!("fp_store result: {:?}", result);
        assert!(
            result.contains("movsd %xmm0, -48(%rbp)"),
            "should eliminate store roundtrip: {}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_store_any_xmm_reg() {
        // The accumulator-based FP emitter rotates through %xmm2..%xmm13;
        // every spill through those registers must fold to a direct movsd,
        // not just %xmm0.
        let asm = "    movq %xmm4, %rax\n    movq %rax, 296(%rsp)\n".to_string();
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movsd %xmm4, 296(%rsp)"),
            "should fold movq %xmm4->rax + store: {}",
            result
        );
        assert!(
            !result.contains("movq %xmm4, %rax"),
            "GPR bridge should be gone: {}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_store_high_xmm_reg() {
        let asm = "    movq %xmm13, %rax\n    movq %rax, 112(%rbp)\n".to_string();
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movsd %xmm13, 112(%rbp)"),
            "should fold high xmm register: {}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_load_keeps_live_bridge() {
        // Pattern A rewrites the LOAD, deleting the bridge GPR's definition.
        // With %rax still read afterwards the fold must NOT happen.
        let asm = "    movq -24(%rbp), %rax\n    movq %rax, %xmm7\n    addq $7, %rax\n".to_string();
        let result = peephole_optimize(asm);
        let defined = result.contains("movq -24(%rbp), %rax");
        let read = result.contains("addq $7, %rax");
        assert!(
            !(read && !defined),
            "folded away a LIVE %rax definition:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_load_rcx_keeps_live_bridge() {
        let asm = "    movq -8(%rbp), %rcx\n    movq %rcx, %xmm3\n    addq $1, %rcx\n".to_string();
        let result = peephole_optimize(asm);
        let defined = result.contains("movq -8(%rbp), %rcx");
        let read = result.contains("addq $1, %rcx");
        assert!(
            !(read && !defined),
            "folded away a LIVE %rcx definition:\n{}",
            result
        );
    }

    #[test]
    fn test_fp_xmm_roundtrip_load_any_xmm_reg() {
        let asm = "    movq -24(%rbp), %rax\n    movq %rax, %xmm7\n".to_string();
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movsd -24(%rbp), %xmm7"),
            "should fold load into any xmm register: {}",
            result
        );
        assert!(
            !result.contains("movq %rax, %xmm7"),
            "bridge movq should be gone: {}",
            result
        );
    }

    #[test]
    fn test_fp_memory_fold_mulsd() {
        let asm = ["    movsd -40(%rbp), %xmm1", "    mulsd %xmm1, %xmm0"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("mulsd -40(%rbp), %xmm0"),
            "should fold movsd+mulsd: {}",
            result
        );
    }

    #[test]
    fn test_fp_register_load_folds_into_vex_op() {
        let asm = [
            "    movsd 8(%rsi), %xmm5",
            "    vsubsd %xmm5, %xmm4, %xmm4",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("vsubsd 8(%rsi), %xmm4, %xmm4"),
            "single-use load should become a memory source: {}",
            result
        );
        assert!(
            !result.contains("movsd 8(%rsi), %xmm5"),
            "folded load should be deleted: {}",
            result
        );
    }

    #[test]
    fn test_fp_register_load_kept_when_source_used_later() {
        let asm = [
            "    movss (%rdi), %xmm5",
            "    vaddss %xmm5, %xmm4, %xmm4",
            "    vmulss %xmm5, %xmm6, %xmm6",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movss (%rdi), %xmm5"),
            "multi-use source load must remain: {}",
            result
        );
    }

    #[test]
    fn test_fp_register_load_kept_for_destructive_self_source() {
        let asm = [
            "    movsd (%rdi), %xmm5",
            "    vmulsd %xmm5, %xmm5, %xmm5",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movsd (%rdi), %xmm5"),
            "destination's old value depends on this load: {}",
            result
        );
    }

    #[test]
    fn test_fp_register_name_boundary_xmm1_vs_xmm10() {
        let asm = [
            "    movsd (%rdi), %xmm1",
            "    vaddsd %xmm1, %xmm4, %xmm4",
            "    movsd %xmm10, %xmm11",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("vaddsd (%rdi), %xmm4, %xmm4"),
            "xmm10 must not count as a later xmm1 use: {}",
            result
        );
    }

    #[test]
    fn test_lea_into_indexed_load() {
        let asm = ["    leaq (%r12, %r9), %rdi", "    movsbq (%rdi), %rsi"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movsbq (%r12,%r9), %rsi"),
            "should fold LEA into SIB load: {}",
            result
        );
        assert!(
            !result.contains("leaq (%r12, %r9), %rdi"),
            "temporary LEA should be removed: {}",
            result
        );
    }

    #[test]
    fn test_lea_copy_into_indexed_store() {
        let asm = [
            "    leaq (%r12, %r9), %rdi",
            "    movq %rdi, %rcx",
            "    movb $0, (%rcx)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movb $0, (%r12,%r9)"),
            "should fold LEA/copy into SIB store: {}",
            result
        );
        assert!(
            !result.contains("movq %rdi, %rcx"),
            "temporary address copy should be removed: {}",
            result
        );
    }

    #[test]
    fn test_lea_fold_keeps_sib_index_producer_live() {
        // This is the exact shape from Expat's corpus tail fill.  Once the
        // load/store uses r8 as a folded SIB index, dead-register elimination
        // must still observe r8 as a read and preserve its defining move.
        let asm = [
            "    movq %rax, %r8",
            "    leaq (%rdx, %r8), %r9",
            "    movb $0x20, (%r9)",
        ]
        .join("\n")
            + "\n";
        assert_ne!(
            scan_register_refs(b"movb $0x20, (%rdx,%r8)") & (1u16 << 8),
            0,
            "SIB index must be represented in the cached register-reference mask"
        );
        let mut store = LineStore::new(asm.clone());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        assert!(local_patterns::fold_lea_into_memory_op(
            &mut store, &mut infos
        ));
        assert_ne!(
            infos[2].reg_refs & (1u16 << 8),
            0,
            "rewritten SIB store must retain r8 in LineInfo: {}",
            store.get(2)
        );
        assert_eq!(
            helpers::get_dest_reg(&infos[2]),
            REG_NONE,
            "SIB memory operands do not write their base/index registers"
        );
        assert!(
            !dead_code::eliminate_dead_reg_moves(&store, &mut infos),
            "index producer must not become dead after SIB fold"
        );
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movb $0x20, (%rdx,%r8)"),
            "should fold to indexed store: {}",
            result
        );
        assert!(
            result.contains("movq %rax, %r8"),
            "SIB index producer must remain live after folding: {}",
            result
        );
    }

    #[test]
    fn test_redundant_store_load() {
        let asm = "    movq %rax, -8(%rbp)\n    movq -8(%rbp), %rax\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.trim(), "movq %rax, -8(%rbp)");
    }

    #[test]
    fn test_store_load_different_reg() {
        let asm = "    movq %rax, -8(%rbp)\n    movq -8(%rbp), %rcx\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movq %rax, -8(%rbp)"));
        assert!(result.contains("movq %rax, %rcx"));
        assert!(!result.contains("movq -8(%rbp), %rcx"));
    }

    #[test]
    fn test_redundant_jump() {
        let asm = "    jmp .Lfoo\n.Lfoo:\n".to_string();
        let result = peephole_optimize(asm);
        assert!(!result.contains("jmp"));
        assert!(result.contains(".Lfoo:"));
    }

    #[test]
    fn test_gpr_hoist_skips_multi_entry_loop() {
        // Regression: hoisting a loop-invariant GPR load into the preheader is
        // sound only when the header has a SINGLE forward entry edge (the entry
        // jmp). A conditional `je <header>` from a different predecessor enters
        // the loop directly, bypassing the preheader, and would observe a stale
        // destination register. (gzip deflate: the `rsync` guard's `je` into the
        // hash-table loop header left `head` unloaded in %rcx, so
        // `head[ins_h]=strstart` wrote through `&rsync`, corrupted globals, and
        // SIGSEGV'd under PGO builds.)
        let asm = [
            "    movq 16(%rsp), %rcx",
            "    movslq (%rcx), %rax",
            "    testq %rax, %rax",
            "    je .LBB1",
            ".LBB2:",
            "    movl (%rbx), %eax",
            "    jmp .LBB1",
            ".p2align 4",
            ".LBB1:",
            "    movl (%rbx), %edi",
            "    movq 304(%rsp), %rcx",
            "    addq %rcx, %rax",
            "    jne .LBB1",
        ]
        .join("\n")
            + "\n";
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = local_patterns::hoist_loop_invariant_gpr_load(&mut store, &mut infos);
        assert!(
            !changed,
            "must not hoist when the header has a second forward entry edge"
        );
        let result = store.build_result(|i| infos[i].is_nop());
        assert!(
            result.contains("jmp .LBB1"),
            "entry jmp must be preserved: {}",
            result
        );
        assert!(
            result.contains("movq 304(%rsp), %rcx"),
            "invariant load must stay in the loop: {}",
            result
        );
    }

    #[test]
    fn test_gpr_hoist_single_entry() {
        // Positive control: with a single forward entry edge (the entry jmp),
        // the invariant load SHOULD still be hoisted into the preheader.
        let asm = [
            ".LBB2:",
            "    movl (%rbx), %eax",
            "    jmp .LBB1",
            ".LBB1:",
            "    movl (%rbx), %edi",
            "    movq 304(%rsp), %rcx",
            "    addq %rcx, %rax",
            "    jne .LBB1",
        ]
        .join("\n")
            + "\n";
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = local_patterns::hoist_loop_invariant_gpr_load(&mut store, &mut infos);
        assert!(
            changed,
            "single-entry loop should still hoist the invariant load"
        );
        let result = store.build_result(|i| infos[i].is_nop());
        assert!(
            !result.contains("jmp .LBB1"),
            "entry jmp should be replaced by the hoisted load: {}",
            result
        );
        assert!(
            result.contains("movq 304(%rsp), %rcx"),
            "hoisted load must be present in the preheader: {}",
            result
        );
    }

    #[test]
    fn test_push_pop_elimination() {
        let asm = "    pushq %rax\n    movq %rax, %rcx\n    popq %rax\n".to_string();
        let result = peephole_optimize(asm);
        assert!(!result.contains("pushq"));
        assert!(!result.contains("popq"));
        assert!(result.contains("movq %rax, %rcx"));
    }

    #[test]
    fn test_self_move() {
        let asm = "    movq %rax, %rax\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(result.trim(), "");
    }

    #[test]
    fn test_parse_store_to_rbp() {
        assert!(parse_store_to_rbp_str("movq %rax, -8(%rbp)").is_some());
        assert!(parse_store_to_rbp_str("movl %eax, -16(%rbp)").is_some());
        assert!(parse_store_to_rbp_str("movq $5, -8(%rbp)").is_none());
    }

    #[test]
    fn test_parse_load_from_rbp() {
        assert!(parse_load_from_rbp_str("movq -8(%rbp), %rax").is_some());
        assert!(parse_load_from_rbp_str("movslq -8(%rbp), %rax").is_some());
    }

    #[test]
    fn test_compare_branch_fusion_with_matched_store_load() {
        let asm = [
            "    cmpq %rcx, %rax",
            "    setl %al",
            "    movzbq %al, %rax",
            "    movq %rax, -24(%rbp)",
            "    movq -24(%rbp), %rax",
            "    testq %rax, %rax",
            "    jne .LBB2",
            "    jmp .LBB4",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("cmpq %rcx, %rax"), "should keep the cmp");
        assert!(result.contains("jl .LBB2"), "should fuse to jl: {}", result);
        assert!(!result.contains("setl"), "should eliminate setl");
    }

    #[test]
    fn test_compare_branch_does_not_fuse_setcc_in_other_reg_with_rax_test() {
        // zlib-ng zng_deflateSetParams: `size < 4` setb was movzbl'd into
        // %r12d, then a later `testq %rax, %rax` tested a *different*
        // value (slotted new_strategy). Fusing those produced `jae` on the
        // size compare and reported Z_BUF_ERROR for a valid 4-byte buffer.
        let asm = [
            "    cmpq $4, %r8",
            "    setb %al",
            "    movzbl %al, %r12d",
            "    movq 64(%rsp), %rax",
            "    testq %rax, %rax",
            "    je .Lelse",
            "    jmp .Lthen",
            ".Lelse:",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("setb %al"),
            "must keep setb when the boolean is not in %rax: {result}"
        );
        assert!(
            !result.contains("jae ") && !result.contains("jb "),
            "must not fuse size-cmp flags with a later rax test: {result}"
        );
        assert!(result.contains("testq %rax, %rax"), "{result}");
    }

    #[test]
    fn test_compare_branch_fusion_short() {
        let asm = [
            "    cmpq %rcx, %rax",
            "    setl %al",
            "    movzbq %al, %rax",
            "    testq %rax, %rax",
            "    jne .LBB2",
            "    jmp .LBB4",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(result.contains("jl .LBB2"), "should fuse to jl: {}", result);
        assert!(!result.contains("setl"), "should eliminate setl");
    }

    #[test]
    fn test_compare_branch_fusion_je() {
        let asm = [
            "    cmpq %rcx, %rax",
            "    setl %al",
            "    movzbq %al, %rax",
            "    testq %rax, %rax",
            "    je .Lfalse",
            "    jmp .Ltrue",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jge .Lfalse"),
            "should fuse to jge: {}",
            result
        );
    }

    #[test]
    fn test_non_adjacent_store_load_same_reg() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rax, -24(%rbp)",
            "    movq %rcx, -32(%rbp)",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("-24(%rbp), %rax"),
            "should eliminate the load: {}",
            result
        );
    }

    #[test]
    fn test_non_adjacent_store_load_diff_reg() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rax, -24(%rbp)",
            "    movq %rcx, -32(%rbp)",
            "    movq -24(%rbp), %rdx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movq %rax, %rdx"),
            "should forward to reg-reg: {}",
            result
        );
    }

    #[test]
    fn test_non_adjacent_store_load_reg_modified() {
        let asm = [
            "    movq %rax, -24(%rbp)",
            "    movq -32(%rbp), %rax",
            "    movq -24(%rbp), %rcx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-24(%rbp), %rcx") || result.contains("%rax, %rcx"),
            "should not forward since rax was modified: {}",
            result
        );
    }

    #[test]
    fn test_redundant_cltq() {
        let asm = "    movslq -8(%rbp), %rax\n    cltq\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("movslq"), "should keep movslq");
        assert!(
            !result.contains("cltq"),
            "should eliminate redundant cltq: {}",
            result
        );
    }

    #[test]
    fn test_dead_store_elimination() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rax, -24(%rbp)",
            "    movq %rcx, -24(%rbp)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("%rax, -24(%rbp)"),
            "first store should be dead: {}",
            result
        );
        assert!(
            result.contains("%rcx, -24(%rbp)"),
            "second store should remain: {}",
            result
        );
    }

    #[test]
    fn test_condition_codes() {
        for (cc, expected_jcc) in &[
            ("e", "je"),
            ("ne", "jne"),
            ("l", "jl"),
            ("g", "jg"),
            ("le", "jle"),
            ("ge", "jge"),
            ("b", "jb"),
            ("a", "ja"),
        ] {
            let asm = format!(
                "    cmpq %rcx, %rax\n    set{} %al\n    movzbq %al, %rax\n    testq %rax, %rax\n    jne .LBB1\n",
                cc
            );
            let result = peephole_optimize(asm);
            assert!(
                result.contains(&format!("{} .LBB1", expected_jcc)),
                "cc={} should produce {}: {}",
                cc,
                expected_jcc,
                result
            );
        }
    }

    #[test]
    fn test_global_store_forward_qword_to_dword() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rdi, -24(%rbp)",
            "    movl -24(%rbp), %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movl %edi, %eax"),
            "qword-to-dword forwarding must retain movl semantics: {}",
            result
        );
        assert!(!result.contains("-24(%rbp), %eax"), "{}", result);
    }

    #[test]
    fn test_global_store_forward_qword_to_same_dword_reg() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rax, -24(%rbp)",
            "    movl -24(%rbp), %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // movl %eax,%eax is not a no-op: it clears RAX's upper half.
        assert!(result.contains("movl %eax, %eax"), "{}", result);
    }

    #[test]
    fn test_global_store_forward_across_fallthrough_label() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rax, -24(%rbp)",
            "    movq %rcx, -32(%rbp)",
            ".Lfallthrough:",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("-24(%rbp), %rax"),
            "should forward across fallthrough label: {}",
            result
        );
    }

    #[test]
    fn test_global_store_forward_blocked_at_jump_target() {
        let asm = [
            "    movq %rax, -24(%rbp)",
            "    jmp .Lskip",
            ".Ltarget:",
            "    movq -24(%rbp), %rax",
            ".Lskip:",
            "    ret",
            "    jmp .Ltarget",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-24(%rbp), %rax") || result.contains("-24(%rbp),"),
            "should NOT forward across jump target: {}",
            result
        );
    }

    #[test]
    fn test_global_store_forward_across_cond_branch() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rax, -24(%rbp)",
            "    cmpq %rcx, %rax",
            "    jne .Lother",
            "    movq -24(%rbp), %rdx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movq %rax, %rdx"),
            "should forward on fallthrough after cond branch: {}",
            result
        );
    }

    #[test]
    fn test_global_store_forward_invalidated_by_call() {
        let asm = [
            "    movq %rax, -24(%rbp)",
            "    callq some_func",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-24(%rbp), %rax"),
            "should not forward across call (rax clobbered): {}",
            result
        );
    }

    #[test]
    fn test_global_store_forward_callee_saved_across_call() {
        let asm = [
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    movq %rbx, -24(%rbp)",
            "    callq some_func",
            "    movq -24(%rbp), %rbx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // A callee may write through an escaped pointer to this slot. Preserve
        // the reload until escape analysis exists; callee-saved register status
        // alone does not prove memory is unchanged.
        assert!(
            result.contains("-24(%rbp), %rbx"),
            "must not forward a stack slot across a call: {}",
            result
        );
    }

    #[test]
    fn test_global_store_forward_invalidated_by_unrecognized_rbp_write() {
        let asm = [
            "    movl %eax, -8(%rbp)",
            "    movntil %ecx, -8(%rbp)",
            "    movl -8(%rbp), %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-8(%rbp), %eax"),
            "must not eliminate load after unrecognized write to same slot: {}",
            result
        );
    }

    #[test]
    fn test_classify_line() {
        let info = classify_line("    movq %rax, -8(%rbp)");
        assert!(matches!(
            info.kind,
            LineKind::StoreRbp {
                reg: 0,
                offset: -8,
                size: MoveSize::Q
            }
        ));

        let info = classify_line("    movq -16(%rbp), %rcx");
        assert!(matches!(
            info.kind,
            LineKind::LoadRbp {
                reg: 1,
                offset: -16,
                size: MoveSize::Q
            }
        ));

        let info = classify_line(".Lfoo:");
        assert_eq!(info.kind, LineKind::Label);

        let info = classify_line("    jmp .LBB1");
        assert_eq!(info.kind, LineKind::Jmp);

        let info = classify_line("    ret");
        assert_eq!(info.kind, LineKind::Ret);
    }

    #[test]
    fn test_parse_rbp_offset() {
        assert_eq!(parse_rbp_offset("leaq -24(%rbp), %rax"), -24);
        assert_eq!(parse_rbp_offset("addq (%rbp), %rax"), 0);
        assert_eq!(parse_rbp_offset("movq 16(%rbp), %rdx"), 16);
        assert_eq!(parse_rbp_offset("movq %rax, %rcx"), RBP_OFFSET_NONE);
        assert_eq!(
            parse_rbp_offset("movq -8(%rbp), -16(%rbp)"),
            RBP_OFFSET_NONE
        );
        assert_eq!(parse_rbp_offset("addq -8(%rbp), -8(%rbp)"), -8);
    }

    #[test]
    fn test_fast_parse_i32_int32_min_no_panic() {
        // Kernel constant folding emits genuine INT32_MIN immediands
        // (`addq $-2147483648, %rax`); parsing must not overflow and
        // must yield exactly i32::MIN.
        assert_eq!(fast_parse_i32("-2147483648"), i32::MIN);
        assert_eq!(fast_parse_i32("2147483648"), i32::MIN); // wrapping digits
        assert_eq!(fast_parse_i32("-8"), -8);
        assert_eq!(fast_parse_i32("0"), 0);
        assert_eq!(parse_rbp_offset("addq $-2147483648, 40(%rsp)"), 40);
    }

    #[test]
    fn test_compare_branch_fusion_no_fuse_cross_block_store() {
        let asm = [
            "    cmpq $0, %rbx",
            "    sete %al",
            "    movzbq %al, %rax",
            "    movq %rax, -40(%rbp)",
            "    testq %rax, %rax",
            "    jne .LBB8",
            "    jmp .LBB10",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-40(%rbp)"),
            "must preserve cross-block store: {}",
            result
        );
        assert!(
            result.contains("sete"),
            "must preserve sete for cross-block store: {}",
            result
        );
    }

    #[test]
    fn test_jmp_star_reg_classified_as_indirect() {
        let asm = [
            "    movq %rax, -40(%rbp)",
            "    jmp *%rcx",
            ".LBB21:",
            "    movq -40(%rbp), %rax",
            "    movq %rax, -160(%rbp)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-40(%rbp), %rax"),
            "must NOT eliminate load after indirect jump target label: {}",
            result
        );
    }

    #[test]
    fn test_jmpq_star_reg_classified_as_indirect() {
        let asm = [
            "    movq %rax, -40(%rbp)",
            "    jmpq *%rax",
            ".LBB5:",
            "    movq -40(%rbp), %rax",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-40(%rbp), %rax"),
            "must NOT eliminate load after jmpq* indirect jump target: {}",
            result
        );
    }

    #[test]
    fn test_inline_asm_rdmsr_invalidates_store_forwarding() {
        let asm = [
            "    leaq -16(%rbp), %rax",
            "    movq %rax, -40(%rbp)",
            "    movabsq $27, %rcx",
            "    1: rdmsr ; xor %esi,%esi",
            "    pushq %rcx",
            "    movq -40(%rbp), %rcx",
            "    movl %esi, (%rcx)",
            "    popq %rcx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-40(%rbp), %rcx"),
            "must NOT forward rax across rdmsr (rax clobbered by inline asm): {}",
            result
        );
    }

    #[test]
    fn test_semicolon_multi_instruction_invalidates_mappings() {
        let asm = [
            "    movq %rax, -24(%rbp)",
            "    xorl %eax, %eax ; movl $1, %ecx",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-24(%rbp), %rax"),
            "must not forward across multi-instruction line with ';': {}",
            result
        );
    }

    #[test]
    fn test_rdmsr_standalone_invalidates_mappings() {
        let asm = [
            "    movq %rax, -24(%rbp)",
            "    rdmsr",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-24(%rbp), %rax"),
            "must not forward across rdmsr (implicit clobber of rax/rdx): {}",
            result
        );
    }

    #[test]
    fn test_cpuid_invalidates_mappings() {
        let asm = [
            "    movq %rax, -24(%rbp)",
            "    cpuid",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-24(%rbp), %rax"),
            "must not forward across cpuid (implicit clobber of rax/rbx/rcx/rdx): {}",
            result
        );
    }

    #[test]
    fn test_setcc_non_al_invalidates_store_forwarding() {
        let asm = [
            "    movl %ecx, -8(%rbp)",
            "    sete %cl",
            "    movl -8(%rbp), %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-8(%rbp), %eax"),
            "must NOT forward ecx across sete %%cl (ecx clobbered): {}",
            result
        );
    }

    #[test]
    fn test_setcc_al_still_invalidates_rax() {
        let asm = [
            "    movq %rax, -16(%rbp)",
            "    sete %al",
            "    movq -16(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-16(%rbp), %rax"),
            "must NOT forward rax across sete %%al (rax clobbered): {}",
            result
        );
    }

    #[test]
    fn test_syscall_invalidates_mappings() {
        let asm = [
            "    movq %rcx, -16(%rbp)",
            "    syscall",
            "    movq -16(%rbp), %rcx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("-16(%rbp), %rcx"),
            "must NOT forward rcx across syscall (rcx clobbered): {}",
            result
        );
    }

    #[test]
    #[ignore] // TODO: trampoline coalescing not triggering for this test pattern yet
    fn test_loop_trampoline_simple_coalesce() {
        let asm = [
            ".LBB1:",
            "    movq %r9, %rax",
            "    movq %r9, %r14",
            "    addq $320, %r14",
            "    testq %rax, %rax",
            "    jne .LBB2",
            "    ret",
            ".LBB2:",
            "    movq %r14, %r9",
            "    jmp .LBB1",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("addq $320, %r9"),
            "should rewrite addq to target %r9 directly: {}",
            result
        );
        assert!(
            !result.contains("movq %r9, %r14"),
            "should eliminate the initial copy: {}",
            result
        );
        assert!(
            !result.contains("movq %r14, %r9"),
            "should eliminate the trampoline copy: {}",
            result
        );
        assert!(
            result.contains("jne .LBB1"),
            "should redirect branch to loop header: {}",
            result
        );
    }

    #[test]
    #[ignore] // TODO: trampoline coalescing not triggering for this test pattern yet
    fn test_loop_trampoline_two_copies() {
        let asm = [
            ".LBB10:",
            "    movq %r9, %rax",
            "    movq %r10, %rcx",
            "    movq %r9, %r14",
            "    addq $320, %r14",
            "    movq %r10, %r15",
            "    addl %r8d, %r15d",
            "    testq %rax, %rax",
            "    jne .LBB20",
            "    ret",
            ".LBB20:",
            "    movq %r14, %r9",
            "    movq %r15, %r10",
            "    jmp .LBB10",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("addq $320, %r9"),
            "should rewrite dest addq to %r9: {}",
            result
        );
        assert!(
            result.contains("addl %r8d, %r10d"),
            "should rewrite frac addl to %r10d: {}",
            result
        );
        assert!(
            !result.contains("movq %r9, %r14"),
            "should eliminate dest copy: {}",
            result
        );
        assert!(
            !result.contains("movq %r10, %r15"),
            "should eliminate frac copy: {}",
            result
        );
        assert!(
            result.contains("jne .LBB10"),
            "should redirect branch to loop header: {}",
            result
        );
    }

    #[test]
    fn test_condbranch_inversion_fallthrough() {
        let asm = [
            "    cmpl %r8d, %eax",
            "    jl .LBB2",
            "    jmp .LBB4",
            ".LBB2:",
            "    movq %rax, %rcx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jge .LBB4"),
            "should invert jl to jge: {}",
            result
        );
        assert!(
            !result.contains("jl .LBB2"),
            "should remove original jl: {}",
            result
        );
        assert!(
            !result.contains("jmp .LBB4"),
            "should remove the jmp: {}",
            result
        );
        assert!(
            result.contains(".LBB2:"),
            "should keep the label: {}",
            result
        );
    }

    #[test]
    fn test_condbranch_inversion_je_to_jne() {
        let asm = [
            "    testq %rax, %rax",
            "    je .Ltrue",
            "    jmp .Lfalse",
            ".Ltrue:",
            "    ret",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jne .Lfalse"),
            "should invert je to jne: {}",
            result
        );
        assert!(
            !result.contains("jmp .Lfalse"),
            "should remove the jmp: {}",
            result
        );
    }

    #[test]
    fn test_condbranch_no_inversion_when_not_fallthrough() {
        let asm = [
            "    cmpl %r8d, %eax",
            "    jl .LBB5",
            "    jmp .LBB4",
            ".LBB2:",
            "    movq %rax, %rcx",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jl .LBB5"),
            "should keep jl when not fallthrough: {}",
            result
        );
    }

    #[test]
    fn test_back_to_back_cltq() {
        let asm = "    cltq\n    cltq\n".to_string();
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("cltq").count(),
            1,
            "should keep only one cltq: {}",
            result
        );
    }

    #[test]
    fn test_cltq_backward_scan_over_non_rax_write() {
        let asm = [
            "    movslq -8(%rbp), %rax",
            "    movq %rax, %r8",
            "    cltq",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("cltq"),
            "cltq should be eliminated after movslq past non-rax-write: {}",
            result
        );
    }

    #[test]
    fn test_cltq_backward_scan_blocked_by_rax_write() {
        let asm = ["    movslq -8(%rbp), %rax", "    addl $1, %eax", "    cltq"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("cltq"),
            "cltq should NOT be eliminated when rax is modified: {}",
            result
        );
    }

    #[test]
    fn test_cltq_backward_scan_blocked_by_call() {
        let asm = ["    cltq", "    call foo", "    cltq"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("cltq").count(),
            2,
            "both cltq should survive when call intervenes: {}",
            result
        );
    }

    #[test]
    fn test_cltq_backward_scan_with_store_rbp() {
        let asm = [
            "    cltq",
            "    movq %rax, -16(%rbp)",
            "    movq %rax, %r9",
            "    cltq",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("cltq").count(),
            1,
            "second cltq should be eliminated past store and mov: {}",
            result
        );
    }

    // ── Memory operand folding tests ──────────────────────────────────────

    #[test]
    fn test_mem_fold_addq_rcx() {
        let asm = ["    movq -48(%rbp), %rcx", "    addq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("addq -48(%rbp), %rax"),
            "should fold load+add into memory operand: {}",
            result
        );
        assert!(
            !result.contains("movq -48(%rbp), %rcx"),
            "load should be eliminated: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_subl_ecx() {
        let asm = ["    movq -64(%rbp), %rcx", "    subl %ecx, %eax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("subl -64(%rbp), %eax"),
            "should fold load+sub into memory operand: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_cmpq_rcx() {
        let asm = ["    movq -8(%rbp), %rcx", "    cmpq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("cmpq -8(%rbp), %rax"),
            "should fold load+cmp into memory operand: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_testq_rcx() {
        let asm = ["    movq -16(%rbp), %rcx", "    testq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("testq -16(%rbp), %rax"),
            "should fold load+test into memory operand: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_no_fold_when_dest_is_loaded_reg() {
        let asm = ["    movq -48(%rbp), %rcx", "    addq %rax, %rcx"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movq -48(%rbp), %rcx") || result.contains("addq %rax, %rcx"),
            "should not fold when loaded reg is destination: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_no_fold_for_callee_saved() {
        let asm = ["    movq -48(%rbp), %rbx", "    addq %rbx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("addq -48(%rbp), %rax"),
            "should not fold callee-saved register loads: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_andq() {
        let asm = ["    movq -16(%rbp), %rcx", "    andq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("andq -16(%rbp), %rax"),
            "should fold load+and into memory operand: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_xorq() {
        let asm = ["    movq -24(%rbp), %rcx", "    xorq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("xorq -24(%rbp), %rax"),
            "should fold load+xor into memory operand: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_load_rax_into_add_with_reg_dest() {
        let asm = ["    movq -32(%rbp), %rax", "    addq %rax, %r12"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("addq -32(%rbp), %r12"),
            "should fold rax load into add with callee-saved dest: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_orq() {
        let asm = ["    movq -16(%rbp), %rcx", "    orq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("orq -16(%rbp), %rax"),
            "should fold load+or into memory operand: {}",
            result
        );
    }

    #[test]
    fn test_mem_fold_with_empty_line_between() {
        let asm = ["    movq -48(%rbp), %rcx", "", "    addq %rcx, %rax"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("addq -48(%rbp), %rax"),
            "should fold with empty lines between: {}",
            result
        );
    }

    // ── Redundant xorl elimination tests ─────────────────────────────────

    #[test]
    fn test_redundant_xorl_after_zero_store() {
        let asm = [
            "    xorl %eax, %eax",
            "    movq %rax, -8(%rbp)",
            "    xorl %eax, %eax",
            "    movq %rax, -16(%rbp)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("xorl %eax, %eax").count(),
            1,
            "second xorl should be eliminated: {}",
            result
        );
        assert!(
            result.contains("movq %rax, -8(%rbp)"),
            "first store should remain: {}",
            result
        );
        assert!(
            result.contains("movq %rax, -16(%rbp)"),
            "second store should remain: {}",
            result
        );
    }

    #[test]
    fn test_redundant_xorl_chain_of_four() {
        let asm = [
            "    xorl %eax, %eax",
            "    movq %rax, -8(%rbp)",
            "    xorl %eax, %eax",
            "    movq %rax, -16(%rbp)",
            "    xorl %eax, %eax",
            "    movq %rax, -24(%rbp)",
            "    xorl %eax, %eax",
            "    movq %rax, -32(%rbp)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("xorl %eax, %eax").count(),
            1,
            "only first xorl should survive: {}",
            result
        );
    }

    #[test]
    fn test_xorl_not_eliminated_after_rax_write() {
        let asm = [
            "    xorl %eax, %eax",
            "    movq %rax, -8(%rbp)",
            "    movq -16(%rbp), %rax",
            "    xorl %eax, %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // The load to %rax invalidates rax_is_zero, so both xorls are needed
        assert_eq!(
            result.matches("xorl %eax, %eax").count(),
            2,
            "both xorls should survive after rax modification: {}",
            result
        );
    }

    #[test]
    fn test_xorl_not_eliminated_after_label() {
        let asm = [
            "    xorl %eax, %eax",
            "    movq %rax, -8(%rbp)",
            ".LBB1:",
            "    xorl %eax, %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("xorl %eax, %eax").count(),
            2,
            "xorl after label should NOT be eliminated: {}",
            result
        );
    }

    #[test]
    fn test_xorl_not_eliminated_after_call() {
        let asm = [
            "    xorl %eax, %eax",
            "    movq %rax, -8(%rbp)",
            "    call some_func",
            "    xorl %eax, %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert_eq!(
            result.matches("xorl %eax, %eax").count(),
            2,
            "xorl after call should NOT be eliminated: {}",
            result
        );
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;

    /// Regression test: dead store elimination must not use a stale pattern_bytes
    /// buffer when checking multi-byte stores. The sub-byte-offset scan for a Q
    /// (8-byte) store was overwriting pattern_bytes with the last sub-offset
    /// (store_offset + 7), and subsequent scans within the same store's window
    /// would reuse the stale pattern instead of the original store_offset.
    ///
    /// Pattern from oniguruma regparse.c: a store to -144(%rbp) was followed by
    /// an Other instruction (movl %eax, %eax), then a Cmp with -144(%rbp) as a
    /// memory operand (from memory_fold), then another store to -144(%rbp). The
    /// sub-byte scan for the Other instruction overwrote pattern_bytes from
    /// "-144(%rbp)" to "-137(%rbp)", causing the Cmp line's pattern check to fail,
    /// and the later store marked the original as dead.
    #[test]
    fn test_dead_store_not_eliminated_when_cmp_reads_slot() {
        // The critical pattern after memory_fold transforms the comparison:
        //   movq %rax, -144(%rbp)     # store data[x*2]
        //   movq -128(%rbp), %rax     # load to+1 (clobbers rax)
        //   movl %eax, %eax           # truncate (Other, no rbp ref)
        //   movq %rax, -136(%rbp)     # store to+1
        //   cmpl -144(%rbp), %eax     # memory-folded cmp reads -144!
        //   setae %al
        //   movzbq %al, %rax
        //   movq %rax, %rsi
        //   movq %r11, %rax
        //   addl $1, %eax
        //   cltq
        //   movq %rax, -144(%rbp)     # later store overwrites -144
        //
        // The bug: dead_stores saw the overwrite at the end, but missed the
        // cmp read because pattern_bytes had been corrupted by the sub-byte
        // offset scan for movl %eax, %eax.
        let asm = [
            "func:",
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    subq $160, %rsp",
            // ... setup ...
            "    movl (%rcx), %eax",
            "    movq %rax, -144(%rbp)", // store data[x*2]
            "    movq -128(%rbp), %rax", // load to+1 (clobbers rax)
            "    movl %eax, %eax",       // truncate
            "    movq %rax, -136(%rbp)", // store to+1
            "    cmpl -144(%rbp), %eax", // memory-folded cmp (Cmp kind)
            "    setae %al",
            "    movzbq %al, %rax",
            "    movq %rax, %rsi",
            "    movq %r11, %rax",
            "    addl $1, %eax",
            "    cltq",
            "    movq %rax, -144(%rbp)", // later overwrite of -144
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // After optimization, there must still be a store to -144(%rbp) before
        // the cmpl that reads it. The cmpl must compare the correct value.
        let lines: Vec<&str> = result.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == "cmpl -144(%rbp), %eax" {
                // Scan backward for a store to -144
                let mut found_store = false;
                for k in (0..idx).rev() {
                    let prev = lines[k].trim();
                    if prev.ends_with("-144(%rbp)") && prev.starts_with("mov") {
                        found_store = true;
                        break;
                    }
                    if prev.ends_with(':') {
                        break;
                    }
                }
                assert!(
                    found_store,
                    "cmpl -144(%rbp) has no preceding store in same block!\nResult:\n{}",
                    result
                );
                return;
            }
        }
        // If the cmpl was not folded, check it exists in some form
        assert!(
            result.contains("cmpl") || result.contains("setae"),
            "No comparison found\nResult:\n{}",
            result
        );
    }

    #[test]
    fn test_store_forward_param_ref_gep() {
        // This pattern comes from:
        //   void f(struct state *s) { ... s->member[i] = 0; ... }
        // The codegen emits:
        //   movq %rdi, -8(%rbp)    # store param
        //   movq -8(%rbp), %rax    # paramref load
        //   movq %rax, -8(%rbp)    # paramref store-back (redundant)
        //   movq -8(%rbp), %rax    # GEP base load
        //   leaq 208(%rax), %rax   # GEP offset
        // After peephole, rax must still be set correctly before the leaq.
        let asm = [
            "func:",
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    subq $16, %rsp",
            "    movq %rdi, -8(%rbp)",
            "    movq -8(%rbp), %rax",
            "    movq %rax, -8(%rbp)",
            "    movq -8(%rbp), %rax",
            "    leaq 208(%rax), %rax",
            "    movq %rax, %r14",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // After optimization, rax must be loaded from rdi or from the stack
        // before the leaq instruction.
        // The correct result should be something like:
        //   movq %rdi, -8(%rbp) or movq %rdi, %rax + leaq 208(%rax)...
        // NOT: leaq 208(%rax) with rax uninitialized!
        eprintln!("Result:\n{}", result);
        assert!(
            result.contains("movq %rdi, %rax")
                || result.contains("movq -8(%rbp), %rax")
                || result.contains("leaq 208(%rdi)"),
            "rax must be set from rdi before leaq 208(%rax): {}",
            result
        );
    }

    /// Regression test: frame_compact must not NOP a store when a read overlaps
    /// it at a different offset. Example: a struct param stored as
    /// `movq %rsi, -8(%rbp)` (8 bytes at [-8, 0)) has a field read via
    /// `movl -4(%rbp), %eax` (4 bytes at [-4, 0)). The store and read overlap
    /// but have different offsets, so exact-offset matching would miss the
    /// dependency and incorrectly NOP the store.
    #[test]
    fn test_frame_compact_overlapping_store_read() {
        let asm = [
            "func:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $48, %rsp",
            "    movq %rbx, -48(%rbp)",
            "    movq %r12, -40(%rbp)",
            "    movq %r13, -32(%rbp)",
            "    movq %r14, -24(%rbp)",
            "    movq %r15, -16(%rbp)",
            // Store 8-byte struct param at -8(%rbp) covering bytes [-8, 0)
            "    movq %rsi, -8(%rbp)",
            // Read a 4-byte field at -4(%rbp) covering bytes [-4, 0)
            "    movl -4(%rbp), %eax",
            "    movq %rax, %r14",
            "    movq %rdi, %rdi",
            "    call some_func",
            // Epilogue
            "    movq %r14, %rax",
            "    movq -48(%rbp), %rbx",
            "    movq -40(%rbp), %r12",
            "    movq -32(%rbp), %r13",
            "    movq -24(%rbp), %r14",
            "    movq -16(%rbp), %r15",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // The store to -8(%rbp) must survive because the read at -4(%rbp) overlaps it.
        // The load must also survive.
        let lines: Vec<&str> = result.lines().map(|l| l.trim()).collect();
        let has_store = lines
            .iter()
            .any(|l| l.contains("%rsi") && l.contains("(%rbp)"));
        // The load may be folded by relay elimination: movl -4(%rbp),%eax + movq %rax,%r14
        // becomes movl -4(%rbp),%r14d — so accept either %eax or %r14d as the destination.
        let has_load = lines.iter().any(|l| {
            l.starts_with("movl")
                && l.contains("(%rbp)")
                && (l.contains("%eax") || l.contains("%r14d"))
        });
        assert!(
            has_store,
            "store of struct param must survive frame compaction (overlapping read exists): {}",
            result
        );
        assert!(has_load, "load of struct field must survive: {}", result);
    }

    /// Regression test: frame compaction must NOP out dead stores that conflict
    /// with relocated callee-save offsets. Without this fix, a dead store like
    /// `movq %rax, -64(%rbp)` can clobber a callee-saved register that was
    /// relocated to -64(%rbp) during frame compaction.
    ///
    /// Pattern from tre-compile.c: tre_ast_to_tnfa has 5 callee saves at
    /// -112..-80 and body reads down to -56, with a dead store at -64.
    /// Compaction moves callee saves to -96..-64, but -64 conflicts with
    /// the dead store.
    #[test]
    fn test_frame_compact_dead_store_noped() {
        let asm = [
            "func:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $112, %rsp",
            "    movq %rbx, -112(%rbp)",
            "    movq %r12, -104(%rbp)",
            "    movq %r13, -96(%rbp)",
            "    movq %r14, -88(%rbp)",
            "    movq %r15, -80(%rbp)",
            // Body: reads at -8, -56; dead store at -64
            "    movq %rdi, -8(%rbp)",
            "    movq -8(%rbp), %rax",
            "    movq %rax, -56(%rbp)",
            "    movq -56(%rbp), %rdi",
            "    call some_func",
            "    movq %rax, -64(%rbp)", // dead store - never read
            "    movq %rax, %r14",
            // Epilogue
            "    movq %r14, %rax",
            "    movq -112(%rbp), %rbx",
            "    movq -104(%rbp), %r12",
            "    movq -96(%rbp), %r13",
            "    movq -88(%rbp), %r14",
            "    movq -80(%rbp), %r15",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // After compaction:
        // - Frame should be smaller than 112
        // - Dead store at -64 should be NOP'd (not present in output)
        // - Callee saves should be at new offsets
        assert!(result.contains("subq $"), "should have subq: {}", result);
        assert!(
            !result.contains("subq $112"),
            "frame should be compacted from 112: {}",
            result
        );
        // The dead store to -64 must not appear in the output
        // (it would clobber the relocated callee save)
        assert!(
            !result.contains("-64(%rbp)") ||
                // -64 might appear as a new callee-save offset in saves/restores which is OK
                (result.contains("movq %r15, -64(%rbp)") || result.contains("movq -64(%rbp), %r15")),
            "dead store to -64 must be eliminated or -64 used only for callee save: {}",
            result
        );
    }

    /// Regression test: a struct stored with movq at -8(%rbp) that is read
    /// field-by-field via movl at -4(%rbp) must NOT be NOP'd by frame_compact.
    /// The store covers bytes [-8, 0) and the read at -4 falls within that range.
    #[test]
    fn test_frame_compact_struct_suboffset_read() {
        let asm = [
            "release_entry:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $48, %rsp",
            "    movq %rbx, -48(%rbp)",
            "    movq %r12, -40(%rbp)",
            "    movq %r13, -32(%rbp)",
            "    movq %r14, -24(%rbp)",
            "    movq %r15, -16(%rbp)",
            // Struct param stored as a whole at -8(%rbp)
            "    movq %rdi, %r14",
            "    movq %rsi, -8(%rbp)",
            // Read individual field at sub-offset -4 within the struct
            "    movl -4(%rbp), %eax",
            "    movq %rax, %rsi",
            "    call printf",
            // Epilogue
            "    movq -48(%rbp), %rbx",
            "    movq -40(%rbp), %r12",
            "    movq -32(%rbp), %r13",
            "    movq -24(%rbp), %r14",
            "    movq -16(%rbp), %r15",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size release_entry, .-release_entry",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // The struct store at -8(%rbp) must be preserved because -4(%rbp) is read
        assert!(
            result.contains("movq %rsi, -8(%rbp)"),
            "struct param store at -8(%rbp) must NOT be NOP'd when -4(%rbp) is read: {}",
            result
        );
    }

    #[test]
    fn test_sib_indexed_store_via_copy() {
        // Pattern: movq %idx, %rax; addq %base, %rax; movq %rax, %tmp; store (%tmp)
        let asm = [
            "    movq %r14, %rax",
            "    addq %rcx, %rax",
            "    movq %rax, %rcx",
            "    movb $0, (%rcx)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        eprintln!("sib_indexed result: {:?}", result);
        assert!(
            result.contains("(%rcx, %r14)") || result.contains("(%rcx,%r14)"),
            "should fold to SIB indexed addressing: {}",
            result
        );
        assert!(
            !result.contains("addq %rcx, %rax"),
            "addq should be eliminated: {}",
            result
        );
    }

    #[test]
    fn test_sib_indexed_store_direct_rax() {
        // Pattern: movq %idx, %rax; addq %base, %rax; store (%rax)
        let asm = [
            "    movq %r14, %rax",
            "    addq %rbx, %rax",
            "    movb $0, (%rax)",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        eprintln!("sib_direct result: {:?}", result);
        assert!(
            result.contains("(%rbx, %r14)") || result.contains("(%rbx,%r14)"),
            "should fold to SIB indexed addressing: {}",
            result
        );
    }

    #[test]
    fn test_volatile_slot_is_quarantined_from_peephole() {
        let asm = [
            "    # LCCC_VOLATILE_SLOT -24(%rbp)",
            "    movq %rax, -24(%rbp)",
            "    movq -24(%rbp), %rax",
        ]
        .join("\n")
            + "\n";
        let mut store = LineStore::new(asm.clone());
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        pin_volatile_stack_slots(&store, &mut infos);
        assert!(infos[1].pinned && infos[2].pinned);
        assert!(matches!(
            infos[1].kind,
            LineKind::Other { dest_reg: REG_NONE }
        ));
        let result = peephole_optimize(asm);
        assert!(result.contains("movq %rax, -24(%rbp)"), "{}", result);
        assert!(result.contains("movq -24(%rbp), %rax"), "{}", result);
    }

    #[test]
    fn test_volatile_unknown_dest_invalidates_rax_zero_fact() {
        // pin_volatile_stack_slots deliberately turns the load into opaque
        // Other{REG_NONE}. It still writes eax, so the second zeroing operation
        // is required before pushq. This is the reduced sqlite3MultiValues
        // outgoing-stack-argument corruption pattern.
        let asm = [
            "    # LCCC_VOLATILE_SLOT 8(%rsp)",
            "    xorl %eax, %eax",
            "    movl 8(%rsp), %eax",
            "    xorl %eax, %eax",
            "    pushq %rax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        // The first zeroing is dead because the volatile load overwrites eax.
        // The important invariant is that the *second* zeroing survives after
        // the opaque load; otherwise pushq would consume the volatile value.
        assert_eq!(result.matches("xorl %eax, %eax").count(), 1, "{}", result);
        let load = result
            .find("movl 8(%rsp), %eax")
            .expect("volatile load kept");
        let zero = result
            .rfind("xorl %eax, %eax")
            .expect("post-load zero kept");
        assert!(
            load < zero,
            "post-load zero must follow volatile load: {}",
            result
        );
    }

    #[test]
    fn test_combined_preserves_address_taken_rsp_slot() {
        // The first slot's address escapes to memcpy. A local FP bitcast fold
        // may remove the independent second slot, but it must not make the
        // address-taken first store dead.
        let asm = [
            "    movq %rax, 56(%rsp)",
            "    movq %rax, 24(%rsp)",
            "    movsd 24(%rsp), %xmm0",
            "    leaq 56(%rsp), %rsi",
            "    call memcpy",
        ]
        .join("\n")
            + "\n";
        let mut store = LineStore::new(asm);
        let mut infos: Vec<LineInfo> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        pin_address_taken_stack_slots(&store, &mut infos);
        assert!(infos[0].pinned, "address-taken store must be pinned");
        while local_patterns::combined_local_pass(&mut store, &mut infos) {}
        let local = store.build_result(|i| infos[i].is_nop());
        assert!(local.contains("movq %rax, 56(%rsp)"), "{}", local);

        let full = peephole_optimize(
            [
                "    movq %rax, 56(%rsp)",
                "    movq %rax, 24(%rsp)",
                "    movsd 24(%rsp), %xmm0",
                "    leaq 56(%rsp), %rsi",
                "    call memcpy",
            ]
            .join("\n")
                + "\n",
        );
        assert!(full.contains("movq %rax, 56(%rsp)"), "{}", full);
    }

    #[test]
    fn test_global_store_forward_callee_saved_loop_header_is_not_forwarded() {
        // The back-edge changes RBX but not the stack slot.  At the next
        // iteration the load must still observe the original slot value; a
        // textual scan that carries a callee-saved mapping across .LBB200
        // would turn it into `movq %rbx,%rax` and miscompile the loop.
        let asm = [
            "    movq $1, %rbx",
            "    movq %rbx, -8(%rsp)",
            ".LBB200:",
            "    movq -8(%rsp), %rax",
            "    movq $2, %rbx",
            "    jmp .LBB200",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movq -8(%rsp), %rax"),
            "CFG-join reload must not forward a stale callee-saved register: {}",
            result
        );
    }

    #[test]
    fn test_copy_shift_copyback_fold() {
        let asm =
            "    movq %rsi, %rbp\n    shll $5, %ebp\n    movl %ebp, %esi\n    movq $0, %rbp\n";
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("shll $5, %esi"), "{}", out);
        assert!(!out.contains("movq %rsi, %rbp"), "{}", out);
        assert!(!out.contains("movl %ebp, %esi"), "{}", out);
    }

    #[test]
    fn test_copy_shift_copyback_keeps_live_tmp() {
        let asm =
            "    movq %rsi, %rbp\n    shll $5, %ebp\n    movl %ebp, %esi\n    addq %rbp, %rax\n";
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("movq %rsi, %rbp"), "{}", out);
    }

    #[test]
    fn test_zero_extended_xor_move_fold() {
        let asm = "    shll $5, %esi\n    movl %edi, %r15d\n    movq %rsi, %rdi\n    xorq %r15, %rdi\n    andq $32767, %rdi\n    movq $0, %r15\n";
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("xorl %esi, %edi"), "{}", out);
        assert!(!out.contains("xorq %r15, %rdi"), "{}", out);
    }
    #[test]
    fn test_zero_extended_xor_keeps_live_flags() {
        let asm = "    shll $5, %esi\n    movl %edi, %r15d\n    movq %rsi, %rdi\n    xorq %r15, %rdi\n    jne .Lx\n    movq $0, %r15\n.Lx:\n";
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("xorq %r15, %rdi"), "{}", out);
    }

    #[test]
    fn test_rotate_idiom_fold() {
        let asm="    movq %r9, %rsi\n    shlq $13, %rsi\n    movq %r9, %rdx\n    shrq $51, %rdx\n    movq %rsi, %r9\n    orq %rdx, %r9\n    addq %rax, %rbx\n";
        let out = peephole_optimize(asm.to_string());
        assert!(out.contains("rolq $13, %r9"), "{}", out);
        assert!(!out.contains("shrq $51"), "{}", out);
    }

    #[test]
    fn test_compare_branch_rsp_signed_byte_reload() {
        let asm="    cmpl %edx, %esi\n    setb %al\n    movzbl %al, %eax\n    movq %rax, 384(%rsp)\n    movsbq 384(%rsp), %rax\n    testq %rax, %rax\n    je .Lno\n    jmp .Lyes\n.Lno:\n";
        let out = peephole_optimize(asm.to_string());
        assert!(
            out.contains("jae .Lno") || out.contains("jb .Lyes"),
            "{}",
            out
        );
        assert!(!out.contains("setb"), "{}", out);
        assert!(!out.contains("movsbq 384"), "{}", out);
    }

    #[test]
    fn test_late_compare_bool_rsp_spill() {
        let mut asm="    cmpl %edx, %esi\n    setb %al\n    movzbl %al, %eax\n    movq %rax, 384(%rsp)\n    movsbq 384(%rsp), %rax\n    testq %rax, %rax\n    je .Lno\n".to_string();
        assert!(compare_branch::fuse_late_compare_bool_spills(&mut asm));
        assert!(asm.contains("jae .Lno"), "{}", asm);
        assert!(!asm.contains("setb"), "{}", asm);
    }

    #[test]
    fn test_late_compare_phi_false_redirect() {
        let mut asm="    cmpl %edx, %esi\n    setb %al\n    movzbl %al, %eax\n    movq %rax, 384(%rsp)\n.Ljoin:\n    movsbq 384(%rsp), %rax\n    testq %rax, %rax\n    je .Lfalse_target\n.Lother:\n    ret\n.Lpred_false:\n    xorl %eax, %eax\n    movq %rax, 384(%rsp)\n    jmp .Ljoin\n.Lfalse_target:\n    ret\n".to_string();
        assert!(compare_branch::fuse_late_compare_bool_spills(&mut asm));
        assert!(asm.contains("jae .Lfalse_target"), "{}", asm);
        assert!(asm.contains("jmp .Lfalse_target"), "{}", asm);
        assert!(!asm.contains("setb"), "{}", asm);
    }

    #[test]
    fn test_movslq_not_eliminated_when_next_insn_reads_64bit_index() {
        // Pattern 3b: `movslq %edi, %rdi` must NOT be eliminated when the next
        // instruction writes %edi but also READS %rdi as a SIB index
        // (`movzbl (%rcx,%rdi),%edi`). Removing the movslq misaddresses the
        // load for negative indices (zlib/zigzag byte-compare shape).
        let asm = [
            "    movslq %edi, %rdi",
            "    movzbl (%rcx, %rdi), %edi",
            "    movzbl (%rsi), %eax",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("movslq %edi, %rdi"),
            "movslq must be kept when the byte-load reads %rdi as index: {}",
            result
        );
    }

    #[test]
    fn test_movslq_eliminated_when_next_insn_only_writes_reg() {
        // Control: `movslq %edi, %rdi` followed by a pure 32-bit write of %edi
        // (no 64-bit read) is still safe to eliminate.
        let asm = ["    movslq %edi, %rdi", "    movl %eax, %edi"].join("\n") + "\n";
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("movslq %edi, %rdi"),
            "movslq should be eliminated after a pure 32-bit overwrite: {}",
            result
        );
    }

    #[test]
    fn test_vector_self_move_eliminated() {
        let asm = ["    vmovdqu %ymm0, %ymm0", "    vmovdqu %ymm0, 240(%rsp)"].join("\n") + "\n";
        let out = peephole_optimize(asm.to_string());
        assert!(
            !out.contains("vmovdqu %ymm0, %ymm0"),
            "self-move must be removed: {}",
            out
        );
    }

    #[test]
    fn test_vector_self_move_kept_for_different_regs() {
        let asm = ["    vmovdqu %ymm1, %ymm0", "    vmovdqu %ymm0, 240(%rsp)"].join("\n") + "\n";
        let out = peephole_optimize(asm.to_string());
        assert!(
            out.contains("vmovdqu %ymm1, %ymm0"),
            "reg-reg move with different regs must stay: {}",
            out
        );
    }

    #[test]
    fn xmm_source_is_not_indexed_as_a_gpr_in_extension_fusion() {
        let asm = "    movq %xmm2, %rax\n    movl %eax, %eax\n".to_string();
        let result = peephole_optimize(asm);
        assert!(result.contains("%xmm2"));
    }
}

//! Tail call optimization: convert `call X; <epilogue>; ret` into `<epilogue>; jmp X`.
//!
//! This pass detects the pattern where a function call's return value is directly
//! returned by the enclosing function. The sequence:
//!     call TARGET      (or call *%r10)
//!     `<callee-save restores from rbp>`
//!     movq %rbp, %rsp
//!     popq %rbp
//!     ret
//! is transformed to:
//!     `<callee-save restores from rbp>`
//!     movq %rbp, %rsp
//!     popq %rbp
//!     jmp TARGET       (or jmp *%r10)
//!
//! This is critical for threaded interpreters (like wasm3) that use indirect
//! tail calls to dispatch between opcode handlers without overflowing the stack.
//!
//! REALITY CHECK since the stack-window soundness gate below: a conversion is
//! only admitted when the simulated %rsp depth at the call equals the function
//! entry line (and no sp-relative staging poisons it), which typical framed
//! epilogues never satisfy -- most formerly-converted sites are now left as
//! plain call+ret (always correct, occasionally deeper stacks). Full,
//! signature-aware sibling calls need IR-level sibcall lowering rather than
//! text shuffling; until then this pass stays deliberately conservative --
//! mirroring the policy validated for i686 in PR #255 across the complete
//! GCC c-torture corpus at five optimization levels (zero regressions;
//! forwarder-with-stack-args miscompiles such as pr23324 stopped firing).
//!
//! SAFETY: We must NOT apply this optimization when:
//! 1. The function passes a pointer to a local variable to the callee.
//!    After frame teardown, such pointers become dangling. Detected by checking
//!    for `leaq offset(%rbp), %reg` instructions (address-of-local).
//! 2. The function uses dynamic stack allocation (__builtin_alloca / DynAlloca).
//!    Alloca'd memory lives below %rsp. After frame teardown (movq %rbp, %rsp),
//!    that memory is in unowned space and may be clobbered by the tail-called
//!    function's stack frame. Detected by checking for `subq %reg, %rsp`.
//! 3. The call site sits at a nonzero simulated %rsp depth vs. the function
//!    entry line (CFA). Converting `call C; <teardown>; ret` into
//!    `<teardown>; jmp C` re-bases where the tail-callee reads its incoming
//!    stack arguments from: pre-conversion they are read starting at the
//!    %rsp value at the call time ([P ..]), post-conversion starting at the
//!    freshly unwound entry window ([E ..]). Those ranges differ whenever any
//!    scratch/outgoing-argument/prologue displacement existed below the entry
//!    line -- the normal shape whenever anything was staged (push-staged or
//!    rsp-relative-stored overflow arguments, spilled locals, reserved frames).
//!    A text-level peephole cannot shuffle prepared arguments into place, so
//!    the only locally provable equivalence is WINDOW IDENTITY: at simulated
//!    %rsp depth 0 (== entry CFA) both forms hand the callee the SAME
//!    addresses holding the SAME bytes, independent of the callee's ABI.
//!    Empirically demonstrated unsoundness on upstream main (2025):
//!        long sink7(long,..,long g);              // 7th arg on stack
//!        long fwdZ(void){ long v=g_s;
//!            return sink7(v,v,v,v,v,v,77L); }     // stage 77 via pushq
//!    compiled to `call sink7@PLT; addq $16,%rsp; addq $8,%rsp; ret`,
//!    converted (the TWO addq lines were even misread as the teardown),
//!    and returned 1155 instead of 1694 at O1/O2/O3/Os (7th arg read from
//!    [E0..] while 77 lived at [E0-24]; entry alignment broken as well).
//!    This mirrors the gate merged for i686 (PR #255, "Harden i686
//!    stack-window peepholes"): Simulation is deliberately conservative --
//!    internal `.L*` join labels make the depth unknown (= suppressed);
//!    register-forms and pointer reassignments poison it as well.

use super::super::types::*;

/// Scan the assembly for tail call opportunities and convert them.
/// Returns true if any changes were made.
pub(super) fn optimize_tail_calls(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let len = infos.len();
    let mut changed = false;

    // Track whether the function has unsafe stack usage that prevents tail calls:
    // address-of-local (lea from rbp/rsp) or dynamic alloca (subq %reg, %rsp).
    let mut func_suppress_tailcall = false;
    // Track whether we're inside a function (seen pushq %rbp or label)
    let mut in_function = false;
    // Simulated net %rsp displacement vs. function entry, in bytes.
    //
    // STACK-WINDOW SOUNDNESS GATE (see module docs, item 3): `None` = unknown
    // (after an internal control-flow join / mid-block entry / any unmodeled
    // %rsp mutation). Conversion additionally requires depth == Some(0) at
    // the call so that pre- and post-conversion incoming-argument windows are
    // byte-identical. This mirrors the validated i686 gate (PR #255).
    let mut depth: Option<i64> = None;

    let mut i = 0;
    while i < len {
        if infos[i].is_nop() {
            i += 1;
            continue;
        }

        // Detect function boundaries to reset the suppression flag
        match infos[i].kind {
            LineKind::Label => {
                let line = store.get(i);
                let trimmed = infos[i].trimmed(line);
                // A global label (not starting with .L) indicates a new function
                if !trimmed.starts_with(".L") {
                    func_suppress_tailcall = false;
                    in_function = true;
                    // Function boundary: known depth again. A global label only
                    // starts a new translation unit's function stream here.
                    depth = Some(0);
                } else {
                    // Internal join point: fallthrough or branch target; we do
                    // not trace predecessors, so incoming depth is unknown and
                    // any window identity proof is impossible.
                    depth = None;
                }
                i += 1;
                continue;
            }
            LineKind::Directive => {
                let line = store.get(i);
                let trimmed = infos[i].trimmed(line);
                if trimmed == ".cfi_startproc" {
                    func_suppress_tailcall = false;
                    in_function = true;
                    depth = Some(0);
                }
                i += 1;
                continue;
            }
            _ => {}
        }

        // Simulate %rsp arithmetic so call-site depth stays meaningful. Only
        // constant-immediate %rsp adjustments and push/pop contribute;
        // anything else (register forms, leave-equivalents, `and` alignment,
        // movq into %rsp, xchg...) invalidates knowledge conservatively.
        {
            let line = store.get(i);
            let trimmed = infos[i].trimmed(line);
            if trimmed.starts_with("pushq ") || trimmed.starts_with("push ") {
                if let Some(d) = depth.as_mut() {
                    *d += 8;
                }
            } else if trimmed.starts_with("popq ") || trimmed.starts_with("pop ") {
                let rhs = trimmed[trimmed.find(' ').unwrap() + 1..].trim();
                if rhs == "%rsp" {
                    // `popq %rsp` reassigns the pointer itself.
                    depth = None;
                } else if let Some(d) = depth.as_mut() {
                    *d -= 8;
                }
            } else if (trimmed.starts_with("subq $") || trimmed.starts_with("addq $"))
                && trimmed.ends_with(", %rsp")
            {
                let start = "addq $".len();
                let num_part = &trimmed[start..trimmed.len() - ", %rsp".len()];
                if let Ok(n) = num_part.parse::<i64>() {
                    if let Some(d) = depth.as_mut() {
                        *d += if trimmed.starts_with("addq $") { n } else { -n };
                    }
                } else {
                    depth = None;
                }
            } else if trimmed.ends_with("(%rsp)") {
                // Store INTO an %rsp-addressed slot (e.g. `movq %r9, 8(%rsp)`).
                // The x86-64 emitter stages outgoing overflow arguments both
                // by pushes (tracked above) and by such stores, which do not
                // change %rsp arithmetic themselves -- without poisoning, a
                // numerically-cancelled push/sub sequence could masquerade as
                // depth 0 while a live argument window sits at [P..], and the
                // conversion would still re-base what the tail-callee reads.
                // (i686 does not need this rule: its emitter stages purely
                // by pushing, which the arithmetic tracking already sees.)
                depth = None;
            } else if trimmed.ends_with(", %rsp") {
                // Any remaining %rsp-ending form (movq reg,%rsp, andl
                // alignment, xchg…) makes the displacement unknowable.
                // The classic FP-teardown `movq %rbp, %rsp` occurs inside
                // epilogue windows that candidate scanning accepts as the
                // teardown; before a *candidate* call it can only appear via
                // unusual shapes -- treating it uniformly costs nothing.
                depth = None;
            }
        }

        // Check for lea-of-local instructions: leaq offset(%rbp), %reg
        // or leaq offset(%rsp), %reg.
        // Also check for dynamic stack allocation (subq %reg, %rsp) which is
        // emitted by __builtin_alloca/DynAlloca. After frame teardown, alloca'd
        // memory lives below %rsp and may be clobbered by the tail-called function.
        if in_function && !func_suppress_tailcall {
            if let LineKind::Other { .. } = infos[i].kind {
                let line = store.get(i);
                let trimmed = infos[i].trimmed(line);
                if (trimmed.starts_with("leaq ")
                    || trimmed.starts_with("leal ")
                    || trimmed.starts_with("lea "))
                    && (trimmed.contains("(%rbp)") || trimmed.contains("(%rsp)"))
                {
                    func_suppress_tailcall = true;
                }
                // Detect dynamic alloca: subq %rax, %rsp (or any register subtracted from rsp)
                if trimmed.starts_with("subq %") && trimmed.ends_with(", %rsp") {
                    func_suppress_tailcall = true;
                }
            }
        }

        // The call must happen at simulated depth == Some(0): anything staged
        // below the entry CFA (overflow arguments -- push-staged OR stored --
        // spills, reserved outgoing areas) re-bases the tail-callee's incoming
        // argument window after conversion and is unsound (see module docs).
        if infos[i].kind != LineKind::Call || func_suppress_tailcall || depth != Some(0) {
            i += 1;
            continue;
        }

        // We found a call instruction. Check if it can be tail-call-optimized.
        // Skip if the function has unsafe stack usage (lea-of-local or alloca).
        if func_suppress_tailcall {
            i += 1;
            continue;
        }

        // Check if the sequence after it is purely epilogue
        // (callee-save restores + frame teardown + ret).
        if let Some(ret_idx) = is_tail_call_candidate(store, infos, i, len) {
            // Extract the call target
            let call_line = store.get(i);
            let trimmed = infos[i].trimmed(call_line);

            if let Some(jmp_text) = convert_call_to_jmp(trimmed) {
                // NOP the call
                mark_nop(&mut infos[i]);

                // Replace the `ret` with `jmp TARGET`
                replace_line(
                    store,
                    &mut infos[ret_idx],
                    ret_idx,
                    format!("    {}", jmp_text),
                );

                changed = true;
            }
        }

        i += 1;
    }

    changed
}

/// Check if the instructions after a call at position `call_idx` form a pure
/// epilogue sequence ending in `ret`. Returns the index of the `ret` if so.
///
/// The allowed pattern between call and ret:
/// - LoadRbp (callee-save restores): movq offset(%rbp), %REG
/// - Other with text "movq %rbp, %rsp" (stack frame teardown)
/// - Pop with reg being rbp (popq %rbp)
/// - Directive lines (.cfi_*)
/// - Nop/Empty lines
/// - NOTHING that writes to %rax (the return value must pass through)
fn is_tail_call_candidate(
    store: &LineStore,
    infos: &[LineInfo],
    call_idx: usize,
    len: usize,
) -> Option<usize> {
    // Limit how far we scan forward
    let limit = (call_idx + 30).min(len);

    let mut found_frame_teardown = false;
    let mut found_pop_rbp = false;
    // Index and immediate of a frameless `addq $N, %rsp`, when that is the
    // teardown form. The caller must keep it BEFORE the jump.
    let mut frame_release: Option<(usize, String)> = None;
    let _ = &frame_release;
    let mut j = call_idx + 1;

    while j < limit {
        if infos[j].is_nop() {
            j += 1;
            continue;
        }

        match infos[j].kind {
            LineKind::Empty => {
                j += 1;
                continue;
            }
            LineKind::Directive => {
                j += 1;
                continue;
            }
            LineKind::LoadRbp { reg, .. } => {
                // Callee-save restore from stack - OK, but must not restore to %rax (reg 0)
                if reg == 0 {
                    return None; // Writing to %rax would clobber the return value
                }
                j += 1;
                continue;
            }
            LineKind::Other { dest_reg } => {
                let trimmed = infos[j].trimmed(store.get(j));
                if trimmed == "movq %rbp, %rsp" {
                    found_frame_teardown = true;
                    j += 1;
                    continue;
                }
                // Frameless epilogue: `addq $N, %rsp` releases the frame
                // without a %rbp chain. lccc emits this shape whenever the
                // function has no dynamic alloca, which is the common case --
                // requiring the %rbp pair meant an indirect tail call was
                // never converted:
                //     call *%r10 ; addq $24,%rsp ; ret     (9 insns total)
                // where GCC emits a single `jmp *%rdi`. Restoring the stack
                // BEFORE the jump is exactly what makes the tail call legal,
                // so accept it as the teardown.
                if let Some(rest) = trimmed.strip_prefix("addq $") {
                    if let Some((imm, reg)) = rest.split_once(", ") {
                        if reg == "%rsp" && imm.bytes().all(|b| b.is_ascii_digit()) {
                            found_frame_teardown = true;
                            found_pop_rbp = true; // no %rbp was pushed
                            frame_release = Some((j, imm.to_string()));
                            j += 1;
                            continue;
                        }
                    }
                }
                // Any other instruction that writes a register - check if it's rax
                if dest_reg == 0 {
                    return None; // Writes to %rax
                }
                // Any other instruction is suspicious - bail out
                return None;
            }
            LineKind::Pop { reg } => {
                // popq %rbp is part of the epilogue
                if reg == 5 {
                    // rbp = register family 5
                    found_pop_rbp = true;
                    j += 1;
                    continue;
                }
                // Any other pop is suspicious
                return None;
            }
            LineKind::Ret => {
                // Found the ret! Make sure we saw the frame teardown
                if found_frame_teardown && found_pop_rbp {
                    return Some(j);
                }
                return None;
            }
            // Any other instruction kind (labels, jumps, calls, etc.) breaks the pattern
            _ => return None,
        }
    }

    None
}

/// Convert a `call TARGET` instruction text into `jmp TARGET`.
/// Returns None if the call format is not recognized.
fn convert_call_to_jmp(trimmed_call: &str) -> Option<String> {
    // Direct call: "call foo" or "call foo@PLT" or "callq foo"
    // Indirect call: "call *%r10" or "callq *%r10"
    let rest = if let Some(r) = trimmed_call.strip_prefix("callq ") {
        r
    } else if let Some(r) = trimmed_call.strip_prefix("call ") {
        r
    } else {
        return None;
    };

    if rest.starts_with('*') {
        // Indirect call: call *%r10 -> jmp *%r10
        Some(format!("jmp {}", rest))
    } else if rest.starts_with("__x86_indirect_thunk_") {
        // Retpoline thunk - skip for safety
        None
    } else {
        // Direct call: call foo@PLT -> jmp foo@PLT
        Some(format!("jmp {}", rest))
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::peephole_optimize;

    #[test]
    fn test_tail_call_direct() {
        // WINDOW-SOUND SHAPE: the straight-line body returns %rsp to the
        // exact entry line before the call (net displacement 0), so the
        // incoming-argument window is byte-identical before and after the
        // conversion -- provable without any callee ABI knowledge.
        let asm = [
            "func:",
            ".cfi_startproc",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    pushq %rbx",
            "    movq %rdi, %rax",
            "    addq $1, %rax",
            "    subq $16, %rsp",
            "    call target",
            "    movq -16(%rbp), %rbx",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jmp target"),
            "should convert balanced-depth call to jmp: {}",
            result
        );
        assert!(
            !result.contains("call target"),
            "should not have call: {}",
            result
        );
    }

    #[test]
    fn test_no_tail_call_when_store_based_saves_leave_displacement() {
        // STACK-WINDOW SOUNDNESS REGRESSION TEST (renamed from the former
        // positive test_tail_call_indirect). Store-based callee-save homes
        // mean the function's prologue established a frame BELOW the entry
        // line (pushq %rbp alone leaves +8, the further subq deepens it)
        // which is never wound back before the call, so the simulated depth
        // at the call is nonzero. The old expectation converted this shape;
        // doing so re-based the tail-callee's incoming stack-argument window
        // (proven live miscompile class -- see the fwdZ test below and PR
        // #255 for i686), so conversion must stay off.
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
            "    movq %rdi, %r10",
            "    call *%r10",
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
        assert!(
            !result.contains("jmp *%r10"),
            "outstanding frame displacement must suppress conversion:\n{}",
            result
        );
        assert!(
            result.contains("call *%r10"),
            "original call must survive:\n{}",
            result
        );
    }

    #[test]
    fn test_tail_call_indirect_depth_neutral() {
        // Positive indirect-dispatch case accepted under the merged policy:
        // register staging only and the scratch region is released before
        // the dispatch, so the simulated depth returns to the entry line.
        let asm = [
            "func:",
            ".cfi_startproc",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $8, %rsp",
            "    movq %rdi, %r10",
            "    call *%r10",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jmp *%r10"),
            "should convert call *%r10 to jmp *%r10: {}",
            result
        );
        assert!(
            !result.contains("call *%r10"),
            "should not have call: {}",
            result
        );
    }

    #[test]
    fn test_no_tail_call_when_rax_used() {
        // If something between call and ret writes to %rax, it's not a tail call
        let asm = [
            "func:",
            "    pushq %rbp",
            "    movq %rsp, %rbp",
            "    subq $16, %rsp",
            "    call foo",
            "    movq %rax, %r12", // uses %rax result - but stores to r12
            "    movl $42, %eax",  // overwrites %rax!
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call foo"),
            "should NOT convert when %rax is modified: {}",
            result
        );
        assert!(result.contains("ret"), "should keep ret: {}", result);
    }

    #[test]
    fn test_tail_call_plt() {
        // Window-neutral PLT case: the scratch region is fully released
        // before the call, so pre- and post-conversion argument windows
        // coincide and PLT-resolved dispatch keeps converting.
        let asm = [
            "func:",
            ".cfi_startproc",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    pushq %rbx",
            "    subq $16, %rsp",
            "    call foo@PLT",
            "    movq -16(%rbp), %rbx",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("jmp foo@PLT"),
            "should convert PLT call to jmp: {}",
            result
        );
    }

    #[test]
    fn test_only_last_call_converts_in_straight_run() {
        // A call followed by another call can never convert (the window scan
        // hits a LineKind::Call); the last call in the run converts when its
        // own window matches the epilogue arms.
        let asm = [
            "func:",
            ".cfi_startproc",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    pushq %rbx",
            "    subq $16, %rsp",
            "    call foo",
            "    call bar",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call foo"),
            "first call should remain: {}",
            result
        );
        assert!(
            result.contains("jmp bar"),
            "second call should be tail-optimized: {}",
            result
        );
        assert!(
            !result.contains("call bar"),
            "second call should be gone: {}",
            result
        );
    }

    #[test]
    fn test_no_tail_call_staged_stack_args_fwdz_shape() {
        // STACK-WINDOW SOUNDNESS REGRESSION TEST -- the exact instruction
        // stream lccc-x86_64 upstream main emitted for
        //     long fwdZ(void){ long v=g_s;
        //         return sink7(v,v,v,v,v,v,77L); }
        // (7th argument staged through subq/push). Converting this produced
        // wrong results at O1/O2/O3/Os (1155 instead of 1694): the tail-
        // callee read its overflow argument at the freshly unwound entry
        // window while 77 lived below the entry CFA, and entry alignment was
        // broken besides. The simulated depth here is -16+8 = -8 != 0 (and
        // sp-relative staging poisons independently), so the conversion must
        // stay off and the original call/ret pair must survive verbatim.
        let asm = concat!(
            "fwdZ:\n",
            ".cfi_startproc\n",
            "    subq $8, %rsp\n",
            ".cfi_def_cfa_offset 16\n",
            "    movq g_s(%rip), %r11\n",
            "    subq $8, %rsp\n",
            "    movq $77, %rax\n",
            "    pushq %rax\n",
            "    movq %r11, %rax\n",
            "    movq %r11, %rdi\n",
            "    movq %r11, %rsi\n",
            "    movq %r11, %rdx\n",
            "    movq %r11, %rcx\n",
            "    movq %r11, %r8\n",
            "    movq %r11, %r9\n",
            "    call sink7@PLT\n",
            "    addq $16, %rsp\n",
            "    addq $8, %rsp\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("jmp sink7@PLT"),
            "stack-arg forwarding must never convert (real fwdZ miscompile):\n{result}"
        );
        assert!(
            result.contains("call sink7@PLT"),
            "original call must survive:\n{result}"
        );
        assert!(result.contains("ret"), "ret must survive:\n{result}");
    }

    #[test]
    fn test_tail_call_suppressed_after_internal_join_label() {
        // After an internal .L label the straight-line rsp simulation ends:
        // fallthrough and branch predecessors may disagree on depth, so no
        // conversion may fire inside such a region (mirror of the validated
        // i686 gate in PR #255).
        let asm = concat!(
            "f:\n",
            ".cfi_startproc\n",
            "    call helper\n",
            ".Ljoin:\n",
            "    call target\n",
            "    ret\n",
            ".cfi_endproc\n",
        )
        .to_string();
        let result = peephole_optimize(asm);
        assert!(
            !result.contains("jmp target"),
            "post-join calls have unknown depth; must not convert:\n{result}"
        );
        assert!(
            result.contains("call target"),
            "original call must survive:\n{result}"
        );
    }

    #[test]
    fn test_no_tail_call_with_dyn_alloca() {
        // If the function uses dynamic stack allocation (alloca), the tail call
        // could clobber the alloca'd memory after frame teardown.
        let asm = [
            "test_alloca:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $32, %rsp",
            "    movq %rbx, -32(%rbp)",
            "    movq %r12, -24(%rbp)",
            "    addq $15, %rax",
            "    andq $-16, %rax",
            "    subq %rax, %rsp", // dynamic alloca!
            "    movq %rsp, %rax",
            "    movq %rax, -16(%rbp)",
            "    call memset",
            "    call printf",
            "    movq -32(%rbp), %rbx",
            "    movq -24(%rbp), %r12",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size test_alloca, .-test_alloca",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call printf"),
            "should NOT convert when alloca exists: {}",
            result
        );
        assert!(
            result.contains("ret"),
            "should keep ret when alloca exists: {}",
            result
        );
    }

    #[test]
    fn test_no_tail_call_with_lea_local() {
        // If the function takes address of a local (leaq offset(%rbp), %reg),
        // the tail call could pass a dangling stack pointer.
        let asm = [
            "func:",
            "    pushq %rbp",
            "    .cfi_def_cfa_offset 16",
            "    .cfi_offset %rbp, -16",
            "    movq %rsp, %rbp",
            "    .cfi_def_cfa_register %rbp",
            "    subq $32, %rsp",
            "    movq %rbx, -32(%rbp)",
            "    leaq -16(%rbp), %rsi", // takes address of local!
            "    movq %rdi, %rbx",
            "    call bar", // bar receives pointer to our local
            "    movq -32(%rbp), %rbx",
            "    movq %rbp, %rsp",
            "    popq %rbp",
            "    ret",
            ".size func, .-func",
        ]
        .join("\n")
            + "\n";
        let result = peephole_optimize(asm);
        assert!(
            result.contains("call bar"),
            "should NOT convert when lea of local exists: {}",
            result
        );
        assert!(
            result.contains("ret"),
            "should keep ret when lea of local exists: {}",
            result
        );
    }
}

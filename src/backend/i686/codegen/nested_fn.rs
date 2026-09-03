//! i686 lowering of the GNU C nested-function support instructions:
//! static chain, trampolines, and non-local goto.
//!
//! ## Static chain
//!
//! The i686 convention mirrors GCC's historical i386 choice: the static
//! chain travels in `%ecx` (GCC: STATIC_CHAIN_REGNUM == ECX). The callee
//! reads it with `GetStaticChain` (placed at function entry by the lowerer,
//! before anything else can clobber %ecx); direct callers load it with
//! `SetStaticChain` immediately before the call.
//!
//! Safety of `%ecx` at call sites relies on two properties of the shared
//! framework:
//!   * Call emission never stages through %ecx in the default (-mregparm=0)
//!     convention: direct calls are a bare `call rel32`, indirect calls stage
//!     through %eax (`call *%eax`). See `calls.rs::emit_call_instruction_impl`.
//!   * The i686 scratch-hazard classifier treats every one of these nested-
//!     function instructions as dirty for BOTH caller-saved registers
//!     (regalloc catch-all arm), so no allocator-owned value can sit in
//!     %ecx across them.
//! A nested call from a `-mregparm` context shares %ecx with argument 3;
//! that combination behaves like upstream GCC's own static-chain/regparm
//! conflict and is intentionally out of scope here (documented ABI limit).
//!
//! ## Trampoline (address-taken nested function)
//!
//! Older GCC release notes and GCC's own ix86 lowering both key off a run-
//! time-initialized stack template; modern reality adds ASLR/PIE, which makes
//! baked absolute addresses wrong. LCCC therefore uses a **statically
//! relocated template** rather than computing any address at emission time:
//!
//! ```text
//!     .section .rodata
//! .LCT<N>:                          ;; 11-byte code template (RO, shared per site)
//!     .byte 0xB9                    ;; opcode: movl $imm32, %ecx
//!     .long 0                       ;; chain slot — patched AFTER copy (offset 1..5)
//!     .byte 0xFF, 0x25              ;; opcode: jmp *disp32
//!     .long .LTS<N>                 ;; disp32 -> slot address (R_386_32, load-resolved)
//!     .section .data.rel.ro,"aw"    ;; relocation-writable, RELRO-protected afterwards
//! .LTS<N>:
//!     .long <func>                  ;; absolute target (R_386_32, load-resolved)
//! ```
//!
//! Both disp32 fields carry ordinary `R_386_32` symbol relocations, so under
//! `-fPIE`/`-fPIC` the program loader resolves them together with the rest of
//! the image — no GOT dance, no %-ebx dependency, and identical machinery for
//! the non-PIC case. Execution model (15 -> 11-byte form):
//!
//!   1. `InitTrampoline` copies the 11 template bytes to the caller-provided
//!      stack buffer (the lowerer allocates 32 bytes @ align 16);
//!   2. patches the captured current-frame chain value over the placeholder
//!      (`movl %edx, -10(%edi)` after an 11-byte rep movsb);
//!   3. marks the module's stack executable (`.note.GNU-stack,"x"`), exactly
//!      like x86-64.
//!
//! When the trampoline later runs, `%ecx` already holds the initializing
//! frame's chain when control enters the callee prologue, matching every
//! `GetStaticChain` reader.
//!
//! Notes:
//!   * Only ebx/esi/edi-width register discipline matters for correctness:
//!     inputs are staged into %edx/%eax/%esi BEFORE any callee-saved register
//!     is pushed, and %esi/%edi are restored bit-exactly afterwards.
//!   * Deep-consistency note: upstream keeps save areas with 8-byte words;
//!     i686 frames consume 4-byte pointers but stay byte-compatible because
//!     save/restore read the very offsets the lowerer handed down.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::emit;
use crate::ir::reexports::{Operand, Value};

use crate::backend::i686::codegen::emit::{phys_reg_name, I686Codegen};

/// Site-unique suffix generator for the static template/slot label pairs.
static TRAMPOLINE_SITES: AtomicU32 = AtomicU32::new(0);

impl I686Codegen {
    /// `GetStaticChain`: move the incoming %ecx into the dest's home.
    pub(super) fn emit_get_static_chain_impl(&mut self, dest: &Value) {
        if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
            let d_name = phys_reg_name(d_reg);
            if d_name != "ecx" {
                emit!(self.state, "    movl %ecx, %{}", d_name);
                self.state.reg_cache.invalidate_all();
            }
            // Already homed in %ecx: nothing to do (and the cached mapping
            // still holds, so no invalidation either).
            return;
        }
        // Dead chain value (no register home, no slot): the function never
        // reads its static chain. Emitting the read would be wasted code.
        if let Some(slot) = self.state.get_slot(dest.0) {
            // Direct reg->memory store; no accumulator staging needed.
            let sr = self.slot_ref(slot);
            emit!(self.state, "    movl %ecx, {}", sr);
            self.state.reg_cache.invalidate_all();
        }
    }

    /// `SetStaticChain`: load `src` into %ecx right before the call.
    pub(super) fn emit_set_static_chain_impl(&mut self, src: &Operand) {
        self.operand_to_ecx(src);
        // %ecx now holds the chain; the immediately following Call must not
        // reload it. invalidate_all() is deliberately NOT called: the chain
        // register is outside the accumulator-cache contract and the very
        // next instruction is the call.
    }

    /// `InitTrampoline`: materialise the per-site template into `buffer`.
    ///
    /// Copies the 11-byte `jmp`-ready template from `.rodata` onto the
    /// caller's stack buffer and patches in the runtime chain value. Both
    /// address words inside the template are link-time relocated (see the
    /// module docs), which keeps non-PIC and PIC/PIE builds equally correct.
    pub(super) fn emit_init_trampoline_impl(
        &mut self,
        buffer: &Value,
        chain: &Operand,
        func: &str,
    ) {
        let site = TRAMPOLINE_SITES.fetch_add(1, Ordering::Relaxed);
        let tpl = format!(".LCT{}", site);
        let slot = format!(".LTS{}", site);

        // ── Static template (COLLECTED, flushed once at module end).
        // Both address words need load-time relocation, and a relocation
        // INTO read-only storage would force DT_TEXTREL under PIE — so the
        // WHOLE template lives in .data.rel.ro like every other relocated
        // constant (writable before relocation, RELRO afterwards). The
        // template itself is never executed from here; only its copied
        // image on the stack runs. ──────────────────────────────────────
        self.state.trampoline_data_blocks.push(format!(
            "\n    .section .data.rel.ro,\"aw\"\n{}:\n    .byte 0xB9\n    .long 0\n\
                 \x20    .byte 0xFF, 0x25\n    .long {}\n{}:\n    .long {}\n    .text",
            tpl, slot, slot, func,
        ));

        // ── Stage inputs with STRICT caller-saved-only discipline ────────
        // The i686 register allocator owns %ebx/%esi/%edi homes across this
        // instruction (they are callee-saved by ABI, and only eax/ecx/edx
        // have dirtiness barriers in regalloc.rs). Any use of %esi/%edi as
        // internal staging therefore corrupts allocator-held values whose
        // users read them straight out of the register after this point
        // (-O1 proved it live: 20000822-1 read the destroyed home and
        // returned template bytes). Consequently THIS emitter never
        // touches %esi/%edi at all:
        //   1) chain    -> %ecx   (dirty-classified home: safe to burn)
        //   2) pushl %ecx        (stash on the stack; balanced below)
        //   3) buffer   -> %eax
        //   4) template-> %edx   (GOT-off idiom under PIC; reads ebx)
        //   5) field-wise copy edx->eax with %ecx as the shuttle
        //   6) popl  %edx        (recover chain; eax kept intact)
        //   7) movl %edx,1(%eax) (patch chain immediate)
        self.operand_to_ecx(chain);
        self.state.emit("    pushl %ecx");
        // The stash lives exactly for the duration of the staging window;
        // keep the esp-relative slot arithmetic correct even under
        // -fomit-frame-pointer (with a frame pointer slot_ref is ebp-based
        // and ignores this counter).
        let saved_esp_adjust = self.esp_adjust;
        self.esp_adjust += 4;

        // Destination = the lowerer's 32-byte stack buffer.
        self.operand_to_eax(&Operand::Value(*buffer));

        // Source = per-site template (address loaded from a register).
        if self.state.pic_mode {
            emit!(self.state, "    leal {}@GOTOFF(%ebx), %edx", tpl);
        } else {
            emit!(self.state, "    movl ${}, %edx", tpl);
        }

        // Field-wise copy of the 11 template bytes (B9 | chain4 | FF 25 |
        // disp32). Unaligned dword loads/stores keep this compact; the
        // chain placeholder copied here is irrelevant — overwritten next.
        self.state.emit("    movl 0(%edx), %ecx");
        self.state.emit("    movl %ecx, 0(%eax)");
        self.state.emit("    movl 4(%edx), %ecx");
        self.state.emit("    movl %ecx, 4(%eax)");
        self.state.emit("    movw 8(%edx), %cx");
        self.state.emit("    movw %cx, 8(%eax)");
        self.state.emit("    movb 10(%edx), %cl");
        self.state.emit("    movb %cl, 10(%eax)");

        // Recover the stashed chain value and write it over the copied
        // placeholder (bytes 1..5 of the trampoline image).
        self.esp_adjust = saved_esp_adjust;
        self.state.emit("    popl %edx");
        self.state.emit("    movl %edx, 1(%eax)");

        // Both staging registers were rewritten inside the window. This is
        // also the point that makes %ecx executable-stack-worthy.
        self.state.reg_cache.invalidate_all();
        // The trampoline executes from the stack -> executable stack needed.
        self.state.requires_executable_stack = true;
    }

    /// `NonlocalGotoSave`: store the restorable core state of this frame
    /// into the save area: %ebp and %esp (4 bytes each), matching the
    /// offsets handed down by the lowerer.
    pub(super) fn emit_nonlocal_goto_save_impl(
        &mut self,
        frame: &Value,
        rbp_off: i64,
        rsp_off: i64,
    ) {
        // Frame-pointer value staging through the accumulator.
        self.operand_to_eax(&Operand::Value(*frame));
        emit!(self.state, "    movl %ebp, {}(%eax)", rbp_off);
        emit!(self.state, "    movl %esp, {}(%eax)", rsp_off);
        // No callee-saved snapshot: ordinary calls preserve ebx/esi/edi by
        // ABI, and a global register variable modified right before the
        // non-local goto must carry its source-visible value (mirrors the
        // x86-64 implementation and GCC itself).
        self.state.reg_cache.invalidate_all();
    }

    /// `NonlocalGoto`: walk `up` frame links from `chain`, restore the
    /// saved %ebp/%esp, and jump to the cross-function label.
    pub(super) fn emit_nonlocal_goto_impl(
        &mut self,
        chain: &Operand,
        up: usize,
        rbp_off: i64,
        rsp_off: i64,
        label: &str,
    ) {
        // Chain base into the accumulator.
        self.operand_to_eax(chain);
        // Walk `up` links: frame[0] of each level is the parent frame ptr.
        for _ in 0..up {
            self.state.emit("    movl (%eax), %eax");
        }
        // Restore the target frame and stack. Callee-saved GPRs are already
        // preserved by the ABI and may carry global-register assignments
        // that must survive this transfer.
        emit!(self.state, "    movl {}(%eax), %ebp", rbp_off);
        emit!(self.state, "    movl {}(%eax), %esp", rsp_off);
        // Jump to the target function's named alias label; block IDs are
        // file-unique so the cross-function `jmp` assembles fine.
        self.state.emit_fmt(format_args!("    jmp {}", label));
        self.state.reg_cache.invalidate_all();
    }
}

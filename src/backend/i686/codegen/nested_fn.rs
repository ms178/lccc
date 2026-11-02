//! i686 GNU nested-function static chains, trampolines and non-local goto.
//!
//! GCC 16.2 ix86_static_chain uses ECX for cdecl and EAX for fastcall,
//! leaving ECX/EDX available for fastcall user arguments. Capture incoming
//! chains in a dedicated frame home before parameter materialization; stage
//! outgoing chains in another home and reload AFTER outgoing arguments.
//!
//! Trampolines copy an 11-byte relocated template to the stack: MOV imm32
//! to the convention's chain register, followed by JMP through a relocated
//! target slot. Both the template and target slot live in .data.rel.ro until
//! copied, avoiding a relocation into read-only text under PIC/PIE. Taking a
//! capturing nested function's address requires an executable stack, as in
//! GNU C; the module explicitly marks .note.GNU-stack executable.
//!
//! Regparm-3 nested functions require GCC's separate ESI/stack-chain entry
//! convention and remain a separate, unsupported ABI combination here.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::emit;
use crate::ir::reexports::{Operand, Value};

use crate::backend::i686::codegen::emit::I686Codegen;

/// Site-unique suffix generator for the static template/slot label pairs.
static TRAMPOLINE_SITES: AtomicU32 = AtomicU32::new(0);

impl I686Codegen {
    /// Read the chain captured before incoming parameter materialization.
    pub(super) fn emit_get_static_chain_impl(&mut self, dest: &Value) {
        let slot = self
            .incoming_chain_slot
            .expect("GetStaticChain needs a home");
        let home = self.slot_ref(slot);
        emit!(self.state, "    movl {}, %eax", home);
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    /// Preserve the chain until AFTER argument materialization. In particular,
    /// fastcall ECX/EDX parameters must never overwrite it.
    pub(super) fn emit_set_static_chain_impl(&mut self, src: &Operand) {
        let slot = self
            .outgoing_chain_slot
            .expect("SetStaticChain needs a home");
        self.operand_to_eax(src);
        let home = self.slot_ref(slot);
        emit!(self.state, "    movl %eax, {}", home);
        self.pending_static_chain = true;
    }

    pub(super) fn emit_pending_static_chain(&mut self, register: &str) {
        if self.pending_static_chain {
            let slot = self
                .outgoing_chain_slot
                .expect("pending static chain needs a home");
            let home = self.slot_ref(slot);
            emit!(self.state, "    movl {}, %{}", home, register);
            self.state.reg_cache.invalidate_all();
            self.pending_static_chain = false;
        }
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
        let chain_opcode = if self.state.i686_fastcall_symbols.contains(func) {
            "0xB8" // mov imm32, EAX (GNU fastcall static chain)
        } else {
            "0xB9" // mov imm32, ECX (cdecl static chain)
        };
        self.state.trampoline_data_blocks.push(format!(
            "\n    .section .data.rel.ro,\"aw\"\n{}:\n    .byte {}\n    .long 0\n\
                 \x20    .byte 0xFF, 0x25\n    .long {}\n{}:\n    .long {}\n    .text",
            tpl, chain_opcode, slot, slot, func,
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

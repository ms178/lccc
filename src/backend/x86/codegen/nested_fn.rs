//! x86-64 lowering of the GNU C nested-function support instructions:
//! static chain, trampolines, and non-local goto.
//!
//! ## Static chain
//!
//! GCC's x86-64 convention passes the static chain in `%r10`. The callee
//! reads it with `GetStaticChain` (placed at function entry by the lowerer,
//! before anything else can clobber %r10); direct callers load it with
//! `SetStaticChain` immediately before the call.
//!
//! ## Trampoline (address-taken nested function)
//!
//! Taking the address of a nested function produces a 15-byte trampoline on
//! the parent's stack (needs an executable stack, marked via
//! `.note.GNU-stack,"x"`):
//!
//! ```text
//! 49 BA <chain imm64>      ; movq $chain, %r10      (10 bytes)
//! E9  <rel32>              ; jmp  func              (5 bytes)
//! 90                       ; nop padding            (1 byte, 16 total)
//! ```
//!
//! The `rel32` is computed at runtime: `func - (buf + 15)` — within one
//! object file every function is within ±2 GB, so a rel32 jump always
//! reaches.
//!
//! ## Non-local goto
//!
//! `goto label;` inside a nested function targeting an enclosing function's
//! label restores the enclosing frame's %rbp/%rsp from the save area in its
//! frame struct (saved once at its entry by `NonlocalGotoSave`) and jumps to
//! the label. The label is a plain block label of the ENCLOSING function;
//! block IDs are file-unique so the cross-function `jmp` assembles fine.

use crate::backend::x86::codegen::emit::X86Codegen;
use crate::ir::reexports::{Operand, Value};

impl X86Codegen {
    /// `GetStaticChain`: move the incoming %r10 into the dest's home.
    pub(super) fn emit_get_static_chain_impl(&mut self, dest: &Value) {
        // Like ParamRef: materialize the incoming register value into the
        // dest's home (register or slot).
        if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
            let d_name = crate::backend::x86::codegen::emit::phys_reg_name(d_reg);
            if d_name != "r10" {
                self.state
                    .emit_fmt(format_args!("    movq %r10, %{}", d_name));
                self.state.reg_cache.invalidate_all();
                return;
            }
            return; // already in r10
        }
        // Dead chain value (no register home, no slot): the function never
        // reads its static chain. Emitting the read would be wasted code.
        if self.state.get_slot(dest.0).is_none() {
            return;
        }
        // Slot home: store through a scratch register.
        self.state.emit("    movq %r10, %rax");
        self.store_rax_to(dest);
        self.state.reg_cache.invalidate_all();
    }

    /// `SetStaticChain`: load `src` into %r10.
    pub(super) fn emit_set_static_chain_impl(&mut self, src: &Operand) {
        self.operand_to_reg(src, "r10");
        // %r10 now holds the chain; the following Call must not reload it.
        // reg_cache.invalidate_all() is deliberately NOT called: the chain
        // register is not part of the accumulator cache contract, and the
        // immediate next instruction is the call.
    }

    /// `InitTrampoline`: write the 24-byte trampoline into `buffer`:
    ///
    /// ```text
    /// 49 BA <chain imm64>      ; movq $chain, %r10     (10 bytes, 0..9)
    /// FF 25 00 00 00 00        ; jmp *[rip+0]          ( 6 bytes, 10..15)
    /// <func addr imm64>        ; absolute jump target  ( 8 bytes, 16..23)
    /// ```
    ///
    /// An ABSOLUTE indirect jump is required: with ASLR the stack
    /// (0x7f...) and text (0x40.../0x55...) segments are far more than
    /// ±2 GiB apart, so a rel32 `jmp` from the stack cannot reach the
    /// function (GCC's trampolines use the same rip-indirect form).
    pub(super) fn emit_init_trampoline_impl(
        &mut self,
        buffer: &Value,
        chain: &Operand,
        func: &str,
    ) {
        // Materialize the buffer address into %rdi.
        self.operand_to_reg(&Operand::Value(*buffer), "rdi");
        // func address into %rcx.
        self.state
            .out
            .emit_instr_sym_base_reg("    leaq", func, "rip", "rcx");
        // chain value into %rsi.
        self.operand_to_reg(chain, "rsi");

        // 49 BA <chain imm64>: opcode word + qword immediate.
        self.state.emit("    movw $0xBA49, (%rdi)"); // bytes 49 BA (LE)
        self.state.emit("    movq %rsi, 2(%rdi)"); // chain at offset 2..9
                                                   // FF 25 00 00 00 00: jmp qword ptr [rip+0]; rip after = buf+16.
        self.state.emit("    movl $0x0025FF, 10(%rdi)"); // FF 25 00 00
        self.state.emit("    movw $0x0000, 14(%rdi)"); // 00 00
                                                       // Absolute function address at offset 16..23.
        self.state.emit("    movq %rcx, 16(%rdi)");

        // The trampoline address (= buffer address) is the function-pointer
        // value; the lowerer uses the buffer alloca value directly.
        self.state.reg_cache.invalidate_all();
        // Executable stack required: record on the module output.
        self.state.requires_executable_stack = true;
    }

    /// `NonlocalGotoSave`: store the full restorable state of this frame
    /// into the save area: %rbp, %rsp, and the SysV callee-saved GPRs
    /// (rbx, r12..r15). Layout: [rbp][rsp][rbx][r12][r13][r14][r15],
    /// 8 bytes each starting at rbp_off.
    pub(super) fn emit_nonlocal_goto_save_impl(
        &mut self,
        frame: &Value,
        rbp_off: i64,
        rsp_off: i64,
    ) {
        // Frame pointer address into %rax (may be a slot or register).
        self.operand_to_reg(&Operand::Value(*frame), "rax");
        self.state
            .emit_fmt(format_args!("    movq %rbp, {}(%rax)", rbp_off));
        self.state
            .emit_fmt(format_args!("    movq %rsp, {}(%rax)", rsp_off));
        // Callee-saved GPRs need no explicit snapshot: every intervening
        // ordinary call preserves them by ABI. More importantly, a global
        // register variable may be intentionally modified immediately before
        // the non-local goto; restoring an entry snapshot would erase that
        // source-visible assignment. GCC saves only rbp/rsp here.
        self.state.reg_cache.invalidate_all();
    }

    /// `NonlocalGoto`: walk `up` frame links from `chain`, restore the
    /// saved %rbp/%rsp, and jump to the (cross-function) label.
    pub(super) fn emit_nonlocal_goto_impl(
        &mut self,
        chain: &Operand,
        up: usize,
        rbp_off: i64,
        rsp_off: i64,
        label: &str,
    ) {
        // Chain base into %rax.
        self.operand_to_reg(chain, "rax");
        // Walk `up` links: frame[0] of each level is the parent frame ptr.
        for _ in 0..up {
            self.state.emit("    movq (%rax), %rax");
        }
        // Restore the target frame and stack. Callee-saved GPRs are already
        // preserved by the ABI and may carry global-register assignments that
        // must survive this transfer (GCC's implementation likewise restores
        // only rbp/rsp).
        self.state
            .emit_fmt(format_args!("    movq {}(%rax), %rbp", rbp_off));
        self.state
            .emit_fmt(format_args!("    movq {}(%rax), %rsp", rsp_off));
        // Jump to the target function's named alias label.
        self.state.emit_fmt(format_args!("    jmp {}", label));
        self.state.reg_cache.invalidate_all();
    }
}

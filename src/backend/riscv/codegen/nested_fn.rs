//! RISC-V (rv64) lowering of the GNU C nested-function support instructions
//! that go beyond the direct-call static chain (`Get/SetStaticChain`, also
//! implemented here with GCC's t2 convention): stack trampolines for
//! address-taken nested functions and non-local goto.
//!
//! ## Static chain (direct nested calls)
//!
//! GCC's RISC-V psABI convention passes the static chain in `t2`.  t2 is
//! caller-saved and never part of the allocator pool (the allocator sees
//! only s1–s11), so the lowerer's contract — GetStaticChain at nested
//! entry before ordinary code can clobber it, SetStaticChain immediately
//! before a direct nested call — needs no spill cooperation from
//! register allocation.
//!
//! ## Trampoline (address-taken nested function)
//!
//! Taking the address of a chain-using nested function produces a 32-byte
//! trampoline on the parent's stack (needs an executable stack, marked via
//! `.note.GNU-stack,"x"`, which `generation.rs` emits from
//! `state.requires_executable_stack`):
//!
//! ```text
//! +0:  auipc t2, 0        # t2 = &trampoline (pc-relative chain anchor)
//! +4:  ld    t0, 24(t2)   # t0 = target function address
//! +8:  ld    t2, 16(t2)   # t2 = static chain (parent frame pointer)
//! +12: jalr  zero, 0(t0)  # tail-jump to the nested function
//! +16: <chain value, 8 bytes>
//! +24: <function address, 8 bytes>
//! ```
//!
//! The word sequence is byte-for-byte GCC 14.2's RISC-V trampoline: the
//! `auipc` anchors the chain register to the trampoline's own address, both
//! data words are read pc-relative through t2, and the tail jump never
//! links (the "call" the caller performed already established the return
//! address to the caller's site).  Everything is position-independent —
//! no absolute stack address is ever embedded, and the function address is
//! materialized at InitTrampoline time with the same la/lla sequence
//! `emit_global_addr_impl` uses (GOT-indirect for externals under PIC).
//!
//! ## Non-local goto
//!
//! `goto label;` inside a nested function targeting an enclosing
//! function's label restores the enclosing frame's `s0`/`sp` from the
//! save area in its frame struct (saved at its entry by
//! `NonlocalGotoSave`) and branches to the label.  The label is a named
//! alias of the ENCLOSING function's block label; block IDs are
//! file-unique so the cross-function branch assembles fine.  Only
//! `s0`/`sp` are restored: callee-saved s-registers are preserved by the
//! ABI through the intervening ordinary calls and may carry
//! source-visible global-register assignments that an entry snapshot
//! would erase (the same reasoning as the x86-64/AArch64 implementations).

use super::emit::{callee_saved_name, RiscvCodegen};
use crate::ir::reexports::{Operand, Value};

impl RiscvCodegen {
    /// `GetStaticChain`: read the incoming static chain from t2 into
    /// `dest` (register home or stack slot).
    pub(super) fn emit_get_static_chain_impl(&mut self, dest: &Value) {
        if let Some(&d_reg) = self.reg_assignments.get(&dest.0) {
            let name = callee_saved_name(d_reg);
            if name != "t2" {
                self.state
                    .emit_fmt(format_args!("    mv {}, t2", name));
            }
            return;
        }
        if self.state.get_slot(dest.0).is_none() {
            return;
        }
        self.state.emit("    mv t0, t2");
        self.store_t0_to(dest);
    }

    /// `SetStaticChain`: load `src` into the static-chain register t2
    /// immediately before a direct nested-function call.
    pub(super) fn emit_set_static_chain_impl(&mut self, src: &Operand) {
        self.operand_to_t0(src);
        self.state.emit("    mv t2, t0");
    }

    /// `InitTrampoline`: write the 32-byte trampoline into `buffer`
    /// (layout in the module docs above).  The chain value is runtime
    /// data (the parent's frame pointer); the function address is
    /// materialized with the same la/lla sequence `emit_global_addr_impl`
    /// uses.
    pub(super) fn emit_init_trampoline_impl(
        &mut self,
        buffer: &Value,
        chain: &Operand,
        func: &str,
    ) {
        // 1. Chain value -> t3 (trampoline DATA at +16: t2 itself is
        //    loaded by the trampoline words at runtime).
        self.operand_to_t0(chain);
        self.state.emit("    mv t3, t0");
        // 2. Buffer address -> t0 (alloca address, register home, or slot).
        if self.state.is_alloca(buffer.0) {
            if let Some(slot) = self.state.get_slot(buffer.0) {
                self.emit_alloca_addr("t0", buffer.0, slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&buffer.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mv t0, {}", reg_name));
        } else if let Some(slot) = self.state.get_slot(buffer.0) {
            self.emit_load_from_s0("t0", slot.0, "ld");
        }
        // 3. Function address -> t1.
        if self.state.needs_got(func) {
            self.state.emit_fmt(format_args!("    la t1, {}", func));
        } else {
            self.state.emit_fmt(format_args!("    lla t1, {}", func));
        }
        // 4. Code words (RV64I encodings; see the module docs).  The nop
        //    keeps the data words 8-byte aligned.
        const CODE: [(i64, u32); 4] = [
            (0, 0x0000_0397),  // auipc t2, 0
            (4, 0x0183_B283),  // ld t0, 24(t2)
            (8, 0x0103_B383),  // ld t2, 16(t2)
            (12, 0x0002_8067), // jalr zero, 0(t0)
        ];
        for (offset, word) in CODE {
            self.state
                .emit_fmt(format_args!("    li t4, {}", word as i64));
            self.state
                .emit_fmt(format_args!("    sw t4, {}(t0)", offset));
        }
        // 5. Data: chain at +16, function address at +24.
        self.state.emit("    sd t3, 16(t0)");
        self.state.emit("    sd t1, 24(t0)");
        self.state.reg_cache.invalidate_all();
        // Executable stack required: record on the module output.
        self.state.requires_executable_stack = true;
    }

    /// `NonlocalGotoSave`: store the restorable state of this frame into
    /// the save area of its frame struct: `s0` (frame pointer) and `sp`.
    /// Unlike AArch64's `str sp`, sp IS a legal sd source on RISC-V, so no
    /// staging register is needed.
    pub(super) fn emit_nonlocal_goto_save_impl(
        &mut self,
        frame: &Value,
        rbp_off: i64,
        rsp_off: i64,
    ) {
        self.operand_to_t0(&Operand::Value(*frame));
        self.state
            .emit_fmt(format_args!("    sd s0, {}(t0)", rbp_off));
        self.state
            .emit_fmt(format_args!("    sd sp, {}(t0)", rsp_off));
        self.state.reg_cache.invalidate_all();
    }

    /// `NonlocalGoto`: walk `up` frame links from `chain`, restore the
    /// saved `s0`/`sp`, and branch to the (cross-function) label.
    pub(super) fn emit_nonlocal_goto_impl(
        &mut self,
        chain: &Operand,
        up: usize,
        rbp_off: i64,
        rsp_off: i64,
        label: &str,
    ) {
        self.operand_to_t0(chain);
        for _ in 0..up {
            self.state.emit("    ld t0, 0(t0)");
        }
        // Restore s0 first, then sp via t1: both loads read the OLD stack
        // through t0, which must stay valid until the switch.
        self.state
            .emit_fmt(format_args!("    ld s0, {}(t0)", rbp_off));
        self.state
            .emit_fmt(format_args!("    ld t1, {}(t0)", rsp_off));
        self.state.emit("    mv sp, t1");
        self.state.emit_fmt(format_args!("    j {}", label));
        self.state.reg_cache.invalidate_all();
    }
}

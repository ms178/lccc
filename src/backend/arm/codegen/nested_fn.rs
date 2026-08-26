//! AArch64 lowering of the GNU C nested-function support instructions that
//! go beyond the direct-call static chain (`returns.rs` implements
//! `Get/SetStaticChain` with GCC's x18 convention): stack trampolines for
//! address-taken nested functions and non-local goto.
//!
//! ## Trampoline (address-taken nested function)
//!
//! Taking the address of a chain-using nested function produces a 32-byte
//! trampoline on the parent's stack (needs an executable stack, marked via
//! `.note.GNU-stack,"x"`, which `generation.rs` emits from
//! `state.requires_executable_stack`):
//!
//! ```text
//! +0:  58000072   ldr x18, [pc, #12]   ; chain from literal at +16
//! +4:  58000091   ldr x17, [pc, #16]   ; func  from literal at +24
//! +8:  d61f0220   br  x17
//! +12: d503201f   nop                  (keeps the literals 8-byte aligned)
//! +16: <chain value, 8 bytes>
//! +24: <function address, 8 bytes>
//! ```
//!
//! The `ldr (literal)` forms make the trampoline position-independent:
//! pc-relative data requires no relocation patching at runtime, and no
//! absolute stack address ever needs to be embedded. This mirrors GCC's
//! AArch64 trampoline shape (pc-relative loads + `br`).
//!
//! ## Non-local goto
//!
//! `goto label;` inside a nested function targeting an enclosing
//! function's label restores the enclosing frame's `x29`/`sp` from the
//! save area in its frame struct (saved at its entry by
//! `NonlocalGotoSave`) and branches to the label. The label is a plain
//! block label of the ENCLOSING function; block IDs are file-unique so
//! the cross-function branch assembles fine. Only `x29`/`sp` are
//! restored: callee-saved X registers are preserved by the ABI through
//! the intervening ordinary calls and may carry source-visible
//! global-register assignments that an entry snapshot would erase (the
//! same reasoning as the x86-64 implementation).

use super::emit::{callee_saved_name, ArmCodegen};
use crate::ir::reexports::{Operand, Value};

impl ArmCodegen {
    /// `InitTrampoline`: write the 32-byte trampoline into `buffer`
    /// (layout in the module docs above). The chain value is runtime data
    /// (the parent's frame pointer); the function address is materialized
    /// with the same adrp+add/:got sequence `emit_global_addr_impl` uses.
    pub(super) fn emit_init_trampoline_impl(
        &mut self,
        buffer: &Value,
        chain: &Operand,
        func: &str,
    ) {
        // 1. Chain value -> x2 (through the generic x0 operand loader).
        self.operand_to_x0(chain);
        self.state.emit("    mov x2, x0");
        // 2. Buffer address -> x0 (alloca address, register home, or slot).
        if self.state.is_alloca(buffer.0) {
            if let Some(slot) = self.state.get_slot(buffer.0) {
                self.emit_alloca_addr("x0", buffer.0, slot.0);
            }
        } else if let Some(&reg) = self.reg_assignments.get(&buffer.0) {
            let reg_name = callee_saved_name(reg);
            self.state
                .emit_fmt(format_args!("    mov x0, {}", reg_name));
        } else if let Some(slot) = self.state.get_slot(buffer.0) {
            self.emit_load_from_sp("x0", slot.0, "ldr");
        }
        // 3. Function address -> x1.
        if self.state.needs_got_aarch64(func) {
            self.state
                .emit_fmt(format_args!("    adrp x1, :got:{}", func));
            self.state
                .emit_fmt(format_args!("    ldr x1, [x1, :got_lo12:{}]", func));
        } else {
            self.state.emit_fmt(format_args!("    adrp x1, {}", func));
            self.state
                .emit_fmt(format_args!("    add x1, x1, :lo12:{}", func));
        }
        // 4. Code words (encodings derived in the module docs).
        const CODE: [(i64, u32); 4] = [
            (0, 0x5800_0072), // ldr x18, [pc, #12]
            (4, 0x5800_0091), // ldr x17, [pc, #16]
            (8, 0xD61F_0220), // br x17
            (12, 0xD503_201F), // nop
        ];
        for (offset, word) in CODE {
            self.load_large_imm("x3", word as i64);
            self.state.emit_fmt(format_args!("    str x3, [x0, #{}]", offset));
        }
        // 5. Data: chain at +16, function address at +24.
        self.state.emit("    str x2, [x0, #16]");
        self.state.emit("    str x1, [x0, #24]");
        self.state.reg_cache.invalidate_all();
        // Executable stack required: record on the module output.
        self.state.requires_executable_stack = true;
    }

    /// `NonlocalGotoSave`: store the restorable state of this frame into
    /// the save area of its frame struct: `x29` and `sp` (callee-saved
    /// registers need no snapshot — see the module docs).
    pub(super) fn emit_nonlocal_goto_save_impl(
        &mut self,
        frame: &Value,
        rbp_off: i64,
        rsp_off: i64,
    ) {
        self.operand_to_x0(&Operand::Value(*frame));
        self.state.emit_fmt(format_args!("    str x29, [x0, #{}]", rbp_off));
        // sp cannot be a str source register; move it through x1.
        self.state.emit("    mov x1, sp");
        self.state.emit_fmt(format_args!("    str x1, [x0, #{}]", rsp_off));
        self.state.reg_cache.invalidate_all();
    }

    /// `NonlocalGoto`: walk `up` frame links from `chain`, restore the
    /// saved `x29`/`sp`, and branch to the (cross-function) label.
    pub(super) fn emit_nonlocal_goto_impl(
        &mut self,
        chain: &Operand,
        up: usize,
        rbp_off: i64,
        rsp_off: i64,
        label: &str,
    ) {
        self.operand_to_x0(chain);
        for _ in 0..up {
            self.state.emit("    ldr x0, [x0]");
        }
        // Restore x29 first, then sp: both loads read the OLD stack
        // through x0, which must stay valid until the switch.
        self.emit_load_from_reg("x29", "x0", rbp_off, "ldr");
        self.emit_load_from_reg("x1", "x0", rsp_off, "ldr");
        self.state.emit("    mov sp, x1");
        self.state.emit_fmt(format_args!("    b {}", label));
        self.state.reg_cache.invalidate_all();
    }
}

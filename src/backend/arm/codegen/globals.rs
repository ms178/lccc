//! ArmCodegen: global address operations.

use super::emit::ArmCodegen;
use crate::ir::reexports::{TlsModel, Value};

impl ArmCodegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.needs_got_aarch64(name) {
            self.state
                .emit_fmt(format_args!("    adrp x0, :got:{}", name));
            self.state
                .emit_fmt(format_args!("    ldr x0, [x0, :got_lo12:{}]", name));
        } else {
            self.state.emit_fmt(format_args!("    adrp x0, {}", name));
            self.state
                .emit_fmt(format_args!("    add x0, x0, :lo12:{}", name));
        }
        self.store_x0_to(dest);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        self.state.emit_fmt(format_args!("    adrp x0, {}", label));
        self.state
            .emit_fmt(format_args!("    add x0, x0, :lo12:{}", label));
        self.store_x0_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        match self.state.tls_model_for_symbol(name) {
            TlsModel::LocalExec => {
                self.state.emit("    mrs x0, tpidr_el0");
                self.state
                    .emit_fmt(format_args!("    add x0, x0, :tprel_hi12:{}", name));
                self.state
                    .emit_fmt(format_args!("    add x0, x0, :tprel_lo12_nc:{}", name));
            }
            TlsModel::InitialExec => {
                // The GOT entry contains a TP-relative offset. Keep the offset
                // and thread pointer in the backend's reserved x0/x1 scratch
                // pair so no allocator home is disturbed.
                self.state
                    .emit_fmt(format_args!("    adrp x0, :gottprel:{}", name));
                self.state
                    .emit_fmt(format_args!("    ldr x0, [x0, :gottprel_lo12:{}]", name));
                self.state.emit("    mrs x1, tpidr_el0");
                self.state.emit("    add x0, x1, x0");
            }
            TlsModel::GlobalDynamic | TlsModel::LocalDynamic => {
                // AArch64's TLSDESC resolver takes the descriptor address in x0
                // and returns a TP-relative offset in x0; `.tlsdesccall` marks
                // the `blr` so a static linker can relax the whole sequence to
                // IE or LE.
                //
                // The thread pointer is read AFTER the call, exactly as GCC and
                // clang 23.1.3 emit it. Only x0's contract is normative --
                // aaelf64 defers the resolver's own register behaviour to the
                // TLSDESC paper -- so hoisting the `mrs` above the call would
                // trade a real correctness dependency on every resolver (glibc,
                // musl, or one a linker synthesizes) for at most the ~1 cycle
                // latency of a dependency-free `mrs`, which an out-of-order core
                // issues early regardless of program order.
                //
                // x1 carries both the resolver target and the thread pointer: x1
                // is dead once `blr x1` has dispatched, and unlike clang's x8 it
                // is not an allocator home here (ARM_CALLER_SAVED is x4-x8 plus
                // x13/x14), so no live value can be displaced.
                self.state
                    .emit_fmt(format_args!("    adrp x0, :tlsdesc:{}", name));
                self.state
                    .emit_fmt(format_args!("    ldr x1, [x0, :tlsdesc_lo12:{}]", name));
                self.state
                    .emit_fmt(format_args!("    add x0, x0, :tlsdesc_lo12:{}", name));
                self.state
                    .emit_fmt(format_args!("    .tlsdesccall {}", name));
                self.state.emit("    blr x1");
                self.state.emit("    mrs x1, tpidr_el0");
                self.state.emit("    add x0, x0, x1");
            }
        }
        self.store_x0_to(dest);
    }
}

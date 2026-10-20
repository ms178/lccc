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
                // AArch64's TLSDESC resolver returns a TP-relative offset in
                // x0 under its special register-preserving ABI. The marker is
                // required for static-link relaxation; the final two
                // instructions materialize the address for unrelaxed DSOs.
                // Read TP before the independent descriptor loads so its
                // latency overlaps their address generation. The TLSDESC ABI
                // preserves x1; x2 carries the resolver target.
                self.state.emit("    mrs x1, tpidr_el0");
                self.state
                    .emit_fmt(format_args!("    adrp x0, :tlsdesc:{}", name));
                self.state
                    .emit_fmt(format_args!("    ldr x2, [x0, :tlsdesc_lo12:{}]", name));
                self.state
                    .emit_fmt(format_args!("    add x0, x0, :tlsdesc_lo12:{}", name));
                self.state
                    .emit_fmt(format_args!("    .tlsdesccall {}", name));
                self.state.emit("    blr x2");
                self.state.emit("    add x0, x1, x0");
            }
        }
        self.store_x0_to(dest);
    }
}

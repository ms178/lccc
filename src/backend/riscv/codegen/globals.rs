//! RiscvCodegen: global address loading, PIC-relative.

use super::emit::{RiscvCodegen, callee_saved_name};
use crate::ir::reexports::{TlsModel, Value};

impl RiscvCodegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.needs_got(name) {
            // PIC mode, external symbol: use `la` which expands to GOT-indirect
            // auipc + ld with R_RISCV_GOT_HI20 relocation.
            self.state.emit_fmt(format_args!("    la t0, {}", name));
        } else {
            // Non-PIC mode or local symbol: use `lla` (local load address) which
            // expands to PC-relative auipc + addi with R_RISCV_PCREL_HI20.
            self.state.emit_fmt(format_args!("    lla t0, {}", name));
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        // Labels are always local symbols -- use lla (local load address) to avoid
        // GOT indirection that `la` may produce in PIC mode.
        self.state.emit_fmt(format_args!("    lla t0, {}", label));
        self.store_t0_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        // Static models can write the allocated destination directly. In the
        // common address-return case this is a0, eliminating the t0 relay and
        // matching the psABI oracle sequence instruction-for-instruction.
        let dest_reg = self
            .reg_assignments
            .get(&dest.0)
            .copied()
            .map(callee_saved_name);
        let out = dest_reg.unwrap_or("t0");
        match self.state.tls_model_for_symbol(name) {
            TlsModel::LocalExec => {
                self.state
                    .emit_fmt(format_args!("    lui {}, %tprel_hi({})", out, name));
                self.state.emit_fmt(format_args!(
                    "    add {0}, {0}, tp, %tprel_add({1})",
                    out, name
                ));
                self.state
                    .emit_fmt(format_args!("    addi {0}, {0}, %tprel_lo({1})", out, name));
            }
            TlsModel::InitialExec => {
                let anchor = self.state.fresh_label("tls_ie");
                self.state.emit_fmt(format_args!("{}:", anchor));
                self.state.emit_fmt(format_args!(
                    "    auipc {}, %tls_ie_pcrel_hi({})",
                    out, name
                ));
                self.state
                    .emit_fmt(format_args!("    ld {0}, %pcrel_lo({1})({0})", out, anchor));
                self.state
                    .emit_fmt(format_args!("    add {0}, {0}, tp", out));
            }
            TlsModel::GlobalDynamic | TlsModel::LocalDynamic => {
                // The psABI's GD sequence is an ordinary call. Prologue
                // analysis therefore treats dynamic TLS GlobalAddr as a hidden
                // call: only callee-saved value homes remain live across it and
                // leaf-frame elision is disabled.
                let anchor = self.state.fresh_label("tls_gd");
                self.state.emit_fmt(format_args!("{}:", anchor));
                self.state
                    .emit_fmt(format_args!("    auipc a0, %tls_gd_pcrel_hi({})", name));
                self.state
                    .emit_fmt(format_args!("    addi a0, a0, %pcrel_lo({})", anchor));
                self.state.emit("    call __tls_get_addr");
                if let Some(reg) = dest_reg {
                    if reg != "a0" {
                        self.state.emit_fmt(format_args!("    mv {}, a0", reg));
                    }
                    return;
                }
                self.state.emit("    mv t0, a0");
                self.store_t0_to(dest);
                return;
            }
        }
        if dest_reg.is_none() {
            self.store_t0_to(dest);
        }
    }
}

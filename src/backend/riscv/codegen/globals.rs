//! RiscvCodegen: global address loading, PIC-relative.

use super::emit::{RiscvCodegen, callee_saved_name};
use crate::ir::reexports::{TlsModel, Value};

impl RiscvCodegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.needs_got(name) {
            // PIC mode, preemptible symbol: the address from its GOT slot.
            // `lga` is the GOT form unconditionally (GAS >= 2.39, LLVM >= 17);
            // plain `la` is only GOT-indirect under `.option pic`, which is
            // not emitted, so it assembled PC-relative to a preemptible symbol.
            self.state.emit_fmt(format_args!("    lga t0, {}", name));
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

    /// The address of TLS symbol `name` in the model `common::tls` chose —
    /// GCC 16.2's sequences (`-mtls-dialect=trad`, its RISC-V default):
    ///
    /// * Local-Exec: `lui %tprel_hi; add tp, %tprel_add; addi %tprel_lo`.
    /// * Initial-Exec: `auipc %tls_ie_pcrel_hi` + `ld %pcrel_lo` (the TP
    ///   offset from the GOT), plus tp.
    /// * Global/Local-Dynamic: `auipc %tls_gd_pcrel_hi` + `addi %pcrel_lo` +
    ///   `call __tls_get_addr`. A real call: the access is a call point to
    ///   the allocator (`common::tls::install_call_sequence_symbols`), so
    ///   nothing live sits in a caller-saved register across it, and
    ///   `riscv_call_free_ir` keeps the function framed (`ra` saved) and out
    ///   of the caller-saved pool.
    ///
    /// The static models write the allocated destination register directly
    /// (no t0 relay), which is the psABI sequence instruction for instruction.
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
        match self.state.tls_model(name) {
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
                // The resolver is a real call: it clobbers t0 (the
                // accumulator) and every other caller-saved register, so no
                // cached accumulator value survives it. Without this the
                // allocated-destination return below left `acc` naming a
                // value whose t0 copy the call destroyed, and the next use's
                // `acc_has` fast path read the dead register.
                self.state.reg_cache.invalidate_all();
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

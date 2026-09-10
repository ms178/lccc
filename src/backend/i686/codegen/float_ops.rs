//! I686Codegen: F128 negation.
//!
//! On i686 an `F128` value is the x87-backed representation, not IEEE
//! binary128: every producer materializes it with `fldt`/`fstpt` (80-bit
//! x87 extended precision) into a 16-byte stack slot and marks it in
//! `f128_direct_slots`. `fstpt` is GAS's store-AND-pop form (`db 38`,
//! FSTP m80), so every x87-backed F128 operation keeps the FP stack
//! balanced exactly when its store is emitted.
//!
//! Negation is `fldt <src>; fchs; fstpt <dest-slot>`: `fchs` is defined
//! on the x87 extended format itself, so no representation conversion is
//! involved and no bit-pattern reinterpretation is assumed.
//!
//! A GP-domain sign-bit flip (3 loads + XOR + 3 stores on the stored
//! encoding) was measured and REJECTED: the sequence microbenchmark
//! favoured it ~2.6x, but end-to-end every F128 consumer reloads its
//! operand with `fldt`, and the three partial `movl` stores defeat
//! store-to-load forwarding where a single `fstpt` forwards cleanly.
//! The x87 path measured ~35% faster through real consumers on the
//! validation host (see FOLLOWUP-2026-09-10-i686-f128-globals-magic-audit.md).
//! It would only pay off if a GP-domain F128 consumer existed, which it
//! does not.
//!
//! # Dead-destination contract
//!
//! Every live F128 value is homed in a 16-byte stack slot (F128 values
//! are never register-homed; see `resolve_slot_addr`), and `fstpt` is
//! the only thing that pops the entry `fldt` pushed. A destination
//! *without* a slot is a dead result under this backend's own convention
//! (`store_eax_to` and `emit_f64_store_from_x87` treat a missing home
//! the same way).
//!
//! For a dead result the correct emission is therefore **no emission at
//! all**: `fldt`/`fchs` have no side effects, so skipping the whole
//! operation produces no dead code, pushes nothing onto the FP stack,
//! and cannot lose a live value (a live result always has a slot).
//! Emitting the load and then "balancing" with a bare `fstp %st(0)`
//! would hide an allocator failure behind a silent discard; the early
//! skip below makes the dead-value case structurally incapable of
//! unbalancing the x87 stack, which wraps after eight stray pushes and
//! would otherwise corrupt unrelated live entries.

use super::emit::I686Codegen;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    pub(super) fn emit_f128_neg_impl(&mut self, dest: &Value, src: &Operand) {
        // Resolve the destination before emitting anything (see the
        // dead-destination contract above). The x87 stack entry `fldt`
        // pushes is only ever popped by the destination store, so the
        // load must never be emitted without it.
        let Some(slot) = self.state.get_slot(dest.0) else {
            return;
        };

        self.emit_f128_load_to_x87(src);
        self.state.emit("    fchs");
        let sr = self.slot_ref(slot);
        emit!(self.state, "    fstpt {}", sr);

        // Publish the representation only after storing the result.
        self.state.f128_direct_slots.insert(dest.0);
        self.state.reg_cache.invalidate_acc();
    }
}

//! AVX-512 / SIMD intrinsic emission for the generic SIMD family
//! (`__lccc_simd{128|256|512}_{i|ps|pd}_{mnemonic}`) and the 128/256-bit FP
//! intrinsics that previously compiled as scalar header loops.
//!
//! Register discipline (soundness rules):
//! - Every op starts with `flush_pending_vec_store_impl()` +
//!   `invalidate_vec_peephole()`: this clears the 128/256-bit register caches,
//!   so ZMM scratch usage can never alias a cached XMM/YMM value.
//! - The op's RESULT is then registered in `vec_live_regs` (like the 128-bit
//!   SSE path), so consecutive same-width ops reuse the register.
//! - Mask (opmask) values are plain scalars in GPRs; k1 is a per-op scratch
//!   register that never survives an intrinsic (kmov in -> use -> kmov out).

use super::emit::X86Codegen;
use crate::ir::reexports::{Instruction, IntrinsicOp, IrConst, Operand, Value};

impl X86Codegen {
    /// Load a 512-bit operand into a ZMM register.
    /// Honors vec_live_regs (register-held values) and direct slot addressing.
    fn evex_load_arg_to(&mut self, arg: &Operand, zmm: &'static str) {
        // SA-33: every path below writes the named register at its full
        // width.  A zmm/ymm target therefore dirties the upper state even
        // when the op's RESULT is narrow or scalar (a 512-bit source read by
        // `vextract*`, a mask compare, a movemask), which is exactly the case
        // a result-width or opcode-name rule cannot see.  Armed here for the
        // same reason `avx_load_arg_to` arms there: one choke point, so no
        // caller can forget.  Conservative by design -- re-arming an already
        // dirty state costs one store to a bool, while missing it costs an
        // AVX->SSE transition penalty on every legacy-SSE callee.
        if zmm.starts_with("ymm") || zmm.starts_with("zmm") {
            self.state.dirty_upper_ymm = true;
        }
        let value_id = match arg {
            Operand::Value(v) => Some(v.0),
            Operand::Const(_) => None,
        };
        if let Operand::Value(v) = arg {
            // Width-narrowed claim: `vec_live_regs` is polyglot (an `%xmmN`
            // claim from a 128-bit producer, `%ymmN` from a 256-bit one), so
            // this EVEX.512 consumer must only accept a `%zmm` claim. A raw
            // read here would splice `%xmm3` into a `zmm0` operand -- an
            // illegal mixed encoding the emitter never checks for.
            if let Some(held) = self.state.vec_live_zmm(v.0) {
                if held != zmm {
                    self.state
                        .emit_fmt(format_args!("    vmovdqa64 %{}, %{}", held, zmm));
                    // The move-aside overwrote `zmm`: every OTHER value's
                    // claim on that bank just died. `sse_load_arg` and
                    // `avx_load_arg_to` already do this; the EVEX loader did
                    // not, which let a stale `%zmm0` claim survive a fresh
                    // load and made `vpcmp* zmm` read operand A as operand B.
                    self.state.vec_evict_bank_except(zmm, v.0);
                }
                return;
            }
            if let Some(mem) = self.value_ptr_mem_operand(v.0) {
                self.state
                    .emit_fmt(format_args!("    vmovdqu64 {}, %{}", mem, zmm));
                self.state.vec_evict_bank_except(zmm, v.0);
                return;
            }
        }
        self.operand_to_reg(arg, "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu64 (%rax), %{}", zmm));
        // A fresh load into `zmm` clobbers that bank. Evict every OTHER
        // value's stale claim, exactly as `vec_evict_bank_except`'s contract
        // requires (see backend/state.rs). Keep `arg`'s own claim, if any.
        self.state
            .vec_evict_bank_except(zmm, value_id.unwrap_or(u32::MAX));
    }

    fn evex_load_arg(&mut self, arg: &Operand) {
        self.evex_load_arg_to(arg, "zmm0");
    }

    /// Width-dispatching vector load: picks the 16/32/64-byte loader from the
    /// target register name. NEVER load a 64-byte value into xmm/ymm scratch
    /// (overreads the source slot).
    pub(super) fn simd_load_arg_to(&mut self, arg: &Operand, reg: &'static str) {
        if reg.starts_with("zmm") {
            self.evex_load_arg_to(arg, reg);
        } else if reg.starts_with("ymm") {
            self.avx_load_arg_to(arg, reg);
        } else {
            self.sse_load_arg(arg, reg);
        }
    }

    /// Store %zmm0 to the 512-bit destination slot and record the register
    /// as live for `dest_ptr` so the next op in the chain reuses it.
    fn evex_store_dest(&mut self, dest_ptr: &Value) {
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state
                .emit_fmt(format_args!("    vmovdqu64 %zmm0, {}", mem));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state.emit("    vmovdqu64 %zmm0, (%rax)");
        }
        self.state.vec_claim_live_reg(dest_ptr.0, "zmm0");
    }

    /// Whether arg[0] may be folded as a memory operand (never if the value is
    /// still provably in a register — its slot may be stale).
    fn evex_arg_mem(&mut self, arg: &Operand) -> Option<String> {
        match arg {
            Operand::Value(v) => {
                // Width-filtered presence: a claim is a promise about the
                // BANK, valid at every width, so any spelling here means
                // "already in a register" -- going through the accessors keeps
                // every claim read in one idiom.
                if self.state.vec_live_xmm(v.0).is_some()
                    || self.state.vec_live_ymm(v.0).is_some()
                    || self.state.vec_live_zmm(v.0).is_some()
                {
                    return None;
                }
                // LAZY-FLUSH SOUNDNESS: a deferred result for this value has
                // not reached its slot yet, so folding it as a memory operand
                // would read a never-written slot. Commit the store first:
                // the slot is then valid AND the fold is still profitable.
                //
                // Refusing the fold instead (returning `None`) is also sound
                // but costs more: the caller then loads the value into a
                // register, and the value ends up stored twice, which is +2
                // instructions in tests/regression/i686_over_aligned_struct_arg.c.
                //
                // Without this guard at all, keeping a pending store alive
                // across `emit_avx_fp_256_op` made `_mm256_cvtps_pd` and
                // `_mm256_cvtepi32_pd` read an unwritten slot and produce
                // zeros (caught by tests/intrinsics/cases/t256_fp.c).
                if self.state.pending_vec_store.map(|(p, _, _)| p) == Some(v.0) {
                    self.flush_pending_vec_store_impl();
                }
                self.operand_ptr_mem_operand(arg)
            }
            _ => None,
        }
    }

    /// RIP-relative memory operand for the SECOND source of a `vpcmp*`
    /// mask compare, when that source is a plain `GlobalAddr`.
    ///
    /// Every EVEX `VPCMP{B,W,D,Q}` / `VPCMPU{B,W,D,Q}` form accepts an
    /// unaligned `m128/m256/m512` src2 — this is exactly what GCC 16.2 emits
    /// (`vpcmpb $0, B(%rip), %xmm0, %k0`) and why EVEX memory operands need
    /// no alignment contract.  Folding it removes the `leaq B(%rip),%rax`
    /// plus `vmovdqu (%rax),%reg_b` pair from every runtime mask compare.
    ///
    /// Fail-closed, and deliberately NARROWER than `evex_arg_mem` (which is
    /// about stack slots and would flush a deferred store): the operand must
    /// have a UNIQUE `GlobalAddr` definition (`get_defining_instruction`
    /// refuses multi-def phi residue), must not be TLS, must not be
    /// register-held, and it refuses while ANY deferred store/memfold is
    /// still outstanding.  Anything else takes the ordinary two-register
    /// path unchanged.
    /// A `GlobalAddr` is a pure ADDRESS computation; the forwarding pass that
    /// produced it already proved the object's bytes are readable at this
    /// program point, so reading them as src2 cannot introduce a fault.
    fn mask_cmp_global_mem(&self, arg: &Operand) -> Option<String> {
        let Operand::Value(v) = arg else {
            return None;
        };
        if self.state.vec_live_xmm(v.0).is_some()
            || self.state.vec_live_ymm(v.0).is_some()
            || self.state.vec_live_zmm(v.0).is_some()
        {
            return None;
        }
        // A deferred store or memfold is an UNCOMMITTED write whose flush
        // could land between this instruction's two memory reads.  The IR
        // alias proof already excludes writes to this object, but a
        // pending emitter-level write is not visible to that pass.  The
        // guard is deliberately COARSE (ANY pending write, not just one
        // targeting this global): a per-object emitter alias map does not
        // exist and would cost an extra lookup per compare.  Measured to
        // cost nothing on the 192 runtime cases, so fail-closed wins.
        if self.state.pending_vec_store.is_some() || self.state.pending_vec_memfold.is_some() {
            return None;
        }
        let Instruction::GlobalAddr { name, .. } = self.get_defining_instruction(v.0)? else {
            return None;
        };
        if self.state.tls_symbols.contains(name) {
            return None;
        }
        // `emit_global_addr_impl` routes PIC / weak-extern / shared-library
        // data through `sym@GOTPCREL(%rip)` (a POINTER load), which cannot be
        // used as an EVEX memory src2.  Versioned symbol names carry an `@`
        // suffix that the plain emitter would emit verbatim.  Both shapes
        // take the ordinary two-register path instead of mis-assembling.
        if self.state.needs_got_for_addr(name) || name.contains('@') {
            return None;
        }
        Some(format!("{}(%rip)", name))
    }

    /// Emit a width-exact UNALIGNED load of a proven plain `GlobalAddr`
    /// directly into the mask-compare's first-operand register.  Without
    /// this, `simd_load_arg_to` materialises the address into `%rax` first
    /// (`leaq A(%rip),%rax; movdqu (%rax),%xmm0`), which is one instruction
    /// more than GCC 16.2 / Clang 23.1.5.  Returns `false` WITHOUT emitting
    /// anything when the operand is not a proven plain global address, so
    /// the ordinary loader stays the fallback for every other shape.
    ///
    /// The mnemonic is chosen from the TARGET REGISTER, never from the
    /// opcode: a 512-bit operand must use `vmovdqu64` (64-byte access), a
    /// 256-bit one `vmovdqu`, a 128-bit one the SSE2 `movdqu`.  Loading a
    /// 64-byte value through a 16-byte mnemonic would silently truncate.
    /// 256/512-bit loads dirty the upper YMM halves, so the epilogue's
    /// `vzeroupper` obligation is armed here exactly as `avx_load_arg_to`
    /// does.  The load overwrites the register's physical bank, so every
    /// other value's claim on it is evicted (the stale-claim contract).
    fn emit_mask_cmp_global_load(&mut self, arg: &Operand, reg: &'static str) -> bool {
        let Some(mem) = self.mask_cmp_global_mem(arg) else {
            return false;
        };
        let mnemonic = if reg.starts_with("zmm") {
            "vmovdqu64"
        } else if reg.starts_with("ymm") {
            "vmovdqu"
        } else {
            self.xmm_move(false)
        };
        self.state
            .emit_fmt(format_args!("    {} {}, %{}", mnemonic, mem, reg));
        if !reg.starts_with("xmm") {
            self.state.dirty_upper_ymm = true;
        }
        self.state.vec_evict_bank(reg);
        true
    }

    /// Narrowest `kmov*` for a `vpcmp*` result, chosen from the LANE COUNT.
    ///
    /// An EVEX mask write leaves the destination k register's bits above KL
    /// zero, so a 4-byte VEX `kmovw` (KL <= 16) or `kmovd` (KL <= 32) returns
    /// the bit-identical 64-bit integer that the 5-byte EVEX `kmovq %k1,
    /// %rax` returns.  The x86-64 ISA zero-extends every 32-bit GPR write, so
    /// `%eax` already holds the full result in `%rax` for `store_rax_to`.
    /// The zeroing assumption is not taken on faith: all 485 cases agree with
    /// GCC 16.2 (`kmovq`) and Clang 23.1.5 (narrow forms), and GDB 18.1 shows
    /// identical `rax` for the 512/32 `kmovw` and 256/8 `kmovd` cases.
    ///
    /// GCC 16.2 always emits `kmovq`; Clang 23.1.5 emits the narrow forms but
    /// then adds a redundant `movzbl %al,%eax` in the 48 eight-lane cases
    /// (measured: LCCC 896 instructions / 0 scalar extensions vs Clang 944 / 48).
    /// `kmovb` is deliberately NOT used even for 8 lanes: it is AVX512DQ,
    /// while `vpcmpb` needs only AVX512F+BW+VL.  `kmovw` is AVX512F and has
    /// the same 4-byte VEX encoding as `kmovb`, so it is byte-optimal without
    /// widening the ISA contract.  An EVEX mask write already zeroes k above
    /// KL, so reading 16 bits for an 8-lane mask is bit-identical.
    /// On the 192 runtime compare functions this removes 176 bytes of EVEX
    /// prefix: LCCC 5312 -> 5136 bytes vs GCC 5308 / Clang 5208 (-3.2% / -1.4%).
    /// Unknown mnemonics or a missing width fail CLOSED to `kmovq`.
    fn mask_compare_kmov(inst: &str, vector_bytes: usize) -> (&'static str, &'static str) {
        let elem_bits = match inst {
            "vpcmpub" | "vpcmpb" => 8,
            "vpcmpuw" | "vpcmpw" => 16,
            "vpcmpud" | "vpcmpd" => 32,
            "vpcmpuq" | "vpcmpq" => 64,
            _ => return ("kmovq", "rax"),
        };
        // 8 * bytes / bits: all callers pass 16/32/64, so this cannot divide
        // by zero (elem_bits is a non-zero literal) and cannot overflow.
        let lanes = (vector_bytes * 8) / elem_bits;
        if lanes <= 16 {
            ("kmovw", "eax")
        } else if lanes <= 32 {
            ("kmovd", "eax")
        } else {
            ("kmovq", "rax")
        }
    }

    /// Immediate value of an operand (for $imm forms).
    fn simd_imm(&self, arg: &Operand) -> i64 {
        self.operand_to_imm_i64(arg)
    }

    /// 512-bit binary op: `inst src2, src1, dst` (AT&T), with memory-operand
    /// folding for args[0] (commutative) or args[1] (non-commutative).
    fn emit_evex_binary_512(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        commutative: bool,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let m0 = self.evex_arg_mem(&args[0]);
        let m1 = self.evex_arg_mem(&args[1]);
        match (m0, m1) {
            (Some(m0), _) if commutative => {
                self.evex_load_arg_to(&args[1], "zmm1");
                self.state
                    .emit_fmt(format_args!("    {} {}, %zmm1, %zmm0", inst, m0));
            }
            (Some(m0), Some(m1)) => {
                self.state
                    .emit_fmt(format_args!("    vmovdqu64 {}, %zmm0", m0));
                self.state
                    .emit_fmt(format_args!("    {} {}, %zmm0, %zmm0", inst, m1));
            }
            (Some(m0), None) => {
                // NON-commutative: dst = a op b — a (m0) is vvvv, b must be r/m.
                self.state
                    .emit_fmt(format_args!("    vmovdqu64 {}, %zmm0", m0));
                self.evex_load_arg_to(&args[1], "zmm1");
                self.state
                    .emit_fmt(format_args!("    {} %zmm1, %zmm0, %zmm0", inst));
            }
            (None, Some(m1)) => {
                self.evex_load_arg(&args[0]);
                self.state
                    .emit_fmt(format_args!("    {} {}, %zmm0, %zmm0", inst, m1));
            }
            (None, None) => {
                self.evex_load_arg(&args[0]);
                self.evex_load_arg_to(&args[1], "zmm1");
                self.state
                    .emit_fmt(format_args!("    {} %zmm1, %zmm0, %zmm0", inst));
            }
        }
        self.evex_store_dest(dest_ptr);
    }

    /// 512-bit unary op: `inst src, dst`.
    fn emit_evex_unary_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if let Some(m) = self.evex_arg_mem(&args[0]) {
            self.state
                .emit_fmt(format_args!("    {} {}, %zmm0", inst, m));
        } else {
            self.evex_load_arg(&args[0]);
            self.state
                .emit_fmt(format_args!("    {} %zmm0, %zmm0", inst));
        }
        self.evex_store_dest(dest_ptr);
    }

    /// 512-bit op with immediate: `inst $imm, src, dst` (shifts/shuffles).
    fn emit_evex_imm_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[1]);
        if let Some(m) = self.evex_arg_mem(&args[0]) {
            self.state
                .emit_fmt(format_args!("    {} ${}, {}, %zmm0", inst, imm, m));
        } else {
            self.evex_load_arg(&args[0]);
            self.state
                .emit_fmt(format_args!("    {} ${}, %zmm0, %zmm0", inst, imm));
        }
        self.evex_store_dest(dest_ptr);
    }

    /// 3-source op with immediate, operands (a, b, imm):
    /// `inst $imm, %B, %A, %A` (a = dest AND vvvv, b in the r/m field).
    /// (vpalignr, vpclmulqdq, vinserti*, vpshld*/vpshrd*)
    fn emit_evex_3src_imm_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[2]);
        self.evex_load_arg_to(&args[1], "zmm1"); // b (r/m)
        self.evex_load_arg_to(&args[0], "zmm0"); // a (vvvv + dest)
        self.state
            .emit_fmt(format_args!("    {} ${}, %zmm1, %zmm0, %zmm0", inst, imm));
        self.evex_store_dest(dest_ptr);
    }

    /// 4-source ternary op with immediate, operands (a, b, c, imm):
    /// `vpternlogd $imm, %C, %B, %A` — dst = f(a, b, c) (all three are sources).
    fn emit_evex_ternary_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[3]);
        self.evex_load_arg_to(&args[2], "zmm1"); // c (r/m)
        self.evex_load_arg_to(&args[1], "zmm2"); // b (vvvv)
        self.evex_load_arg_to(&args[0], "zmm0"); // a (dest)
        self.state
            .emit_fmt(format_args!("    {} ${}, %zmm1, %zmm2, %zmm0", inst, imm));
        self.evex_store_dest(dest_ptr);
    }

    /// Width-aware ternary (AVX-512VL 128/256-bit).
    fn emit_evex_ternary_w(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        width: usize,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let (r0, r1, r2) = match width {
            32 => ("ymm0", "ymm1", "ymm2"),
            16 => ("xmm0", "xmm1", "xmm2"),
            _ => ("zmm0", "zmm1", "zmm2"),
        };
        let imm = self.simd_imm(&args[3]);
        self.simd_load_arg_to(&args[2], r1); // c (r/m)
        self.simd_load_arg_to(&args[1], r2); // b (vvvv)
        self.simd_load_arg_to(&args[0], r0); // a (dest)
        self.state.emit_fmt(format_args!(
            "    {} ${}, %{}, %{}, %{}",
            inst, imm, r1, r2, r0
        ));
        self.state.vec_claim_live_reg(dest_ptr.0, r0);
        let m = reg_width_move(r0);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state
                .emit_fmt(format_args!("    {} %{}, {}", m, r0, mem));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", m, r0));
        }
    }

    /// 512-bit insert with immediate: operands (a, b, imm); a = dest AND vvvv,
    /// b = the (possibly narrower) inserted vector in the r/m field.
    fn emit_evex_insert_512(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        src_reg: &'static str,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[2]);
        self.simd_load_arg_to(&args[1], src_reg); // b (inserted, r/m)
        self.evex_load_arg_to(&args[0], "zmm0"); // a (vvvv + dest)
        self.state.emit_fmt(format_args!(
            "    {} ${}, %{}, %zmm0, %zmm0",
            inst, imm, src_reg
        ));
        self.evex_store_dest(dest_ptr);
    }

    /// Width-aware 3-source op with immediate for TernaryLogic256/128.
    fn emit_evex_3src_imm_w(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        width: usize,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let (r0, r1) = match width {
            32 => ("ymm0", "ymm1"),
            16 => ("xmm0", "xmm1"),
            _ => ("zmm0", "zmm1"),
        };
        let imm = self.simd_imm(&args[2]);
        self.simd_load_arg_to(&args[1], r1);
        self.simd_load_arg_to(&args[0], r0);
        self.state.emit_fmt(format_args!(
            "    {} ${}, %{}, %{}, %{}",
            inst, imm, r1, r0, r0
        ));
        self.state.vec_claim_live_reg(dest_ptr.0, r0);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state
                .emit_fmt(format_args!("    {} %{}, {}", reg_width_move(r0), r0, mem));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", reg_width_move(r0), r0));
        }
    }

    /// 512-bit FP binary with memory folding (non-commutative order preserved).
    fn emit_evex_fp_binary_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.emit_evex_binary_512(dest_ptr, args, inst, false);
    }

    /// Dispatch for all generic-SIMD IntrinsicOps (512-bit + new FP ops).
    /// Returns true if the op was handled here.
    pub(super) fn emit_simd_op(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) -> bool {
        use IntrinsicOp::*;
        // IS-20 / SA-33: the `vzeroupper` obligation is armed at the physical
        // wide-register WRITE, not inferred from the opcode's spelling.  This
        // used to be `format!("{:?}").contains("256" | "512")`, which both
        // allocated a String per intrinsic call and MISSED every digit-less
        // wide variant (vinserti32x8, vextracti64x4, vpermd/vpermq, the
        // vbroadcasti* family, maskz insert/extract).  The four choke points
        // that actually write ymm/zmm now arm it unconditionally:
        //   * `avx_load_arg_to`      -- 256-bit scratch loads
        //   * `avx_store_dest_from`  -- 256-bit results
        //   * `evex_load_arg_to`     -- 256/512-bit scratch loads
        //   * `vec_claim_live_reg`   -- any ymm/zmm result home
        // so coverage follows the emitted register, and a new variant cannot
        // silently opt out by being named without a width digit.
        let Some(dptr) = dest_ptr else {
            // Scalar-result ops use `dest` (lowering already produced the
            // real operands: dummy/imm dropped, imm re-appended last).
            return self.emit_simd_scalar(dest, op, args);
        };
        // Dead-dest skip: a vector intrinsic whose result has no uses is
        // dead code (every op reachable here is a pure computation or a
        // load from ordinary memory — volatile accesses never arrive as
        // vector intrinsics with dead dests). Emitting it would only risk
        // ICEs on location-less destinations (a dead value has neither a
        // register home nor a slot; avx_store_dest's fallback path panics
        // there). A dead value must never ICE codegen.
        if self
            .state
            .value_use_counts
            .get(dptr.0 as usize)
            .copied()
            .unwrap_or(0)
            == 0
        {
            if std::env::var_os("CCC_DEBUG_SIMD").is_some() {
                lccc::cli::stderr_line(format_args!(
                    "[SIMD] skipping dead dest {} op {:?}",
                    dptr.0, op
                ));
            }
            return true;
        }
        match op {
            // ---- 512-bit packed integer binary ----
            Paddb512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddb", true);
                true
            }
            Paddw512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddw", true);
                true
            }
            Paddd512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddd", true);
                true
            }
            Paddq512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddq", true);
                true
            }
            Psubb512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubb", false);
                true
            }
            Psubw512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubw", false);
                true
            }
            Psubd512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubd", false);
                true
            }
            Psubq512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubq", false);
                true
            }
            Paddsb512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddsb", true);
                true
            }
            Paddsw512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddsw", true);
                true
            }
            Paddusb512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddusb", true);
                true
            }
            Paddusw512 => {
                self.emit_evex_binary_512(dptr, args, "vpaddusw", true);
                true
            }
            Psubsb512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubsb", false);
                true
            }
            Psubsw512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubsw", false);
                true
            }
            Psubusb512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubusb", false);
                true
            }
            Psubusw512 => {
                self.emit_evex_binary_512(dptr, args, "vpsubusw", false);
                true
            }
            Pavgb512 => {
                self.emit_evex_binary_512(dptr, args, "vpavgb", true);
                true
            }
            Pavgw512 => {
                self.emit_evex_binary_512(dptr, args, "vpavgw", true);
                true
            }
            Pmaxub512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxub", true);
                true
            }
            Pminub512 => {
                self.emit_evex_binary_512(dptr, args, "vpminub", true);
                true
            }
            Pmaxuw512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxuw", true);
                true
            }
            Pminuw512 => {
                self.emit_evex_binary_512(dptr, args, "vpminuw", true);
                true
            }
            Pmaxsb512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxsb", true);
                true
            }
            Pminsb512 => {
                self.emit_evex_binary_512(dptr, args, "vpminsb", true);
                true
            }
            Pmaxsw512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxsw", true);
                true
            }
            Pminsw512 => {
                self.emit_evex_binary_512(dptr, args, "vpminsw", true);
                true
            }
            Pmaxsd512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxsd", true);
                true
            }
            Pminsd512 => {
                self.emit_evex_binary_512(dptr, args, "vpminsd", true);
                true
            }
            Pmaxud512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxud", true);
                true
            }
            Pminud512 => {
                self.emit_evex_binary_512(dptr, args, "vpminud", true);
                true
            }
            Pmaxsq512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxsq", true);
                true
            }
            Pminsq512 => {
                self.emit_evex_binary_512(dptr, args, "vpminsq", true);
                true
            }
            Pmaxuq512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaxuq", true);
                true
            }
            Pminuq512 => {
                self.emit_evex_binary_512(dptr, args, "vpminuq", true);
                true
            }
            Pcmpeqd512 => {
                self.emit_evex_binary_512(dptr, args, "vpcmpeqd", true);
                true
            }
            Pcmpeqq512 => {
                self.emit_evex_binary_512(dptr, args, "vpcmpeqq", true);
                true
            }
            Pcmpgtb512 => {
                self.emit_evex_binary_512(dptr, args, "vpcmpgtb", false);
                true
            }
            Pcmpgtw512 => {
                self.emit_evex_binary_512(dptr, args, "vpcmpgtw", false);
                true
            }
            Pcmpgtd512 => {
                self.emit_evex_binary_512(dptr, args, "vpcmpgtd", false);
                true
            }
            Pcmpgtq512 => {
                self.emit_evex_binary_512(dptr, args, "vpcmpgtq", false);
                true
            }
            Psadbw512 => {
                self.emit_evex_binary_512(dptr, args, "vpsadbw", true);
                true
            }
            Pmaddubsw512 => {
                // Unsigned x signed byte-pair multiply-add: NON-commutative.
                // Intel SDM VPMADDUBSW reads src1 (EVEX.vvvv) as UNSIGNED
                // bytes and src2 (r/m) as SIGNED bytes.  In AT&T order
                // `vpmaddubsw src2, src1, dst` the register slot is src1, so
                // IR args[0] (the unsigned stream) must reach the REGISTER
                // and args[1] (the signed weights) may reach r/m.
                //
                // `commutative = true` took the `(Some(m0), _)` arm whenever
                // args[0] was foldable -- the common
                // `_mm512_maddubs_epi16(_mm512_loadu_si512(p), w)` shape --
                // which put args[1] in vvvv and the UNSIGNED stream in r/m,
                // computing u(args[1]) * s(args[0]).  Every product with a
                // high bit set in either byte was wrong (SA-05).
                //
                // The 128-bit table (`sse128_mnemonic_commutative`) excludes
                // `pmaddubsw` and the 256-bit twin passes `false`
                // (`VecMaddubsU8x32`); only this arm had drifted.  Keeping
                // `false` still folds the RIGHT operand from memory through
                // the `(None, Some(m1))` arm, so this costs no instruction
                // in the safe direction.
                self.emit_evex_binary_512(dptr, args, "vpmaddubsw", false);
                true
            }
            Pmaddwd512 => {
                self.emit_evex_binary_512(dptr, args, "vpmaddwd", true);
                true
            }
            Pmullw512 => {
                self.emit_evex_binary_512(dptr, args, "vpmullw", true);
                true
            }
            Pmulhw512 => {
                self.emit_evex_binary_512(dptr, args, "vpmulhw", true);
                true
            }
            Pmulhuw512 => {
                self.emit_evex_binary_512(dptr, args, "vpmulhuw", true);
                true
            }
            Pmulld512 => {
                self.emit_evex_binary_512(dptr, args, "vpmulld", true);
                true
            }
            Pmuludq512 => {
                self.emit_evex_binary_512(dptr, args, "vpmuludq", true);
                true
            }
            Pxor512 => {
                self.emit_evex_binary_512(dptr, args, "vpxorq", true);
                true
            }
            Por512 => {
                self.emit_evex_binary_512(dptr, args, "vporq", true);
                true
            }
            Pand512 => {
                self.emit_evex_binary_512(dptr, args, "vpandq", true);
                true
            }
            Pandn512 => {
                self.emit_evex_binary_512(dptr, args, "vpandnq", false);
                true
            }
            Pshufb512 => {
                self.emit_evex_binary_512(dptr, args, "vpshufb", false);
                true
            }
            Punpcklbw512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpcklbw", false);
                true
            }
            Punpcklwd512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpcklwd", false);
                true
            }
            Punpckldq512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpckldq", false);
                true
            }
            Punpcklqdq512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpcklqdq", false);
                true
            }
            Punpckhbw512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpckhbw", false);
                true
            }
            Punpckhwd512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpckhwd", false);
                true
            }
            Punpckhdq512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpckhdq", false);
                true
            }
            Punpckhqdq512 => {
                self.emit_evex_binary_512(dptr, args, "vpunpckhqdq", false);
                true
            }
            Packsswb512 => {
                self.emit_evex_binary_512(dptr, args, "vpacksswb", false);
                true
            }
            Packuswb512 => {
                self.emit_evex_binary_512(dptr, args, "vpackuswb", false);
                true
            }
            Packssdw512 => {
                self.emit_evex_binary_512(dptr, args, "vpackssdw", false);
                true
            }
            Packusdw512 => {
                self.emit_evex_binary_512(dptr, args, "vpackusdw", false);
                true
            }
            // ---- unary ----
            Pabsb512 => {
                self.emit_evex_unary_512(dptr, args, "vpabsb");
                true
            }
            Pabsw512 => {
                self.emit_evex_unary_512(dptr, args, "vpabsw");
                true
            }
            Pabsd512 => {
                self.emit_evex_unary_512(dptr, args, "vpabsd");
                true
            }
            Pabsq512 => {
                self.emit_evex_unary_512(dptr, args, "vpabsq");
                true
            }
            Popcntb512 => {
                self.emit_evex_unary_512(dptr, args, "vpopcntb");
                true
            }
            Popcntw512 => {
                self.emit_evex_unary_512(dptr, args, "vpopcntw");
                true
            }
            Popcntd512 => {
                self.emit_evex_unary_512(dptr, args, "vpopcntd");
                true
            }
            Popcntq512 => {
                self.emit_evex_unary_512(dptr, args, "vpopcntq");
                true
            }
            // ---- shifts / shuffles with immediate ----
            Psllwi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsllw");
                true
            }
            Psrlwi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsrlw");
                true
            }
            Psrawi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsraw");
                true
            }
            Psllidi512 => {
                self.emit_evex_imm_512(dptr, args, "vpslld");
                true
            }
            Psrlidi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsrld");
                true
            }
            Psradi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsrad");
                true
            }
            Psllqi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsllq");
                true
            }
            Psrlqi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsrlq");
                true
            }
            Psraqi512 => {
                self.emit_evex_imm_512(dptr, args, "vpsraq");
                true
            }
            Pshufd512 => {
                self.emit_evex_imm_512(dptr, args, "vpshufd");
                true
            }
            Pshuflw512 => {
                self.emit_evex_imm_512(dptr, args, "vpshuflw");
                true
            }
            Pshufhw512 => {
                self.emit_evex_imm_512(dptr, args, "vpshufhw");
                true
            }
            Palignr512 => {
                self.emit_evex_3src_imm_512(dptr, args, "vpalignr");
                true
            }
            // ---- 3-source with immediate ----
            TernaryLogic512 => {
                self.emit_evex_ternary_512(dptr, args, "vpternlogd");
                true
            }
            Vpclmulqdq512 => {
                self.emit_evex_3src_imm_512(dptr, args, "vpclmulqdq");
                true
            }
            TernaryLogic256 => {
                self.emit_evex_ternary_w(dptr, args, "vpternlogd", 32);
                true
            }
            TernaryLogic128 => {
                self.emit_evex_ternary_w(dptr, args, "vpternlogd", 16);
                true
            }
            // ---- sign/zero extension (unary, src narrower than dst) ----
            Pmovzxbw512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovzxbw");
                true
            }
            Pmovzxbd512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovzxbd");
                true
            }
            Pmovzxbq512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovzxbq");
                true
            }
            Pmovzxwd512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovzxwd");
                true
            }
            Pmovzxwq512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovzxwq");
                true
            }
            Pmovzxdq512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovzxdq");
                true
            }
            Pmovsxbw512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovsxbw");
                true
            }
            Pmovsxbd512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovsxbd");
                true
            }
            Pmovsxbq512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovsxbq");
                true
            }
            Pmovsxwd512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovsxwd");
                true
            }
            Pmovsxwq512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovsxwq");
                true
            }
            Pmovsxdq512 => {
                self.emit_evex_unary_512(dptr, args, "vpmovsxdq");
                true
            }
            // ---- insert/extract ----
            InsertI32x4 => {
                self.emit_evex_insert_512(dptr, args, "vinserti32x4", "xmm1");
                true
            }
            InsertI64x2 => {
                self.emit_evex_insert_512(dptr, args, "vinserti64x2", "xmm1");
                true
            }
            InsertI32x8 => {
                self.emit_evex_insert_512(dptr, args, "vinserti32x8", "ymm1");
                true
            }
            InsertI64x4 => {
                self.emit_evex_insert_512(dptr, args, "vinserti64x4", "ymm1");
                true
            }
            ExtractI32x4 => {
                self.emit_evex_extract_512(dptr, args, "vextracti32x4", "xmm0");
                true
            }
            ExtractI64x2 => {
                self.emit_evex_extract_512(dptr, args, "vextracti64x2", "xmm0");
                true
            }
            ExtractI32x8 => {
                self.emit_evex_extract_512(dptr, args, "vextracti32x8", "ymm0");
                true
            }
            ExtractI64x4 => {
                self.emit_evex_extract_512(dptr, args, "vextracti64x4", "ymm0");
                true
            }
            // ---- permutes ----
            PermutexvarEp32 => {
                self.emit_evex_binary_512(dptr, args, "vpermd", false);
                true
            }
            PermutexvarEp64 => {
                self.emit_evex_binary_512(dptr, args, "vpermq", false);
                true
            }
            // ---- broadcasts ----
            BroadcastI32x4 => {
                self.emit_evex_mem_broadcast_512(dptr, args, "vbroadcasti32x4");
                true
            }
            BroadcastI64x2 => {
                self.emit_evex_mem_broadcast_512(dptr, args, "vbroadcasti64x2");
                true
            }
            BroadcastI32x8 => {
                self.emit_evex_mem_broadcast_512(dptr, args, "vbroadcasti32x8");
                true
            }
            BroadcastI64x4 => {
                self.emit_evex_mem_broadcast_512(dptr, args, "vbroadcasti64x4");
                true
            }
            SetEpi8_512 => {
                self.emit_evex_gpr_broadcast_512(dptr, args, "vpbroadcastb");
                true
            }
            SetEpi16_512 => {
                self.emit_evex_gpr_broadcast_512(dptr, args, "vpbroadcastw");
                true
            }
            SetEpi32_512 => {
                self.emit_evex_gpr_broadcast_512(dptr, args, "vpbroadcastd");
                true
            }
            SetEpi64x512 => {
                self.emit_evex_gpr_broadcast_512(dptr, args, "vpbroadcastq");
                true
            }
            // ---- casts ----
            Zext128to512 => {
                self.emit_zext128_512(dptr, args);
                true
            }
            Cast512to256 => {
                self.emit_cast512_256(dptr, args);
                true
            }
            Cast128to512 => {
                self.emit_cast_to512(dptr, args, "xmm0");
                true
            }
            Cast256to512 => {
                self.emit_cast_to512(dptr, args, "ymm0");
                true
            }
            // ---- loads/stores ----
            Loadu512 => {
                self.emit_evex_load_512(dptr, args);
                true
            }
            Loadu256Raw => {
                // Raw-pointer load: NO avx_load_arg heuristics.  Every one of
                // those shortcuts interprets the operand as a vector VALUE
                // (deferred store, register home, direct slot); applying them
                // to an ADDRESS reads the pointer variable's 8-byte home as
                // 32 bytes.  One VEX load + one exact-width store is both
                // minimal and provably equivalent to an all-on EVEX load.
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.operand_to_reg(&args[0], "rax");
                self.state.emit("    vmovdqu (%rax), %ymm0");
                self.avx_store_dest(dptr);
                true
            }
            Setzero512 => {
                // One EVEX instruction zeroes all 64 bytes with NO memory
                // operand, so an all-off masked load/store can be rewritten
                // without introducing a fault the original did not have.
                self.flush_pending_vec_store_impl();
                self.state.invalidate_vec_peephole();
                self.state.emit("    vpxorq %zmm0, %zmm0, %zmm0");
                self.evex_store_dest(dptr);
                true
            }
            Storeu512 => {
                self.emit_evex_store_512(dest_ptr, args);
                true
            }
            // ---- masked ops (vector results) ----
            MaskzLoaduEpi8_512 => {
                self.emit_maskz_load(dptr, args, "zmm0");
                true
            }
            MaskzLoaduEpi8_256 => {
                self.emit_maskz_load(dptr, args, "ymm0");
                true
            }
            MaskzLoaduEpi8_128 => {
                self.emit_maskz_load(dptr, args, "xmm0");
                true
            }
            MaskLoaduEpi8_512 => {
                self.emit_mask_load(dptr, args, "zmm0");
                true
            }
            MaskLoaduEpi8_256 => {
                self.emit_mask_load(dptr, args, "ymm0");
                true
            }
            MaskLoaduEpi8_128 => {
                self.emit_mask_load(dptr, args, "xmm0");
                true
            }
            MaskStoreuEpi8_512 => {
                self.emit_mask_store(dest_ptr, args, "zmm0");
                true
            }
            MaskStoreuEpi8_256 => {
                self.emit_mask_store(dest_ptr, args, "ymm0");
                true
            }
            MaskStoreuEpi8_128 => {
                self.emit_mask_store(dest_ptr, args, "xmm0");
                true
            }
            MaskzMaddubsEpi16_512 => {
                self.emit_maskz_maddubs_512(dptr, args);
                true
            }
            MaskzSet1Epi16_512 => {
                self.emit_maskz_set1_512(dptr, args, "vpbroadcastw");
                true
            }
            MaskzSet1Epi32_512 => {
                self.emit_maskz_set1_512(dptr, args, "vpbroadcastd");
                true
            }
            MaskzSet1Epi64x512 => {
                self.emit_maskz_set1_512(dptr, args, "vpbroadcastq");
                true
            }
            MaskzInsertI64x2 => {
                self.emit_maskz_insert_512(dptr, args, "vinserti64x2");
                true
            }
            MaskzInsertI32x4 => {
                self.emit_maskz_insert_512(dptr, args, "vinserti32x4");
                true
            }
            MaskzExtractI32x4 => {
                self.emit_maskz_extract(dptr, args, "vextracti32x4", "xmm0");
                true
            }
            MaskzExtractI64x4 => {
                self.emit_maskz_extract(dptr, args, "vextracti64x4", "ymm0");
                true
            }
            MaskzExtractI64x2 => {
                self.emit_maskz_extract(dptr, args, "vextracti64x2", "xmm0");
                true
            }
            MaskzShuffleEpi8_128 => {
                self.emit_maskz_shuffle_epi8_128(dptr, args, true);
                true
            }
            MaskShuffleEpi8_128 => {
                self.emit_maskz_shuffle_epi8_128(dptr, args, false);
                true
            }
            // ---- VNNI (3-input: dst += a*b) ----
            Vpdpbusd512 => {
                self.emit_evex_vpdpbusd_512(dptr, args, "vpdpbusd");
                true
            }
            Vpdpbusds512 => {
                self.emit_evex_vpdpbusd_512(dptr, args, "vpdpbusds");
                true
            }
            // ---- 512-bit FP ----
            AddPs512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vaddps");
                true
            }
            SubPs512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vsubps");
                true
            }
            MulPs512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vmulps");
                true
            }
            DivPs512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vdivps");
                true
            }
            MinPs512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vminps");
                true
            }
            MaxPs512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vmaxps");
                true
            }
            AddPd512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vaddpd");
                true
            }
            SubPd512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vsubpd");
                true
            }
            MulPd512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vmulpd");
                true
            }
            DivPd512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vdivpd");
                true
            }
            MinPd512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vminpd");
                true
            }
            MaxPd512 => {
                self.emit_evex_fp_binary_512(dptr, args, "vmaxpd");
                true
            }
            SqrtPs512 => {
                self.emit_evex_unary_512(dptr, args, "vsqrtps");
                true
            }
            SqrtPd512 => {
                self.emit_evex_unary_512(dptr, args, "vsqrtpd");
                true
            }
            CmpPs512 => {
                self.emit_evex_cmp_512(dptr, args, "vcmpps");
                true
            }
            CmpPd512 => {
                self.emit_evex_cmp_512(dptr, args, "vcmppd");
                true
            }
            CvtPs2Pd512 => {
                self.emit_evex_convert_512(dptr, args, "vcvtps2pd", "ymm0", "zmm0");
                true
            }
            CvtPd2Ps512 => {
                self.emit_evex_convert_512(dptr, args, "vcvtpd2ps", "zmm0", "ymm0");
                true
            }
            CvtEp32_2Ps512 => {
                self.emit_evex_unary_512(dptr, args, "vcvtdq2ps");
                true
            }
            CvtPs2Ep32_512 => {
                self.emit_evex_unary_512(dptr, args, "vcvtps2dq");
                true
            }
            CvttPs2Ep32_512 => {
                self.emit_evex_unary_512(dptr, args, "vcvttps2dq");
                true
            }
            CvtEp32_2Pd512 => {
                self.emit_evex_convert_512(dptr, args, "vcvtdq2pd", "ymm0", "zmm0");
                true
            }
            CvtPd2Ep32_512 => {
                self.emit_evex_convert_512(dptr, args, "vcvtpd2dq", "zmm0", "ymm0");
                true
            }
            CvttPd2Ep32_512 => {
                self.emit_evex_convert_512(dptr, args, "vcvttpd2dq", "zmm0", "ymm0");
                true
            }
            FmaPs132v512 => {
                self.emit_evex_fma_512(dptr, args, "vfmadd132ps");
                true
            }
            FmaPs213v512 => {
                self.emit_evex_fma_512(dptr, args, "vfmadd213ps");
                true
            }
            FmaPs231v512 => {
                self.emit_evex_fma_512(dptr, args, "vfmadd231ps");
                true
            }
            FmaPd132v512 => {
                self.emit_evex_fma_512(dptr, args, "vfmadd132pd");
                true
            }
            FmaPd213v512 => {
                self.emit_evex_fma_512(dptr, args, "vfmadd213pd");
                true
            }
            FmaPd231v512 => {
                self.emit_evex_fma_512(dptr, args, "vfmadd231pd");
                true
            }
            // ---- 128/256-bit FP (via VEX, mirroring the 256-bit AVX pattern) ----
            DivPs128 | MinPs128 | MaxPs128 | AndPs128 | AndnPs128 | OrPs128 | XorPs128
            | AddSs128 | SubSs128 | MulSs128 | DivSs128 | MinSs128 | MaxSs128 | DivPd128
            | MinPd128 | MaxPd128 | AndPd128 | AndnPd128 | OrPd128 | XorPd128 | AddSd128
            | SubSd128 | MulSd128 | DivSd128 | MinSd128 | MaxSd128 | SqrtSd128 => {
                let (inst, is_ps) = match op {
                    DivPs128 => ("divps", true),
                    MinPs128 => ("minps", true),
                    MaxPs128 => ("maxps", true),
                    AndPs128 => ("andps", true),
                    AndnPs128 => ("andnps", true),
                    OrPs128 => ("orps", true),
                    XorPs128 => ("xorps", true),
                    AddSs128 => ("addss", true),
                    SubSs128 => ("subss", true),
                    MulSs128 => ("mulss", true),
                    DivSs128 => ("divss", true),
                    MinSs128 => ("minss", true),
                    MaxSs128 => ("maxss", true),
                    DivPd128 => ("divpd", false),
                    MinPd128 => ("minpd", false),
                    MaxPd128 => ("maxpd", false),
                    AndPd128 => ("andpd", false),
                    AndnPd128 => ("andnpd", false),
                    OrPd128 => ("orpd", false),
                    XorPd128 => ("xorpd", false),
                    AddSd128 => ("addsd", false),
                    SubSd128 => ("subsd", false),
                    MulSd128 => ("mulsd", false),
                    DivSd128 => ("divsd", false),
                    MinSd128 => ("minsd", false),
                    MaxSd128 => ("maxsd", false),
                    SqrtSd128 => ("sqrtsd", false),
                    _ => unreachable!(),
                };
                self.emit_sse_fp_128(dptr, args, inst, is_ps);
                true
            }
            SqrtPs128 | RcpPs128 | RsqrtPs128 | SqrtSs128 | RcpSs128 | RsqrtSs128 | SqrtPd128 => {
                // unary: these belong to emit_sse_fp_128_op
                self.emit_sse_fp_128_op(dptr, op, args);
                true
            }
            CmpPs128 | CmpPd128 | ShufPs128 | ShufPd128 | UnpcklPs128 | UnpckhPs128
            | UnpcklPd128 | UnpckhPd128 | HaddPs128 | HsubPs128 | AddsubPs128 | HaddPd128
            | HsubPd128 | AddsubPd128 | Movddup128 | Movsldup128 | Movshdup128 | RoundPs128
            | RoundPd128 | BlendPs128 | BlendPd128 | BlendvPs128 | BlendvPd128 | DpPs128
            | DpPd128 | InsertPs128 | InsertPd128 | VpermilPs128 | Movss128 | Movsd128
            | CvtSi2Ss128 | CvtSi2Sd128 | CvtSi2Ss64_128 | CvtSi2Sd64_128 | CvtSs2Sd128
            | CvtSd2Ss128 | CvtPs2Ep32_128 | CvtEp32ToPs128 | CvttPs2Ep32_128 | CvtPs2Pd128
            | CvtPd2Ps128 | CvtPd2Ep32_128 | CvtEp32ToPd128 | CvttPd2Ep32_128 | FmaPs132
            | FmaPs213 | FmaPs231 | FmaPd132 | FmaPd213 | FmaPd231 => {
                self.emit_sse_fp_128_op(dptr, op, args);
                true
            }
            DivPs256 | MinPs256 | MaxPs256 | SqrtPs256 | DivPd256 | MinPd256 | MaxPd256
            | SqrtPd256 | CmpPs256 | CmpPd256 | ShufPs256 | ShufPd256 | UnpcklPs256
            | UnpckhPs256 | UnpcklPd256 | UnpckhPd256 | HaddPs256 | HsubPs256 | AddsubPs256
            | RoundPs256 | RoundPd256 | BlendPs256 | BlendPd256 | BlendvPs256 | BlendvPd256
            | VpermilPs256 | VpermilPd256 | Vperm2f128 | Vinsertf128 | Vextractf128
            | Vbroadcastss | Vbroadcastsd | CvtPs2Ep32_256 | CvtEp32ToPs256 | CvttPs2Ep32_256
            | CvtPs2Pd256 | CvtPd2Ps256 | CvtPd2Ep32_256 | CvtEp32ToPd256 | CvttPd2Ep32_256
            | VpermilvarPs256 | VpermilvarPd256 | FmaPs132v256 | FmaPs213v256 | FmaPs231v256
            | FmaPd132v256 | FmaPd213v256 | FmaPd231v256 => {
                self.emit_avx_fp_256_op(dptr, op, args);
                true
            }
            _ => false,
        }
    }
}

impl X86Codegen {
    /// `inst $imm, src(zmm), dst` where dst is a 128/256-bit register
    /// (vextracti32x4 etc.). Result width = class of the op.
    fn emit_evex_extract_512(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        dst_reg: &'static str,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[1]);
        self.evex_load_arg(&args[0]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %zmm0, %{}", inst, imm, dst_reg));
        self.state.vec_claim_live_reg(dest_ptr.0, dst_reg);
        let m = reg_width_move(dst_reg);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state
                .emit_fmt(format_args!("    {} %{}, {}", m, dst_reg, mem));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", m, dst_reg));
        }
    }

    /// Broadcast 128/256-bit memory into a 512-bit register.
    fn emit_evex_mem_broadcast_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        // args[0] is a pointer to 16/32 bytes.
        self.operand_to_reg(&args[0], "rax");
        self.state
            .emit_fmt(format_args!("    {} (%rax), %zmm0", inst));
        self.evex_store_dest(dest_ptr);
    }

    /// Broadcast a GPR value into a 512-bit register (vpbroadcastb/w/d/q).
    fn emit_evex_gpr_broadcast_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.operand_to_reg(&args[0], "rax");
        let reg = if matches!(inst, "vpbroadcastq") {
            "rax"
        } else {
            "eax"
        };
        self.state
            .emit_fmt(format_args!("    {} %{}, %zmm0", inst, reg));
        self.evex_store_dest(dest_ptr);
    }

    /// VEX.128 writes zero bits 128..MAXVL, including the upper ZMM lanes.
    /// This is both a zero-extension and a legal implementation of the cast
    /// whose upper lanes are unspecified. Never read 64 bytes from a 16-byte
    /// source merely because the destination is 512 bits wide.
    fn emit_zext128_512(&mut self, dest_ptr: &Value, args: &[Operand]) {
        self.emit_cast_to512(dest_ptr, args, "xmm0");
    }

    /// 512 -> 256 cast (keep low 256 bits).
    fn emit_cast512_256(&mut self, dest_ptr: &Value, args: &[Operand]) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.evex_load_arg(&args[0]);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state
                .emit_fmt(format_args!("    vmovdqu %ymm0, {}", mem));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state.emit("    vmovdqu %ymm0, (%rax)");
        }
        self.state.vec_claim_live_reg(dest_ptr.0, "ymm0");
    }

    /// Narrow -> 512 cast; zeroing the undefined upper lanes is legal.
    fn emit_cast_to512(&mut self, dest_ptr: &Value, args: &[Operand], reg: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if let Some(m) = self.evex_arg_mem(&args[0]) {
            self.state
                .emit_fmt(format_args!("    vmovdqu {}, %{}", m, reg));
        } else {
            self.operand_to_reg(&args[0], "rax");
            self.state
                .emit_fmt(format_args!("    vmovdqu (%rax), %{}", reg));
        }
        self.evex_store_dest(dest_ptr);
    }

    /// EVEX widening reads 32 bytes and writes 64; narrowing does the reverse.
    /// Fold the exact-width memory tuple, avoiding a separate source load.
    fn emit_evex_convert_512(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        src_reg: &'static str,
        dst_reg: &'static str,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if let Some(mem) = self.evex_arg_mem(&args[0]) {
            self.state
                .emit_fmt(format_args!("    {} {}, %{}", inst, mem, dst_reg));
        } else {
            self.simd_load_arg_to(&args[0], src_reg);
            self.state
                .emit_fmt(format_args!("    {} %{}, %{}", inst, src_reg, dst_reg));
        }
        if dst_reg == "ymm0" {
            self.avx_store_dest(dest_ptr);
        } else {
            self.evex_store_dest(dest_ptr);
        }
    }

    /// Unaligned 512-bit load: vmovdqu64 (ptr), %zmm0.
    fn emit_evex_load_512(&mut self, dest_ptr: &Value, args: &[Operand]) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.operand_to_reg(&args[0], "rax");
        self.state.emit("    vmovdqu64 (%rax), %zmm0");
        self.evex_store_dest(dest_ptr);
    }

    /// Unaligned 512-bit store: vmovdqu64 %zmm0, (ptr).
    fn emit_evex_store_512(&mut self, dest_ptr: &Option<Value>, args: &[Operand]) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let Some(ptr) = dest_ptr else { return };
        self.evex_load_arg(&args[0]);
        self.value_to_reg(ptr, "rax");
        self.state.emit("    vmovdqu64 %zmm0, (%rax)");
    }

    /// Move a GPR mask into k1 (scratch; never survives the intrinsic).
    fn simd_gpr_to_k1(&mut self, mask_arg: &Operand) {
        self.operand_to_reg(mask_arg, "rax");
        self.state.emit("    kmovq %rax, %k1");
    }

    /// Masked zeroing load: vmovdqu8 (ptr), %zmm0{%k1}{z}.
    fn emit_maskz_load(&mut self, dest_ptr: &Value, args: &[Operand], reg: &'static str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.simd_gpr_to_k1(&args[0]);
        self.operand_to_reg(&args[1], "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu8 (%rax), %{}{{%k1}}{{z}}", reg));
        self.state.vec_claim_live_reg(dest_ptr.0, reg);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state.emit_fmt(format_args!(
                "    {} %{}, {}",
                reg_width_move(reg),
                reg,
                mem
            ));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", reg_width_move(reg), reg));
        }
    }

    /// Masked merge load: vmovdqu8 (ptr), %reg{%k1}.
    fn emit_mask_load(&mut self, dest_ptr: &Value, args: &[Operand], reg: &'static str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        // args: (mask, ptr, old) — old provides the merge base.
        self.simd_load_arg_to(&args[2], reg);
        self.simd_gpr_to_k1(&args[0]);
        self.operand_to_reg(&args[1], "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu8 (%rax), %{}{{%k1}}", reg));
        self.state.vec_claim_live_reg(dest_ptr.0, reg);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state.emit_fmt(format_args!(
                "    {} %{}, {}",
                reg_width_move(reg),
                reg,
                mem
            ));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state
                .emit_fmt(format_args!("    {} %{}, (%rax)", reg_width_move(reg), reg));
        }
    }

    /// Masked store: vmovdqu8 %reg, (ptr){%k1}.
    fn emit_mask_store(&mut self, dest_ptr: &Option<Value>, args: &[Operand], reg: &'static str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let Some(ptr) = dest_ptr else { return };
        // args: (mask, val) — the destination pointer was moved to dest_ptr.
        self.simd_gpr_to_k1(&args[0]);
        self.simd_load_arg_to(&args[1], reg);
        self.value_to_reg(ptr, "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu8 %{}, (%rax){{%k1}}", reg));
    }

    /// Masked zeroing maddubs: vpmaddubsw %zmm_b, %zmm_a, %zmm0{%k1}{z}.
    fn emit_maskz_maddubs_512(&mut self, dest_ptr: &Value, args: &[Operand]) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.simd_gpr_to_k1(&args[0]);
        self.evex_load_arg_to(&args[1], "zmm1");
        self.evex_load_arg_to(&args[2], "zmm2");
        self.state
            .emit("    vpmaddubsw %zmm2, %zmm1, %zmm0{%k1}{z}");
        self.evex_store_dest(dest_ptr);
    }

    /// Masked zeroing broadcast: vpbroadcast* %gpr, %zmm0{%k1}{z}.
    fn emit_maskz_set1_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.simd_gpr_to_k1(&args[0]);
        self.operand_to_reg(&args[1], "rax");
        let reg = if matches!(inst, "vpbroadcastq") {
            "rax"
        } else {
            "eax"
        };
        self.state
            .emit_fmt(format_args!("    {} %{}, %zmm0{{%k1}}{{z}}", inst, reg));
        self.evex_store_dest(dest_ptr);
    }

    /// Masked zeroing insert: vinserti* $imm, %xmm_src, %zmm_dst, %zmm0{%k1}{z}.
    fn emit_maskz_insert_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[3]);
        self.simd_gpr_to_k1(&args[0]);
        self.evex_load_arg_to(&args[1], "zmm1"); // dst base (a)
        let src_reg = if matches!(inst, "vinserti32x8" | "vinserti64x4") {
            "ymm2"
        } else {
            "xmm2"
        };
        self.simd_load_arg_to(&args[2], src_reg); // inserted src (b)
        self.state.emit_fmt(format_args!(
            "    {} ${}, %{}, %zmm1, %zmm0{{%k1}}{{z}}",
            inst, imm, src_reg
        ));
        self.evex_store_dest(dest_ptr);
    }

    /// Masked zeroing extract: vextracti* $imm, %zmm_src, %xmm0{%k1}{z}.
    fn emit_maskz_extract(
        &mut self,
        dest_ptr: &Value,
        args: &[Operand],
        inst: &str,
        dst_reg: &'static str,
    ) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[2]);
        self.simd_gpr_to_k1(&args[0]);
        self.simd_load_arg_to(&args[1], "zmm0");
        self.state.emit_fmt(format_args!(
            "    {} ${}, %zmm0, %{}{{%k1}}{{z}}",
            inst, imm, dst_reg
        ));
        self.state.vec_claim_live_reg(dest_ptr.0, dst_reg);
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state.emit_fmt(format_args!(
                "    {} %{}, {}",
                reg_width_move(dst_reg),
                dst_reg,
                mem
            ));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state.emit_fmt(format_args!(
                "    {} %{}, (%rax)",
                reg_width_move(dst_reg),
                dst_reg
            ));
        }
    }

    /// Masked (zeroing or merging) 128-bit pshufb: vpshufb %xmm_c, %xmm_b, %xmm_a{%k1}[{z}].
    /// _mm_mask_shuffle_epi8(src, k, a, b) / _mm_maskz_shuffle_epi8(k, a, b).
    fn emit_maskz_shuffle_epi8_128(&mut self, dest_ptr: &Value, args: &[Operand], zeroing: bool) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let mask = &args[0];
        let zsuf = if zeroing { "{z}" } else { "" };
        if zeroing {
            self.simd_gpr_to_k1(mask);
            self.evex_load_arg_to(&args[1], "xmm1");
            self.evex_load_arg_to(&args[2], "xmm2");
            self.state.emit_fmt(format_args!(
                "    vpshufb %xmm2, %xmm1, %xmm0{{%k1}}{}",
                zsuf
            ));
        } else {
            // merge: dest = src (first arg) merged with shuffle result
            self.evex_load_arg_to(&args[1], "xmm0"); // src (dest)
            self.simd_gpr_to_k1(mask);
            self.evex_load_arg_to(&args[2], "xmm1"); // a
            self.evex_load_arg_to(&args[3], "xmm2"); // b
            self.state
                .emit_fmt(format_args!("    vpshufb %xmm2, %xmm1, %xmm0{{%k1}}"));
        }
        self.state.vec_claim_live_reg(dest_ptr.0, "xmm0");
        if let Some(mem) = self.value_ptr_mem_operand(dest_ptr.0) {
            self.state
                .emit_fmt(format_args!("    vmovdqu %xmm0, {}", mem));
        } else {
            self.value_to_reg(dest_ptr, "rax");
            self.state.emit("    vmovdqu %xmm0, (%rax)");
        }
    }

    /// 512-bit FP compare to vector: vcmpps $imm, %zmm_b, %zmm_a, %zmm0.
    fn emit_evex_cmp_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let imm = self.simd_imm(&args[2]);
        self.evex_load_arg(&args[0]);
        self.evex_load_arg_to(&args[1], "zmm1");
        self.state
            .emit_fmt(format_args!("    {} ${}, %zmm1, %zmm0, %zmm0", inst, imm));
        self.evex_store_dest(dest_ptr);
    }

    /// 512-bit VNNI dot-product: args (a, b, c) -> `vpdpbusd %C, %B, %A`
    /// (dst = a + b*c; AT&T: r/m = c, vvvv = b, dest = a — verified against
    /// GCC's _mm512_dpbusd_epi32(a, b, c) codegen).
    fn emit_evex_vpdpbusd_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.evex_load_arg_to(&args[2], "zmm1"); // c (r/m)
        self.evex_load_arg_to(&args[1], "zmm2"); // b (vvvv)
        self.evex_load_arg_to(&args[0], "zmm0"); // a (dest)
        self.state
            .emit_fmt(format_args!("    {} %zmm1, %zmm2, %zmm0", inst));
        self.evex_store_dest(dest_ptr);
    }

    /// 512-bit FMA: args (a, b, c) -> vfmadd132ps %zmm_c, %zmm_b, %zmm_a.
    fn emit_evex_fma_512(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.evex_load_arg_to(&args[0], "zmm0"); // a (dest)
        self.evex_load_arg_to(&args[1], "zmm1"); // b (r/m)
        self.evex_load_arg_to(&args[2], "zmm2"); // c (vvvv)
        // vfmadd132ps %B, %C, %A: dst = dst*r/m + vvvv = a*b + c
        self.state
            .emit_fmt(format_args!("    {} %zmm1, %zmm2, %zmm0", inst));
        self.evex_store_dest(dest_ptr);
    }

    /// Scalar-result SIMD ops (mask compares, movemask, extracts to GPR,
    /// horizontal reduce). Result lands in rax and is stored to `dest`.
    fn emit_simd_scalar(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        args: &[Operand],
    ) -> bool {
        use IntrinsicOp::*;
        // RED-TEAM, two measured rounds (pinned so neither is retried blindly):
        // 1. Skipping `invalidate_vec_peephole()` for `vpcmp*` alone was a
        //    MISCOMPILE (`rt_eq_w512_epi8` returned all-ones): `evex_load_arg_to`
        //    did not evict a stale `vec_live_regs` claim when it clobbered
        //    `%zmm0`, so operand B was copied from the clobbered register.
        // 2. After fixing that eviction (below), skipping the invalidation is
        //    CORRECT (485-case three-way oracle green) but yields NO win:
        //    7579 -> 7579 instructions. The roundtrip is a property of the
        //    vector write-through HOME representation, not of this peephole.
        // A risk with zero measured benefit is not worth taking: keep the
        // unconditional invalidation, keep the eviction fix.
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        let (inst, w, opcode_ok) = match op {
            CmpeqEpu8Mask128 | CmpeqEpu8Mask256 | CmpeqEpu8Mask512 => ("vpcmpub", 0, true),
            CmpEpi8Mask128 | CmpEpi8Mask256 | CmpEpi8Mask512 => ("vpcmpb", 0, true),
            CmpeqEpu16Mask512 | CmpEpu16Mask128 | CmpEpu16Mask256 => ("vpcmpuw", 1, true),
            CmpEpi16Mask512 | CmpEpi16Mask128 | CmpEpi16Mask256 => ("vpcmpw", 1, true),
            CmpeqEpu32Mask512 | CmpEpu32Mask128 | CmpEpu32Mask256 => ("vpcmpud", 0, true),
            CmpEpi32Mask512 | CmpEpi32Mask128 | CmpEpi32Mask256 => ("vpcmpd", 0, true),
            CmpeqEpu64Mask512 | CmpEpu64Mask128 | CmpEpu64Mask256 => ("vpcmpuq", 1, true),
            CmpEpi64Mask512 | CmpEpi64Mask128 | CmpEpi64Mask256 => ("vpcmpq", 1, true),
            MovemaskPs128 => ("movmskps", 0, true),
            MovemaskPd128 => ("movmskpd", 0, true),
            MovemaskPs256 => ("vmovmskps", 0, true),
            MovemaskPd256 => ("vmovmskpd", 0, true),
            ExtractPs128 => ("extractps", 0, true),
            CvtSs2Si128 => ("cvtss2si", 0, true),
            CvtSd2Si128 => ("cvtsd2si", 0, true),
            CvttSs2Si128 => ("cvttss2si", 0, true),
            CvttSd2Si128 => ("cvttsd2si", 0, true),
            TestzPs128 => ("vtestps", 0, true),
            TestzPs256 => ("vtestps", 0, true),
            ReduceAddEpu32_512 => ("__reduce_add_epu32", 0, false),
            _ => return false,
        };
        let _ = (w, opcode_ok);
        match op {
            CmpeqEpu8Mask128 | CmpeqEpu8Mask256 | CmpeqEpu8Mask512 | CmpEpi8Mask128
            | CmpEpi8Mask256 | CmpEpi8Mask512 | CmpeqEpu16Mask512 | CmpEpu16Mask128
            | CmpEpu16Mask256 | CmpEpi16Mask512 | CmpEpi16Mask128 | CmpEpi16Mask256
            | CmpeqEpu32Mask512 | CmpEpu32Mask128 | CmpEpu32Mask256 | CmpEpi32Mask512
            | CmpEpi32Mask128 | CmpEpi32Mask256 | CmpeqEpu64Mask512 | CmpEpu64Mask128
            | CmpEpu64Mask256 | CmpEpi64Mask512 | CmpEpi64Mask128 | CmpEpi64Mask256 => {
                // args: (a, b, imm) — vpcmp* $imm, src2, src1, %k1; narrowest kmov*
                //
                // RED-TEAM, two rounds with OPPOSITE measured verdicts:
                // * Round 10 folded B via the STACK-SLOT `evex_arg_mem` gate.
                //   Correct but NET-NEGATIVE (7579 -> 7963): that gate flushes
                //   deferred stores the register path elides, and over-aligned
                //   512-bit homes did not resolve at all.  REJECTED.
                // * Round 11 removed the HOME from the IR instead (copy-chain
                //   collapse + the audited `fuse_vector_loads` alias proof
                //   forward the load's original `GlobalAddr` into B).  There
                //   is now no home and no deferred store to flush, so the
                //   dedicated `mask_cmp_global_mem` gate below IS profitable:
                //   it deletes `leaq B(%rip),%rax` + `vmovdqu (%rax),%reg_b`
                //   from EVERY runtime mask compare.  The rejected slot gate
                //   stays untouched for all other intrinsic families.
                let (reg_a, reg_b) = match op {
                    CmpeqEpu8Mask512 | CmpEpi8Mask512 | CmpeqEpu16Mask512 | CmpEpi16Mask512
                    | CmpeqEpu32Mask512 | CmpEpi32Mask512 | CmpeqEpu64Mask512 | CmpEpi64Mask512 => {
                        ("zmm0", "zmm1")
                    }
                    CmpeqEpu8Mask256 | CmpEpi8Mask256 | CmpEpu16Mask256 | CmpEpi16Mask256
                    | CmpEpu32Mask256 | CmpEpi32Mask256 | CmpEpu64Mask256 | CmpEpi64Mask256 => {
                        ("ymm0", "ymm1")
                    }
                    _ => ("xmm0", "xmm1"),
                };
                if !self.emit_mask_cmp_global_load(&args[0], reg_a) {
                    self.simd_load_arg_to(&args[0], reg_a);
                }
                let imm = if args.len() > 2 {
                    self.simd_imm(&args[2])
                } else {
                    0
                };
                if let Some(mem) = self.mask_cmp_global_mem(&args[1]) {
                    self.state.emit_fmt(format_args!(
                        "    {} ${}, {}, %{}, %k1",
                        inst, imm, mem, reg_a
                    ));
                } else {
                    self.simd_load_arg_to(&args[1], reg_b);
                    self.state.emit_fmt(format_args!(
                        "    {} ${}, %{}, %{}, %k1",
                        inst, imm, reg_b, reg_a
                    ));
                }
                let (kmov, kmov_dst) =
                    Self::mask_compare_kmov(inst, op.mask_compare_vector_width().unwrap_or(64));
                self.state
                    .emit_fmt(format_args!("    {} %k1, %{}", kmov, kmov_dst));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
                true
            }
            MovemaskPs128 | MovemaskPd128 | MovemaskPs256 | MovemaskPd256 => {
                let reg = if matches!(op, MovemaskPs256 | MovemaskPd256) {
                    "ymm0"
                } else {
                    "xmm0"
                };
                self.simd_load_arg_to(&args[0], reg);
                self.state
                    .emit_fmt(format_args!("    {} %{}, %eax", inst, reg));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
                true
            }
            ExtractPs128 => {
                // args: (a, imm) — extractps $imm, %xmm_a, %eax
                let imm = self.simd_imm(&args[1]);
                self.sse_load_arg(&args[0], "xmm0");
                self.state
                    .emit_fmt(format_args!("    extractps ${}, %xmm0, %eax", imm));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
                true
            }
            CvtSs2Si128 | CvtSd2Si128 | CvttSs2Si128 | CvttSd2Si128 => {
                self.sse_load_arg(&args[0], "xmm0");
                self.state
                    .emit_fmt(format_args!("    {} %xmm0, %eax", inst));
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
                true
            }
            TestzPs128 | TestzPs256 => {
                let (reg0, reg1) = if matches!(op, TestzPs256) {
                    ("ymm0", "ymm1")
                } else {
                    ("xmm0", "xmm1")
                };
                self.simd_load_arg_to(&args[0], reg0);
                self.simd_load_arg_to(&args[1], reg1);
                self.state
                    .emit_fmt(format_args!("    vtestps %{}, %{}", reg1, reg0));
                self.state.emit("    sete %al");
                self.state.emit("    movzbl %al, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
                true
            }
            ReduceAddEpu32_512 => {
                // args: (a) — horizontal sum of 16x u32 (adler32 hot path).
                // 512 -> 256 -> 128 -> 64 -> 32 reduction; every step is
                // cross-lane-safe (vextract before the narrow adds, since
                // EVEX xmm/ymm destinations zero the upper bits).
                self.evex_load_arg(&args[0]);
                self.state.emit("    vextracti64x4 $1, %zmm0, %ymm1");
                self.state.emit("    vpaddd %ymm1, %ymm0, %ymm0");
                self.state.emit("    vextracti128 $1, %ymm0, %xmm1");
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vpshufd $0xEE, %xmm0, %xmm1");
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vpshufd $0x55, %xmm0, %xmm1");
                self.state.emit("    vpaddd %xmm1, %xmm0, %xmm0");
                self.state.emit("    vmovd %xmm0, %eax");
                if let Some(d) = dest {
                    self.store_rax_to(d);
                }
                true
            }
            _ => false,
        }
    }
}

/// Move mnemonic for a register name: xmm/ymm -> "vmovdqu", zmm -> "vmovdqu64".
fn reg_width_move(reg: &str) -> &'static str {
    if reg.starts_with("zmm") {
        "vmovdqu64"
    } else {
        "vmovdqu"
    }
}

/// Value id of an operand, if it is a Value.
fn simd_value_id(op: &Operand) -> u32 {
    match op {
        Operand::Value(v) => v.0,
        _ => u32::MAX,
    }
}

impl X86Codegen {
    /// 128-bit SSE FP binary (2-op AT&T): `inst %xmm1, %xmm0`.
    fn emit_sse_fp_128(&mut self, dest_ptr: &Value, args: &[Operand], inst: &str, _is_ps: bool) {
        self.emit_sse_binary_128(dest_ptr, args, inst);
    }

    /// 128-bit SSE FP op dispatch (all shapes).
    fn emit_sse_fp_128_op(&mut self, dest_ptr: &Value, op: &IntrinsicOp, args: &[Operand]) {
        use IntrinsicOp::*;
        // unary 2-op (legacy SSE: no memory operand — alignment)
        let unary = |this: &mut Self, inst: &str| {
            this.sse_load_arg(&args[0], "xmm0");
            this.state
                .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
            this.sse_store_dest(dest_ptr, "xmm0");
        };
        match op {
            SqrtPs128 => unary(self, "sqrtps"),
            RcpPs128 => unary(self, "rcpps"),
            RsqrtPs128 => unary(self, "rsqrtps"),
            SqrtSs128 => unary(self, "sqrtss"),
            RcpSs128 => unary(self, "rcpss"),
            RsqrtSs128 => unary(self, "rsqrtss"),
            SqrtPd128 => unary(self, "sqrtpd"),
            Movddup128 => unary(self, "movddup"),
            Movsldup128 => unary(self, "movsldup"),
            Movshdup128 => unary(self, "movshdup"),
            CvtPs2Ep32_128 => unary(self, "cvtps2dq"),
            CvtEp32ToPs128 => unary(self, "cvtdq2ps"),
            CvttPs2Ep32_128 => unary(self, "cvttps2dq"),
            CvtPs2Pd128 => unary(self, "cvtps2pd"),
            CvtPd2Ps128 => unary(self, "cvtpd2ps"),
            CvtPd2Ep32_128 => unary(self, "cvtpd2dq"),
            CvtEp32ToPd128 => unary(self, "cvtdq2pd"),
            CvttPd2Ep32_128 => unary(self, "cvttpd2dq"),
            HaddPs128 | HsubPs128 | AddsubPs128 | HaddPd128 | HsubPd128 | AddsubPd128
            | UnpcklPs128 | UnpckhPs128 | UnpcklPd128 | UnpckhPd128 => {
                let inst = match op {
                    HaddPs128 => "haddps",
                    HsubPs128 => "hsubps",
                    AddsubPs128 => "addsubps",
                    HaddPd128 => "haddpd",
                    HsubPd128 => "hsubpd",
                    AddsubPd128 => "addsubpd",
                    UnpcklPs128 => "unpcklps",
                    UnpckhPs128 => "unpckhps",
                    UnpcklPd128 => "unpcklpd",
                    UnpckhPd128 => "unpckhpd",
                    _ => unreachable!(),
                };
                self.emit_sse_binary_128(dest_ptr, args, inst);
            }
            // 2-op with imm: cmpps/cmppd/shufps/shufpd (a, b, imm)
            CmpPs128 | CmpPd128 | ShufPs128 | ShufPd128 => {
                let inst = match op {
                    CmpPs128 => "cmpps",
                    CmpPd128 => "cmppd",
                    ShufPs128 => "shufps",
                    ShufPd128 => "shufpd",
                    _ => unreachable!(),
                };
                let imm = self.simd_imm(&args[2]);
                self.sse_load_arg(&args[1], "xmm1");
                self.sse_load_arg(&args[0], "xmm0");
                self.state
                    .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0", inst, imm));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            // 1-op with imm: roundps/roundpd/vpermilps (a, imm)
            RoundPs128 | RoundPd128 | VpermilPs128 => {
                let inst = match op {
                    RoundPs128 => "roundps",
                    RoundPd128 => "roundpd",
                    VpermilPs128 => "vpermilps",
                    _ => unreachable!(),
                };
                let imm = self.simd_imm(&args[1]);
                self.sse_load_arg(&args[0], "xmm0");
                self.state
                    .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", inst, imm));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            // blend with imm: blendps/blendpd/dpps/dppd (a, b, imm)
            BlendPs128 | BlendPd128 | DpPs128 | DpPd128 => {
                let inst = match op {
                    BlendPs128 => "blendps",
                    BlendPd128 => "blendpd",
                    DpPs128 => "dpps",
                    DpPd128 => "dppd",
                    _ => unreachable!(),
                };
                let imm = self.simd_imm(&args[2]);
                self.sse_load_arg(&args[1], "xmm1");
                self.sse_load_arg(&args[0], "xmm0");
                self.state
                    .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0", inst, imm));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            // blendv: operands (mask, a, b); mask implicit in xmm0 (legacy).
            BlendvPs128 | BlendvPd128 => {
                let inst = if matches!(op, BlendvPs128) {
                    "blendvps"
                } else {
                    "blendvpd"
                };
                self.sse_load_arg(&args[0], "xmm0"); // mask
                self.sse_load_arg(&args[1], "xmm1"); // a (dst)
                self.sse_load_arg(&args[2], "xmm2"); // b (src)
                self.state
                    .emit_fmt(format_args!("    {} %xmm2, %xmm1", inst));
                self.sse_store_dest(dest_ptr, "xmm1");
            }
            // insertps (a, b, imm): insertps $imm, %xmm_b, %xmm_a
            InsertPs128 => {
                let imm = self.simd_imm(&args[2]);
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
                self.state
                    .emit_fmt(format_args!("    insertps ${}, %xmm1, %xmm0", imm));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            // movss/movsd (a, b): movss %xmm_b, %xmm_a
            Movss128 | Movsd128 => {
                let inst = if matches!(op, Movss128) {
                    "movss"
                } else {
                    "movsd"
                };
                if !self.avx2_enabled {
                    self.emit_sse_binary_128(dest_ptr, args, inst);
                } else {
                    self.sse_load_arg(&args[1], "xmm1");
                    self.sse_load_arg(&args[0], "xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    self.sse_store_dest(dest_ptr, "xmm0");
                }
            }
            // cvtsi2ss/cvtsi2sd (a, i): convert i into xmm0 holding a
            CvtSi2Ss128 | CvtSi2Sd128 => {
                let inst = if matches!(op, CvtSi2Ss128) {
                    "cvtsi2ss"
                } else {
                    "cvtsi2sd"
                };
                self.sse_load_arg(&args[0], "xmm0");
                self.operand_to_reg(&args[1], "rax");
                self.state
                    .emit_fmt(format_args!("    {} %eax, %xmm0", inst));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            CvtSi2Ss64_128 | CvtSi2Sd64_128 => {
                let inst = if matches!(op, CvtSi2Ss64_128) {
                    "cvtsi2ss"
                } else {
                    "cvtsi2sd"
                };
                self.sse_load_arg(&args[0], "xmm0");
                self.operand_to_reg(&args[1], "rax");
                self.state
                    .emit_fmt(format_args!("    {} %rax, %xmm0", inst));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            // cvtss2sd/cvtsd2ss (a, b)
            CvtSs2Sd128 | CvtSd2Ss128 => {
                let inst = if matches!(op, CvtSs2Sd128) {
                    "cvtss2sd"
                } else {
                    "cvtsd2ss"
                };
                self.emit_sse_binary_128(dest_ptr, args, inst);
            }
            // FMA 128 (a, b, c): vfmadd132ps %xmm_c, %xmm_b, %xmm_a
            FmaPs132 | FmaPs213 | FmaPs231 | FmaPd132 | FmaPd213 | FmaPd231 => {
                let inst = match op {
                    FmaPs132 => "vfmadd132ps",
                    FmaPs213 => "vfmadd213ps",
                    FmaPs231 => "vfmadd231ps",
                    FmaPd132 => "vfmadd132pd",
                    FmaPd213 => "vfmadd213pd",
                    FmaPd231 => "vfmadd231pd",
                    _ => unreachable!(),
                };
                self.sse_load_arg(&args[0], "xmm0"); // a (dest)
                self.sse_load_arg(&args[1], "xmm1"); // b (r/m)
                self.sse_load_arg(&args[2], "xmm2"); // c (vvvv)
                // vfmadd132ps %B, %C, %A: dst = dst*r/m + vvvv = a*b + c
                self.state
                    .emit_fmt(format_args!("    {} %xmm1, %xmm2, %xmm0", inst));
                self.sse_store_dest(dest_ptr, "xmm0");
            }
            _ => {}
        }
    }

    /// 256-bit AVX FP op dispatch (all shapes).
    fn emit_avx_fp_256_op(&mut self, dest_ptr: &Value, op: &IntrinsicOp, args: &[Operand]) {
        use IntrinsicOp::*;
        // Preserve a deferred vector result — and the register peephole that
        // lets the operand load be elided — when THIS op is the pending
        // value's cache-aware consumer. That is exactly the contract
        // `compute_vector_defer_values` establishes for back-to-back op
        // chains: the clamp idiom `vminps; vmaxps` must keep its intermediate
        // in %ymm0, not round-trip it through a stack slot.
        //
        // Flushing and invalidating unconditionally here defeated that for
        // every AVX FP 256-bit op, so `avx_fp_private` spent a store plus a
        // reload on each of its eight clamp stages.
        //
        // Soundness: `avx_load_arg_to` clears `pending_vec_store` on every
        // register path and flushes it before every memory path, so keeping
        // the store pending can never expose an unwritten slot.
        if !self.pending_vec_store_consumed_by(op, args) {
            self.flush_pending_vec_store_impl();
            self.state.invalidate_vec_peephole();
        }
        // binary 3-op
        let binary = |this: &mut Self, inst: &str, commutative: bool| {
            this.emit_avx_binary_256(dest_ptr, args, inst, commutative);
        };
        // unary
        let unary = |this: &mut Self, inst: &str| {
            if let Some(m) = this.evex_arg_mem(&args[0]) {
                this.state
                    .emit_fmt(format_args!("    {} {}, %ymm0", inst, m));
            } else {
                this.avx_load_arg(&args[0]);
                this.state
                    .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
            }
            this.avx_store_dest(dest_ptr);
        };
        // Width-changing packed converts do NOT fit the ymm->ymm `unary`
        // helper: VCVTPS2PD/VCVTDQ2PD are 1:2 widening (VEX source is always
        // xmm: 4 floats -> 4 doubles), and VCVTPD2PS/VCVTPD2DQ/VCVTTPD2DQ are
        // 2:1 narrowing (VEX destination is always xmm: 4 doubles -> 4
        // floats/ints). Emitting the generic form produced
        // `vcvtps2pd %ymm0, %ymm0`, which the assembler rejects as an operand
        // size mismatch, so `_mm256_cvtps_pd` and its four siblings did not
        // compile at all.
        let widen = |this: &mut Self, inst: &str| {
            if let Some(m) = this.evex_arg_mem(&args[0]) {
                // A VEX widening convert's memory tuple is 16 bytes.
                this.state
                    .emit_fmt(format_args!("    {} {}, %ymm0", inst, m));
            } else {
                this.sse_load_arg(&args[0], "xmm0");
                this.state
                    .emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
            }
            this.avx_store_dest(dest_ptr);
        };
        // Both VEX lengths narrow to XMM, so memory operands require the
        // AT&T y suffix to select a 256-bit source rather than an ambiguous
        // unsuffixed 128/256-bit tuple. Register sources encode their width.
        let narrow = |this: &mut Self, inst: &str| {
            if let Some(m) = this.evex_arg_mem(&args[0]) {
                this.state
                    .emit_fmt(format_args!("    {}y {}, %xmm0", inst, m));
            } else {
                this.avx_load_arg(&args[0]);
                this.state
                    .emit_fmt(format_args!("    {} %ymm0, %xmm0", inst));
            }
            this.sse_store_dest(dest_ptr, "xmm0");
        };
        match op {
            DivPs256 => binary(self, "vdivps", false),
            // MIN/MAX select the second operand for NaNs and equal zeros.
            // Memory folding must not commute their operands.
            MinPs256 => binary(self, "vminps", false),
            MaxPs256 => binary(self, "vmaxps", false),
            DivPd256 => binary(self, "vdivpd", false),
            MinPd256 => binary(self, "vminpd", false),
            MaxPd256 => binary(self, "vmaxpd", false),
            SqrtPs256 => unary(self, "vsqrtps"),
            SqrtPd256 => unary(self, "vsqrtpd"),
            HaddPs256 => binary(self, "vhaddps", false),
            HsubPs256 => binary(self, "vhsubps", false),
            AddsubPs256 => binary(self, "vaddsubps", false),
            // The 256-bit unpack family previously had NO arm here: the ops
            // were routed to this dispatcher but silently emitted nothing,
            // leaving the result alloca uninitialized (garbage output for
            // _mm256_unpacklo/hi_ps/_pd).
            UnpcklPs256 => binary(self, "vunpcklps", false),
            UnpckhPs256 => binary(self, "vunpckhps", false),
            UnpcklPd256 => binary(self, "vunpcklpd", false),
            UnpckhPd256 => binary(self, "vunpckhpd", false),
            // Variable-index permute: vpermilps/vpermilpd %idx, %src, %dst.
            VpermilvarPs256 => binary(self, "vpermilps", false),
            VpermilvarPd256 => binary(self, "vpermilpd", false),
            CvtPs2Ep32_256 => unary(self, "vcvtps2dq"),
            CvtEp32ToPs256 => unary(self, "vcvtdq2ps"),
            CvttPs2Ep32_256 => unary(self, "vcvttps2dq"),
            CvtPs2Pd256 => widen(self, "vcvtps2pd"),
            CvtEp32ToPd256 => widen(self, "vcvtdq2pd"),
            CvtPd2Ps256 => narrow(self, "vcvtpd2ps"),
            CvtPd2Ep32_256 => narrow(self, "vcvtpd2dq"),
            CvttPd2Ep32_256 => narrow(self, "vcvttpd2dq"),
            // 3-op with imm: vcmpps/vcmppd/vshufps/vshufpd/vblendps/vblendpd (a, b, imm)
            CmpPs256 | CmpPd256 | ShufPs256 | ShufPd256 | BlendPs256 | BlendPd256 => {
                let inst = match op {
                    CmpPs256 => "vcmpps",
                    CmpPd256 => "vcmppd",
                    ShufPs256 => "vshufps",
                    ShufPd256 => "vshufpd",
                    BlendPs256 => "vblendps",
                    BlendPd256 => "vblendpd",
                    _ => unreachable!(),
                };
                let imm = self.simd_imm(&args[2]);
                self.avx_load_arg(&args[0]);
                self.avx_load_arg_to(&args[1], "ymm1");
                self.state
                    .emit_fmt(format_args!("    {} ${}, %ymm1, %ymm0, %ymm0", inst, imm));
                self.avx_store_dest(dest_ptr);
            }
            // 2-op with imm: vroundps/vroundpd/vpermilps (a, imm)
            RoundPs256 | RoundPd256 | VpermilPs256 | VpermilPd256 => {
                let inst = match op {
                    RoundPs256 => "vroundps",
                    RoundPd256 => "vroundpd",
                    VpermilPs256 => "vpermilps",
                    VpermilPd256 => "vpermilpd",
                    _ => unreachable!(),
                };
                let imm = self.simd_imm(&args[1]);
                if let Some(m) = self.evex_arg_mem(&args[0]) {
                    self.state
                        .emit_fmt(format_args!("    {} ${}, {}, %ymm0", inst, imm, m));
                } else {
                    self.avx_load_arg(&args[0]);
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                }
                self.avx_store_dest(dest_ptr);
            }
            // vperm2f128 (a, b, imm)
            Vperm2f128 => {
                let imm = self.simd_imm(&args[2]);
                self.avx_load_arg(&args[0]);
                self.avx_load_arg_to(&args[1], "ymm1");
                self.state
                    .emit_fmt(format_args!("    vperm2f128 ${}, %ymm1, %ymm0, %ymm0", imm));
                self.avx_store_dest(dest_ptr);
            }
            // vinsertf128 (a, b, imm) — b is 128-bit
            Vinsertf128 => {
                let imm = self.simd_imm(&args[2]);
                self.avx_load_arg(&args[0]);
                self.sse_load_arg(&args[1], "xmm1");
                self.state.emit_fmt(format_args!(
                    "    vinsertf128 ${}, %xmm1, %ymm0, %ymm0",
                    imm
                ));
                self.avx_store_dest(dest_ptr);
            }
            // vextractf128 (a, imm) -> xmm
            Vextractf128 => {
                let imm = self.simd_imm(&args[1]);
                self.avx_load_arg(&args[0]);
                self.state
                    .emit_fmt(format_args!("    vextractf128 ${}, %ymm0, %xmm0", imm));
                self.state.vec_claim_live_reg(dest_ptr.0, "xmm0");
                if let Some(m) = self.value_ptr_mem_operand(dest_ptr.0) {
                    self.state
                        .emit_fmt(format_args!("    vmovdqu %xmm0, {}", m));
                } else {
                    self.value_to_reg(dest_ptr, "rax");
                    self.state.emit("    vmovdqu %xmm0, (%rax)");
                }
            }
            // vbroadcastss/vbroadcastsd (a) — src xmm or mem
            Vbroadcastss | Vbroadcastsd => {
                let inst = if matches!(op, Vbroadcastss) {
                    "vbroadcastss"
                } else {
                    "vbroadcastsd"
                };
                // Width-filtered claim, exactly like `avx_load_arg_to`: this
                // emitter writes `%ymm0` with a VEX.256 broadcast, so the
                // source must be a `%ymm` register. `vec_live_regs` also
                // carries `%xmm` claims (128-bit producers) and `%zmm` claims
                // (EVEX producers), neither of which is a legal VEX.256
                // source -- a raw read would splice the wrong bank width into
                // the operand and the emitter does no width check.
                if let Some(held) = self.state.vec_live_ymm(simd_value_id(&args[0])) {
                    self.state
                        .emit_fmt(format_args!("    {} %{}, %ymm0", inst, held));
                } else if let Some(m) = self.evex_arg_mem(&args[0]) {
                    self.state
                        .emit_fmt(format_args!("    {} {}, %ymm0", inst, m));
                } else {
                    self.sse_load_arg(&args[0], "xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm1, %ymm0", inst));
                }
                self.avx_store_dest(dest_ptr);
            }
            // vblendvps/vblendvpd: operands (mask, a, b): mask, b(src2), a(src1,dst)
            BlendvPs256 | BlendvPd256 => {
                let inst = if matches!(op, BlendvPs256) {
                    "vblendvps"
                } else {
                    "vblendvpd"
                };
                // The result MUST land in %ymm0: avx_store_dest stores %ymm0,
                // and the old code computed the blend into %ymm1 then stored
                // %ymm0 (the mask) instead — blendv returned the mask operand
                // rather than the blended result. AT&T operand order:
                //   %mask (is4), %src2, %src1(dst), %dst.
                self.avx_load_arg_to(&args[0], "ymm2"); // mask
                self.avx_load_arg_to(&args[1], "ymm0"); // a (src1 = dst)
                self.avx_load_arg_to(&args[2], "ymm1"); // b (src2)
                self.state
                    .emit_fmt(format_args!("    {} %ymm2, %ymm1, %ymm0, %ymm0", inst));
                self.avx_store_dest(dest_ptr);
            }
            // FMA 256 (a, b, c)
            FmaPs132v256 | FmaPs213v256 | FmaPs231v256 | FmaPd132v256 | FmaPd213v256
            | FmaPd231v256 => {
                let inst = match op {
                    FmaPs132v256 => "vfmadd132ps",
                    FmaPs213v256 => "vfmadd213ps",
                    FmaPs231v256 => "vfmadd231ps",
                    FmaPd132v256 => "vfmadd132pd",
                    FmaPd213v256 => "vfmadd213pd",
                    FmaPd231v256 => "vfmadd231pd",
                    _ => unreachable!(),
                };
                self.avx_load_arg_to(&args[0], "ymm0"); // a (dest)
                self.avx_load_arg_to(&args[1], "ymm1"); // b (r/m)
                self.avx_load_arg_to(&args[2], "ymm2"); // c (vvvv)
                self.state
                    .emit_fmt(format_args!("    {} %ymm1, %ymm2, %ymm0", inst));
                self.avx_store_dest(dest_ptr);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod avx_fp_order_tests {
    use super::*;
    use crate::backend::regalloc::PhysReg;
    use crate::backend::state::PendingVecMemfold;
    use crate::ir::reexports::IrConst;

    #[test]
    fn avx_minmax_never_commute_left_memory_but_keep_right_memory_folding() {
        for (op, mnemonic) in [
            (IntrinsicOp::MinPs256, "vminps"),
            (IntrinsicOp::MaxPs256, "vmaxps"),
            (IntrinsicOp::MinPd256, "vminpd"),
            (IntrinsicOp::MaxPd256, "vmaxpd"),
        ] {
            for folded in [0, 1] {
                let mut cg = X86Codegen::new();
                cg.reg_assignments.insert(0, PhysReg(24)); // ymm6
                cg.reg_assignments.insert(1, PhysReg(23)); // ymm5
                cg.reg_assignments.insert(2, PhysReg(25)); // ymm7
                cg.state.vector_values.extend([0, 1, 2]);
                cg.state.pending_vec_memfold = Some(PendingVecMemfold {
                    val: folded,
                    mem: "(%rdi)".to_string(),
                    mnemonic: "vmovdqu",
                    width: 32,
                });
                cg.emit_avx_fp_256_op(
                    &Value(2),
                    &op,
                    &[Operand::Value(Value(0)), Operand::Value(Value(1))],
                );
                let out = &cg.state.out.buf;
                assert!(out.contains(mnemonic), "{op:?}: {out}");
                if folded == 0 {
                    assert!(
                        !out.contains(&format!("{mnemonic} (%rdi), %ymm5")),
                        "NaN/signed-zero operand order was reversed: {out}"
                    );
                    assert!(out.contains("vmovdqu (%rdi)"), "{out}");
                } else {
                    assert!(
                        out.contains(&format!("{mnemonic} (%rdi), %ymm6")),
                        "safe right-operand memory fold was lost: {out}"
                    );
                }
                assert!(cg.state.pending_vec_memfold.is_none(), "{out}");
            }
        }
    }

    /// Bare codegen with values 0/1/2 homed in YMM registers, matching the
    /// shape `emit_avx_fp_256_op` sees from the real pipeline.
    fn codegen_with_homes() -> X86Codegen {
        let mut cg = X86Codegen::new();
        cg.reg_assignments.insert(0, PhysReg(24)); // ymm6
        cg.reg_assignments.insert(1, PhysReg(23)); // ymm5
        cg.reg_assignments.insert(2, PhysReg(25)); // ymm7
        cg.state.vector_values.extend([0, 1, 2]);
        cg
    }

    /// A back-to-back AVX FP 256-bit op chain must keep its intermediate in
    /// the scratch register rather than round-tripping it through the slot.
    ///
    /// `emit_avx_fp_256_op` used to flush the pending store and invalidate the
    /// register peephole unconditionally on entry, so the clamp idiom
    /// `vminps; vmaxps` paid a store plus a reload on every stage. Eight such
    /// stages made `avx_fp_private` 16.9x slower than the fastest reference.
    #[test]
    fn avx_fp_256_op_keeps_a_deferred_result_for_its_consumer() {
        let mut cg = codegen_with_homes();
        // Value 0 is the intermediate: its slot store is still pending and
        // %ymm0 still holds it.
        cg.state.pending_vec_store = Some((0, "ymm0", true));
        cg.state.vec_last_store_val = Some(0);
        cg.state.vec_last_store_reg = true;
        cg.state.vec_last_store_reg_name = Some("ymm0");
        cg.emit_avx_fp_256_op(
            &Value(2),
            &IntrinsicOp::MaxPs256,
            &[Operand::Value(Value(0)), Operand::Value(Value(1))],
        );
        let out = &cg.state.out.buf;
        assert!(
            out.contains("vmaxps"),
            "the op must still be emitted: {out}"
        );
        assert!(
            !out.contains("vmovdqu %ymm0, (%rax)"),
            "the pending store was flushed even though this op consumes the \
             value; the intermediate must stay in %ymm0: {out}"
        );
        assert!(
            !out.contains("(%rax), %ymm0"),
            "the operand was reloaded from the never-written slot: {out}"
        );
    }

    /// The mirror image: when the op does NOT read the pending value, the
    /// store MUST still be flushed. Keeping it would leave the slot unwritten
    /// for whatever reads it next.
    #[test]
    fn avx_fp_256_op_flushes_a_deferred_result_it_does_not_consume() {
        let mut cg = codegen_with_homes();
        cg.state.pending_vec_store = Some((0, "ymm0", true));
        cg.state.vec_last_store_val = Some(0);
        cg.state.vec_last_store_reg = true;
        cg.state.vec_last_store_reg_name = Some("ymm0");
        // Neither operand is the pending value 0.
        cg.emit_avx_fp_256_op(
            &Value(2),
            &IntrinsicOp::MaxPs256,
            &[Operand::Value(Value(1)), Operand::Value(Value(1))],
        );
        assert!(
            cg.state.pending_vec_store.is_none(),
            "an unrelated op must flush the pending store"
        );
        assert!(
            cg.state.out.buf.contains("vmovdqu %ymm0, (%rax)"),
            "the deferred value must be committed to its slot: {}",
            cg.state.out.buf
        );
    }

    /// `pending_vec_store_consumed_by` is the single predicate shared by the
    /// generic lazy-flush service and the AVX FP entry check. Pin the classes
    /// that matter: a plain elementwise consumer, a non-consumer, a raw
    /// reader, and an op that writes through its args.
    #[test]
    fn pending_vec_store_consumed_by_classifies_consumers() {
        let mut cg = codegen_with_homes();
        cg.state.pending_vec_store = Some((0, "ymm0", true));

        // Elementwise op reading the value: kept.
        assert!(
            cg.pending_vec_store_consumed_by(
                &IntrinsicOp::MaxPs256,
                &[Operand::Value(Value(0)), Operand::Value(Value(1))]
            ),
            "MaxPs256 reads the value as a whole vector operand"
        );
        // A different value: not a consumer.
        assert!(
            !cg.pending_vec_store_consumed_by(
                &IntrinsicOp::MaxPs256,
                &[Operand::Value(Value(1)), Operand::Value(Value(2))]
            ),
            "the pending value is not among the operands"
        );
        // A raw reader reads the slot's bytes, so the store must be flushed.
        assert!(
            !cg.pending_vec_store_consumed_by(
                &IntrinsicOp::VecLoadF32x8,
                &[
                    Operand::Value(Value(0)),
                    Operand::Const(crate::ir::constants::IrConst::I64(0))
                ]
            ),
            "VecLoadF32x8 is a raw reader: the slot must be written first"
        );
        // An op that writes through args carries its destination there, so
        // nothing at this level can tell datum from destination. The whole
        // family is refused rather than guessed at. (The emitter for the
        // VecStore family IS pending-aware on all three of its paths, so this
        // is deliberate conservatism costing a redundant store in rare cases,
        // not a correctness requirement -- soundness must not depend on which
        // arg position happens to be the destination.)
        assert!(
            !cg.pending_vec_store_consumed_by(
                &IntrinsicOp::VecStoreF32x8,
                &[Operand::Value(Value(0)), Operand::Value(Value(1))]
            ),
            "ops writing through args are refused wholesale"
        );
        // No pending store at all: not a consumer.
        cg.state.pending_vec_store = None;
        assert!(
            !cg.pending_vec_store_consumed_by(&IntrinsicOp::MaxPs256, &[Operand::Value(Value(0))])
        );
    }

    /// A value whose slot store is still pending has NOT been written to its
    /// slot, so folding it as a memory operand requires committing that store
    /// first.
    ///
    /// Regression: keeping a pending store alive across `emit_avx_fp_256_op`
    /// (so `vminps; vmaxps` keeps its intermediate in %ymm0) made the `widen`
    /// and `unary` arms fold such a value through `evex_arg_mem`, which read
    /// the never-written slot. `_mm256_cvtps_pd` and `_mm256_cvtepi32_pd`
    /// returned zeros — caught by tests/intrinsics/cases/t256_fp.c via CI.
    #[test]
    fn evex_arg_mem_commits_a_pending_store_before_folding_it() {
        let mut cg = codegen_with_homes();
        cg.state.pending_vec_store = Some((0, "ymm0", true));
        let _ = cg.evex_arg_mem(&Operand::Value(Value(0)));
        assert!(
            cg.state.pending_vec_store.is_none(),
            "the pending store must be committed before the slot can be read"
        );
        assert!(
            cg.state.out.buf.contains("vmovdqu %ymm0"),
            "the committed value must actually be written: {}",
            cg.state.out.buf
        );
        // A DIFFERENT value must not be flushed: the guard is value-specific.
        let mut cg = codegen_with_homes();
        cg.state.pending_vec_store = Some((7, "ymm0", true));
        let _ = cg.evex_arg_mem(&Operand::Value(Value(0)));
        assert!(
            cg.state.pending_vec_store.is_some(),
            "an unrelated value must not trigger the flush"
        );
        assert!(
            !cg.state.out.buf.contains("vmovdqu %ymm0"),
            "an unrelated value must not be written: {}",
            cg.state.out.buf
        );
    }

    /// The end-to-end shape the guard protects: a width-changing convert whose
    /// operand is a deferred 256-bit result must not read the slot.
    #[test]
    fn widening_convert_of_a_deferred_operand_does_not_read_the_slot() {
        let mut cg = codegen_with_homes();
        cg.state.pending_vec_store = Some((0, "ymm0", true));
        cg.state.vec_last_store_val = Some(0);
        cg.state.vec_last_store_reg = true;
        cg.state.vec_last_store_reg_name = Some("ymm0");
        // CvtPs2Pd256 is the `widen` arm: it reads args[0]'s LOW 128 bits.
        cg.emit_avx_fp_256_op(
            &Value(2),
            &IntrinsicOp::CvtPs2Pd256,
            &[Operand::Value(Value(0))],
        );
        let out = &cg.state.out.buf;
        assert!(
            out.contains("vcvtps2pd"),
            "the convert must still be emitted: {out}"
        );
        // A folded memory operand would be `vcvtps2pd <mem>, %ymm0`; the only
        // sound form here is the register one, `%xmm0, %ymm0`.
        assert!(
            out.contains("vcvtps2pd %xmm0, %ymm0"),
            "the convert must read the deferred value from the register, \
             not from its never-written slot: {out}"
        );
    }
    /// A fresh load into a ZMM bank MUST evict every OTHER value's stale
    /// `vec_live_regs` claim.  Regression measured on the 485-case three-way
    /// matrix: `rt_eq_w512_epi8` returned all-ones because operand B's
    /// `%zmm0` claim survived the load of operand A into `%zmm0`, so the
    /// second resolver copied the clobbered register as operand B.
    /// `sse_load_arg`/`avx_load_arg_to` already did this; the EVEX loader did
    /// not.  Both load paths (direct slot and `(%rax)` fallback) are pinned.
    #[test]
    fn evex_load_arg_to_evicts_stale_bank_claims() {
        for stale_bank in ["zmm0", "ymm0", "xmm0"] {
            let mut cg = X86Codegen::new();
            cg.state.vec_live_regs.insert(7, stale_bank);
            cg.evex_load_arg_to(&Operand::Const(IrConst::I64(0)), "zmm0");
            let out = cg.state.out.buf.clone();
            assert!(
                !cg.state.vec_live_regs.contains_key(&7),
                "a stale {stale_bank} claim survived a fresh %zmm0 load: {:?}\n{out}",
                cg.state.vec_live_regs
            );
            assert!(
                out.contains("vmovdqu64 (%rax), %zmm0"),
                "the fallback load must still be emitted: {out}"
            );
        }
        // A claim on a DIFFERENT physical bank must survive: `vector_reg_bank`
        // keys on the register NUMBER, so xmm0/ymm0/zmm0 share bank 0 and
        // zmm3 is bank 3.
        let mut cg = X86Codegen::new();
        cg.state.vec_live_regs.insert(7, "zmm3");
        cg.evex_load_arg_to(&Operand::Const(IrConst::I64(0)), "zmm0");
        assert!(
            cg.state.vec_live_regs.get(&7) == Some(&"zmm3"),
            "an unrelated bank must not be evicted"
        );
    }
}

#[cfg(test)]
mod mask_cmp_global_mem_tests {
    use super::*;
    use crate::backend::state::{CodegenState, PendingVecMemfold};
    use crate::common::types::IrType;
    use crate::ir::reexports::{BasicBlock, BlockId, IrFunction, Terminator};

    /// A function whose Value(0) has exactly one `GlobalAddr` definition.
    /// `current_func` is a raw pointer kept alive by the returned IrFunction,
    /// so the fixture must outlive every use of the codegen object.
    fn func_with_global(name: &str, duplicate: bool) -> IrFunction {
        let mut f = IrFunction::new("g".into(), IrType::I64, vec![], false);
        let mut insts = vec![Instruction::GlobalAddr {
            dest: Value(0),
            name: name.into(),
        }];
        if duplicate {
            insts.push(Instruction::GlobalAddr {
                dest: Value(0),
                name: name.into(),
            });
        }
        f.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: insts,
            terminator: Terminator::Return(None),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        f.next_value_id = 4;
        f
    }

    fn cg_with(func: &IrFunction) -> X86Codegen {
        let mut cg = X86Codegen::new();
        cg.current_func = Some(func as *const IrFunction);
        cg
    }

    fn arg() -> Operand {
        Operand::Value(Value(0))
    }

    fn pending_memfold() -> PendingVecMemfold {
        PendingVecMemfold {
            val: 3,
            mem: "(%rdi)".into(),
            mnemonic: "vmovdqu",
            width: 32,
        }
    }

    #[test]
    fn accepts_only_a_unique_plain_global_address() {
        let f = func_with_global("A", false);
        let cg = cg_with(&f);
        assert_eq!(cg.mask_cmp_global_mem(&arg()), Some("A(%rip)".into()));
        // A constant operand has no address proof at all.
        assert_eq!(
            cg.mask_cmp_global_mem(&Operand::Const(IrConst::I64(0))),
            None
        );
    }

    /// Every gate below is a NEGATIVE test paired with the positive above.
    /// Each one corresponds to a way a compiler engineer could turn the
    /// optimization into a miscompile or a broken assembly operand.
    #[test]
    fn refuses_every_unsafe_or_uncommitted_shape() {
        // TLS is addressed through a descriptor, not a plain RIP-relative
        // data symbol; `A(%rip)` would read the wrong object.
        let f = func_with_global("A", false);
        let mut cg = cg_with(&f);
        cg.state.tls_symbols.insert("A".into());
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "TLS");

        // Full PIC / weak-extern data goes through the GOT.  A GOTPCREL
        // operand is a POINTER, not the object's bytes, so it cannot be an
        // EVEX src2.
        let mut cg = cg_with(&f);
        cg.state.pic_mode = true;
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "PIC/GOT");
        let mut cg = cg_with(&f);
        cg.state.pie_mode = true;
        cg.state.weak_extern_symbols.insert("A".into());
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "PIE weak extern");

        // Versioned symbol names carry an `@` that must not be emitted raw.
        let vf = func_with_global("A@GLIBC_2.2.5", false);
        let cg = cg_with(&vf);
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "versioned symbol");

        // Register-held values must go through the ordinary register path.
        let mut cg = cg_with(&f);
        cg.state.vec_live_regs.insert(0, "xmm0");
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "register-held");

        // ANY outstanding deferred write is invisible to the IR alias proof.
        let mut cg = cg_with(&f);
        cg.state.pending_vec_store = Some((3, "ymm0", true));
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "pending store");
        let mut cg = cg_with(&f);
        cg.state.pending_vec_memfold = Some(pending_memfold());
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "pending memfold");

        // An ambiguous (multi-def) value has no unique producer proof.
        let dup = func_with_global("A", true);
        let cg = cg_with(&dup);
        assert_eq!(cg.mask_cmp_global_mem(&arg()), None, "ambiguous definition");

        // A value that is not a GlobalAddr at all.
        let mut other = IrFunction::new("g".into(), IrType::I64, vec![], false);
        other.blocks.push(BasicBlock {
            label: BlockId(0),
            instructions: vec![Instruction::Copy {
                dest: Value(0),
                src: Operand::Const(IrConst::I64(0)),
            }],
            terminator: Terminator::Return(None),
            source_spans: vec![],
            source_inline_ids: Vec::new(),
        });
        other.next_value_id = 4;
        let cg = cg_with(&other);
        assert_eq!(
            cg.mask_cmp_global_mem(&arg()),
            None,
            "non-global definition"
        );
    }

    /// Width-exact UNALIGNED mnemonics: a 64-byte operand loaded through a
    /// 16-byte mnemonic would silently truncate.  `vzeroupper` obligation is
    /// armed only for 256/512, matching `avx_load_arg_to`.
    #[test]
    fn global_load_uses_width_exact_mnemonics_and_evicts_the_whole_bank() {
        for (reg, mnemonic, dirty) in [
            ("xmm0", "movdqu", false),
            ("ymm0", "vmovdqu", true),
            ("zmm0", "vmovdqu64", true),
        ] {
            let f = func_with_global("A", false);
            let mut cg = cg_with(&f);
            // Two stale claims on bank 0 and one unrelated claim on bank 3.
            cg.state.vec_live_regs.insert(1, "xmm0");
            cg.state.vec_live_regs.insert(2, "ymm0");
            cg.state.vec_live_regs.insert(3, "zmm3");
            assert!(cg.emit_mask_cmp_global_load(&arg(), reg), "{reg}");
            let out = &cg.state.out.buf;
            assert!(
                out.contains(&format!("{mnemonic} A(%rip), %{reg}")),
                "{reg}: {out}"
            );
            assert_eq!(cg.state.dirty_upper_ymm, dirty, "{reg} dirty_upper_ymm");
            assert_eq!(
                cg.state.vec_live_regs.len(),
                1,
                "{reg}: stale bank-0 claims survived the clobber"
            );
            assert_eq!(
                cg.state.vec_live_regs.get(&3),
                Some(&"zmm3"),
                "{reg}: unrelated bank was evicted"
            );
        }
    }

    /// The loader must be a pure predicate: on refusal it emits NOTHING, so
    /// the ordinary `simd_load_arg_to` fallback owns the register.
    #[test]
    fn global_load_refusal_emits_nothing() {
        let f = func_with_global("A", false);
        let mut cg = cg_with(&f);
        cg.state.tls_symbols.insert("A".into());
        assert!(!cg.emit_mask_cmp_global_load(&arg(), "xmm0"));
        assert!(cg.state.out.buf.is_empty(), "{}", cg.state.out.buf);
        assert!(!cg.state.dirty_upper_ymm);
    }

    /// `vec_evict_bank` must drop EVERY claim on the register's physical
    /// bank, and only that bank (no `u32::MAX` sentinel needed).
    #[test]
    fn bank_eviction_is_exact_and_needs_no_sentinel() {
        let mut state = CodegenState::new();
        state.vec_live_regs.insert(u32::MAX, "xmm0");
        state.vec_live_regs.insert(1, "ymm0");
        state.vec_live_regs.insert(2, "zmm0");
        state.vec_live_regs.insert(3, "zmm3");
        state.vec_evict_bank("zmm0");
        assert_eq!(state.vec_live_regs.len(), 1, "u32::MAX must NOT survive");
        assert_eq!(state.vec_live_regs.get(&3), Some(&"zmm3"));
    }
}

#[cfg(test)]
mod mask_compare_kmov_tests {
    use super::*;
    use crate::ir::intrinsics::IntrinsicOp as O;

    /// Exhaustive parity: every one of the 24 mask-compare opcodes at its
    /// three widths must select the narrowest move that still covers KL, and
    /// `mask_compare_vector_width` must agree with the emitter's register
    /// class.  A wrong lane count would truncate the returned mask, which the
    /// 485-case three-way oracle catches; this test pins the *reason*.
    #[test]
    fn every_opcode_and_width_selects_a_covering_kmov() {
        let ops = [
            O::CmpeqEpu8Mask128,
            O::CmpEpi8Mask128,
            O::CmpeqEpu8Mask256,
            O::CmpEpi8Mask256,
            O::CmpeqEpu8Mask512,
            O::CmpEpi8Mask512,
            O::CmpeqEpu16Mask512,
            O::CmpEpu16Mask128,
            O::CmpEpu16Mask256,
            O::CmpEpi16Mask512,
            O::CmpEpi16Mask128,
            O::CmpEpi16Mask256,
            O::CmpeqEpu32Mask512,
            O::CmpEpu32Mask128,
            O::CmpEpu32Mask256,
            O::CmpEpi32Mask512,
            O::CmpEpi32Mask128,
            O::CmpEpi32Mask256,
            O::CmpeqEpu64Mask512,
            O::CmpEpu64Mask128,
            O::CmpEpu64Mask256,
            O::CmpEpi64Mask512,
            O::CmpEpi64Mask128,
            O::CmpEpi64Mask256,
        ];
        for op in ops {
            let bytes = op
                .mask_compare_vector_width()
                .expect("every mask-compare opcode has a width");
            // Independently derive the mnemonic from the emitter's own table.
            let inst = match op {
                O::CmpeqEpu8Mask128
                | O::CmpeqEpu8Mask256
                | O::CmpeqEpu8Mask512
                | O::CmpEpu8Mask128
                | O::CmpEpu8Mask256
                | O::CmpEpu8Mask512 => "vpcmpub",
                O::CmpEpi8Mask128 | O::CmpEpi8Mask256 | O::CmpEpi8Mask512 => "vpcmpb",
                O::CmpeqEpu16Mask512 | O::CmpEpu16Mask128 | O::CmpEpu16Mask256 => "vpcmpuw",
                O::CmpEpi16Mask512 | O::CmpEpi16Mask128 | O::CmpEpi16Mask256 => "vpcmpw",
                O::CmpeqEpu32Mask512 | O::CmpEpu32Mask128 | O::CmpEpu32Mask256 => "vpcmpud",
                O::CmpEpi32Mask512 | O::CmpEpi32Mask128 | O::CmpEpi32Mask256 => "vpcmpd",
                O::CmpeqEpu64Mask512 | O::CmpEpu64Mask128 | O::CmpEpu64Mask256 => "vpcmpuq",
                O::CmpEpi64Mask512 | O::CmpEpi64Mask128 | O::CmpEpi64Mask256 => "vpcmpq",
                _ => unreachable!(),
            };
            let (kmov, dst) = X86Codegen::mask_compare_kmov(inst, bytes);
            let elem_bits: usize = match inst {
                "vpcmpub" | "vpcmpb" => 8,
                "vpcmpuw" | "vpcmpw" => 16,
                "vpcmpud" | "vpcmpd" => 32,
                _ => 64,
            };
            let lanes = bytes * 8 / elem_bits;
            let bits = match kmov {
                "kmovw" => 16,
                "kmovd" => 32,
                "kmovq" => 64,
                _ => panic!("unknown kmov {kmov}"),
            };
            assert!(bits >= lanes, "{op:?}: KL={lanes} does not fit in {kmov}");
            // Minimality: stepping DOWN one size must under-cover KL.
            assert!(
                bits == 16 || (bits >> 1) < lanes,
                "{op:?}: {kmov} is not the narrowest covering move"
            );
            assert_eq!(dst, if kmov == "kmovq" { "rax" } else { "eax" });
        }
    }

    /// Fail-closed unknowns and the exact 512-bit/8-bit `kmovq` boundary.
    #[test]
    fn unknown_or_widest_shapes_keep_the_64_bit_move() {
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpb", 64),
            ("kmovq", "rax")
        );
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpeqb", 64),
            ("kmovq", "rax")
        );
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpw", 64),
            ("kmovd", "eax")
        );
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpd", 64),
            ("kmovw", "eax")
        );
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpq", 32),
            ("kmovw", "eax")
        );
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpb", 32),
            ("kmovd", "eax")
        );
        assert_eq!(
            X86Codegen::mask_compare_kmov("vpcmpb", 16),
            ("kmovw", "eax")
        );
    }
}

/// SA-05 / SA-33: operand-role and upper-state contracts for the generic-SIMD
/// (512-bit EVEX) dispatcher.
///
/// Both defects were *table drift*, not algorithm bugs: the 512-bit arm
/// disagreed with its 128/256-bit twins about commutativity, and the
/// `vzeroupper` obligation was inferred from a `format!("{:?}")` substring
/// instead of the register actually written.  The tests below pin the emitted
/// assembly, so a future re-broadening of either rule fails here rather than
/// in a user's `vpmaddubsw` result or in an AVX->SSE transition penalty.
#[cfg(test)]
mod simd_operand_role_and_upper_state_tests {
    use super::*;
    use crate::backend::state::StackSlot;
    use crate::ir::reexports::IrConst;

    /// Values 0/1 are 512-bit vector values with real frame slots; `folded`
    /// selects which one is memory-foldable.  The other is claimed in a ZMM
    /// live register, which is exactly what makes `evex_arg_mem` refuse to
    /// fold it (its slot may be stale).  This drives the real fold decision,
    /// not a mocked predicate.
    fn codegen_with_one_foldable_arg(folded: u32) -> X86Codegen {
        let mut cg = X86Codegen::new();
        cg.state.vector_values.extend([0, 1, 2]);
        // The dispatcher skips a destination with zero uses as dead code, so
        // the fixture must give the result a real use or every assertion below
        // would be vacuous (empty output, nothing emitted).
        cg.state.value_use_counts = vec![1; 3];
        cg.state.value_locations.insert(0, StackSlot(-64));
        cg.state.value_locations.insert(1, StackSlot(-128));
        cg.state.value_locations.insert(2, StackSlot(-192));
        let reg_only = if folded == 0 { 1 } else { 0 };
        cg.state.vec_claim_live_reg(reg_only, "zmm5");
        // The claim itself arms the upper state; clear it so each assertion
        // observes only what the emitted op does.
        cg.state.dirty_upper_ymm = false;
        cg
    }

    /// `vpmaddubsw` reads vvvv as UNSIGNED bytes and r/m as SIGNED bytes, so
    /// IR args[0] must reach the register and only args[1] may be folded.
    ///
    /// With `commutative = true` the `(Some(m0), _)` arm fired for a foldable
    /// args[0] -- the ubiquitous
    /// `_mm512_maddubs_epi16(_mm512_loadu_si512(p), w)` shape -- and emitted
    /// `vpmaddubsw <a>, %zmm1, %zmm0`, feeding the unsigned stream to the
    /// signed multiplier.  Every lane product with a high bit set was wrong.
    #[test]
    fn maddubs512_never_folds_the_unsigned_stream_into_the_signed_slot() {
        for folded in [0u32, 1u32] {
            let mut cg = codegen_with_one_foldable_arg(folded);
            let mem0 = cg.slot_ref(-64);
            let mem1 = cg.slot_ref(-128);
            assert!(
                cg.emit_simd_op(
                    &None,
                    &IntrinsicOp::Pmaddubsw512,
                    &Some(Value(2)),
                    &[Operand::Value(Value(0)), Operand::Value(Value(1))],
                ),
                "Pmaddubsw512 must be handled by the SIMD dispatcher"
            );
            let out = cg.state.out.buf.clone();
            assert!(out.contains("vpmaddubsw"), "op not emitted: {out}");
            if folded == 0 {
                // args[0] (unsigned) MUST be materialized into the register.
                assert!(
                    !out.contains(&format!("vpmaddubsw {mem0},")),
                    "unsigned stream was folded into the SIGNED r/m slot: {out}"
                );
                assert!(
                    out.contains(&format!("vmovdqu64 {mem0}, %zmm0")),
                    "args[0] must be loaded into the vvvv register: {out}"
                );
            } else {
                // Folding the RIGHT (signed) operand is legal and must survive
                // -- the fix may not cost an instruction in the safe direction.
                assert!(
                    out.contains(&format!("vpmaddubsw {mem1}, %zmm0, %zmm0")),
                    "safe right-operand memory fold was lost: {out}"
                );
            }
        }
    }

    /// The commutative twin must KEEP folding both sides: `vpmaddwd` has two
    /// signed word sources, so the roles are symmetric.  Without this control
    /// a blanket `commutative = false` would pass the test above while
    /// pessimizing every genuinely commutative 512-bit op.
    #[test]
    fn maddwd512_still_folds_the_left_operand_because_it_is_symmetric() {
        let mut cg = codegen_with_one_foldable_arg(0);
        let mem0 = cg.slot_ref(-64);
        assert!(cg.emit_simd_op(
            &None,
            &IntrinsicOp::Pmaddwd512,
            &Some(Value(2)),
            &[Operand::Value(Value(0)), Operand::Value(Value(1))],
        ));
        let out = &cg.state.out.buf;
        assert!(
            out.contains(&format!("vpmaddwd {mem0}, %zmm1, %zmm0")),
            "a symmetric op must keep its left memory fold: {out}"
        );
    }

    /// SA-33: the digit-less wide EVEX ops write ymm/zmm and MUST arm the
    /// `vzeroupper` obligation.  Every one of these was missed by the old
    /// `format!("{:?}").contains("256" | "512")` proxy.
    #[test]
    fn digitless_wide_evex_ops_arm_vzeroupper() {
        let cases: &[(IntrinsicOp, &[Operand])] = &[
            (
                IntrinsicOp::InsertI32x8,
                &[
                    Operand::Value(Value(0)),
                    Operand::Value(Value(1)),
                    Operand::Const(IrConst::I64(1)),
                ],
            ),
            (
                IntrinsicOp::InsertI64x4,
                &[
                    Operand::Value(Value(0)),
                    Operand::Value(Value(1)),
                    Operand::Const(IrConst::I64(1)),
                ],
            ),
            (
                IntrinsicOp::PermutexvarEp32,
                &[Operand::Value(Value(0)), Operand::Value(Value(1))],
            ),
            (
                IntrinsicOp::PermutexvarEp64,
                &[Operand::Value(Value(0)), Operand::Value(Value(1))],
            ),
            (
                IntrinsicOp::ExtractI32x8,
                &[Operand::Value(Value(0)), Operand::Const(IrConst::I64(1))],
            ),
            (
                IntrinsicOp::ExtractI64x4,
                &[Operand::Value(Value(0)), Operand::Const(IrConst::I64(1))],
            ),
        ];
        for (op, args) in cases {
            let mut cg = codegen_with_one_foldable_arg(1);
            assert!(
                cg.emit_simd_op(&None, op, &Some(Value(2)), args),
                "{op:?} must be handled by the SIMD dispatcher"
            );
            assert!(
                cg.state.dirty_upper_ymm,
                "{op:?} writes a wide register but did not arm vzeroupper: {}",
                cg.state.out.buf
            );
        }
    }

    /// The rule must stay EXACT, not conservative-by-name: an xmm claim writes
    /// only the low 128 bits (VEX/EVEX zeroes above), so it must NOT arm.
    /// Arming on every claim would emit a pointless `vzeroupper` in pure-SSE
    /// functions and hide the real regression above.
    #[test]
    fn narrow_register_claims_do_not_arm_vzeroupper() {
        let mut cg = X86Codegen::new();
        assert!(!cg.state.dirty_upper_ymm);
        cg.state.vec_claim_live_reg(0, "xmm3");
        assert!(
            !cg.state.dirty_upper_ymm,
            "an xmm home cannot dirty bits above 128"
        );
        // ... while the ymm and zmm views of the SAME bank must.
        for wide in ["ymm3", "zmm3"] {
            cg.state.dirty_upper_ymm = false;
            cg.state.vec_claim_live_reg(0, wide);
            assert!(cg.state.dirty_upper_ymm, "{wide} must arm vzeroupper");
        }
    }

    /// A slot-homed vector value: the shape `vec_arg_mem` folds and every
    /// loader can reload from. `-32(%rbp)` is the same fixture the fold tests
    /// use, so these cases drive the real emitter rather than a stub.
    ///
    /// `value_use_counts` must cover BOTH ids: the dispatcher treats a
    /// destination with zero uses as dead code and emits nothing, which would
    /// make every assertion below vacuous (an empty `out`), and the loader
    /// needs a materialised operand. Ids 0 (source) and 9 (destination) are
    /// the ones these tests use.
    fn slot_vector(cg: &mut X86Codegen, id: u32) {
        cg.state.value_locations.insert(id, StackSlot(-32));
        cg.state.vector_values.insert(id);
        cg.state.value_use_counts = vec![1; 10];
    }

    /// `vec_live_regs` is WIDTH-POLYGLOT: one map, three spellings (`%xmmN`
    /// from a 128-bit producer, `%ymmN` from 256, `%zmmN` from EVEX). The EVEX
    /// loader writes `%zmm{target}`, so it may only consume a `%zmm` claim. A
    /// raw read here spliced `%xmm3`/`%ymm3` into a `zmm0` operand:
    /// `vmovdqa64 %xmm3, %zmm0` mixes widths, which is an illegal encoding the
    /// assembler rejects outright -- a valid program failed to build.
    ///
    /// The `%zmm` case is the control: the filtered path must still take the
    /// register and still move it aside, or the de-facto fix would be "never
    /// reuse a claim here" and every EVEX operand would pay a reload.
    #[test]
    fn evex_load_arg_to_ignores_claims_of_the_wrong_width() {
        for wrong in ["xmm3", "ymm3"] {
            let mut cg = X86Codegen::new();
            slot_vector(&mut cg, 7);
            cg.state.vec_live_regs.insert(7, wrong);
            cg.evex_load_arg_to(&Operand::Value(Value(7)), "zmm0");
            let out = cg.state.out.buf.clone();
            assert!(
                !out.contains("vmovdqa64"),
                "a {wrong} claim must not be moved into a zmm operand: the \
                 widths would not match: {out}"
            );
            assert!(
                out.contains("vmovdqu64 -32(%rbp), %zmm0"),
                "the width-matched slot load must run instead: {out}"
            );
        }
        // Control: a `%zmm` claim takes the register path, with no reload.
        let mut cg = X86Codegen::new();
        slot_vector(&mut cg, 7);
        cg.state.vec_live_regs.insert(7, "zmm3");
        cg.evex_load_arg_to(&Operand::Value(Value(7)), "zmm0");
        let out = cg.state.out.buf.clone();
        assert!(
            out.contains("vmovdqa64 %zmm3, %zmm0"),
            "a width-matched claim must still be reused: {out}"
        );
        assert!(
            !out.contains("vmovdqu64"),
            "and must not be reloaded: {out}"
        );
    }

    /// The VEX.256 broadcast reads a `%ymm` source operand. `vec_live_regs`
    /// also carries `%xmm` (128-bit producers) and `%zmm` (EVEX producers)
    /// claims, neither legal in a VEX.256 source: `vbroadcastss %xmm3, %ymm0`
    /// mixes widths and `%zmm3` is not encodable in a VEX instruction at all.
    /// The loader read the raw map, so a value homed at 128 bits by an earlier
    /// `VecAddF32x4` turned a valid broadcast into an assembler error.
    ///
    /// The control pins the other direction: a `%ymm` claim must still be
    /// consumed, or the de-facto fix would be "never use the claim here" and
    /// every broadcast after a 256-bit producer would reload.
    #[test]
    fn avx_broadcast_ignores_claims_of_the_wrong_width() {
        for wrong in ["xmm3", "zmm3"] {
            let mut cg = X86Codegen::new();
            slot_vector(&mut cg, 0);
            // The result needs a home of its own: `avx_store_dest` resolves the
            // destination's slot or register, and asserts (debug builds) when
            // it has neither. Value 9 is the destination in this fixture.
            slot_vector(&mut cg, 9);
            cg.state.vec_live_regs.insert(0, wrong);
            let handled = cg.emit_simd_op(
                &Some(Value(9)),
                &IntrinsicOp::Vbroadcastss,
                &Some(Value(9)),
                &[Operand::Value(Value(0))],
            );
            let out = cg.state.out.buf.clone();
            assert!(handled, "the Vbroadcastss arm must handle the op");
            assert!(
                !out.contains(&format!("vbroadcastss %{wrong}, %ymm0")),
                "a {wrong} claim must not feed a VEX.256 source operand: {out}"
            );
            // Either legitimate fallback is acceptable, and the emitter picks
            // the better one: the MEMORY FOLD (`vbroadcastss -32(%rbp), %ymm0`,
            // one instruction, no staging) when the value is slot-homed, or the
            // staging load (`%xmm1`) when it is not. Pinning only the wrong
            // register is the invariant; pinning which fallback runs would
            // forbid an improvement.
            assert!(
                out.contains("vbroadcastss -32(%rbp), %ymm0")
                    || out.contains("vbroadcastss %xmm1, %ymm0"),
                "a wrong-width claim must fall back to the slot (fold or \
                 staging load): {out}"
            );
        }
        // Control: an `%ymm` claim is exactly the right width and is used.
        let mut cg = X86Codegen::new();
        slot_vector(&mut cg, 0);
        slot_vector(&mut cg, 9);
        cg.state.vec_live_regs.insert(0, "ymm3");
        let handled = cg.emit_simd_op(
            &Some(Value(9)),
            &IntrinsicOp::Vbroadcastss,
            &Some(Value(9)),
            &[Operand::Value(Value(0))],
        );
        let out = cg.state.out.buf.clone();
        assert!(handled, "the Vbroadcastss arm must handle the op");
        assert!(
            out.contains("vbroadcastss %ymm3, %ymm0"),
            "a width-matched claim must be reused: {out}"
        );
    }
}

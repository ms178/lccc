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
use crate::ir::reexports::{IntrinsicOp, IrConst, Operand, Value};

impl X86Codegen {
    /// Load a 512-bit operand into a ZMM register.
    /// Honors vec_live_regs (register-held values) and direct slot addressing.
    fn evex_load_arg_to(&mut self, arg: &Operand, zmm: &'static str) {
        if let Operand::Value(v) = arg {
            if let Some(&held) = self.state.vec_live_regs.get(&v.0) {
                if held != zmm {
                    self.state
                        .emit_fmt(format_args!("    vmovdqa64 %{}, %{}", held, zmm));
                }
                return;
            }
            if let Some(mem) = self.value_ptr_mem_operand(v.0) {
                self.state
                    .emit_fmt(format_args!("    vmovdqu64 {}, %{}", mem, zmm));
                return;
            }
        }
        self.operand_to_reg(arg, "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu64 (%rax), %{}", zmm));
    }

    fn evex_load_arg(&mut self, arg: &Operand) {
        self.evex_load_arg_to(arg, "zmm0");
    }

    /// Width-dispatching vector load: picks the 16/32/64-byte loader from the
    /// target register name. NEVER load a 64-byte value into xmm/ymm scratch
    /// (overreads the source slot).
    fn simd_load_arg_to(&mut self, arg: &Operand, reg: &'static str) {
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
        self.state.vec_live_regs.insert(dest_ptr.0, "zmm0");
    }

    /// Whether arg[0] may be folded as a memory operand (never if the value is
    /// still provably in a register — its slot may be stale).
    fn evex_arg_mem(&self, arg: &Operand) -> Option<String> {
        match arg {
            Operand::Value(v) => {
                if self.state.vec_live_regs.contains_key(&v.0) {
                    return None;
                }
                self.operand_ptr_mem_operand(arg)
            }
            _ => None,
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
        self.state.vec_live_regs.insert(dest_ptr.0, r0);
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
        self.state.vec_live_regs.insert(dest_ptr.0, r0);
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
        // IS-20: 256/512-bit vector ops dirty the upper YMM halves; the
        // epilogue will emit vzeroupper before ret.
        {
            let name = format!("{:?}", op);
            if name.contains("256") || name.contains("512") || name.ends_with("Si256") {
                self.state.dirty_upper_ymm = true;
            }
        }
        let Some(dptr) = dest_ptr else {
            // Scalar-result ops use `dest` (lowering already produced the
            // real operands: dummy/imm dropped, imm re-appended last).
            return self.emit_simd_scalar(dest, op, args);
        };
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
                self.emit_evex_binary_512(dptr, args, "vpmaddubsw", true);
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
                self.emit_cast128_512(dptr, args);
                true
            }
            // ---- loads/stores ----
            Loadu512 => {
                self.emit_evex_load_512(dptr, args);
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
                self.emit_evex_unary_512(dptr, args, "vcvtps2pd");
                true
            }
            CvtPd2Ps512 => {
                self.emit_evex_unary_512(dptr, args, "vcvtpd2ps");
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
                self.emit_evex_unary_512(dptr, args, "vcvtdq2pd");
                true
            }
            CvtPd2Ep32_512 => {
                self.emit_evex_unary_512(dptr, args, "vcvtpd2dq");
                true
            }
            CvttPd2Ep32_512 => {
                self.emit_evex_unary_512(dptr, args, "vcvttpd2dq");
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
            DivPs128 | MinPs128 | MaxPs128 | DivPd128 | MinPd128 | MaxPd128 => {
                let (inst, is_ps) = match op {
                    DivPs128 => ("divps", true),
                    MinPs128 => ("minps", true),
                    MaxPs128 => ("maxps", true),
                    DivPd128 => ("divpd", false),
                    MinPd128 => ("minpd", false),
                    MaxPd128 => ("maxpd", false),
                    _ => unreachable!(),
                };
                self.emit_sse_fp_128(dptr, args, inst, is_ps);
                true
            }
            SqrtPs128 | RcpPs128 | RsqrtPs128 | SqrtPd128 => {
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
            | VpermilPs256 | Vperm2f128 | Vinsertf128 | Vextractf128 | Vbroadcastss
            | Vbroadcastsd | CvtPs2Ep32_256 | CvtEp32ToPs256 | CvttPs2Ep32_256 | CvtPs2Pd256
            | CvtPd2Ps256 | CvtPd2Ep32_256 | CvtEp32ToPd256 | CvttPd2Ep32_256 | VpermilvarPs256
            | VpermilvarPd256 | FmaPs132v256 | FmaPs213v256 | FmaPs231v256 | FmaPd132v256
            | FmaPd213v256 | FmaPd231v256 => {
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
        self.state.vec_live_regs.insert(dest_ptr.0, dst_reg);
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

    /// Zero-extend a 128-bit value into 512: vpxord + vinserti32x4 $0.
    fn emit_zext128_512(&mut self, dest_ptr: &Value, args: &[Operand]) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.state.emit("    vpxord %zmm0, %zmm0, %zmm0");
        if let Some(&held) = self.state.vec_live_regs.get(&simd_value_id(&args[0])) {
            self.state
                .emit_fmt(format_args!("    vinserti32x4 $0, %{}, %zmm0, %zmm0", held));
        } else if let Some(m) = self.evex_arg_mem(&args[0]) {
            self.state
                .emit_fmt(format_args!("    vinserti32x4 $0, {}, %zmm0, %zmm0", m));
        } else {
            self.operand_to_reg(&args[0], "rax");
            self.state.emit("    vmovdqu (%rax), %xmm1");
            self.state.emit("    vinserti32x4 $0, %xmm1, %zmm0, %zmm0");
        }
        self.evex_store_dest(dest_ptr);
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
        self.state.vec_live_regs.insert(dest_ptr.0, "ymm0");
    }

    /// 128 -> 512 cast (register move; upper bits undefined like GCC).
    fn emit_cast128_512(&mut self, dest_ptr: &Value, args: &[Operand]) {
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        if let Some(&held) = self.state.vec_live_regs.get(&simd_value_id(&args[0])) {
            if held != "zmm0" {
                self.state
                    .emit_fmt(format_args!("    vmovdqu64 %{}, %zmm0", held));
            }
        } else if let Some(m) = self.evex_arg_mem(&args[0]) {
            self.state
                .emit_fmt(format_args!("    vmovdqu64 {}, %zmm0", m));
        } else {
            self.operand_to_reg(&args[0], "rax");
            self.state.emit("    vmovdqu (%rax), %xmm0");
        }
        self.evex_store_dest(dest_ptr);
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
            .emit_fmt(format_args!("    vmovdqu8 (%rax), %{}{{k1}}{{z}}", reg));
        self.state.vec_live_regs.insert(dest_ptr.0, reg);
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
        self.evex_load_arg_to(&args[2], reg);
        self.simd_gpr_to_k1(&args[0]);
        self.operand_to_reg(&args[1], "rax");
        self.state
            .emit_fmt(format_args!("    vmovdqu8 (%rax), %{}{{k1}}", reg));
        self.state.vec_live_regs.insert(dest_ptr.0, reg);
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
            .emit_fmt(format_args!("    vmovdqu8 %{}, (%rax){{k1}}", reg));
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
            .emit_fmt(format_args!("    {} %{}, %zmm0{{k1}}{{z}}", inst, reg));
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
            "    {} ${}, %{}, %zmm1, %zmm0{{k1}}{{z}}",
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
            "    {} ${}, %zmm0, %{}{{k1}}{{z}}",
            inst, imm, dst_reg
        ));
        self.state.vec_live_regs.insert(dest_ptr.0, dst_reg);
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
                "    vpshufb %xmm2, %xmm1, %xmm0{{k1}}{}",
                zsuf
            ));
        } else {
            // merge: dest = src (first arg) merged with shuffle result
            self.evex_load_arg_to(&args[1], "xmm0"); // src (dest)
            self.simd_gpr_to_k1(mask);
            self.evex_load_arg_to(&args[2], "xmm1"); // a
            self.evex_load_arg_to(&args[3], "xmm2"); // b
            self.state
                .emit_fmt(format_args!("    vpshufb %xmm2, %xmm1, %xmm0{{k1}}"));
        }
        self.state.vec_live_regs.insert(dest_ptr.0, "xmm0");
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
                // args: (a, b, imm) — vpcmp* $imm, %zmm_b, %zmm_a, %k1; kmovq %k1, %rax
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
                self.simd_load_arg_to(&args[0], reg_a);
                self.simd_load_arg_to(&args[1], reg_b);
                let imm = if args.len() > 2 {
                    self.simd_imm(&args[2])
                } else {
                    0
                };
                self.state.emit_fmt(format_args!(
                    "    {} ${}, %{}, %{}, %k1",
                    inst, imm, reg_b, reg_a
                ));
                self.state.emit("    kmovq %k1, %rax");
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
            CvtSs2Si128 | CvtSd2Si128 => {
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
        // Legacy SSE arithmetic with a MEMORY operand requires 16-byte
        // alignment (unlike VEX). Slots are only 8-aligned, so memory
        // operands must NEVER be folded here — always load via movups.
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
        self.sse_load_arg(&args[0], "xmm0");
        self.sse_load_arg(&args[1], "xmm1");
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
        self.sse_store_dest(dest_ptr, "xmm0");
    }

    /// 128-bit SSE FP op dispatch (all shapes).
    fn emit_sse_fp_128_op(&mut self, dest_ptr: &Value, op: &IntrinsicOp, args: &[Operand]) {
        use IntrinsicOp::*;
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
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
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
                self.state
                    .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                self.sse_store_dest(dest_ptr, "xmm0");
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
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
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
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
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
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
                self.state
                    .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                self.sse_store_dest(dest_ptr, "xmm0");
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
                self.sse_load_arg(&args[0], "xmm0");
                self.sse_load_arg(&args[1], "xmm1");
                self.state
                    .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                self.sse_store_dest(dest_ptr, "xmm0");
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
        self.flush_pending_vec_store_impl();
        self.state.invalidate_vec_peephole();
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
        match op {
            DivPs256 => binary(self, "vdivps", false),
            MinPs256 => binary(self, "vminps", true),
            MaxPs256 => binary(self, "vmaxps", true),
            DivPd256 => binary(self, "vdivpd", false),
            MinPd256 => binary(self, "vminpd", true),
            MaxPd256 => binary(self, "vmaxpd", true),
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
            CvtPs2Pd256 => unary(self, "vcvtps2pd"),
            CvtPd2Ps256 => unary(self, "vcvtpd2ps"),
            CvtPd2Ep32_256 => unary(self, "vcvtpd2dq"),
            CvtEp32ToPd256 => unary(self, "vcvtdq2pd"),
            CvttPd2Ep32_256 => unary(self, "vcvttpd2dq"),
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
            RoundPs256 | RoundPd256 | VpermilPs256 => {
                let inst = match op {
                    RoundPs256 => "vroundps",
                    RoundPd256 => "vroundpd",
                    VpermilPs256 => "vpermilps",
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
                self.state.vec_live_regs.insert(dest_ptr.0, "xmm0");
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
                if let Some(&held) = self.state.vec_live_regs.get(&simd_value_id(&args[0])) {
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
